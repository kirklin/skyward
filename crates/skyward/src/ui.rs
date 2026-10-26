use crate::backup;
use crate::decrypt::{self, Decryptor};
use crate::filework;
use crate::haptics::Haptics;
use crate::relabel;
use crate::input::{monotonic_ms, Event, Inputs};
use crate::merge;
use crate::partwork;
use crate::pty;
use crate::readings::{Reading, Readings};
use crate::secure;
use crate::services;
use crate::updates;
use crate::wipe::{self, Eraser, Step};
use crate::wlan;
use anyhow::{anyhow, ensure, Context, Result};
use skyward_block::fsck::Fixed;
use skyward_block::{fstab, mount, probe};
use skyward_config::{DeviceConfig, Service};
use skyward_crypto::data::{Closed, Opened};
use skyward_install::asked::{self, Needs};
use skyward_install::{Detach, Installer, Options};
use skyward_sys::bcb::{self, Work, REFORMATS};
use skyward_sys::control::{Control, Request};
use skyward_sys::Power;
use skyward_files::Ownership;
use skyward_block::usb::{self, Drive};
use skyward_net::download::{self, Failure};
use skyward_net::http::Servers;
use skyward_net::release::{Installed, Release, Unanswered};
use skyward_net::wlan::{Power as WlanPower, Wlan};
use skyward_ui::app::{
    App, Asked, BackupAt, BackupEnd, BackupEntry, BackupRefusal, ClashChoice, Data, DownloadEnd, Effect, EraseStop, FileKind, FileOp,
    FileRow, FileTask, FsAction, FsRow, Have, Held, ItemEnd, ItemState, JobEnd, Keys, MountFailure, Part, PartKind, PartStop, Place,
    RenameRefused, Screen, Shot, Target, UsbDrive, UsbLink, Wipe, DATA_OPENING, DATA_PARTITION, DOWNLOADS, INSTALL_UNFINISHED, USB_DRIVE,
};
use skyward_ui::surface::kms::KmsSurface;
use skyward_ui::{Canvas, Fonts, Lang, Surface};
use skyward_vab::{Merge, Standing};
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long init may take to hand the USB port to another function: the
/// program behind the new one has to start and get ready first.
const USB_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the page shows that the work the system asked for is done
/// before the phone starts the system or powers off.
const LEAVE_AFTER_MS: u64 = 1500;

/// How long after the screen starts a touch panel may still come up; with
/// none by then, the pages are worked with the keys, their light shown.
const TOUCH_WAIT: Duration = Duration::from_secs(5);

/// Where logs saved to the temporary folder go, and where in internal
/// storage the others do.
const TMP_LOGS: &str = "/tmp/skyward-logs";
const STORAGE_LOGS: &str = "Skyward/logs";

/// The folder of internal storage the system saves its own screenshots in,
/// which the gallery shows as its screenshots, and where a screenshot waits
/// for internal storage to open.
const SCREENSHOTS: &str = "DCIM/Screenshots";
const HELD_SCREENSHOTS: &str = "/tmp/screenshots";

/// The panel backlight under /sys/class/backlight.
struct Backlight {
    path: PathBuf,
    /// The raw level at the top of the slider: the panel's normal maximum.
    top: u32,
    /// The raw level last written; at first the one the bootloader left.
    level: u32,
}

impl Backlight {
    fn open() -> Result<Self> {
        let mut dirs: Vec<PathBuf> = fs::read_dir("/sys/class/backlight")
            .context("read /sys/class/backlight")?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        dirs.sort();
        // Qualcomm panels expose panel0-backlight; take it when present.
        let dir = dirs
            .iter()
            .find(|d| d.file_name().is_some_and(|n| n == "panel0-backlight"))
            .or(dirs.first())
            .ok_or_else(|| anyhow!("no backlight device"))?
            .clone();
        let read = |name: &str| -> Result<u32> {
            fs::read_to_string(dir.join(name))
                .with_context(|| format!("read {}/{name}", dir.display()))?
                .trim()
                .parse()
                .with_context(|| format!("parse {name}"))
        };
        let top = normal_max(read("max_brightness")?)?;
        let level = read("brightness")?;
        Ok(Backlight { path: dir.join("brightness"), top, level })
    }

    /// Light the panel at `level` of its normal maximum, or turn it off
    /// with `None`. The file is written only when the raw level changes.
    fn show(&mut self, level: Option<f32>) -> Result<()> {
        let raw = level.map_or(0, |v| ((v.clamp(0.0, 1.0) * self.top as f32).round() as u32).max(1));
        if raw != self.level {
            fs::write(&self.path, raw.to_string()).with_context(|| format!("write {}", self.path.display()))?;
            eprintln!("backlight {raw}");
            self.level = raw;
        }
        Ok(())
    }
}

/// How a panel's node in the device tree may give its normal maximum: the
/// property holding it, and the one holding the whole range on the same
/// scale. The 13 Ultra's panel gives both on a finer scale of its own that
/// spans the same light as the backlight's range; the 17 Ultra's gives them
/// on the backlight's own scale. The panels of the Xiaomi 13 and 15, of the
/// 14 and the K80 series and one of the K90's give it only as the level
/// their factory tests drive the panel to, on the backlight's own scale:
/// the 13 Ultra's panels give that level as well, and it comes to the same
/// half of their range as the normal maximum they give.
const NORMAL_MAX: [(&str, &str); 3] = [
    ("mi,normal-max-brightness-clone", "mi,max-brightness-clone"),
    ("mi,max-brightness-normal", "qcom,mdss-dsi-bl-max-level"),
    ("mi,mdss-dsi-fac-bl-max-level", "qcom,mdss-dsi-bl-max-level"),
];

/// The raw backlight level, out of `max`, at the panel's normal maximum.
/// Above it the panel runs in high brightness mode, which the stock system
/// keeps for strong sunlight and its slider does not reach. The panel the
/// display driver booted with gives that maximum in the device tree.
fn normal_max(max: u32) -> Result<u32> {
    let param = "/sys/module/msm_drm/parameters/dsi_display0";
    let boot = fs::read_to_string(param).with_context(|| format!("read {param}"))?;
    // The panel's node, then its configuration after a colon, as in
    // "qcom,mdss_dsi_m1_42_02_0a_dsc_cmd:"; the driver takes no name
    // without the colon either.
    let (name, _) = boot.trim().split_once(':').with_context(|| format!("{param} names no panel: {boot:?}"))?;
    let soc = Path::new("/proc/device-tree/soc");
    let node = fs::read_dir(soc)
        .with_context(|| format!("read {}", soc.display()))?
        .map(|e| e.map(|e| e.path().join(name)))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .find(|p| p.is_dir())
        .ok_or_else(|| anyhow!("panel {name} is not in the device tree"))?;
    let cell = |prop: &str| -> Result<u32> {
        let path = node.join(prop);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let cell: [u8; 4] = bytes.as_slice().try_into().map_err(|_| anyhow!("{} is not one cell", path.display()))?;
        Ok(u32::from_be_bytes(cell))
    };
    let (normal, full) = NORMAL_MAX
        .iter()
        .find(|(normal, _)| node.join(normal).exists())
        .with_context(|| format!("panel {name} gives its normal maximum as none of {NORMAL_MAX:?}"))?;
    let (normal, full) = (cell(normal)?, cell(full)?);
    ensure!(normal > 0 && normal <= full, "panel {name}: normal maximum {normal} out of {full}");
    Ok((f64::from(max) * f64::from(normal) / f64::from(full)).round() as u32)
}

/// Leave recovery for the chosen target.
fn power(target: Target) -> Result<()> {
    skyward_sys::power(match target {
        Target::System => Power::System,
        Target::Recovery => Power::Recovery,
        Target::Fastboot => Power::Fastboot,
        Target::Bootloader => Power::Bootloader,
        Target::Edl => Power::Edl,
        Target::PowerOff => Power::Off,
    })
}

/// File transfer: the service offering internal storage to a computer over
/// USB, whether it offers the storage now, and the zone, by its place in
/// the settings, it tells the times of files in.
struct Transfer {
    service: skyward_mtp::Service,
    offered: bool,
    zone: usize,
}

impl Transfer {
    /// Serve file transfer; the port offers it from when it next rests.
    fn start(cfg: &DeviceConfig, info: &skyward_info::DeviceInfo, zone: usize) -> Result<Transfer> {
        let device = skyward_mtp::Device {
            manufacturer: cfg.device.brand.clone(),
            model: info.model.clone(),
            version: format!("Skyward {}", skyward_info::VERSION),
            serial: skyward_sys::prop("ro.serialno").unwrap_or_default(),
            name: info.model.clone(),
            battery: Box::new(|| skyward_info::battery_now().ok().flatten().map(|b| b.level)),
        };
        let service = skyward_mtp::Service::start(device, skyward_ui::fmt::rules(zone).clone())?;
        skyward_sys::usb::set_mtp(true);
        Ok(Transfer { service, offered: false, zone })
    }

    /// Offer internal storage, named for the computer in `lang`, or take it
    /// back. Once this returns, nothing the computer had under way in it
    /// goes on.
    fn offer(&mut self, offer: bool, lang: Lang) -> Result<()> {
        if offer != self.offered {
            let description = skyward_ui::fmt::storage_name(lang).into();
            let storage = offer.then(|| skyward_mtp::Storage { top: PathBuf::from(skyward_crypto::data::SDCARD), description });
            self.service.offer(storage)?;
            self.offered = offer;
        }
        Ok(())
    }

    /// Stop serving: a port at rest goes back to adb alone first.
    fn stop(self, rests: bool) -> Result<()> {
        skyward_sys::usb::set_mtp(false);
        let switched = if rests { skyward_sys::usb::switch("adb", USB_TIMEOUT) } else { Ok(()) };
        let stopped = self.service.stop();
        switched.and(stopped)
    }
}

/// Take internal storage back from file transfer, if it is on, before work
/// that erases or rewrites the data partition, or installs; whether that
/// work can start, which the page is told otherwise.
fn withhold(transfer: &mut Option<Transfer>, app: &mut App, now: u64) -> Result<bool> {
    let Some(t) = transfer else { return Ok(true) };
    match t.offer(false, app.state.lang) {
        Ok(()) => Ok(true),
        Err(e) => {
            skyward_sys::kmsg(&format!("file transfer kept internal storage: {e:#}"))?;
            app.report(&skyward_ui::fmt::transfer_kept(app.state.lang, &format!("{e:#}")), now);
            Ok(false)
        }
    }
}

/// Whether the phone's fastbootd runs, as init tells it, and how far a
/// computer has the port, as the USB controller tells it.
fn fastboot_status() -> Result<(bool, UsbLink)> {
    let running = skyward_sys::prop("init.svc.fastbootd").as_deref() == Some("running");
    let controller = skyward_sys::prop("sys.usb.controller").context("init names no USB controller")?;
    let path = Path::new("/sys/class/udc").join(controller).join("state");
    let state = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let link = match state.trim() {
        "configured" => UsbLink::Connected,
        "not attached" => UsbLink::Unplugged,
        _ => UsbLink::Connecting,
    };
    Ok((running, link))
}

/// What a request from misc shows on the pages that answer it.
fn asked(request: &bcb::Request) -> Asked {
    Asked {
        reason: request.reason.clone(),
        shutdown_after: request.shutdown_after,
        unhandled: match &request.work {
            Work::Unsupported(what) => Some(what.clone()),
            _ => None,
        },
    }
}

/// Read a folder for the file manager, as it is now.
fn list(app: &mut App, folder: String) -> Result<()> {
    let files = read_folder(Path::new(&folder)).map_err(|e| format!("{e:#}"));
    app.set_files(folder, files, unix_now()?);
    Ok(())
}

/// Read a folder of `place` to choose a file to install in, as it is now;
/// `folder` is its path from the place's top.
fn list_place(app: &mut App, place: Place, folder: String) -> Result<()> {
    let files = read_folder(&in_place(place, &folder)).map_err(|e| format!("{e:#}"));
    app.set_place(place, folder, files, unix_now()?);
    Ok(())
}

/// Where `path`, from the top of `place`, is: internal storage is the
/// owner's shared storage, where the data partition shows it.
fn in_place(place: Place, path: &str) -> PathBuf {
    let top = match place {
        Place::Storage => Path::new(skyward_crypto::data::SDCARD),
        Place::UsbDrive => Path::new(USB_DRIVE),
    };
    if path.is_empty() {
        top.to_path_buf()
    } else {
        top.join(path)
    }
}

/// Save the logs chosen into a folder of their own in `into`, named by when
/// they were saved in the zone the settings chose: the recovery's own, with
/// what update_engine wrote while it last installed an update and what the
/// last install script wrote, and the kernel's. Each file is owned as a
/// copy into the folder would be. Gives the folder.
fn export_logs(recovery: bool, kernel: bool, into: &Path, zone: usize) -> Result<PathBuf> {
    let there = into.ancestors().find(|p| p.exists()).context("the root is always there")?;
    let ownership = ownership(there)?;
    let folder = into.join(skyward_ui::fmt::stamp(unix_now()?, zone));
    skyward_files::make_folders(&folder, ownership)?;
    let save = |name: &str, contents: &[u8]| -> Result<()> {
        let path = folder.join(name);
        let mut file = skyward_files::make_file(&path, ownership)?;
        file.write_all(contents).with_context(|| format!("write {}", path.display()))?;
        file.sync_all().with_context(|| format!("write {}", path.display()))
    };
    if recovery {
        save("recovery.log", &fs::read(skyward_sys::LOG).with_context(|| format!("read {}", skyward_sys::LOG))?)?;
        for (name, path) in [("update_engine.log", skyward_install::ENGINE_LOG), ("install-script.log", skyward_install::SCRIPT_LOG)] {
            match fs::read(path) {
                Ok(log) => save(name, &log)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("read {path}")),
            }
        }
    }
    if kernel {
        save("kernel.log", skyward_sys::kernel_log()?.as_bytes())?;
    }
    Ok(folder)
}

/// Save `frame`, what the panel shows, as a picture named for when it was
/// taken: into the gallery's screenshots where internal storage is open, as
/// `storage` says, and otherwise into the recovery's own memory until it
/// opens. Each is made as the system makes what it saves there.
fn save_screenshot(frame: &Canvas, storage: bool, zone: usize) -> Result<Shot> {
    let (folder, shot) = match storage {
        true => (Path::new(skyward_crypto::data::SDCARD).join(SCREENSHOTS), Shot::Album),
        false => (PathBuf::from(HELD_SCREENSHOTS), Shot::Held),
    };
    let there = folder.ancestors().find(|p| p.exists()).context("the root is always there")?;
    let ownership = ownership(there)?;
    skyward_files::make_folders(&folder, ownership)?;
    let path = folder.join(skyward_ui::fmt::screenshot_name(unix_ms_now()?, zone));
    let file = skyward_files::make_file(&path, ownership)?;
    let mut out = std::io::BufWriter::new(&file);
    frame.write_png(&mut out).with_context(|| format!("write {}", path.display()))?;
    out.flush().with_context(|| format!("write {}", path.display()))?;
    drop(out);
    file.sync_all().with_context(|| format!("write {}", path.display()))?;
    skyward_sys::kmsg(&format!("screenshot saved as {}", path.display()))?;
    Ok(shot)
}

/// Move the screenshots held while internal storage was closed into the
/// gallery's, now that it is open: each keeps its name, and is made there
/// as the system makes what it saves.
fn keep_held_screenshots() -> Result<()> {
    let held = Path::new(HELD_SCREENSHOTS);
    let shots = match fs::read_dir(held) {
        Ok(entries) => entries.map(|e| e.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>().with_context(|| format!("read {}", held.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("read {}", held.display())),
    };
    if shots.is_empty() {
        return Ok(());
    }
    let album = Path::new(skyward_crypto::data::SDCARD).join(SCREENSHOTS);
    let there = album.ancestors().find(|p| p.exists()).context("the root is always there")?;
    let ownership = ownership(there)?;
    skyward_files::make_folders(&album, ownership)?;
    let stop = std::sync::atomic::AtomicBool::new(false);
    skyward_files::move_into(&shots, &album, skyward_files::Clash::KeepBoth, ownership, &stop, &mut |_| {})?;
    skyward_sys::kmsg(&format!("{} screenshots held in {} moved into {}", shots.len(), held.display(), album.display()))
}

/// The folder backups go in and are found in, in `place`.
fn backups_in(place: Place) -> PathBuf {
    in_place(place, skyward_backup::FOLDER)
}

/// The backups in `place`: each folder in its backups folder, with what its
/// manifest records or why that cannot be read. A place with no backups
/// folder has none.
fn backups_at(place: Place) -> Result<Vec<BackupEntry>> {
    let folder = backups_in(place);
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", folder.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type().with_context(|| format!("look at {}", entry.path().display()))?.is_dir() {
            continue;
        }
        let held = skyward_backup::Manifest::read(&entry.path()).map(|m| held(&m)).map_err(|e| format!("{e:#}"));
        found.push(BackupEntry { place, name, held });
    }
    Ok(found)
}

/// What a backup's manifest records, as the pages show it.
fn held(m: &skyward_backup::Manifest) -> Held {
    Held {
        made: m.made,
        size: m.size(),
        parts: m.parts(),
        device: m.device.clone(),
        system: m.system.clone(),
        slot: skyward_vab::Slot::from_suffix(&m.slot),
    }
}

/// Look for backups in the places asked, the restore page showing what is
/// found and saying why a place could not be looked in.
fn find_backups(app: &mut App, storage: bool, usb: bool, now: u64) {
    let mut found = Vec::new();
    for (place, look) in [(Place::Storage, storage), (Place::UsbDrive, usb)] {
        if look {
            match backups_at(place) {
                Ok(mut there) => found.append(&mut there),
                Err(e) => app.report(&format!("{e:#}"), now),
            }
        }
    }
    app.set_backups(found);
}

/// Delete the backup in `folder` of `place`; a USB drive, shown read-only,
/// is made writable for it and read-only again after.
fn delete_backup(place: Place, folder: &Path) -> Result<()> {
    let drive = Path::new(USB_DRIVE);
    if place == Place::UsbDrive {
        skyward_block::mount::remount_writable(drive, true)?;
    }
    let deleted = fs::remove_dir_all(folder).with_context(|| format!("delete {}", folder.display()));
    if place == Place::UsbDrive {
        let read_only = skyward_block::mount::remount_writable(drive, false);
        deleted?;
        return read_only;
    }
    deleted
}

/// How backing up or restoring ended, as its page tells it.
fn backup_end(end: backup::End) -> BackupEnd {
    match end {
        backup::End::Saved(folder) => {
            BackupEnd::Saved(folder.file_name().expect("a backup's folder has a name").to_string_lossy().into_owned())
        }
        backup::End::Restored { unmade } => BackupEnd::Restored { unmade: unmade.iter().map(|p| p.display().to_string()).collect() },
        backup::End::Stopped => BackupEnd::Stopped,
        backup::End::Refused(refusal) => BackupEnd::Refused(match refusal {
            backup::Refusal::Room { needed, free } => BackupRefusal::Room { needed, free },
            backup::Refusal::Update(state) => BackupRefusal::Update(state),
            backup::Refusal::OtherPhone(device) => BackupRefusal::OtherPhone(device),
            backup::Refusal::Resized(name) => BackupRefusal::Resized(name),
            backup::Refusal::DataReset => BackupRefusal::DataReset,
        }),
        backup::End::Failed(why) => BackupEnd::Failed(why),
    }
}

/// Now, in Unix seconds.
fn unix_now() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).context("the clock is before 1970")?.as_secs() as i64)
}

/// Now, in Unix milliseconds.
fn unix_ms_now() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).context("the clock is before 1970")?.as_millis() as i64)
}

/// The USB drive plugged in, and whether it is shown at [`USB_DRIVE`] or
/// why it could not be.
struct Plugged {
    drive: Drive,
    shown: Result<(), String>,
}

/// Look for the USB drive plugged in: one gone is taken away from where it
/// was shown, even with files on it still open, and one found in two looks
/// running is shown, the kernel having listed its partitions by then.
/// `seen` is a drive found by the look before that is not shown yet. What
/// the pages are told comes back when it changed.
fn look_for_drive(plugged: &mut Option<Plugged>, seen: &mut Option<String>) -> Result<Option<Option<UsbDrive>>> {
    let found = usb::drives()?.into_iter().find(Drive::ready);
    if let Some(p) = plugged {
        if found.is_some_and(|d| d.disk == p.drive.disk) {
            return Ok(None);
        }
        if p.shown.is_ok() {
            skyward_block::mount::umount_detach(Path::new(USB_DRIVE))?;
        }
        skyward_sys::kmsg(&format!("USB drive {} is gone", p.drive.disk))?;
        *plugged = None;
        *seen = None;
        return Ok(Some(None));
    }
    let Some(drive) = found else {
        *seen = None;
        return Ok(None);
    };
    if seen.as_deref() != Some(drive.disk.as_str()) {
        *seen = Some(drive.disk.clone());
        return Ok(None);
    }
    let shown = match usb::mount(&drive, Path::new(USB_DRIVE)) {
        Ok((part, filesystem)) => {
            skyward_sys::kmsg(&format!("USB drive {part} shown at {USB_DRIVE} as {filesystem}"))?;
            Ok(())
        }
        Err(e) => {
            skyward_sys::kmsg(&format!("USB drive {} not shown: {e:#}", drive.disk))?;
            Err(format!("{e:#}"))
        }
    };
    let told = match &shown {
        Ok(()) => UsbDrive::Shown,
        Err(why) => UsbDrive::Unreadable(why.clone()),
    };
    *plugged = Some(Plugged { drive, shown });
    *seen = None;
    Ok(Some(Some(told)))
}

/// What a choice on the clash page asks of the work on files.
fn clash_rule(choice: ClashChoice) -> skyward_files::Clash {
    match choice {
        ClashChoice::Replace => skyward_files::Clash::Replace,
        ClashChoice::Skip => skyward_files::Clash::Skip,
        ClashChoice::KeepBoth => skyward_files::Clash::KeepBoth,
    }
}

/// What a copy or move into the folder `into` gives the entries it makes.
/// On the data partition each takes the owner, group and label the folder
/// gives its own, as the system makes them there; in shared storage, the
/// folder's permissions as well. Elsewhere each keeps what it had.
fn ownership(into: &Path) -> Result<Ownership> {
    use skyward_crypto::data;
    let at = Path::new(data::MOUNT);
    if !skyward_block::mount::is_mount_point(at)? {
        return Ok(Ownership::Kept);
    }
    let here = fs::metadata(into).with_context(|| format!("look at {}", into.display()))?;
    if here.dev() != fs::metadata(at).with_context(|| format!("look at {}", at.display()))?.dev() {
        return Ok(Ownership::Kept);
    }
    let real = fs::canonicalize(into).with_context(|| format!("find {}", into.display()))?;
    let shared = [data::SHARED_STORAGE, data::SDCARD].iter().any(|s| real.starts_with(s));
    Ok(if shared { Ownership::Shared } else { Ownership::Folder })
}

/// The page's name for a part erasing goes through.
fn part(step: Step) -> Part {
    match step {
        Step::Data => Part::Data,
        Step::Cache => Part::Cache,
        Step::Metadata => Part::Metadata,
        Step::AppData => Part::AppData,
        Step::Media => Part::Media,
        Step::AppCaches => Part::AppCaches,
        Step::SystemCaches => Part::SystemCaches,
        Step::Art => Part::Art,
        Step::Logs => Part::Logs,
        Step::Updates => Part::Updates,
    }
}

/// The part of the phone a choice of the wipe page other than everything
/// erases.
fn step(choice: Wipe) -> Step {
    match choice {
        Wipe::AppData => Step::AppData,
        Wipe::Media => Step::Media,
        Wipe::AppCaches => Step::AppCaches,
        Wipe::SystemCaches => Step::SystemCaches,
        Wipe::Art => Step::Art,
        Wipe::Cache => Step::Cache,
        Wipe::Logs => Step::Logs,
        Wipe::Updates => Step::Updates,
        Wipe::All => unreachable!("everything is a factory reset, not a part of one"),
    }
}

/// How erasing stopped, as its page tells it.
fn erase_stop(stop: wipe::Stop) -> EraseStop {
    match stop {
        wipe::Stop::Refused(wipe::Refusal::Update(hold)) => EraseStop::Update(hold),
        wipe::Stop::Refused(wipe::Refusal::NotRebuilt) => EraseStop::NotRebuilt,
        wipe::Stop::Refused(wipe::Refusal::NoCache) => EraseStop::NoCache,
        wipe::Stop::Closed(why) => EraseStop::Closed(why),
        wipe::Stop::Locked(users) => EraseStop::Locked(users),
        wipe::Stop::Failed(step, e) => EraseStop::Failed(part(step), format!("{e:#}")),
    }
}

/// Start erasing all user data, its page following it; why it cannot
/// start ends it at once, on the same page. `asked` when the system asked
/// for it; `reformat` the filesystem every part is made afresh as, where
/// one was asked for.
fn factory_reset(app: &mut App, keep_memtag_mode: bool, asked: bool, reformat: Option<String>) -> Result<Eraser> {
    match wipe::factory_reset(reformat) {
        Ok(plan) => {
            app.erase_started(true, asked, plan.steps().into_iter().map(part).collect());
            Eraser::start(move |tell| plan.carry_out(keep_memtag_mode, tell))
        }
        Err(stop) => {
            app.erase_started(true, asked, Vec::new());
            Eraser::stopped(stop)
        }
    }
}

/// Start erasing all user data for rescue, the installing page saying so
/// meanwhile: as a factory reset erases it, the memory tagging asked of the
/// bootloader forgotten too.
fn rescue_wipe(app: &mut App) -> Result<Eraser> {
    app.rescue_wiping();
    match wipe::factory_reset(None) {
        Ok(plan) => Eraser::start(move |tell| plan.carry_out(false, tell)),
        Err(stop) => Eraser::stopped(stop),
    }
}

/// Start erasing parts on their own, its page following it: parts of the
/// data partition, and the cache partition, made afresh as `reformat`
/// where the system asked for that filesystem; `logs` are the phone's own
/// log folders. `asked` when the system asked for it.
fn parts_wipe(app: &mut App, steps: Vec<Step>, logs: &[String], asked: bool, reformat: Option<String>) -> Result<Eraser> {
    match wipe::parts(steps, logs, reformat) {
        Ok(parts) => {
            app.erase_started(false, asked, parts.steps().into_iter().map(part).collect());
            Eraser::start(move |tell| parts.carry_out(tell))
        }
        Err(stop) => {
            app.erase_started(false, asked, Vec::new());
            Eraser::stopped(stop)
        }
    }
}

/// The reasons the bootloader gives for a start that followed the kernel
/// stopping, as Android's own recovery lists them; it does not start an
/// install the system asked for before such a start.
const PANICS: [&str; 2] = ["kernel_panic", "Panic"];

/// Whether the phone came to the recovery after its kernel stopped.
pub fn panicked() -> bool {
    skyward_sys::prop("ro.boot.bootreason").is_some_and(|reason| PANICS.iter().any(|p| p.eq_ignore_ascii_case(&reason)))
}

/// An install the system asked for that waits for the data partition to
/// open, or for the user whose own files hold the package to unlock them.
struct Pending {
    package: asked::Asked,
    needs: Needs,
    retry: bool,
}

/// Start installing the package the system asked for.
fn install_asked(cfg: &DeviceConfig, app: &App, pending: Pending) -> Result<Installer> {
    if pending.needs == Needs::Cache {
        let cache = wipe::cache()?.context("the phone has no cache partition")?;
        let at = Path::new(&cache.mount_point);
        if !mount::is_mount_point(at)? {
            fs::create_dir_all(at).with_context(|| format!("make {}", at.display()))?;
            mount::mount_ro(Path::new(&fstab::device(&cache)?), at)?;
        }
    }
    Installer::asked(pending.package, pending.retry, detach(cfg.services.clone()), install_options(app))
}

/// Where a reboot rescue asked for goes: the target, and whether it is
/// rescue again, which the recovery is told in misc.
fn rescue_target(target: &str) -> (Target, bool) {
    match target {
        "bootloader" => (Target::Bootloader, false),
        "fastboot" => (Target::Fastboot, false),
        "recovery" => (Target::Recovery, false),
        "rescue" => (Target::Recovery, true),
        // As minadbd has it, anything else starts the system.
        _ => (Target::System, false),
    }
}

/// How a partition's work ended, as its page tells it.
fn item_end(item: partwork::Item) -> ItemEnd {
    match item {
        partwork::Item::Erased => ItemEnd::Erased,
        partwork::Item::Unshared => ItemEnd::Unshared,
        partwork::Item::NotShared(filesystem) => ItemEnd::NotShared(filesystem),
        partwork::Item::Fixed(Fixed::Clean) => ItemEnd::Clean,
        partwork::Item::Fixed(Fixed::Fixed) => ItemEnd::Fixed,
        partwork::Item::Made => ItemEnd::Made,
        // What the program said shows below.
        partwork::Item::Fixed(Fixed::Failed) => ItemEnd::Failed(None),
        partwork::Item::Failed(why) => ItemEnd::Failed(Some(why)),
    }
}

/// Why work on whole partitions stopped, as its page tells it.
fn part_stop(stop: partwork::Stop) -> PartStop {
    match stop {
        partwork::Stop::Refused(refusal) => PartStop::Refused(refusal),
        partwork::Stop::Unmounted(base) => PartStop::Unmounted(base),
        partwork::Stop::Failed(why) => PartStop::Failed(why),
    }
}

/// The partitions the filesystem page lists, in this order, where the
/// phone's table has them: the data partition, /metadata and the cache
/// partition. Each comes with what it holds now, its size, and the other
/// filesystems the table lists for it that the recovery makes. The data
/// partition reads only through its decryption, so what it holds is known
/// only once it is open; until then, any of the table's filesystems may be
/// it, and where the table lists one, it is that. /metadata keeps the key
/// the data partition opens with, which making it afresh would lose, so it
/// is offered no other.
fn read_filesystems() -> Result<Vec<FsRow>> {
    let mut rows = Vec::new();
    for (name, point) in [(DATA_PARTITION, "/data"), ("metadata", "/metadata"), ("cache", "/cache")] {
        let entries = fstab::read(point)?;
        let Some(entry) = entries.first() else { continue };
        let device = fstab::device(entry)?;
        let size = skyward_block::size_bytes(&device)?;
        let mut listed: Vec<String> = Vec::new();
        for e in &entries {
            if REFORMATS.contains(&e.filesystem.as_str()) && !listed.contains(&e.filesystem) {
                listed.push(e.filesystem.clone());
            }
        }
        let filesystem = match name {
            DATA_PARTITION => mount::visible(Path::new(point))?.map(|v| v.filesystem),
            _ => probe::filesystem(Path::new(&device))?,
        };
        let others = match (name, &filesystem) {
            ("metadata", _) => Vec::new(),
            (_, Some(held)) => listed.into_iter().filter(|fs| fs != held).collect(),
            (_, None) if listed.len() > 1 => listed,
            (_, None) => Vec::new(),
        };
        rows.push(FsRow { name: name.to_string(), mount_point: point.to_string(), filesystem, size, others });
    }
    Ok(rows)
}

/// The filesystem page's partition `name` for its work: the data partition,
/// or the table's first entry for another.
fn fs_part(name: &str) -> Result<partwork::Part> {
    if name == DATA_PARTITION {
        return Ok(partwork::Part::Data);
    }
    let point = format!("/{name}");
    let entry = fstab::read(&point)?.into_iter().next().with_context(|| format!("the phone's table has no {point}"))?;
    Ok(partwork::Part::Entry(entry))
}

/// What the pages act on: a touch or a key, what a program asked on the
/// recovery socket, or a USB drive plugged in or pulled out.
enum Incoming {
    /// The page shown first, which may ask for work as it shows.
    Arrived,
    Input(Event),
    Asked(Request),
    Drive(Option<UsbDrive>),
}

/// What a request on the recovery socket comes to. adbd asks for fastboot
/// while the slot's partitions are up, which fastbootd cannot work with, so
/// the phone restarts into fastboot as from the reboot page; fastbootd asks
/// for the recovery it runs in. A request that finds the recovery already
/// where it asks to be, as when the keys left fastboot first, needs nothing.
fn answer(request: Request, fastboot: bool) -> Vec<Effect> {
    match (request, fastboot) {
        (Request::Fastboot, false) => vec![Effect::Reboot(Target::Fastboot)],
        (Request::Recovery, true) => vec![Effect::EnterRecovery],
        _ => Vec::new(),
    }
}

/// The time of day for the status bar, in the zone the settings chose.
fn clock(zone: usize) -> Result<String> {
    Ok(skyward_ui::fmt::clock(unix_now()?, zone))
}

/// Tell the kernel log how the data partition was mounted, where opening
/// it mounted it: the check made first, in full where it found something,
/// and the device it is read through.
fn log_mounted(mounted: &Opened) -> Result<()> {
    if let Some(checked) = &mounted.checked {
        skyward_sys::kmsg(&format!("the check of {} ended with {}", mounted.mapped.display(), checked.status))?;
        if checked.status != 0 {
            for line in checked.said.lines() {
                skyward_sys::kmsg(line)?;
            }
        }
    }
    skyward_sys::kmsg(&format!("{} holds {} at {}", mounted.mapped.display(), mounted.filesystem, skyward_crypto::data::MOUNT))
}

/// Show what the data partition still keeps closed: the first user whose
/// own files wait on a screen lock they set themselves, with the control
/// that lock is answered with, or nothing. Users whose credential the phone
/// made up open behind the owner and are never asked; one that did not
/// open goes to the kernel log with why.
fn show_closed(app: &mut App, closed: &[Closed]) -> Result<()> {
    use skyward_crypto::user::Lock;
    skyward_sys::kmsg(&format!("the users whose own files are still closed: {:?}", closed.iter().map(|c| c.user).collect::<Vec<_>>()))?;
    for c in closed {
        if let Some(why) = &c.why {
            skyward_sys::kmsg(&format!("the files of user {} stay closed: {why}", c.user))?;
        }
    }
    let waiting = closed.iter().find(|c| !c.made_up).map(|c| {
        let keys = match c.lock {
            Lock::PATTERN => Keys::Pattern,
            Lock::PIN => Keys::Digits,
            _ => Keys::Letters,
        };
        (c.user, keys)
    });
    app.data_waiting(waiting);
    Ok(())
}

/// What the file manager lists for a folder: its entries, folders and
/// links to folders first, then by name ignoring case, each folder with how
/// many entries it holds. An entry gone by the time it is looked at, as
/// processes in /proc go, is left out.
fn read_folder(path: &Path) -> Result<Vec<FileRow>> {
    let gone = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let mut rows = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let full = entry.with_context(|| format!("read {}", path.display()))?.path();
        let meta = match fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(e) if gone(&e) => continue,
            Err(e) => return Err(e).with_context(|| format!("stat {}", full.display())),
        };
        let kind = if meta.file_type().is_symlink() {
            let target = fs::read_link(&full).with_context(|| format!("read the link {}", full.display()))?;
            // A link to nothing leads to no folder.
            let folder = match fs::metadata(&full) {
                Ok(to) => to.is_dir(),
                Err(e) if gone(&e) => false,
                Err(e) => return Err(e).with_context(|| format!("follow {}", full.display())),
            };
            FileKind::Link { target: target.to_string_lossy().into_owned(), folder }
        } else if meta.is_dir() {
            // A folder that may not be read shows that in place of a count.
            let items = match fs::read_dir(&full) {
                Ok(inside) => Some(inside.count()),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => None,
                Err(e) if gone(&e) => continue,
                Err(e) => return Err(e).with_context(|| format!("read {}", full.display())),
            };
            FileKind::Folder { items }
        } else if meta.is_file() {
            FileKind::File
        } else {
            FileKind::Special
        };
        let name = full.file_name().with_context(|| format!("{} has no name", full.display()))?.to_string_lossy().into_owned();
        rows.push(FileRow { name, kind, size: meta.len(), modified: meta.mtime(), mode: meta.mode(), uid: meta.uid(), gid: meta.gid() });
    }
    rows.sort_by_key(|r| (!matches!(r.kind, FileKind::Folder { .. } | FileKind::Link { folder: true, .. }), r.name.to_lowercase()));
    Ok(rows)
}

/// The slot's logical partitions and how each shows now, for the mount page.
/// A mounted partition is in use when a program holds it, or when one of
/// `services` this phone's build of the hardware runs is run from it: those
/// are started again from there whenever they end.
fn read_mounts(services: &[Service]) -> Result<Vec<skyward_ui::app::MountRow>> {
    use skyward_ui::app::MountRow;
    let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let shown = skyward_vab::mounted_partitions(slot)?;
    let holders = skyward_block::mount::holders()?;
    let props = skyward_info::installed_props(slot)?;
    let services: Vec<&Service> = services.iter().filter(|s| s.runs_on(&props)).collect();
    let mut rows = Vec::new();
    for base in skyward_vab::slot_bases(slot)? {
        let point = skyward_vab::mount_point(&base);
        let name = format!("{base}{}", slot.suffix());
        let mounted = shown.iter().find(|m| m.partition == name);
        let mut using = BTreeSet::new();
        if let Some(m) = mounted {
            using.extend(holders.get(&skyward_block::devnum(&m.device)?).into_iter().flatten().cloned());
            for s in services.iter().filter(|s| !s.oneshot && s.uses(&point)) {
                let program = Path::new(&s.path).file_name().with_context(|| format!("{} names no program", s.path))?;
                using.insert(program.to_string_lossy().into_owned());
            }
        }
        rows.push(MountRow {
            base,
            point: point.display().to_string(),
            mounted: mounted.is_some(),
            holders: using.into_iter().collect(),
        });
    }
    Ok(rows)
}

/// What the phone's boot control holds of the slots, or why it cannot be
/// asked.
fn read_slots() -> Result<skyward_vab::BootSlots, String> {
    let read = || -> Result<skyward_vab::BootSlots> {
        let (binder, hardware) = (skyward_binder::driver::Binder::open()?, skyward_binder::driver::Binder::hardware()?);
        let control = skyward_vab::boot::BootControl::open(&binder, &hardware)?;
        control.slots()
    };
    read().map_err(|e| format!("{e:#}"))
}

/// Have the bootloader start `slot` next and give what the boot control
/// holds of the slots then, unless an update is in flight: it set the
/// slot it needs itself.
fn set_slot(slot: skyward_vab::Slot) -> Result<skyward_vab::BootSlots, String> {
    let set = || -> Result<skyward_vab::BootSlots> {
        let state = skyward_vab::snapshot_state()?;
        ensure!(state == skyward_vab::SnapshotState::None, "an update is in flight ({state:?}), and it set the slot it needs");
        let (binder, hardware) = (skyward_binder::driver::Binder::open()?, skyward_binder::driver::Binder::hardware()?);
        let control = skyward_vab::boot::BootControl::open(&binder, &hardware)?;
        control.set_active(slot)?;
        skyward_sys::kmsg(&format!("slot {} starts next", slot.letter()))?;
        control.slots()
    };
    set().map_err(|e| format!("{e:#}"))
}

/// What has to happen before an update writes over the installed system's
/// partitions: the files the phone's own programs run from stay in memory,
/// and the booted slot's partitions come off.
fn detach(services: Vec<Service>) -> Detach {
    Box::new(move || {
        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
        let mounted = skyward_vab::mounted_partitions(slot)?;
        let busy = services::pin(&services, &mounted)?;
        skyward_vab::release_partitions(slot, &busy)
    })
}

/// What an install does besides installing, as the install page's switches
/// are set: puts Skyward back into the recovery partitions it writes over,
/// on a phone that has them, and checks a zip's signature before its
/// install script runs.
fn install_options(app: &App) -> Options {
    Options { keep: app.state.keep_recovery && app.state.recovery_partitions, signed: app.state.check_signature }
}

/// How often the WLAN page scans while it shows.
const SCAN_EVERY: Duration = Duration::from_secs(10);

fn wlan_worker(w: &Option<wlan::Worker>) -> Result<&wlan::Worker> {
    w.as_ref().context("the phone's WLAN is not brought up")
}

/// The installed system, as the update server is told of it.
fn installed(cfg: &DeviceConfig, info: &skyward_info::DeviceInfo) -> Result<Installed> {
    let slot = info.slot.context("only an A/B phone is updated")?;
    let props = skyward_info::installed_props(slot)?;
    let prop = |name: &str| props.get(name).cloned().with_context(|| format!("the installed system says no {name}"));
    Ok(Installed {
        device: prop("ro.product.mod_device")?,
        version: prop(&cfg.device.os_version_prop)?,
        android: prop("ro.build.version.release")?,
        unlocked: skyward_block::boot_param("androidboot.flash.locked")?.as_deref() == Some("0"),
    })
}

/// Where `release`'s full package is downloaded to.
fn downloaded(release: &Release) -> PathBuf {
    Path::new(skyward_crypto::data::SDCARD).join(DOWNLOADS).join(&release.name)
}

/// What internal storage holds of `release`'s full package; nothing while it
/// cannot be read.
fn have(app: &App, release: &Release) -> Have {
    if !app.state.data.storage_open() {
        return Have::Nothing;
    }
    let path = downloaded(release);
    let size = |p: &Path| match fs::metadata(p) {
        Ok(m) => Some(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            let _ = skyward_sys::kmsg(&format!("read {}: {e}", p.display()));
            None
        }
    };
    match (size(&path), size(&download::partial(&path))) {
        (Some(whole), _) => Have::Whole(whole),
        (None, Some(part)) => Have::Part(part),
        (None, None) => Have::Nothing,
    }
}

/// Wait until a device, the installer or the recovery socket has something,
/// or `timeout_ms` passes.
fn wait(fds: &[i32], timeout_ms: i32) -> Result<()> {
    let mut polls: Vec<libc::pollfd> = fds.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
    let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, timeout_ms) };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err).context("poll input devices");
        }
    }
    Ok(())
}

/// Run the pages. What `request`, read from misc, asks for decides the
/// first of them; asking for nothing, they start at `start`.
pub fn run(cfg: &DeviceConfig, font: &Path, mono: &Path, request: bcb::Request, start: Screen, dump: Option<&Path>) -> Result<()> {
    if cfg.display.density == 0 {
        return Err(anyhow!("{}: [display] density is required to lay out the UI", cfg.device.codename));
    }
    let mut info = skyward_info::collect(cfg)?;
    let fonts = Fonts::load(font, 0, mono)?;
    let mut surface = KmsSurface::open("/dev/dri/card0")?;
    let (w, h) = surface.size();
    eprintln!("display {}", surface.describe());
    let mut inputs = Inputs::open(w, h, cfg.touch.writes.clone(), cfg.touch.reversed.clone())?;
    let started = Instant::now();
    let mut touch_known = false;
    // adbd and fastbootd ask over it to go over to fastboot and back.
    let control = Control::open()?;
    let mut haptics = cfg.haptics.as_ref().map(Haptics::open).transpose()?;
    let mut backlight = Backlight::open()?;
    let first = match &request.work {
        Work::Fastboot => Screen::Fastboot,
        Work::PromptWipeData => Screen::CantStart,
        Work::Unsupported(_) => Screen::Unhandled,
        Work::Menu
        | Work::Exit
        | Work::Update(_)
        | Work::WipeData { .. }
        | Work::WipeCache { .. }
        | Work::WipeAb { .. }
        | Work::Sideload { .. }
        | Work::Rescue
        | Work::UnshareBlocks => start,
    };
    let mut app = App::new(first, w, h, cfg.display.density as f32 / 160.0);
    app.set_screen_corners(cfg.display.corner_radius as f32);
    // The pages speak the language the system named with the work it asks
    // for, or else the one the installed system is built to start in.
    if let Some(locale) = request.locale.as_deref().or(info.locale.as_deref()) {
        app.state.lang = Lang::for_locale(locale)?;
        skyward_sys::kmsg(&format!("the pages speak {:?}, for the locale {locale}", app.state.lang))?;
    }
    // The home page tells of a restart into the slot the system does not
    // run from now: an update or a switch left for the next boot.
    if info.slot.is_some() {
        app.set_boot_slots(read_slots());
    }
    app.set_asked(asked(&request));
    // Opening the data partition, or a user's own files in it, under way,
    // and the users whose own files are still closed once it is open.
    let mut decryptor: Option<Decryptor> = None;
    let mut closed: Vec<Closed> = Vec::new();
    let mut arriving = Some(Incoming::Arrived);
    // The mount page shows the partitions as they are when it opens and
    // after each change; why they cannot be read is the page's to say.
    let mounts = || read_mounts(&cfg.services).map_err(|e| format!("{e:#}"));

    let mut screen = Canvas::new(w, h);
    // A page sliding over another draws on three screens' worth of canvas
    // besides the panel's: made ready now, the first slide finds them so.
    Canvas::keep_ready(w, h, 3);
    let mut shown_clock = String::new();
    // The readings are taken again on a thread of their own; those taken
    // here as work ends say how it left the phone, and readings started
    // before them are out of date.
    let readings = Readings::start(cfg)?;
    let mut read_at = Instant::now();
    let mut installer: Option<Installer> = None;
    // Whether the panel's frame memory has been blacked out for an off
    // panel; cleared once a real frame is shown again.
    let mut blanked = false;
    // Started for fastboot, the port goes to the phone's own fastbootd. The
    // page says whether it came up and how far a computer has the port;
    // why it did not come up goes to the kernel log and on screen.
    let mut fastboot = first == Screen::Fastboot;
    let mut port_at = Instant::now();
    // File transfer runs while its switch is on, and offers internal
    // storage while that is open; one that does not start says why on
    // screen, and the recovery carries on.
    let mut transfer: Option<Transfer> = None;
    if app.state.file_transfer {
        match Transfer::start(cfg, &info, app.state.time_zone) {
            Ok(t) => transfer = Some(t),
            Err(e) => {
                skyward_sys::kmsg(&format!("file transfer did not start: {e:#}"))?;
                app.file_transfer_failed(false, &skyward_ui::fmt::transfer_failed(app.state.lang, &format!("{e:#}")), monotonic_ms());
            }
        }
    }
    // A sideload has the port, which goes back to rest once it ends.
    let mut sideloading = false;
    if fastboot {
        if let Err(e) = skyward_sys::usb::switch("fastboot", USB_TIMEOUT) {
            skyward_sys::kmsg(&format!("fastbootd has not taken the port: {e:#}"))?;
            app.report(&format!("{e:#}"), monotonic_ms());
        }
        let (running, link) = fastboot_status()?;
        app.set_fastboot(running, link);
    }
    // Work the request asks for that goes on keeps the request in misc,
    // so a restart halfway comes back to it, until it is done or the user
    // leaves it for somewhere else.
    let misc = bcb::misc();
    let mut kept = request.work.lasts();
    // The wipe page offers the cache partition where the phone's table has
    // one; a table that does not read leaves it out, and the page still
    // works.
    app.state.cache_partition = match wipe::cache() {
        Ok(cache) => cache.is_some(),
        Err(e) => {
            skyward_sys::kmsg(&format!("whether the phone has a cache partition is not known: {e:#}"))?;
            false
        }
    };
    app.state.phone_logs = cfg.wipe.logs.len();
    // Skyward lives in a recovery partition of each slot where the phone
    // has them, and the install page then offers to keep it there.
    let by_name = Path::new(skyward_block::BY_NAME);
    app.state.recovery_partitions = by_name.join("recovery_a").try_exists()? && by_name.join("recovery_b").try_exists()?;
    let mut eraser: Option<Eraser> = None;
    // Erasing the system asked for leaves on its own once it is done, as
    // does erasing an update asked for, sideloaded with a restart after it;
    // what the user chose waits for them.
    let mut erase_leaves = false;
    // Erasing all user data rescue asked for, whose end goes back to rescue
    // rather than to a page of its own.
    let mut rescue_erasing = false;
    // Where rescue asked the phone to leave for, once it lets USB go.
    let mut rescue_leaves: Option<(Target, bool)> = None;
    // The install the system asked for, until the data partition it is in
    // opens, or the user whose own files hold it unlocks them.
    let mut pending: Option<Pending> = None;
    // Work on whole partitions under way, and whether the system asked for
    // it before the recovery started.
    let mut part_worker: Option<partwork::Worker> = None;
    match &request.work {
        Work::WipeData { reformat } => {
            eraser = Some(factory_reset(&mut app, request.keep_memtag_mode, true, reformat.clone())?);
            erase_leaves = true;
        }
        Work::WipeCache { reformat } => {
            eraser = Some(parts_wipe(&mut app, vec![Step::Cache], &cfg.wipe.logs, true, reformat.clone())?);
            erase_leaves = true;
        }
        Work::Sideload { .. } => {
            app.start_sideload();
            installer = Some(Installer::start(detach(cfg.services.clone()), install_options(&app))?);
            sideloading = true;
        }
        Work::Rescue => {
            app.start_rescue();
            installer = Some(Installer::rescue(detach(cfg.services.clone()), install_options(&app))?);
            sideloading = true;
        }
        Work::Update(update) => {
            app.start_asked(update.security, update.package.clone());
            let retry = update.retry > 0;
            if !retry && panicked() {
                skyward_sys::kmsg("the phone came here after its kernel stopped, so the update asked for is not installed")?;
                app.install(skyward_install::Event::Finished(skyward_install::Outcome::Refused(skyward_install::Refusal::Panicked)), monotonic_ms());
                bcb::clear(&misc)?;
                kept = false;
            } else {
                let (package, needs) = asked::locate(&update.package, app.state.cache_partition);
                skyward_sys::kmsg(&format!("the update asked for is {package:?}, which needs {needs:?}"))?;
                let wanted = Pending { package, needs, retry };
                match needs {
                    Needs::Data | Needs::User(_) => {
                        app.asked_opening();
                        decryptor = Some(Decryptor::start(decrypt::Job::Open)?);
                        pending = Some(wanted);
                    }
                    Needs::Nothing | Needs::Cache => installer = Some(install_asked(cfg, &app, wanted)?),
                }
            }
        }
        Work::WipeAb { package_size } => {
            app.part_work_started(PartKind::Disable, true);
            part_worker = Some(partwork::Worker::start(partwork::Task::Disable { package_size: *package_size })?);
        }
        Work::UnshareBlocks => {
            let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
            app.part_work_started(PartKind::Unshare, true);
            part_worker = Some(partwork::Worker::start(partwork::Task::Unshare { slot })?);
        }
        Work::Menu | Work::Exit | Work::Fastboot | Work::PromptWipeData | Work::Unsupported(_) => {}
    }
    // The port came up with adb alone; at rest it offers file transfer
    // beside it.
    if transfer.is_some() && !fastboot && !sideloading {
        if let Err(e) = skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT) {
            skyward_sys::kmsg(&format!("the port did not take file transfer: {e:#}"))?;
            app.report(&format!("{e:#}"), monotonic_ms());
        }
    }
    // Once the work the system asked for is done, the page shows it for a
    // moment; then the phone starts the system, or powers off where the
    // request says to.
    let leave = if request.shutdown_after { Target::PowerOff } else { Target::System };
    let mut leave_at: Option<u64> = None;
    // The terminal page's shell, from when the page first asks for one.
    let mut shell: Option<pty::Shell> = None;
    // Copying, moving or deleting under way.
    let mut worker: Option<filework::Worker> = None;
    // Giving the data partition's files their SELinux labels under way.
    let mut relabeler: Option<filework::Worker> = None;
    // Backing up or restoring under way.
    let mut backuper: Option<backup::Worker> = None;
    // An update's merge followed: one the recovery found under way, which
    // the kernel took up again as the snapshots were mapped, or one started
    // on the update page.
    let mut merger: Option<merge::Merger> = None;
    if let Some(slot) = info.slot {
        let standing = skyward_vab::standing(slot)?;
        if matches!(standing, Standing::Merging(_)) && skyward_vab::snapshots_mapped(slot)? {
            skyward_sys::kmsg("the update's merge goes on")?;
            merger = Some(merge::Merger::start(slot, false)?);
        }
        app.set_update(standing);
    }
    // The DNS servers the network joined gave, for looking names up.
    let servers: Servers = Arc::new(Mutex::new(Vec::new()));
    // The WLAN, on a phone whose driver the recovery brings up; it starts
    // off. Without the bundle of the vendor interface's libraries the
    // supplicant does not start, and the WLAN page says why.
    let wlan = match &cfg.wlan {
        Some(w) => {
            app.state.wlan = Some(Wlan::default());
            let modules = cfg.modules.sources.iter().map(|s| skyward_modules::source_dir(s)).collect::<Result<Vec<_>>>()?;
            let libraries = secure::libraries().unwrap_or_else(|e| {
                let _ = skyward_sys::kmsg(&format!("the vendor interface's libraries are not open: {e:#}"));
                None
            });
            Some(wlan::Worker::start(w.clone(), modules, libraries, servers.clone())?)
        }
        None => None,
    };
    let mut scanned_at = Instant::now();
    app.state.update_server = cfg.update.is_some();
    // Asking the update server, and downloading the full package.
    let mut checker: Option<updates::Worker> = None;
    let mut downloader: Option<updates::Worker> = None;
    // The USB drive plugged in, looked for each second.
    let mut plugged: Option<Plugged> = None;
    let mut seen: Option<String> = None;
    let mut drive_at = Instant::now();
    loop {
        let now = monotonic_ms();
        if leave_at.is_some_and(|at| now >= at) {
            leave_at = None;
            power(leave)?;
        }
        if !touch_known && started.elapsed() >= TOUCH_WAIT {
            touch_known = true;
            if !inputs.has_panel() {
                skyward_sys::kmsg("no touch panel came up; the pages are worked with the keys")?;
                app.touchscreen_missing();
            }
        }
        for fx in app.tick(now) {
            match fx {
                Effect::Haptic => {
                    if let Some(h) = &mut haptics {
                        h.click(app.state.vibration)?;
                    }
                }
                other => unreachable!("time passing asks for nothing but a tap of the motor, not {other:?}"),
            }
        }
        let time = clock(app.state.time_zone)?;
        if time != shown_clock {
            shown_clock = time;
            app.redraw();
        }
        for reading in readings.taken() {
            match reading {
                Reading::Battery(battery) => {
                    let battery = battery?;
                    if battery != info.battery {
                        info.battery = battery;
                        app.refresh();
                    }
                }
                Reading::All(all, at) => {
                    let all = (*all)?;
                    if at >= read_at && all != info {
                        info = all;
                        app.refresh();
                    }
                }
            }
        }
        // In fastboot the page follows fastbootd and the cable each second.
        if fastboot && port_at.elapsed().as_secs() >= 1 {
            let (running, link) = fastboot_status()?;
            app.set_fastboot(running, link);
            port_at = Instant::now();
        }
        let panel = app.panel(now);
        let mut shown = false;
        if panel.is_some() {
            if app.take_dirty() {
                let t0 = Instant::now();
                app.compose(&mut screen, &fonts, &info, &shown_clock, now);
                surface.present(&screen)?;
                eprintln!("frame {:?} in {} ms", app.screen(), t0.elapsed().as_millis());
                if let Some(path) = dump {
                    fs::write(path, &screen.data).with_context(|| format!("write {}", path.display()))?;
                }
                shown = true;
            }
            blanked = false;
        } else if !blanked {
            // Black out the panel's frame memory before the backlight goes
            // to 0, so an OLED that floors the backlight above zero rather
            // than cutting it shows nothing and cannot burn in.
            screen.data.fill(0);
            surface.present(&screen)?;
            surface.settle()?;
            blanked = true;
        }
        // After the frame is on the panel, so a panel coming back on lights
        // with what it should show now, never with what it showed when it
        // went off.
        if backlight.level == 0 && panel.is_some() {
            surface.settle()?;
        }
        backlight.show(panel)?;
        // A page at rest draws the rows it has yet to while nothing else
        // is to be done, a band at a time, looking for input between bands.
        let ahead = panel.is_some() && !shown && app.draw_ahead(&fonts, &info);
        // A frame shown has waited for the one before it to reach the
        // panel, which keeps the loop to the panel's pace; otherwise the CPU
        // gets at least a millisecond.
        let floor = if shown { 0 } else { 1 };
        let until_due = app.deadline(now).map(|d| d.saturating_sub(now) as i32);
        let timeout = if ahead { 0 } else { until_due.map_or(1000, |t| t.clamp(floor, 1000)) };
        let mut fds = inputs.fds();
        if let Some(i) = &installer {
            fds.push(i.fd());
        }
        if let Some(c) = &control {
            fds.push(c.fd());
        }
        if let Some(e) = &eraser {
            fds.push(e.fd());
        }
        if let Some(sh) = &shell {
            fds.extend(sh.fds());
        }
        if let Some(w) = &worker {
            fds.push(w.fd());
        }
        if let Some(r) = &relabeler {
            fds.push(r.fd());
        }
        if let Some(b) = &backuper {
            fds.push(b.fd());
        }
        if let Some(m) = &merger {
            fds.push(m.fd());
        }
        if let Some(d) = &decryptor {
            fds.push(d.fd());
        }
        if let Some(p) = &part_worker {
            fds.push(p.fd());
        }
        if let Some(w) = &wlan {
            fds.push(w.fd());
        }
        if let Some(c) = &checker {
            fds.push(c.fd());
        }
        if let Some(d) = &downloader {
            fds.push(d.fd());
        }
        fds.push(readings.fd());
        wait(&fds, timeout)?;
        let now = monotonic_ms();
        let mut drive_told = None;
        if drive_at.elapsed().as_secs() >= 1 {
            drive_told = look_for_drive(&mut plugged, &mut seen)?;
            drive_at = Instant::now();
        }
        // What the installer and the eraser told comes first, so a touch
        // acts on the page as it now stands.
        {
            let told = installer.as_ref().map(Installer::events).unwrap_or_default();
            for ev in told {
                use skyward_install::{Event as Told, Outcome};
                let finished = matches!(ev, Told::Finished(_));
                let installed = matches!(ev, Told::Finished(Outcome::Installed { .. } | Outcome::Ran { .. }));
                let wipe_after = matches!(ev, Told::Finished(Outcome::Installed { wipe: true, .. }));
                let script_cache = matches!(ev, Told::Finished(Outcome::Ran { wipe_cache: true, .. }));
                eprintln!("install {ev:?}");
                match &ev {
                    // Rescue's own erasing tells rescue how it went once it
                    // ends; file transfer that will not let internal
                    // storage go keeps it from starting.
                    Told::Wipe => {
                        ensure!(eraser.is_none(), "erasing is under way already");
                        if withhold(&mut transfer, &mut app, now)? {
                            eraser = Some(rescue_wipe(&mut app)?);
                            rescue_erasing = true;
                        } else {
                            installer.as_ref().context("rescue asked for erasing")?.wiped(false);
                        }
                    }
                    Told::Reboot(target) => rescue_leaves = Some(rescue_target(target)),
                    _ => {}
                }
                app.install(ev, now);
                if finished {
                    installer = None;
                    sideloading = false;
                    // A sideload or an update the system asked for is done
                    // with however it went, and so is rescue, once its page
                    // is left or the computer asks the phone to leave. A
                    // sideload asked to leaves on its own after it, and an
                    // update that went in does, once what the package or the
                    // request asked to be erased is.
                    if matches!(request.work, Work::Sideload { .. } | Work::Update(_) | Work::Rescue) && kept {
                        bcb::clear(&misc)?;
                        kept = false;
                    }
                    let (leaves, cache_after) = match &request.work {
                        Work::Sideload { auto_reboot } => (*auto_reboot, script_cache),
                        Work::Update(update) => (installed, script_cache || (installed && update.wipe_cache)),
                        _ => (false, script_cache),
                    };
                    if cache_after && !app.state.cache_partition {
                        skyward_sys::kmsg("the install asked for the cache partition to be erased, and the phone has none")?;
                    }
                    if wipe_after {
                        ensure!(eraser.is_none(), "erasing is under way already");
                        eraser = Some(factory_reset(&mut app, false, false, None)?);
                        erase_leaves = leaves;
                    } else if cache_after && app.state.cache_partition {
                        ensure!(eraser.is_none(), "erasing is under way already");
                        eraser = Some(parts_wipe(&mut app, vec![Step::Cache], &cfg.wipe.logs, false, None)?);
                        erase_leaves = leaves;
                    } else if leaves {
                        leave_at = Some(now + LEAVE_AFTER_MS);
                    }
                    // Rescue asked to come back is told so in misc, as
                    // Android's own recovery tells it.
                    if let Some((target, again)) = rescue_leaves.take() {
                        if again {
                            bcb::write(&misc, &bcb::Bcb { command: "boot-rescue".into(), args: Vec::new() })?;
                        }
                        power(target)?;
                    }
                }
            }
        }
        let mut erased = false;
        if let Some(e) = &eraser {
            for ev in e.events() {
                match ev {
                    // Rescue's page says only that erasing is under way.
                    wipe::Event::At(_) | wipe::Event::Progress(..) if rescue_erasing => {}
                    wipe::Event::At(step) => app.erase_at(part(step)),
                    wipe::Event::Progress(done, total) => app.erase_progress(done, total),
                    // Rescue stays on: its request stays in misc, and the
                    // computer hears how erasing went.
                    wipe::Event::Ended(outcome) if rescue_erasing => {
                        let said = outcome.as_ref().map_or_else(wipe::said, |_| "done".to_string());
                        skyward_sys::kmsg(&format!("erasing for rescue ended: {said}"))?;
                        installer.as_ref().context("rescue waits for erasing to end")?.wiped(outcome.is_ok());
                        app.rescue_wiped(outcome.map_err(erase_stop), now);
                        if app.state.data == Data::Erased {
                            closed.clear();
                        }
                        rescue_erasing = false;
                        erased = true;
                    }
                    wipe::Event::Opened(opening) => {
                        if let Some(mounted) = &opening.mounted {
                            log_mounted(mounted)?;
                        }
                        closed = opening.closed;
                        show_closed(&mut app, &closed)?;
                    }
                    wipe::Event::Ended(outcome) => {
                        let said = outcome.as_ref().map_or_else(wipe::said, |_| "done".to_string());
                        skyward_sys::kmsg(&format!("erasing ended: {said}"))?;
                        // As with the phone's own recovery, the request is
                        // done with either way, so a restart starts the
                        // system rather than erasing again. A factory
                        // reset done is what an update that asked for one
                        // asked, too.
                        if kept {
                            bcb::clear(&misc)?;
                            kept = false;
                        }
                        if outcome.is_ok() && app.state.erase.reset {
                            bcb::clear(&misc)?;
                        }
                        if outcome.is_ok() && erase_leaves {
                            leave_at = Some(now + LEAVE_AFTER_MS);
                        }
                        app.erase_ended(outcome.map_err(erase_stop), now);
                        // Users of a data partition erased whole have no
                        // files left to open.
                        if app.state.data == Data::Erased {
                            closed.clear();
                        }
                        erased = true;
                    }
                }
            }
        }
        if erased {
            eraser = None;
        }
        let job_end = |end: filework::End| match end {
            filework::End::Done => JobEnd::Done,
            filework::End::Stopped => JobEnd::Stopped,
            filework::End::Failed(why) => JobEnd::Failed(why),
        };
        let mut worked = false;
        if let Some(w) = &worker {
            for ev in w.events() {
                match ev {
                    filework::Event::Progress(p) => app.file_progress((p.bytes, p.total_bytes), (p.entries, p.total_entries)),
                    filework::Event::Relabeling(_) => unreachable!("work on files gives no labels"),
                    filework::Event::Ended(end) => {
                        app.file_job_ended(job_end(end), now);
                        worked = true;
                    }
                }
            }
        }
        let mut relabeled = false;
        if let Some(r) = &relabeler {
            for ev in r.events() {
                match ev {
                    filework::Event::Relabeling(p) => app.relabel_progress(p.checked, p.total, p.changed),
                    filework::Event::Progress(_) => unreachable!("relabeling moves no bytes"),
                    filework::Event::Ended(end) => {
                        app.relabel_ended(job_end(end), now);
                        relabeled = true;
                    }
                }
            }
        }
        if relabeled {
            relabeler = None;
        }
        let mut parted = false;
        if let Some(p) = &part_worker {
            for ev in p.events() {
                match ev {
                    partwork::Event::Listed(names) => app.part_listed(names),
                    partwork::Event::At(index) => app.part_at(index),
                    partwork::Event::Ended(index, item, said) => {
                        skyward_sys::kmsg(&format!("partition work: item {index} ended as {item:?}"))?;
                        app.part_ended(index, item_end(item), &said);
                    }
                    partwork::Event::Done(done) => {
                        skyward_sys::kmsg(&format!("partition work ended: {done:?}"))?;
                        let work = app.state.part_work.as_ref().context("partition work shows on its page")?;
                        let whole = done.is_ok() && !work.items.iter().any(|(_, s)| matches!(s, ItemState::Ended(ItemEnd::Failed(_))));
                        let data = work.items.iter().any(|(name, _)| name == DATA_PARTITION);
                        // What the system asked for is done with either way,
                        // and the phone leaves on its own once it went
                        // through every partition.
                        if work.asked {
                            if kept {
                                bcb::clear(&misc)?;
                                kept = false;
                            }
                            if whole {
                                leave_at = Some(now + LEAVE_AFTER_MS);
                            }
                        }
                        // Work on the data partition leaves it closed.
                        if data {
                            closed.clear();
                        }
                        app.part_work_ended(done.map_err(part_stop), now);
                        parted = true;
                    }
                }
            }
        }
        if parted {
            part_worker = None;
        }
        // The folder shown is read again for what the work changed.
        if worked {
            worker = None;
            let folder = app.state.files.path.clone();
            list(&mut app, folder)?;
        }
        let mut backed = false;
        if let Some(b) = &backuper {
            for ev in b.events() {
                match ev {
                    backup::Event::Plan(plan) => app.backup_plan(plan),
                    backup::Event::At { item, done, of, checking } => app.backup_at(BackupAt { item, done, of, checking }),
                    backup::Event::Ended(end) => {
                        app.backup_ended(backup_end(end), now);
                        backed = true;
                    }
                }
            }
        }
        if backed {
            backuper = None;
        }
        if let Some(m) = &merger {
            let slot = info.slot.context("only an A/B phone merges an update")?;
            for ev in m.events() {
                match ev {
                    merge::Event::Looked(look) => {
                        eprintln!("merge {look:?}");
                        app.set_update(match look {
                            Merge::Going(done) => Standing::Merging(done),
                            Merge::Done | Merge::NeedsReboot => Standing::Merged,
                            Merge::Failed(code) => Standing::Failed(code),
                            Merge::Cancelled | Merge::Idle(_) => skyward_vab::standing(slot)?,
                        });
                    }
                    merge::Event::Failed(why) => {
                        skyward_sys::kmsg(&format!("the update did not merge: {why}"))?;
                        app.report(&skyward_ui::fmt::merge_failed(app.state.lang, &why), now);
                        app.set_update(skyward_vab::standing(slot)?);
                    }
                }
            }
            // Merged or not, the phone's readings say where the update
            // stands now.
            if m.ended() {
                merger = None;
                read_at = Instant::now();
                info = skyward_info::collect(cfg)?;
                app.refresh();
            }
        }
        if let Some((told, traffic)) = wlan.as_ref().map(wlan::Worker::told) {
            if let Some(w) = told {
                app.set_wlan(w);
            }
            if let Some(t) = traffic {
                app.set_traffic(t);
            }
        }
        // The WLAN page scans again now and then while it shows, as the
        // stock settings do.
        let wlan_on = app.state.wlan.as_ref().is_some_and(|w| w.power == WlanPower::On);
        if app.screen() == Screen::Wlan && wlan_on && scanned_at.elapsed() >= SCAN_EVERY {
            scanned_at = Instant::now();
            wlan_worker(&wlan)?.send(wlan::Command::Scan);
        }
        let mut checked = false;
        if let Some(c) = &checker {
            for ev in c.events() {
                let updates::Event::Checked(answer) = ev else { unreachable!("asking the server downloads nothing") };
                let have = match &answer {
                    Ok(Some(release)) => have(&app, release),
                    _ => Have::Nothing,
                };
                app.update_checked(answer, have);
                checked = true;
            }
        }
        if checked {
            checker = None;
        }
        let mut fetched = false;
        if let Some(d) = &downloader {
            for ev in d.events() {
                match ev {
                    updates::Event::Progress(progress, rate) => app.download_progress(progress, rate),
                    updates::Event::Ended(end) => {
                        let release = app.state.newest().context("a download is of the newest system")?.clone();
                        let end = match end {
                            updates::End::Downloaded => DownloadEnd::Downloaded,
                            updates::End::Stopped => DownloadEnd::Stopped,
                            updates::End::Failed(why) => DownloadEnd::Failed(why),
                        };
                        skyward_sys::kmsg(&format!("downloading {} ended: {end:?}", release.name))?;
                        app.download_ended(end, have(&app, &release), now);
                        fetched = true;
                    }
                    updates::Event::Checked(_) => unreachable!("a download asks nothing of its own"),
                }
            }
        }
        if fetched {
            downloader = None;
        }
        if let Some(ended) = decryptor.as_ref().and_then(Decryptor::ended) {
            decryptor = None;
            match ended {
                decrypt::Ended::Opened(Ok(opening)) => {
                    if let Some(mounted) = &opening.mounted {
                        log_mounted(mounted)?;
                    }
                    closed = opening.closed;
                    show_closed(&mut app, &closed)?;
                }
                decrypt::Ended::Opened(Err(why)) => {
                    skyward_sys::kmsg(&format!("the data partition stays closed: {why}"))?;
                    // The update the system asked for cannot be read, and
                    // is done with.
                    if pending.take().is_some() {
                        let failed = skyward_install::Outcome::Failed(skyward_install::Failure::Unreadable(why.clone()));
                        app.install(skyward_install::Event::Finished(failed), now);
                        if kept {
                            bcb::clear(&misc)?;
                            kept = false;
                        }
                    }
                    app.data_failed(why);
                }
                decrypt::Ended::Unlocked(Ok(still)) => {
                    closed = still;
                    show_closed(&mut app, &closed)?;
                }
                decrypt::Ended::Unlocked(Err(why)) => {
                    skyward_sys::kmsg(&format!("what was entered did not open the user's files: {why}"))?;
                    app.unlock_failed(why, now);
                }
            }
            // Screenshots taken while internal storage was closed go into
            // the gallery's once it opens.
            if app.state.place_open(Place::Storage) {
                if let Err(e) = keep_held_screenshots() {
                    let why = format!("{e:#}");
                    skyward_sys::kmsg(&format!("the screenshots held did not go into the gallery's: {why}"))?;
                    app.screenshots_not_kept(&why, now);
                }
            }
            // The update the system asked for starts once what holds it is
            // open; until the user whose own files hold it unlocks them,
            // its page says to.
            if let Some(wanted) = pending.take() {
                match wanted.needs {
                    Needs::User(user) if closed.iter().any(|c| c.user == user) => {
                        app.asked_locked(user, now);
                        pending = Some(wanted);
                    }
                    _ => installer = Some(install_asked(cfg, &app, wanted)?),
                }
            }
        }
        if let Some(sh) = &mut shell {
            let wrote = sh.read()?;
            if !wrote.is_empty() {
                app.term_output(&wrote);
            }
            if let Some(status) = sh.ended()? {
                // What it wrote last shows before its end does.
                let last = sh.read()?;
                if !last.is_empty() {
                    app.term_output(&last);
                }
                app.term_ended(pty::how(status));
                shell = None;
            }
        }
        // Then what programs asked on the recovery socket, each answered as
        // the recovery stands when its turn comes.
        let requests = match &control {
            Some(c) => c.requests()?,
            None => Vec::new(),
        };
        let incoming = arriving
            .take()
            .into_iter()
            .chain(drive_told.map(Incoming::Drive))
            .chain(requests.into_iter().map(Incoming::Asked))
            .chain(inputs.read()?.into_iter().map(Incoming::Input));
        for came in incoming {
            let effects = match came {
                Incoming::Arrived => app.arrive(),
                Incoming::Drive(drive) => app.set_usb_drive(drive),
                Incoming::Asked(asked) => {
                    skyward_sys::kmsg(&format!("asked on the recovery socket: {asked:?}"))?;
                    answer(asked, fastboot)
                }
                Incoming::Input(Event::Touch(t, at)) => app.touch(t, &fonts, at),
                Incoming::Input(Event::Key(k, true)) => app.key(k, now),
                Incoming::Input(Event::Key(k, false)) => app.key_up(k, now),
            };
            // What the pages ask for in answer to the work done goes on
            // after it.
            let mut queue = VecDeque::from(effects);
            while let Some(fx) = queue.pop_front() {
                eprintln!("effect {fx:?}");
                match fx {
                    // Leaving is the user's answer to whatever the request
                    // still kept in misc asked.
                    Effect::Reboot(target) => {
                        if kept {
                            bcb::clear(&misc)?;
                            kept = false;
                        }
                        power(target)?
                    }
                    Effect::Dismiss => {
                        if kept {
                            bcb::clear(&misc)?;
                            kept = false;
                        }
                    }
                    Effect::FactoryReset => {
                        ensure!(eraser.is_none(), "erasing is under way already");
                        if !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        eraser = Some(factory_reset(&mut app, request.keep_memtag_mode, true, None)?);
                        erase_leaves = true;
                    }
                    // Opening the data partition and erasing it are not to
                    // cross; the page says to wait for the one under way.
                    Effect::Wipe(chosen) => {
                        ensure!(eraser.is_none(), "erasing is under way already");
                        if decryptor.is_some() {
                            app.report(app.state.lang.pick(DATA_OPENING), now);
                            continue;
                        }
                        if !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        erase_leaves = false;
                        if chosen.contains(&Wipe::All) {
                            eraser = Some(factory_reset(&mut app, false, false, None)?);
                        } else if chosen.iter().any(|&w| w != Wipe::Cache) && app.state.data == Data::Erased {
                            app.erase_started(false, false, Vec::new());
                            app.erase_ended(Err(EraseStop::Erased), now);
                        } else {
                            let steps = chosen.into_iter().map(step).collect();
                            eraser = Some(parts_wipe(&mut app, steps, &cfg.wipe.logs, false, None)?);
                        }
                    }
                    Effect::Install => installer.as_ref().context("no installer holds an update")?.install(),
                    // A shell that will not start, or will not take keys,
                    // is the page's to report; the recovery carries on.
                    Effect::StartShell(folder) => {
                        ensure!(shell.is_none(), "a shell runs already");
                        let (rows, cols) = app.term_size(&fonts);
                        match pty::Shell::start(rows, cols, &cfg.device.codename, Path::new(&folder)) {
                            Ok(sh) => {
                                shell = Some(sh);
                                app.term_started(rows, cols);
                            }
                            Err(e) => {
                                skyward_sys::kmsg(&format!("no shell for the terminal: {e:#}"))?;
                                app.report(&format!("{e:#}"), now);
                            }
                        }
                    }
                    Effect::TermInput(bytes) => {
                        let sh = shell.as_mut().context("keys went to a shell that has ended")?;
                        if let Err(e) = sh.write(&bytes) {
                            app.report(&format!("{e:#}"), now);
                        }
                    }
                    Effect::ShellTo(folder) => {
                        let sh = shell.as_mut().context("a folder went to a shell that has ended")?;
                        let went = match sh.waiting() {
                            Ok(true) => sh.change_to(&folder),
                            Ok(false) => {
                                app.report(app.state.lang.pick(skyward_ui::fmt::TERMINAL_BUSY), now);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        if let Err(e) = went {
                            app.report(&format!("{e:#}"), now);
                        }
                    }
                    Effect::Haptic => {
                        if let Some(h) = &mut haptics {
                            h.click(app.state.vibration)?;
                        }
                    }
                    // The panel shows the last frame drawn, which is what is
                    // saved; one that cannot be is the page's to report.
                    Effect::Screenshot { storage } => {
                        let t0 = Instant::now();
                        let saved = save_screenshot(&screen, storage, app.state.time_zone).map_err(|e| format!("{e:#}"));
                        eprintln!("screenshot {saved:?} in {} ms", t0.elapsed().as_millis());
                        app.screenshot_saved(saved, now);
                    }
                    Effect::Sideload => {
                        ensure!(installer.is_none(), "an installer is running already");
                        if !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        installer = Some(Installer::start(detach(cfg.services.clone()), install_options(&app))?);
                        sideloading = true;
                    }
                    Effect::InstallFile(place, path) => {
                        ensure!(installer.is_none(), "an installer is running already");
                        if !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        installer = Some(Installer::open(in_place(place, &path), detach(cfg.services.clone()), install_options(&app))?);
                    }
                    Effect::InstallPath(path) => {
                        ensure!(installer.is_none(), "an installer is running already");
                        if !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        installer = Some(Installer::open(PathBuf::from(path), detach(cfg.services.clone()), install_options(&app))?);
                    }
                    Effect::Flash { partition, pick } => {
                        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                        installer.as_ref().context("no installer holds an image")?.flash(partition, pick.slots(slot));
                    }
                    Effect::Cancel => match &installer {
                        Some(i) => i.cancel()?,
                        // The update the system asked for, waiting on the
                        // lock of the user whose files hold it, is given up
                        // as its page is left, and is done with.
                        None => {
                            ensure!(pending.take().is_some(), "no installer to cancel");
                            skyward_sys::kmsg("the update asked for is given up, its page left")?;
                            if kept {
                                bcb::clear(&misc)?;
                                kept = false;
                            }
                            let cancelled = skyward_install::Event::Finished(skyward_install::Outcome::Cancelled);
                            app.install(cancelled, now);
                        }
                    },
                    Effect::ReadMounts => app.set_mounts(mounts()),
                    Effect::ReadSlots => app.set_boot_slots(read_slots()),
                    Effect::SetSlot(slot) => app.slot_switched(set_slot(slot), now),
                    // The decrypt page shows meanwhile that the work is
                    // under way, and takes nothing more until it is done.
                    Effect::OpenData => {
                        ensure!(decryptor.is_none(), "the data partition is being opened already");
                        decryptor = Some(Decryptor::start(decrypt::Job::Open)?);
                    }
                    Effect::Unlock { user, code } => {
                        ensure!(decryptor.is_none(), "the data partition is being opened already");
                        let job = decrypt::Job::Unlock { closed: closed.clone(), user, credential: code.into_bytes() };
                        decryptor = Some(Decryptor::start(job)?);
                    }
                    Effect::ReadDir(folder) => list(&mut app, folder)?,
                    Effect::ReadPlace(place, folder) => list_place(&mut app, place, folder)?,
                    // A name taken where an entry goes is the user's to
                    // decide on before anything is written.
                    Effect::Paste { op, path, into, clash } => {
                        ensure!(worker.is_none(), "work on files is under way already");
                        let clash = match clash {
                            Some(choice) => clash_rule(choice),
                            None => match skyward_files::clashes(&[PathBuf::from(&path)], Path::new(&into)) {
                                Ok(taken) if taken.is_empty() => skyward_files::Clash::Refuse,
                                Ok(taken) => {
                                    let names = taken.iter().map(|t| t.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).collect();
                                    app.file_clash(op, path, into, names);
                                    continue;
                                }
                                Err(e) => {
                                    app.report(&format!("{e:#}"), now);
                                    continue;
                                }
                            },
                        };
                        let ownership = match ownership(Path::new(&into)) {
                            Ok(ownership) => ownership,
                            Err(e) => {
                                app.report(&format!("{e:#}"), now);
                                continue;
                            }
                        };
                        let (from, to) = (PathBuf::from(&path), PathBuf::from(&into));
                        let (task, shown) = match op {
                            FileOp::Copy => (filework::Task::Copy { from, into: to, clash, ownership }, FileTask::Copy { path, into }),
                            FileOp::Move => (filework::Task::Move { from, into: to, clash, ownership }, FileTask::Move { path, into }),
                        };
                        worker = Some(filework::Worker::start(task)?);
                        app.file_job_started(shown);
                    }
                    Effect::Delete(path) => {
                        ensure!(worker.is_none(), "work on files is under way already");
                        worker = Some(filework::Worker::start(filework::Task::Delete { path: PathBuf::from(&path) })?);
                        app.file_job_started(FileTask::Delete { path });
                    }
                    Effect::StopJob => worker.as_ref().context("there is no work on files to stop")?.stop(),
                    // Rules kept on a partition that is not mounted are the
                    // page's to report, with what to do first.
                    Effect::Relabel => {
                        ensure!(relabeler.is_none(), "relabeling is under way already");
                        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                        match relabel::rule_files(slot) {
                            Ok(rules) => {
                                relabeler = Some(filework::Worker::start(filework::Task::Relabel { rules })?);
                                app.relabel_started();
                            }
                            Err(e) => match e.downcast_ref::<relabel::Unmounted>() {
                                Some(relabel::Unmounted(base)) => app.report(&skyward_ui::fmt::mount_first(app.state.lang, base), now),
                                None => app.report(&skyward_ui::fmt::rules_unread(app.state.lang, &format!("{e:#}")), now),
                            },
                        }
                    }
                    Effect::StopRelabel => relabeler.as_ref().context("there is no relabeling to stop")?.stop(),
                    // Partitions that cannot be read are the page's to say.
                    Effect::ReadFilesystems => app.set_filesystems(read_filesystems().map_err(|e| format!("{e:#}"))),
                    // The data partition made afresh as another filesystem
                    // is a factory reset, as the system asks for one; the
                    // rest is work on one partition. Opening the data
                    // partition and working on it are not to cross, and file
                    // transfer lets internal storage go first.
                    Effect::Filesystem { name, action } => {
                        ensure!(eraser.is_none() && part_worker.is_none(), "work on partitions is under way already");
                        if name == DATA_PARTITION && decryptor.is_some() {
                            app.report(app.state.lang.pick(DATA_OPENING), now);
                            continue;
                        }
                        if name == DATA_PARTITION && !withhold(&mut transfer, &mut app, now)? {
                            continue;
                        }
                        let part = fs_part(&name)?;
                        let (kind, task) = match action {
                            FsAction::Remake(filesystem) if name == DATA_PARTITION => {
                                erase_leaves = false;
                                eraser = Some(factory_reset(&mut app, false, false, Some(filesystem))?);
                                continue;
                            }
                            FsAction::Repair => (PartKind::Repair, partwork::Task::Repair(part)),
                            FsAction::Remake(filesystem) => (PartKind::Remake(filesystem.clone()), partwork::Task::Remake(part, filesystem)),
                        };
                        skyward_sys::kmsg(&format!("{kind:?} the filesystem of {name}, as the filesystem page asked"))?;
                        app.part_work_started(kind, false);
                        part_worker = Some(partwork::Worker::start(task)?);
                    }
                    // An install still stopping may yet write; a backup or
                    // restore waits for it.
                    Effect::Backup { parts, place, compress, verify } => {
                        ensure!(backuper.is_none(), "a backup or restore is under way already");
                        if installer.is_some() {
                            app.report(app.state.lang.pick(INSTALL_UNFINISHED), now);
                            continue;
                        }
                        let into = backups_in(place);
                        let there = into.ancestors().find(|p| p.exists()).context("the root is always there")?;
                        let ownership = match ownership(there) {
                            Ok(ownership) => ownership,
                            Err(e) => {
                                app.report(&format!("{e:#}"), now);
                                continue;
                            }
                        };
                        let made = unix_now()?;
                        let task = backup::BackupTask {
                            parts,
                            stamp: skyward_ui::fmt::stamp(made, app.state.time_zone),
                            into,
                            ownership,
                            compress,
                            verify,
                            remount: (place == Place::UsbDrive).then(|| PathBuf::from(USB_DRIVE)),
                            made,
                            device: cfg.device.codename.clone(),
                            system: info.os_version.clone(),
                            efs: cfg.backup.efs.clone(),
                            skipped: backup::data_skipped(&cfg.backup.data_skipped),
                        };
                        backuper = Some(backup::Worker::start(backup::Task::Backup(task))?);
                        app.backup_started(false, place, verify);
                    }
                    Effect::Restore { place, name, parts } => {
                        ensure!(backuper.is_none(), "a backup or restore is under way already");
                        if installer.is_some() {
                            app.report(app.state.lang.pick(INSTALL_UNFINISHED), now);
                            continue;
                        }
                        let task = backup::RestoreTask {
                            folder: backups_in(place).join(&name),
                            parts,
                            device: cfg.device.codename.clone(),
                            skipped: backup::data_skipped(&cfg.backup.data_skipped),
                            detach: detach(cfg.services.clone()),
                        };
                        backuper = Some(backup::Worker::start(backup::Task::Restore(task))?);
                        app.backup_started(true, place, true);
                    }
                    Effect::FindBackups { storage, usb } => find_backups(&mut app, storage, usb, now),
                    Effect::DeleteBackup { place, name } => {
                        let deleted = delete_backup(place, &backups_in(place).join(&name)).map_err(|e| format!("{e:#}"));
                        queue.extend(app.backup_deleted(deleted, now));
                    }
                    Effect::StopBackup => backuper.as_ref().context("there is no backup or restore to stop")?.stop(),
                    Effect::ReadUpdate => {
                        let slot = info.slot.context("only an A/B phone has an update to merge")?;
                        // A merge followed tells where it stands itself.
                        if merger.is_none() {
                            app.set_update(skyward_vab::standing(slot)?);
                        }
                        // An update in the other slot starts with that slot,
                        // which a system's updater may not have set yet.
                        app.set_boot_slots(read_slots());
                    }
                    Effect::Merge => {
                        ensure!(merger.is_none(), "the update is being merged already");
                        let slot = info.slot.context("only an A/B phone merges an update")?;
                        skyward_sys::kmsg("merging the update, as the update page asked")?;
                        merger = Some(merge::Merger::start(slot, true)?);
                    }
                    Effect::Wlan(on) => wlan_worker(&wlan)?.send(if on { wlan::Command::On } else { wlan::Command::Off }),
                    Effect::ScanWlan => {
                        scanned_at = Instant::now();
                        wlan_worker(&wlan)?.send(wlan::Command::Scan);
                    }
                    Effect::JoinWlan { ssid, name, security, password } => {
                        wlan_worker(&wlan)?.send(wlan::Command::Join { ssid, name, security, password: password.0 })
                    }
                    Effect::LeaveWlan => wlan_worker(&wlan)?.send(wlan::Command::Leave),
                    // What the installed system is called is the page's to
                    // report when it cannot be read.
                    Effect::CheckUpdate => {
                        ensure!(checker.is_none(), "the update server is being asked already");
                        let server = cfg.update.as_ref().context("the phone's update server is not known")?.server.clone();
                        let probe = cfg.wlan.as_ref().context("the phone's WLAN is not brought up")?.probe.clone();
                        match installed(cfg, &info) {
                            Ok(installed) => {
                                checker = Some(updates::Worker::start(updates::Task::Check { server, probe, installed }, servers.clone())?)
                            }
                            Err(e) => app.update_checked(Err(Unanswered::Failed(format!("{e:#}"))), Have::Nothing),
                        }
                    }
                    Effect::Download(release) => {
                        ensure!(downloader.is_none(), "the full package is being downloaded already");
                        let server = cfg.update.as_ref().context("the phone's update server is not known")?.server.clone();
                        let path = downloaded(&release);
                        match installed(cfg, &info) {
                            Ok(installed) => {
                                let task = updates::Task::Download { server, installed, release, path };
                                downloader = Some(updates::Worker::start(task, servers.clone())?);
                            }
                            Err(e) => app.download_ended(DownloadEnd::Failed(Failure::Other(format!("{e:#}"))), have(&app, &release), now),
                        }
                    }
                    Effect::StopDownload => downloader.as_ref().context("there is no download to stop")?.stop(),
                    // Logs that cannot be saved are the page's to report;
                    // the recovery carries on.
                    Effect::ExportLogs { recovery, kernel, storage } => {
                        let into = match storage {
                            true => Path::new(skyward_crypto::data::SDCARD).join(STORAGE_LOGS),
                            false => PathBuf::from(TMP_LOGS),
                        };
                        match export_logs(recovery, kernel, &into, app.state.time_zone) {
                            Ok(folder) => app.report(&skyward_ui::fmt::exported(app.state.lang, &folder.display().to_string()), now),
                            Err(e) => app.report(&skyward_ui::fmt::export_failed(app.state.lang, &format!("{e:#}")), now),
                        }
                    }
                    Effect::SetMode { path, mode } => {
                        let outcome = skyward_files::set_mode(Path::new(&path), mode).map_err(|e| format!("{e:#}"));
                        app.mode_set(mode, outcome, now);
                    }
                    Effect::OpenText(path) => {
                        let read = skyward_files::read_up_to(Path::new(&path), skyward_ui::editor::MAX_BYTES).map_err(|e| format!("{e:#}"));
                        app.text_opened(&fonts, path, read, now);
                    }
                    // The folder is listed again, for the file's new size
                    // and time.
                    Effect::SaveText { path, text } => match skyward_files::replace(Path::new(&path), text.as_bytes()) {
                        Ok(()) => {
                            app.text_saved(Ok(()), now);
                            let folder = app.state.files.path.clone();
                            list(&mut app, folder)?;
                        }
                        Err(e) => app.text_saved(Err(format!("{e:#}")), now),
                    },
                    Effect::Rename { path, name } => match skyward_files::rename(Path::new(&path), &name) {
                        Ok(_) => {
                            app.renamed(Ok(()), now);
                            let folder = app.state.files.path.clone();
                            list(&mut app, folder)?;
                        }
                        Err(e) if e.is::<skyward_files::Taken>() => app.renamed(Err(RenameRefused::Taken), now),
                        Err(e) => app.renamed(Err(RenameRefused::Failed(format!("{e:#}"))), now),
                    },
                    // A partition that will not mount, or is in use, is the
                    // page's to report; the recovery carries on.
                    Effect::Mount(base) => {
                        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                        if let Err(e) = skyward_vab::mount_one(slot, &base) {
                            app.mount_failed(MountFailure { mounting: true, base, detail: format!("{e:#}") });
                        }
                        app.set_mounts(mounts());
                    }
                    Effect::Unmount(base) => {
                        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                        if let Err(e) = skyward_vab::unmount_one(slot, &base) {
                            app.mount_failed(MountFailure { mounting: false, base, detail: format!("{e:#}") });
                        }
                        app.set_mounts(mounts());
                    }
                    // init stops fastbootd as the port goes through "none";
                    // then the slot's partitions come up as at boot.
                    Effect::EnterRecovery => {
                        skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT)?;
                        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                        crate::prepare(cfg, slot)?;
                        read_at = Instant::now();
                        info = skyward_info::collect(cfg)?;
                        fastboot = false;
                        app.enter_recovery();
                    }
                    // A port that something else has takes file transfer
                    // on, or leaves it, when it comes back to rest.
                    Effect::FileTransfer(on) => {
                        let rests = !fastboot && !sideloading;
                        match (on, transfer.take()) {
                            (true, None) => match Transfer::start(cfg, &info, app.state.time_zone) {
                                Ok(t) => {
                                    transfer = Some(t);
                                    if rests {
                                        if let Err(e) = skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT) {
                                            skyward_sys::kmsg(&format!("the port did not take file transfer: {e:#}"))?;
                                            app.report(&format!("{e:#}"), now);
                                        }
                                    }
                                }
                                Err(e) => {
                                    skyward_sys::kmsg(&format!("file transfer did not start: {e:#}"))?;
                                    app.file_transfer_failed(false, &skyward_ui::fmt::transfer_failed(app.state.lang, &format!("{e:#}")), now);
                                }
                            },
                            (false, Some(t)) => {
                                if let Err(e) = t.stop(rests) {
                                    skyward_sys::kmsg(&format!("file transfer did not stop cleanly: {e:#}"))?;
                                    app.report(&skyward_ui::fmt::transfer_left_on(app.state.lang, &format!("{e:#}")), now);
                                }
                            }
                            (on, t) => unreachable!("the switch turned file transfer {on} while it was {}", t.is_some()),
                        }
                    }
                }
            }
        }
        // Internal storage goes to the computer while it is open, and no
        // erasing or installing could take it away. File transfer that does
        // not follow, or whose service ended, stops, and the page says why.
        let kept = match &mut transfer {
            Some(t) if !t.service.running() => Err(anyhow!("the file transfer service ended")),
            Some(t) => {
                let offer = app.state.data.storage_open() && eraser.is_none() && installer.is_none();
                t.offer(offer, app.state.lang).and_then(|()| {
                    if t.zone != app.state.time_zone {
                        t.service.set_zone(skyward_ui::fmt::rules(app.state.time_zone).clone())?;
                        t.zone = app.state.time_zone;
                    }
                    Ok(())
                })
            }
            None => Ok(()),
        };
        if let Err(e) = kept {
            // An ended service's own error says more than its ending does.
            let why = match transfer.take().expect("only file transfer that runs fails").stop(!fastboot && !sideloading) {
                Err(cause) => cause,
                Ok(()) => e,
            };
            skyward_sys::kmsg(&format!("file transfer stops: {why:#}"))?;
            app.file_transfer_failed(false, &skyward_ui::fmt::transfer_stopped(app.state.lang, &format!("{why:#}")), now);
        }
    }
}
