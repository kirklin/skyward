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
use crate::settings;
use crate::systems;
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
use skyward_net::wlan::{Power as WlanPower, Stage as Joining, Wlan};
use skyward_sys::sleep::{Sleeper, Slept};
use skyward_sys::uevent;
use skyward_systems::Who;
use skyward_ui::app::{
    App, Asked, BackupAt, BackupEnd, BackupEntry, BackupRefusal, ClashChoice, Data, DownloadEnd, Effect, EraseStop, FileKind, FileOp,
    FileRow, FileTask, FsAction, FsRow, Have, Held, ItemEnd, ItemState, JobEnd, Keys, MountFailure, Part, PartKind, PartStop, Place,
    RenameRefused, Screen, Shot, SkywardEnd, SkywardStage, Target, UsbDrive, UsbLink, Wipe, DATA_OPENING, DATA_PARTITION, DOWNLOADS,
    INSTALL_UNFINISHED, USB_DRIVE,
};
use skyward_ui::settings::Keeper;
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

/// How long a panel gone dark rests, ready to light again at once, before
/// the display turns off; the phone's own system keeps its panel dozing
/// about as long after the screen goes off.
const REST: Duration = Duration::from_secs(5);

/// Where logs saved to the temporary folder go, in the recovery service's
/// own folder, and where in internal storage the others do.
const TMP_LOGS: &str = "/skyward/run/logs";
const STORAGE_LOGS: &str = "Skyward/logs";

/// The folder of internal storage the system saves its own screenshots in,
/// which the gallery shows as its screenshots, and where in the recovery
/// service's own folder a screenshot waits for internal storage to open.
const SCREENSHOTS: &str = "DCIM/Screenshots";
const HELD_SCREENSHOTS: &str = "/skyward/run/screenshots";

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
    Ok((running, usb_link()?))
}

/// Light the panel, as on a cable going in or out, when the battery as it
/// `was` and as it `is` disagree on whether a charger or a computer powers
/// the phone.
fn plug(app: &mut App, was: &Option<skyward_info::Battery>, is: &Option<skyward_info::Battery>, now: u64) -> Result<()> {
    let plugged = |b: &Option<skyward_info::Battery>| b.as_ref().is_some_and(|b| b.plugged);
    if plugged(was) != plugged(is) {
        skyward_sys::kmsg(if plugged(is) { "a charger or a computer powers the phone" } else { "the phone runs on its battery" })?;
        app.plugged(now, plugged(is));
    }
    Ok(())
}

/// The phone's readings that could not be taken, and why the log was last
/// told each could not: the battery's, read each second, and all the
/// phone's details, read less often and after what changes them.
#[derive(Default)]
struct Untaken {
    battery: skyward_sys::ToldOnce,
    details: skyward_sys::ToldOnce,
}

/// Take in how reading `what` went, `why` it failed where it did. A reading
/// that cannot be taken leaves the pages with the one they have, and what
/// is under way goes on; the log is told why, once for as long as the
/// reason stays the same.
fn untaken(told: &mut skyward_sys::ToldOnce, what: &str, why: Option<&anyhow::Error>) -> Result<()> {
    let why = why.map(|e| format!("{e:#}"));
    if told.untold(why.as_deref()) {
        skyward_sys::kmsg(&format!("{what} could not be read, and the pages keep what they have: {}", why.unwrap_or_default()))?;
    }
    Ok(())
}

/// Read all the phone's details anew into `info`, which keeps what it has
/// where they cannot be read.
fn read_details(cfg: &DeviceConfig, info: &mut skyward_info::DeviceInfo, told: &mut skyward_sys::ToldOnce) -> Result<()> {
    let read = skyward_info::collect(cfg);
    untaken(told, "the phone's details", read.as_ref().err())?;
    if let Ok(read) = read {
        *info = read;
    }
    Ok(())
}

/// How far a computer has the USB port, as the port's controller tells.
fn usb_link() -> Result<UsbLink> {
    let controller = skyward_sys::prop("sys.usb.controller").context("init names no USB controller")?;
    let path = Path::new("/sys/class/udc").join(controller).join("state");
    let state = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(match state.trim() {
        "configured" => UsbLink::Connected,
        "not attached" => UsbLink::Unplugged,
        _ => UsbLink::Connecting,
    })
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
fn install_asked(programs: &[Service], app: &App, pending: Pending) -> Result<Installer> {
    if pending.needs == Needs::Cache {
        let cache = wipe::cache()?.context("the phone has no cache partition")?;
        let at = Path::new(&cache.mount_point);
        if !mount::is_mount_point(at)? {
            fs::create_dir_all(at).with_context(|| format!("make {}", at.display()))?;
            mount::mount_ro(Path::new(&fstab::device(&cache)?), at)?;
        }
    }
    Installer::asked(pending.package, pending.retry, detach(programs.to_vec()), install_options(app))
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
        // The second system starts only from the slot its boot set was put
        // in, and the installed system's goes back there first.
        skyward_systems::install::ensure_installed()?;
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

/// How work on files ended, as its page tells it.
fn job_end(end: filework::End) -> JobEnd {
    match end {
        filework::End::Done => JobEnd::Done,
        filework::End::Stopped => JobEnd::Stopped,
        filework::End::Failed(why) => JobEnd::Failed(why),
    }
}

/// Run the pages. What `request`, read from misc, asks for decides the
/// first of them; asking for nothing, they start at `start`. `programs`
/// are the phone's own that the recovery keeps running: the
/// configuration's, and those the phone's partitions name for its keys.
pub fn run(cfg: &DeviceConfig, programs: &[Service], font: &Path, mono: &Path, request: bcb::Request, start: Screen, dump: Option<&Path>) -> Result<()> {
    let mut pages = Pages::open(cfg, programs, font, mono, request, start, dump)?;
    pages.begin()?;
    // The phone sleeps while its screen is off and nothing needs it awake.
    let sleeper = Sleeper::start()?;
    // A uevent may be a charger or a computer attached or taken away: the
    // battery is read at once, and the phone stays awake until it is.
    let uevents = uevent::Listener::open()?;
    loop {
        pages.turn(&sleeper, &uevents)?;
    }
}

/// The pages, and all the loop that runs them holds from one turn to the
/// next: the panel and what is drawn on it, what the phone tells of
/// itself, the request from misc, and the work under way, each on a thread
/// of its own.
struct Pages<'a> {
    cfg: &'a DeviceConfig,
    /// The phone's own programs that the recovery keeps running.
    programs: &'a [Service],
    /// What misc asked for as the recovery started.
    request: bcb::Request,
    /// Where each frame shown is written as well.
    dump: Option<&'a Path>,
    app: App,
    info: skyward_info::DeviceInfo,
    unread: Untaken,
    fonts: Fonts,
    surface: KmsSurface,
    /// The frame last drawn.
    screen: Canvas,
    inputs: Inputs,
    started: Instant,
    /// Whether the wait for a touch panel to come up is over.
    touch_known: bool,
    /// adbd and fastbootd ask over it to go over to fastboot and back.
    control: Option<Control>,
    haptics: Option<Haptics>,
    backlight: Backlight,
    keeper: Keeper,
    shown_clock: String,
    /// The page shown first, until it has been told it shows.
    arriving: Option<Incoming>,
    /// The readings are taken again on a thread of their own; those taken
    /// here as work ends say how it left the phone, and readings started
    /// before them are out of date.
    readings: Readings,
    read_at: Instant,
    /// When the battery was asked for at once, after a uevent; the phone
    /// stays awake until it is read.
    asked: Option<Instant>,
    /// Whether the panel's frame memory has been blacked out for an off
    /// panel; cleared once a real frame is shown again.
    blanked: bool,
    /// When the panel gone dark began to rest.
    rested_at: Option<Instant>,
    /// Started for fastboot, the port goes to the phone's own fastbootd. The
    /// page says whether it came up and how far a computer has the port;
    /// why it did not come up goes to the kernel log and on screen.
    fastboot: bool,
    port_at: Instant,
    /// File transfer runs while its switch is on, and offers internal
    /// storage while that is open; one that does not start says why on
    /// screen, and the recovery carries on.
    transfer: Option<Transfer>,
    /// A sideload has the port, which goes back to rest once it ends.
    sideloading: bool,
    misc: PathBuf,
    /// Work the request asks for that goes on keeps the request in misc,
    /// so a restart halfway comes back to it, until it is done or the user
    /// leaves it for somewhere else.
    kept: bool,
    /// Once the work the system asked for is done, the page shows it for a
    /// moment; then the phone starts the system, or powers off where the
    /// request says to.
    leave: Target,
    leave_at: Option<u64>,
    installer: Option<Installer>,
    /// The install the system asked for, until the data partition it is in
    /// opens, or the user whose own files hold it unlocks them.
    pending: Option<Pending>,
    /// Where rescue asked the phone to leave for, once it lets USB go.
    rescue_leaves: Option<(Target, bool)>,
    eraser: Option<Eraser>,
    /// Erasing the system asked for leaves on its own once it is done, as
    /// does erasing an update asked for, sideloaded with a restart after it;
    /// what the user chose waits for them.
    erase_leaves: bool,
    /// Erasing all user data rescue asked for, whose end goes back to rescue
    /// rather than to a page of its own.
    rescue_erasing: bool,
    /// Opening the data partition, or a user's own files in it, under way,
    /// and the users whose own files are still closed once it is open.
    decryptor: Option<Decryptor>,
    closed: Vec<Closed>,
    /// Work on whole partitions under way.
    part_worker: Option<partwork::Worker>,
    /// Copying, moving or deleting under way.
    worker: Option<filework::Worker>,
    /// Giving the data partition's files their SELinux labels under way.
    relabeler: Option<filework::Worker>,
    /// Backing up or restoring under way.
    backuper: Option<backup::Worker>,
    /// An update's merge followed: one the recovery found under way, which
    /// the kernel took up again as the snapshots were mapped, or one started
    /// on the update page.
    merger: Option<merge::Merger>,
    /// The second system's work, on a phone that can hold one.
    systems_worker: Option<systems::Worker>,
    /// A switch the system asked for, under way; then the phone starts the
    /// system switched to.
    switch_asked: bool,
    /// A switch went through and the phone is restarting into the system.
    switched: bool,
    /// The DNS servers the network joined gave, for looking names up.
    servers: Servers,
    /// The WLAN, on a phone whose driver the recovery brings up.
    wlan: Option<wlan::Worker>,
    scanned_at: Instant,
    /// Asking the update server, and downloading the full package.
    checker: Option<updates::Worker>,
    downloader: Option<updates::Worker>,
    /// Asking GitHub for Skyward's newest release, and updating to it.
    skyward_checker: Option<updates::Worker>,
    skyward_updater: Option<updates::Worker>,
    /// The terminal page's shell, from when the page first asks for one.
    shell: Option<pty::Shell>,
    /// The USB drive plugged in, looked for each second.
    plugged: Option<Plugged>,
    seen: Option<String>,
    drive_at: Instant,
}

impl<'a> Pages<'a> {
    /// Bring up the panel, the touchscreen and the pages, as the settings
    /// kept from the starts before leave them, with none of the work the
    /// request asks for started yet.
    fn open(cfg: &'a DeviceConfig, programs: &'a [Service], font: &Path, mono: &Path, request: bcb::Request, start: Screen, dump: Option<&'a Path>) -> Result<Pages<'a>> {
        if cfg.display.density == 0 {
            return Err(anyhow!("{}: [display] density is required to lay out the UI", cfg.device.codename));
        }
        // The pages come up without a battery that cannot be read, as while no
        // driver for it is loaded; the readings taken each second bring it in
        // once it can.
        let mut unread = Untaken::default();
        let battery = skyward_info::battery_now();
        untaken(&mut unread.battery, "the battery", battery.as_ref().err())?;
        let info = skyward_info::collect_with(cfg, battery.unwrap_or_default())?;
        let fonts = Fonts::load(font, 0, mono)?;
        let mut surface = KmsSurface::open("/dev/dri/card0")?;
        let (w, h) = surface.size();
        // A touch controller that follows the panel wakes as the panel goes
        // from rest to running, and at no other time. The recovery service
        // running again finds it as the earlier run left it, at rest where the
        // panel rested then, so the panel is put to rest and woken with a dark
        // frame, as every later wake wakes it, before the touchscreen is taken
        // in and told its settings: a controller at rest answers none of them.
        if skyward_sys::runs_again() {
            surface.rest()?;
            surface.present(&Canvas::new(w, h))?;
            surface.settle()?;
        }
        eprintln!("display {}", surface.describe());
        let inputs = Inputs::open(w, h, cfg.touch.writes.clone())?;
        let started = Instant::now();
        let control = Control::open()?;
        let haptics = cfg.haptics.as_ref().map(Haptics::open).transpose()?;
        let backlight = Backlight::open()?;
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
            | Work::UnshareBlocks
            | Work::SwitchSystem { .. }
            | Work::SystemStopped => start,
        };
        let mut app = App::new(first, w, h, cfg.display.density as f32 / 160.0);
        app.set_screen_corners(cfg.display.corner_radius as f32);
        // The settings kept from the starts before are back before the first
        // frame, and before file transfer starts with its switch and the time
        // zone. The pages speak the language chosen then, or else the one the
        // system named with the work it asks for, or the one the installed
        // system is built to start in, or else the one they last spoke.
        let kept = settings::read()?;
        let locale = request.locale.as_deref().or(info.locale.as_deref());
        let named = locale.map(Lang::for_locale).transpose()?;
        app.take_settings(kept.settings(), named);
        let keeper = Keeper::new(&kept);
        skyward_sys::kmsg(&format!(
            "the pages speak {:?}, {}, and the locale named is {}",
            app.state.lang,
            if app.state.language_chosen { "the language chosen" } else { "with none chosen" },
            locale.unwrap_or("none")
        ))?;
        // The home page tells of a restart into the slot the system does not
        // run from now: an update or a switch left for the next boot.
        if info.slot.is_some() {
            app.set_boot_slots(read_slots());
        }
        app.set_asked(asked(&request));
        let screen = Canvas::new(w, h);
        // A page sliding over another draws on three screens' worth of canvas
        // besides the panel's: made ready now, the first slide finds them so.
        Canvas::keep_ready(w, h, 3);
        let readings = Readings::start(cfg)?;
        let read_at = Instant::now();
        let fastboot = first == Screen::Fastboot;
        let port_at = Instant::now();
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
        if fastboot {
            if let Err(e) = skyward_sys::usb::switch("fastboot", USB_TIMEOUT) {
                skyward_sys::kmsg(&format!("fastbootd has not taken the port: {e:#}"))?;
                app.report(&format!("{e:#}"), monotonic_ms());
            }
            let (running, link) = fastboot_status()?;
            app.set_fastboot(running, link);
        }
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
        // has them, and the install page then offers to keep it there. Such a
        // phone holds a second system where it also has boot and init_boot
        // partitions in both slots, which a switch writes.
        let by_name = Path::new(skyward_block::BY_NAME);
        let both = |base: &str| -> Result<bool> { Ok(by_name.join(format!("{base}_a")).try_exists()? && by_name.join(format!("{base}_b")).try_exists()?) };
        app.state.recovery_partitions = both("recovery")?;
        app.state.dual_boot = app.state.recovery_partitions && both("boot")? && both("init_boot")?;
        // The second system's work; the systems are read at once, for the home
        // page to name the one that starts, once a switch cut off before is
        // finished.
        let systems_worker = match app.state.dual_boot {
            true => Some(systems::Worker::start(cfg.device.os_version_prop.clone())?),
            false => None,
        };
        // fastbootd may write the boot partitions, which the installed system's
        // boot set goes back into first, and finds /metadata as it was; why the
        // boot set could not go back is shown, and fastboot goes on, as the way
        // left to repair the phone.
        if fastboot && app.state.dual_boot {
            let metadata = Path::new("/metadata");
            let mounted = mount::is_mount_point(metadata)?;
            let put_back = skyward_systems::install::ensure_installed();
            if !mounted && mount::is_mount_point(metadata)? {
                mount::umount(metadata)?;
            }
            if let Err(e) = put_back {
                skyward_sys::kmsg(&format!("the installed system's boot set did not go back before fastboot: {e:#}"))?;
                app.report(&format!("{e:#}"), monotonic_ms());
            }
        }
        Ok(Pages {
            cfg,
            programs,
            dump,
            app,
            info,
            unread,
            fonts,
            surface,
            screen,
            inputs,
            started,
            touch_known: false,
            control,
            haptics,
            backlight,
            keeper,
            shown_clock: String::new(),
            arriving: Some(Incoming::Arrived),
            readings,
            read_at,
            asked: None,
            blanked: false,
            rested_at: None,
            fastboot,
            port_at,
            transfer,
            sideloading: false,
            misc: bcb::misc(),
            kept: request.work.lasts(),
            leave: if request.shutdown_after { Target::PowerOff } else { Target::System },
            leave_at: None,
            request,
            installer: None,
            pending: None,
            rescue_leaves: None,
            eraser: None,
            erase_leaves: false,
            rescue_erasing: false,
            decryptor: None,
            closed: Vec::new(),
            part_worker: None,
            worker: None,
            relabeler: None,
            backuper: None,
            merger: None,
            systems_worker,
            switch_asked: false,
            switched: false,
            servers: Arc::new(Mutex::new(Vec::new())),
            wlan: None,
            scanned_at: Instant::now(),
            checker: None,
            downloader: None,
            skyward_checker: None,
            skyward_updater: None,
            shell: None,
            plugged: None,
            seen: None,
            drive_at: Instant::now(),
        })
    }

    /// Start the work the request in misc asks for, take up what an earlier
    /// start left under way, and bring the port and the WLAN to where the
    /// pages start from.
    fn begin(&mut self) -> Result<()> {
        let cfg = self.cfg;
        match &self.request.work {
            Work::WipeData { reformat } => {
                self.eraser = Some(factory_reset(&mut self.app, self.request.keep_memtag_mode, true, reformat.clone())?);
                self.erase_leaves = true;
            }
            Work::WipeCache { reformat } => {
                self.eraser = Some(parts_wipe(&mut self.app, vec![Step::Cache], &cfg.wipe.logs, true, reformat.clone())?);
                self.erase_leaves = true;
            }
            Work::Sideload { .. } => {
                self.app.start_sideload();
                self.installer = Some(Installer::start(detach(self.programs.to_vec()), install_options(&self.app))?);
                self.sideloading = true;
            }
            Work::Rescue => {
                self.app.start_rescue();
                self.installer = Some(Installer::rescue(detach(self.programs.to_vec()), install_options(&self.app))?);
                self.sideloading = true;
            }
            Work::Update(update) => {
                self.app.start_asked(update.security, update.package.clone());
                let retry = update.retry > 0;
                if !retry && panicked() {
                    skyward_sys::kmsg("the phone came here after its kernel stopped, so the update asked for is not installed")?;
                    self.app.install(skyward_install::Event::Finished(skyward_install::Outcome::Refused(skyward_install::Refusal::Panicked)), monotonic_ms());
                    bcb::clear(&self.misc)?;
                    self.kept = false;
                } else {
                    let (package, needs) = asked::locate(&update.package, self.app.state.cache_partition);
                    skyward_sys::kmsg(&format!("the update asked for is {package:?}, which needs {needs:?}"))?;
                    let wanted = Pending { package, needs, retry };
                    match needs {
                        Needs::Data | Needs::User(_) => {
                            self.app.asked_opening();
                            self.decryptor = Some(Decryptor::start(decrypt::Job::Open)?);
                            self.pending = Some(wanted);
                        }
                        Needs::Nothing | Needs::Cache => self.installer = Some(install_asked(self.programs, &self.app, wanted)?),
                    }
                }
            }
            Work::WipeAb { package_size } => {
                self.app.part_work_started(PartKind::Disable, true);
                self.part_worker = Some(partwork::Worker::start(partwork::Task::Disable { package_size: *package_size })?);
            }
            Work::UnshareBlocks => {
                let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                self.app.part_work_started(PartKind::Unshare, true);
                self.part_worker = Some(partwork::Worker::start(partwork::Task::Unshare { slot })?);
            }
            // The installed system is asked for by a word, and the second by
            // its id.
            Work::SwitchSystem { to } => match self.systems_worker.as_mut() {
                Some(worker) => {
                    self.app.switch_asked(if to == "installed" { Who::Installed } else { Who::Other });
                    worker.send(systems::Task::SwitchNamed(to.clone()));
                    self.switch_asked = true;
                }
                // A phone that cannot hold a second system has none to switch
                // to, and the request is done with.
                None => {
                    skyward_sys::kmsg(&format!("asked to switch to the system {to:?}, which this phone cannot hold"))?;
                    bcb::clear(&self.misc)?;
                    self.kept = false;
                }
            },
            // The second system's start stopped and brought the recovery up to
            // say why; it still starts next, unless the user switches.
            Work::SystemStopped => {
                let why = self.request.reason.clone().unwrap_or_default();
                skyward_sys::kmsg(&format!("the second system's start stopped: {why}"))?;
                if self.app.state.dual_boot {
                    self.app.system_stopped(why);
                }
            }
            Work::Menu | Work::Exit | Work::Fastboot | Work::PromptWipeData | Work::Unsupported(_) => {}
        }
        // A switch asked for finishes one cut off before itself; fastboot leaves
        // /metadata to fastbootd until the recovery's pages come up.
        if let (Some(worker), false, false) = (self.systems_worker.as_mut(), self.switch_asked, self.fastboot) {
            worker.send(systems::Task::Resume);
        }
        // The port came up with adb alone; at rest it offers file transfer
        // beside it.
        if self.transfer.is_some()
            && !self.fastboot
            && !self.sideloading
            && let Err(e) = skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT)
        {
            skyward_sys::kmsg(&format!("the port did not take file transfer: {e:#}"))?;
            self.app.report(&format!("{e:#}"), monotonic_ms());
        }
        if let Some(slot) = self.info.slot {
            let standing = skyward_vab::standing(slot)?;
            if matches!(standing, Standing::Merging(_)) && skyward_vab::snapshots_mapped(slot)? {
                skyward_sys::kmsg("the update's merge goes on")?;
                self.merger = Some(merge::Merger::start(slot, false)?);
            }
            self.app.set_update(standing);
        }
        // The WLAN starts off. Without the bundle of the vendor interface's
        // libraries the supplicant does not start, and the WLAN page says why.
        if let Some(w) = &cfg.wlan {
            self.app.state.wlan = Some(Wlan::default());
            let modules = cfg.modules.sources.iter().map(|s| skyward_modules::source_dir(s)).collect::<Result<Vec<_>>>()?;
            let libraries = secure::libraries().unwrap_or_else(|e| {
                let _ = skyward_sys::kmsg(&format!("the vendor interface's libraries are not open: {e:#}"));
                None
            });
            self.wlan = Some(wlan::Worker::start(w.clone(), modules, libraries, self.servers.clone())?);
        }
        self.scanned_at = Instant::now();
        self.app.state.update_server = cfg.update.is_some();
        self.drive_at = Instant::now();
        Ok(())
    }

    /// One turn of the loop: the page is shown as it stands, the loop waits
    /// for something to happen, and what happened is taken in. What the work
    /// under way told comes first, so a touch acts on the page as it now
    /// stands.
    fn turn(&mut self, sleeper: &Sleeper, uevents: &uevent::Listener) -> Result<()> {
        let now = monotonic_ms();
        if self.leave_at.is_some_and(|at| now >= at) {
            self.leave_at = None;
            power(self.leave)?;
        }
        if !self.touch_known && self.started.elapsed() >= TOUCH_WAIT {
            self.touch_known = true;
            if !self.inputs.has_panel() {
                skyward_sys::kmsg("no touch panel came up; the pages are worked with the keys")?;
                self.app.touchscreen_missing();
            }
        }
        self.tick(now)?;
        self.take_readings(now)?;
        // In fastboot the page follows fastbootd and the cable each second.
        if self.fastboot && self.port_at.elapsed().as_secs() >= 1 {
            let (running, link) = fastboot_status()?;
            self.app.set_fastboot(running, link);
            self.port_at = Instant::now();
        }
        let (lit, shown) = self.show(now)?;
        self.wait(sleeper, uevents, now, lit, shown)?;
        let now = monotonic_ms();
        for slept in sleeper.slept() {
            match slept {
                Slept::Woke => skyward_sys::kmsg("the phone slept and woke")?,
                Slept::Refused(why) => skyward_sys::kmsg(&format!("the phone did not sleep: {why}"))?,
            }
        }
        if uevents.heard()? {
            self.readings.ask();
            self.asked = Some(Instant::now());
        }
        let mut drive_told = None;
        if self.drive_at.elapsed().as_secs() >= 1 {
            drive_told = look_for_drive(&mut self.plugged, &mut self.seen)?;
            self.drive_at = Instant::now();
        }
        self.installer_told(now)?;
        self.eraser_told(now)?;
        self.files_told(now)?;
        self.relabel_told(now);
        self.parts_told(now)?;
        self.backup_told(now);
        self.merge_told(now)?;
        self.systems_told(now)?;
        self.wlan_told()?;
        self.keep_settings(now)?;
        self.updates_told(now)?;
        self.skyward_told(now)?;
        self.decrypt_told(now)?;
        self.shell_told()?;
        self.take_in(drive_told, now)?;
        self.offer_storage(now)
    }

    /// What time passing asks for: the motor's clicks and ticks, and the
    /// status bar's clock drawn again as its minute turns.
    fn tick(&mut self, now: u64) -> Result<()> {
        for fx in self.app.tick(now) {
            match fx {
                Effect::Haptic => self.click()?,
                // The power key held past its wait for volume down turns
                // the screen off.
                Effect::Ticks => self.ticks()?,
                other => unreachable!("time passing asks for nothing but the motor, not {other:?}"),
            }
        }
        let time = clock(self.app.state.time_zone)?;
        if time != self.shown_clock {
            self.shown_clock = time;
            self.app.redraw();
        }
        Ok(())
    }

    /// One click of the motor, as strong as the settings say.
    fn click(&mut self) -> Result<()> {
        if let Some(h) = &mut self.haptics {
            h.click(self.app.state.vibration)?;
        }
        Ok(())
    }

    /// The motor's ticks of the screen going off.
    fn ticks(&mut self) -> Result<()> {
        if let Some(h) = &mut self.haptics {
            h.ticks()?;
        }
        Ok(())
    }

    /// Take in the readings taken since the last turn.
    fn take_readings(&mut self, now: u64) -> Result<()> {
        for reading in self.readings.taken() {
            match reading {
                Reading::Battery(battery, at) => {
                    if self.asked.is_some_and(|asked| at >= asked) {
                        self.asked = None;
                    }
                    untaken(&mut self.unread.battery, "the battery", battery.as_ref().err())?;
                    if let Ok(battery) = battery
                        && battery != self.info.battery
                    {
                        plug(&mut self.app, &self.info.battery, &battery, now)?;
                        self.info.battery = battery;
                        self.app.refresh();
                    }
                }
                Reading::All(all, at) => {
                    if self.asked.is_some_and(|asked| at >= asked) {
                        self.asked = None;
                    }
                    untaken(&mut self.unread.details, "the phone's details", all.as_ref().as_ref().err())?;
                    if let Ok(all) = *all
                        && at >= self.read_at
                        && all != self.info
                    {
                        plug(&mut self.app, &self.info.battery, &all.battery, now)?;
                        self.info = all;
                        self.app.refresh();
                    }
                }
            }
        }
        Ok(())
    }

    /// Put the page on the panel where it changed, and light the panel,
    /// rest it or turn it off as the page asks. Gives whether the panel is
    /// lit, and whether a frame was shown.
    fn show(&mut self, now: u64) -> Result<(bool, bool)> {
        let panel = self.app.panel(now);
        let mut shown = false;
        if panel.is_some() {
            // A display turned off comes on again with a frame drawn for it.
            if self.app.take_dirty() || !self.surface.is_on() {
                let t0 = Instant::now();
                self.app.compose(&mut self.screen, &self.fonts, &self.info, &self.shown_clock, now);
                self.surface.present(&self.screen)?;
                eprintln!("frame {:?} in {} ms", self.app.screen(), t0.elapsed().as_millis());
                if let Some(path) = self.dump {
                    fs::write(path, &self.screen.data).with_context(|| format!("write {}", path.display()))?;
                }
                shown = true;
            }
            self.blanked = false;
        } else if !self.blanked {
            // Black out the panel's frame memory before the backlight goes
            // to 0 and the display off, so an OLED that floors the backlight
            // above zero rather than cutting it shows nothing as it goes off.
            self.screen.data.fill(0);
            self.surface.present(&self.screen)?;
            self.surface.settle()?;
            self.blanked = true;
        }
        // After the frame is on the panel, so a panel coming back on lights
        // with what it should show now, never with what it showed when it
        // went off.
        if self.backlight.level == 0 && panel.is_some() {
            self.surface.settle()?;
        }
        self.backlight.show(panel)?;
        if panel.is_some() {
            // Lit, a panel that came on again switches to its fastest
            // refresh.
            self.surface.speed_up()?;
            self.rested_at = None;
        } else if self.surface.is_on() {
            // With the backlight off, the panel rests, and the touch
            // controller following it stops drawing power.
            self.surface.rest()?;
            self.rested_at = Some(Instant::now());
        } else if self.rested_at.is_some_and(|at| at.elapsed() >= REST) {
            // Rested long enough, the display goes off: the panel and the
            // display processor feeding it stop drawing power.
            self.surface.turn_off()?;
            self.rested_at = None;
        }
        Ok((panel.is_some(), shown))
    }

    /// Wait until something the loop listens to has something, or the page
    /// is due to change; the phone is let sleep meanwhile where nothing
    /// needs it awake. `lit` and `shown` are as [`Pages::show`] gave them.
    fn wait(&mut self, sleeper: &Sleeper, uevents: &uevent::Listener, now: u64, lit: bool, shown: bool) -> Result<()> {
        // A page at rest draws the rows it has yet to while nothing else
        // is to be done, a band at a time, looking for input between bands.
        let ahead = lit && !shown && self.app.draw_ahead(&self.fonts, &self.info);
        // A frame shown has waited for the one before it to reach the
        // panel, which keeps the loop to the panel's pace; otherwise the CPU
        // gets at least a millisecond.
        let floor = if shown { 0 } else { 1 };
        let until_due = self.app.deadline(now).map(|d| d.saturating_sub(now) as i32);
        // A resting panel turns off once its rest is over.
        let rest_left = self.rested_at.map(|at| REST.saturating_sub(at.elapsed()).as_millis() as i32);
        let timeout = if ahead { 0 } else { until_due.into_iter().chain(rest_left).min().map_or(1000, |t| t.clamp(floor, 1000)) };
        sleeper.allow(self.may_sleep()?);
        let mut fds = vec![self.inputs.fd(), uevents.fd(), sleeper.fd()];
        if let Some(i) = &self.installer {
            fds.push(i.fd());
        }
        if let Some(c) = &self.control {
            fds.push(c.fd());
        }
        if let Some(e) = &self.eraser {
            fds.push(e.fd());
        }
        if let Some(sh) = &self.shell {
            fds.extend(sh.fds());
        }
        if let Some(w) = &self.worker {
            fds.push(w.fd());
        }
        if let Some(r) = &self.relabeler {
            fds.push(r.fd());
        }
        if let Some(b) = &self.backuper {
            fds.push(b.fd());
        }
        if let Some(m) = &self.merger {
            fds.push(m.fd());
        }
        if let Some(d) = &self.decryptor {
            fds.push(d.fd());
        }
        if let Some(p) = &self.part_worker {
            fds.push(p.fd());
        }
        if let Some(w) = &self.wlan {
            fds.push(w.fd());
        }
        if let Some(c) = &self.checker {
            fds.push(c.fd());
        }
        if let Some(d) = &self.downloader {
            fds.push(d.fd());
        }
        if let Some(c) = &self.skyward_checker {
            fds.push(c.fd());
        }
        if let Some(u) = &self.skyward_updater {
            fds.push(u.fd());
        }
        if let Some(s) = &self.systems_worker {
            fds.push(s.fd());
        }
        fds.push(self.readings.fd());
        wait(&fds, timeout)
    }

    /// Whether the phone may sleep. With the display off, it sleeps unless
    /// something needs it awake: work under way, fastboot or sideload, a
    /// restart on its way, a command running in the terminal, a network
    /// being joined, the battery read after a uevent, or a computer on the
    /// USB port, whose adb and file transfer stay up.
    fn may_sleep(&self) -> Result<bool> {
        let systems_busy = self.systems_worker.as_ref().is_some_and(systems::Worker::busy);
        let busy = self.installer.is_some()
            || self.eraser.is_some()
            || self.worker.is_some()
            || self.relabeler.is_some()
            || self.backuper.is_some()
            || self.merger.is_some()
            || self.decryptor.is_some()
            || self.part_worker.is_some()
            || self.checker.is_some()
            || self.downloader.is_some()
            || self.skyward_checker.is_some()
            || self.skyward_updater.is_some()
            || systems_busy;
        let joining = self.app.state.wlan.as_ref().and_then(|w| w.joined.as_ref()).is_some_and(|j| matches!(j.stage, Joining::Joining | Joining::Addressing));
        let running = match &self.shell {
            Some(sh) => !sh.waiting()?,
            None => false,
        };
        Ok(self.surface.is_off()
            && !busy
            && !self.fastboot
            && !self.sideloading
            && self.leave_at.is_none()
            && !joining
            && !running
            && self.asked.is_none()
            && usb_link()? != UsbLink::Connected)
    }

    /// The request in misc is done with: a restart from here on starts the
    /// system rather than coming back to what it asked.
    fn done_with_request(&mut self) -> Result<()> {
        if self.kept {
            bcb::clear(&self.misc)?;
            self.kept = false;
        }
        Ok(())
    }

    /// The mount page shows the partitions as they are when it opens and
    /// after each change; why they cannot be read is the page's to say.
    fn mounts(&self) -> Result<Vec<skyward_ui::app::MountRow>, String> {
        read_mounts(self.programs).map_err(|e| format!("{e:#}"))
    }
}

/// What the work under way told since the last turn, each kind of work
/// taken in by a method of its own.
impl Pages<'_> {
    /// What the installer told: its page follows it, and its end starts
    /// what the package, the request or rescue asked for after it.
    fn installer_told(&mut self, now: u64) -> Result<()> {
        use skyward_install::{Event as Told, Outcome};
        let told = self.installer.as_ref().map(Installer::events).unwrap_or_default();
        for ev in told {
            let finished = matches!(ev, Told::Finished(_));
            let installed = matches!(ev, Told::Finished(Outcome::Installed { .. } | Outcome::Ran { .. }));
            let wipe_after = matches!(ev, Told::Finished(Outcome::Installed { wipe: true, .. }));
            let script_cache = matches!(ev, Told::Finished(Outcome::Ran { wipe_cache: true, .. }));
            // A second system installed is read for the pages to show.
            if matches!(ev, Told::Done(Outcome::SystemInstalled { .. }) | Told::Finished(Outcome::SystemInstalled { .. })) {
                self.systems_worker.as_mut().context("a second system went in on a phone that cannot hold one")?.send(systems::Task::Read);
            }
            eprintln!("install {ev:?}");
            match &ev {
                // Rescue's own erasing tells rescue how it went once it
                // ends; file transfer that will not let internal
                // storage go keeps it from starting.
                Told::Wipe => {
                    ensure!(self.eraser.is_none(), "erasing is under way already");
                    if withhold(&mut self.transfer, &mut self.app, now)? {
                        self.eraser = Some(rescue_wipe(&mut self.app)?);
                        self.rescue_erasing = true;
                    } else {
                        self.installer.as_ref().context("rescue asked for erasing")?.wiped(false);
                    }
                }
                Told::Reboot(target) => self.rescue_leaves = Some(rescue_target(target)),
                _ => {}
            }
            self.app.install(ev, now);
            if finished {
                self.installer = None;
                self.sideloading = false;
                // A sideload or an update the system asked for is done
                // with however it went, and so is rescue, once its page
                // is left or the computer asks the phone to leave. A
                // sideload asked to leaves on its own after it, and an
                // update that went in does, once what the package or the
                // request asked to be erased is.
                if matches!(self.request.work, Work::Sideload { .. } | Work::Update(_) | Work::Rescue) {
                    self.done_with_request()?;
                }
                let (leaves, cache_after) = match &self.request.work {
                    Work::Sideload { auto_reboot } => (*auto_reboot, script_cache),
                    Work::Update(update) => (installed, script_cache || (installed && update.wipe_cache)),
                    _ => (false, script_cache),
                };
                if cache_after && !self.app.state.cache_partition {
                    skyward_sys::kmsg("the install asked for the cache partition to be erased, and the phone has none")?;
                }
                if wipe_after {
                    ensure!(self.eraser.is_none(), "erasing is under way already");
                    self.eraser = Some(factory_reset(&mut self.app, false, false, None)?);
                    self.erase_leaves = leaves;
                } else if cache_after && self.app.state.cache_partition {
                    ensure!(self.eraser.is_none(), "erasing is under way already");
                    self.eraser = Some(parts_wipe(&mut self.app, vec![Step::Cache], &self.cfg.wipe.logs, false, None)?);
                    self.erase_leaves = leaves;
                } else if leaves {
                    self.leave_at = Some(now + LEAVE_AFTER_MS);
                }
                // Rescue asked to come back is told so in misc, as
                // Android's own recovery tells it.
                if let Some((target, again)) = self.rescue_leaves.take() {
                    if again {
                        bcb::write(&self.misc, &bcb::Bcb { command: "boot-rescue".into(), args: Vec::new() })?;
                    }
                    power(target)?;
                }
            }
        }
        Ok(())
    }

    /// What the eraser told: its page follows it, or rescue's does, and
    /// its end is the end of what asked for it.
    fn eraser_told(&mut self, now: u64) -> Result<()> {
        let told = self.eraser.as_ref().map(Eraser::events).unwrap_or_default();
        let mut erased = false;
        for ev in told {
            match ev {
                // Rescue's page says only that erasing is under way.
                wipe::Event::At(_) | wipe::Event::Progress(..) if self.rescue_erasing => {}
                wipe::Event::At(step) => self.app.erase_at(part(step)),
                wipe::Event::Progress(done, total) => self.app.erase_progress(done, total),
                // Rescue stays on: its request stays in misc, and the
                // computer hears how erasing went.
                wipe::Event::Ended(outcome) if self.rescue_erasing => {
                    let said = outcome.as_ref().map_or_else(wipe::said, |_| "done".to_string());
                    skyward_sys::kmsg(&format!("erasing for rescue ended: {said}"))?;
                    self.installer.as_ref().context("rescue waits for erasing to end")?.wiped(outcome.is_ok());
                    self.app.rescue_wiped(outcome.map_err(erase_stop), now);
                    if self.app.state.data == Data::Erased {
                        self.closed.clear();
                    }
                    self.rescue_erasing = false;
                    erased = true;
                }
                wipe::Event::Opened(opening) => {
                    if let Some(mounted) = &opening.mounted {
                        log_mounted(mounted)?;
                    }
                    self.closed = opening.closed;
                    show_closed(&mut self.app, &self.closed)?;
                }
                wipe::Event::Ended(outcome) => {
                    let said = outcome.as_ref().map_or_else(wipe::said, |_| "done".to_string());
                    skyward_sys::kmsg(&format!("erasing ended: {said}"))?;
                    // As with the phone's own recovery, the request is
                    // done with either way, so a restart starts the
                    // system rather than erasing again. A factory
                    // reset done is what an update that asked for one
                    // asked, too.
                    self.done_with_request()?;
                    if outcome.is_ok() && self.app.state.erase.reset {
                        bcb::clear(&self.misc)?;
                    }
                    if outcome.is_ok() && self.erase_leaves {
                        self.leave_at = Some(now + LEAVE_AFTER_MS);
                    }
                    self.app.erase_ended(outcome.map_err(erase_stop), now);
                    // Users of a data partition erased whole have no
                    // files left to open.
                    if self.app.state.data == Data::Erased {
                        self.closed.clear();
                    }
                    erased = true;
                }
            }
        }
        // Erasing all data makes /metadata afresh, and the settings with it
        // are written there again.
        if erased {
            self.eraser = None;
            self.keeper.remade();
        }
        Ok(())
    }

    /// What copying, moving or deleting told; once it has ended, the
    /// folder shown is read again for what the work changed.
    fn files_told(&mut self, now: u64) -> Result<()> {
        let told = self.worker.as_ref().map(filework::Worker::events).unwrap_or_default();
        let mut worked = false;
        for ev in told {
            match ev {
                filework::Event::Progress(p) => self.app.file_progress((p.bytes, p.total_bytes), (p.entries, p.total_entries)),
                filework::Event::Relabeling(_) => unreachable!("work on files gives no labels"),
                filework::Event::Ended(end) => {
                    self.app.file_job_ended(job_end(end), now);
                    worked = true;
                }
            }
        }
        if worked {
            self.worker = None;
            let folder = self.app.state.files.path.clone();
            list(&mut self.app, folder)?;
        }
        Ok(())
    }

    /// What giving the data partition's files their labels told.
    fn relabel_told(&mut self, now: u64) {
        let told = self.relabeler.as_ref().map(filework::Worker::events).unwrap_or_default();
        let mut relabeled = false;
        for ev in told {
            match ev {
                filework::Event::Relabeling(p) => self.app.relabel_progress(p.checked, p.total, p.changed),
                filework::Event::Progress(_) => unreachable!("relabeling moves no bytes"),
                filework::Event::Ended(end) => {
                    self.app.relabel_ended(job_end(end), now);
                    relabeled = true;
                }
            }
        }
        if relabeled {
            self.relabeler = None;
        }
    }

    /// What the work on whole partitions told.
    fn parts_told(&mut self, now: u64) -> Result<()> {
        let told = self.part_worker.as_ref().map(partwork::Worker::events).unwrap_or_default();
        let mut parted = false;
        for ev in told {
            match ev {
                partwork::Event::Listed(names) => self.app.part_listed(names),
                partwork::Event::At(index) => self.app.part_at(index),
                partwork::Event::Ended(index, item, said) => {
                    skyward_sys::kmsg(&format!("partition work: item {index} ended as {item:?}"))?;
                    self.app.part_ended(index, item_end(item), &said);
                }
                partwork::Event::Done(done) => {
                    skyward_sys::kmsg(&format!("partition work ended: {done:?}"))?;
                    let work = self.app.state.part_work.as_ref().context("partition work shows on its page")?;
                    let whole = done.is_ok() && !work.items.iter().any(|(_, s)| matches!(s, ItemState::Ended(ItemEnd::Failed(_))));
                    let data = work.items.iter().any(|(name, _)| name == DATA_PARTITION);
                    let asked = work.asked;
                    // What the system asked for is done with either way,
                    // and the phone leaves on its own once it went
                    // through every partition.
                    if asked {
                        self.done_with_request()?;
                        if whole {
                            self.leave_at = Some(now + LEAVE_AFTER_MS);
                        }
                    }
                    // Work on the data partition leaves it closed.
                    if data {
                        self.closed.clear();
                    }
                    self.app.part_work_ended(done.map_err(part_stop), now);
                    parted = true;
                }
            }
        }
        // Work on whole partitions may have erased or made afresh the one
        // the settings are kept on.
        if parted {
            self.part_worker = None;
            self.keeper.remade();
        }
        Ok(())
    }

    /// What backing up or restoring told.
    fn backup_told(&mut self, now: u64) {
        let told = self.backuper.as_ref().map(backup::Worker::events).unwrap_or_default();
        let mut backed = false;
        for ev in told {
            match ev {
                backup::Event::Plan(plan) => self.app.backup_plan(plan),
                backup::Event::At { item, done, of, checking } => self.app.backup_at(BackupAt { item, done, of, checking }),
                backup::Event::Ended(end) => {
                    self.app.backup_ended(backup_end(end), now);
                    backed = true;
                }
            }
        }
        if backed {
            self.backuper = None;
        }
    }

    /// What the merge followed told.
    fn merge_told(&mut self, now: u64) -> Result<()> {
        let Some(m) = &self.merger else { return Ok(()) };
        let slot = self.info.slot.context("only an A/B phone merges an update")?;
        for ev in m.events() {
            match ev {
                merge::Event::Looked(look) => {
                    eprintln!("merge {look:?}");
                    self.app.set_update(match look {
                        Merge::Going(done) => Standing::Merging(done),
                        Merge::Done | Merge::NeedsReboot => Standing::Merged,
                        Merge::Failed(code) => Standing::Failed(code),
                        Merge::Cancelled | Merge::Idle(_) => skyward_vab::standing(slot)?,
                    });
                }
                merge::Event::Failed(why) => {
                    skyward_sys::kmsg(&format!("the update did not merge: {why}"))?;
                    self.app.report(&skyward_ui::fmt::merge_failed(self.app.state.lang, &why), now);
                    self.app.set_update(skyward_vab::standing(slot)?);
                }
            }
        }
        // Merged or not, the phone's readings say where the update
        // stands now.
        if m.ended() {
            self.merger = None;
            self.read_at = Instant::now();
            read_details(self.cfg, &mut self.info, &mut self.unread.details)?;
            self.app.refresh();
        }
        Ok(())
    }

    /// What the work on the two systems told.
    fn systems_told(&mut self, now: u64) -> Result<()> {
        let Some(worker) = &mut self.systems_worker else { return Ok(()) };
        for ev in worker.events() {
            let lang = self.app.state.lang;
            match ev {
                systems::Event::Resumed(to) => {
                    skyward_sys::kmsg(&format!("a switch cut off before is finished, and the {to:?} system starts next"))?;
                    self.app.report(skyward_ui::fmt::switch_resumed(lang), now);
                }
                systems::Event::NotResumed(why) => {
                    skyward_sys::kmsg(&format!("a switch cut off before is not finished: {why}"))?;
                    self.app.report(&skyward_ui::fmt::switch_unfinished(lang, &why), now);
                }
                systems::Event::Switched(to) => {
                    skyward_sys::kmsg(&format!("the {to:?} system starts next"))?;
                    // A switch the system asked for is done with.
                    if self.switch_asked {
                        self.done_with_request()?;
                    }
                    self.switch_asked = false;
                    self.switched = true;
                    power(Target::System)?;
                }
                systems::Event::Failed(why) => {
                    skyward_sys::kmsg(&format!("work on the systems failed: {why}"))?;
                    if self.switch_asked {
                        self.done_with_request()?;
                    }
                    self.switch_asked = false;
                    self.app.systems_work_failed(&why, now);
                }
                // The page stays as it is while the phone restarts.
                systems::Event::Read(read) => {
                    if !self.switched {
                        self.app.set_systems(read);
                    }
                }
            }
        }
        Ok(())
    }

    /// What the WLAN told; its page scans again now and then while it
    /// shows, as the stock settings do.
    fn wlan_told(&mut self) -> Result<()> {
        if let Some((told, traffic)) = self.wlan.as_ref().map(wlan::Worker::told) {
            if let Some(w) = told {
                self.app.set_wlan(w);
            }
            if let Some(t) = traffic {
                self.app.set_traffic(t);
            }
        }
        let wlan_on = self.app.state.wlan.as_ref().is_some_and(|w| w.power == WlanPower::On);
        if self.app.screen() == Screen::Wlan && wlan_on && self.scanned_at.elapsed() >= SCAN_EVERY {
            self.scanned_at = Instant::now();
            wlan_worker(&self.wlan)?.send(wlan::Command::Scan);
        }
        Ok(())
    }

    /// Settings changed go into their file a moment after the last change,
    /// while nothing else has /metadata: not in fastboot, whose fastbootd
    /// may, nor while an install, a merge, erasing, a backup or work on
    /// whole partitions is under way.
    fn keep_settings(&mut self, now: u64) -> Result<()> {
        let metadata_busy = self.fastboot
            || self.installer.is_some()
            || self.merger.is_some()
            || self.eraser.is_some()
            || self.backuper.is_some()
            || self.part_worker.is_some()
            || self.systems_worker.as_ref().is_some_and(systems::Worker::busy);
        if !metadata_busy
            && let Some(due) = self.keeper.due(self.app.settings(), now)
        {
            match settings::write(&due) {
                Ok(()) => {
                    skyward_sys::kmsg("the settings are kept")?;
                    self.keeper.written(due);
                }
                Err(e) => {
                    skyward_sys::kmsg(&format!("the settings were not kept: {e:#}"))?;
                    self.keeper.failed();
                }
            }
        }
        Ok(())
    }

    /// What asking the update server, and downloading the system's full
    /// package, told.
    fn updates_told(&mut self, now: u64) -> Result<()> {
        let told = self.checker.as_ref().map(updates::Worker::events).unwrap_or_default();
        let mut checked = false;
        for ev in told {
            let updates::Event::Checked(answer) = ev else { unreachable!("asking the server downloads nothing") };
            let have = match &answer {
                Ok(Some(release)) => have(&self.app, release),
                _ => Have::Nothing,
            };
            self.app.update_checked(answer, have);
            checked = true;
        }
        if checked {
            self.checker = None;
        }
        let told = self.downloader.as_ref().map(updates::Worker::events).unwrap_or_default();
        let mut fetched = false;
        for ev in told {
            match ev {
                updates::Event::Progress(progress, rate) => self.app.download_progress(progress, rate),
                updates::Event::Ended(end) => {
                    let release = self.app.state.newest().context("a download is of the newest system")?.clone();
                    let end = match end {
                        updates::End::Downloaded => DownloadEnd::Downloaded,
                        updates::End::Stopped => DownloadEnd::Stopped,
                        updates::End::Failed(why) => DownloadEnd::Failed(why),
                        updates::End::Updated(_) | updates::End::NotWritten(_) | updates::End::WriteFailed(..) => {
                            unreachable!("a system's full package is only downloaded")
                        }
                    };
                    skyward_sys::kmsg(&format!("downloading {} ended: {end:?}", release.name))?;
                    let have = have(&self.app, &release);
                    self.app.download_ended(end, have, now);
                    fetched = true;
                }
                updates::Event::Checked(_) | updates::Event::SkywardChecked(_) | updates::Event::Writing { .. } => {
                    unreachable!("a download asks nothing of its own, and writes nothing")
                }
            }
        }
        if fetched {
            self.downloader = None;
        }
        Ok(())
    }

    /// What asking GitHub for Skyward's newest release, and updating to it,
    /// told.
    fn skyward_told(&mut self, now: u64) -> Result<()> {
        let told = self.skyward_checker.as_ref().map(updates::Worker::events).unwrap_or_default();
        let mut asked = false;
        for ev in told {
            let updates::Event::SkywardChecked(answer) = ev else { unreachable!("asking GitHub downloads nothing") };
            skyward_sys::kmsg(&format!("GitHub told of Skyward's newest release: {answer:?}"))?;
            self.app.skyward_checked(answer);
            asked = true;
        }
        if asked {
            self.skyward_checker = None;
        }
        let told = self.skyward_updater.as_ref().map(updates::Worker::events).unwrap_or_default();
        let mut updated = false;
        for ev in told {
            match ev {
                updates::Event::Progress(progress, rate) => self.app.skyward_progress(SkywardStage::Download(progress), rate),
                updates::Event::Writing { done, total } => self.app.skyward_progress(SkywardStage::Write { done, total }, 0),
                updates::Event::Ended(end) => {
                    let end = match end {
                        updates::End::Updated(slots) => SkywardEnd::Updated(slots),
                        updates::End::Stopped => SkywardEnd::Stopped,
                        updates::End::Failed(why) => SkywardEnd::Download(why),
                        updates::End::NotWritten(why) => SkywardEnd::NotWritten(why),
                        updates::End::WriteFailed(slots, why) => SkywardEnd::WriteFailed(slots, why),
                        updates::End::Downloaded => unreachable!("Skyward's build is written once it is downloaded"),
                    };
                    skyward_sys::kmsg(&format!("updating Skyward ended: {end:?}"))?;
                    self.app.skyward_ended(end, now);
                    updated = true;
                }
                updates::Event::Checked(_) | updates::Event::SkywardChecked(_) => unreachable!("an update asks nothing of its own"),
            }
        }
        if updated {
            self.skyward_updater = None;
        }
        Ok(())
    }

    /// How opening the data partition, or a user's own files in it, ended.
    fn decrypt_told(&mut self, now: u64) -> Result<()> {
        let Some(ended) = self.decryptor.as_ref().and_then(Decryptor::ended) else { return Ok(()) };
        self.decryptor = None;
        match ended {
            decrypt::Ended::Opened(Ok(opening)) => {
                if let Some(mounted) = &opening.mounted {
                    log_mounted(mounted)?;
                }
                self.closed = opening.closed;
                show_closed(&mut self.app, &self.closed)?;
            }
            decrypt::Ended::Opened(Err(why)) => {
                skyward_sys::kmsg(&format!("the data partition stays closed: {why}"))?;
                // The update the system asked for cannot be read, and
                // is done with.
                if self.pending.take().is_some() {
                    let failed = skyward_install::Outcome::Failed(skyward_install::Failure::Unreadable(why.clone()));
                    self.app.install(skyward_install::Event::Finished(failed), now);
                    self.done_with_request()?;
                }
                self.app.data_failed(why);
            }
            decrypt::Ended::Unlocked(Ok(still)) => {
                self.closed = still;
                show_closed(&mut self.app, &self.closed)?;
            }
            decrypt::Ended::Unlocked(Err(why)) => {
                skyward_sys::kmsg(&format!("what was entered did not open the user's files: {why}"))?;
                self.app.unlock_failed(why, now);
            }
        }
        // Screenshots taken while internal storage was closed go into
        // the gallery's once it opens.
        if self.app.state.place_open(Place::Storage)
            && let Err(e) = keep_held_screenshots()
        {
            let why = format!("{e:#}");
            skyward_sys::kmsg(&format!("the screenshots held did not go into the gallery's: {why}"))?;
            self.app.screenshots_not_kept(&why, now);
        }
        // The update the system asked for starts once what holds it is
        // open; until the user whose own files hold it unlocks them,
        // its page says to.
        if let Some(wanted) = self.pending.take() {
            match wanted.needs {
                Needs::User(user) if self.closed.iter().any(|c| c.user == user) => {
                    self.app.asked_locked(user, now);
                    self.pending = Some(wanted);
                }
                _ => self.installer = Some(install_asked(self.programs, &self.app, wanted)?),
            }
        }
        Ok(())
    }

    /// What the terminal's shell wrote, and its end.
    fn shell_told(&mut self) -> Result<()> {
        let Some(sh) = &mut self.shell else { return Ok(()) };
        let wrote = sh.read()?;
        if !wrote.is_empty() {
            self.app.term_output(&wrote);
        }
        if let Some(status) = sh.ended()? {
            // What it wrote last shows before its end does.
            let last = sh.read()?;
            if !last.is_empty() {
                self.app.term_output(&last);
            }
            self.app.term_ended(pty::how(status));
            self.shell = None;
        }
        Ok(())
    }

    /// Internal storage goes to the computer while it is open, and no
    /// erasing or installing could take it away. File transfer that does
    /// not follow, or whose service ended, stops, and the page says why.
    fn offer_storage(&mut self, now: u64) -> Result<()> {
        let kept = match &mut self.transfer {
            Some(t) if !t.service.running() => Err(anyhow!("the file transfer service ended")),
            Some(t) => {
                let offer = self.app.state.data.storage_open() && self.eraser.is_none() && self.installer.is_none();
                t.offer(offer, self.app.state.lang).and_then(|()| {
                    if t.zone != self.app.state.time_zone {
                        t.service.set_zone(skyward_ui::fmt::rules(self.app.state.time_zone).clone())?;
                        t.zone = self.app.state.time_zone;
                    }
                    Ok(())
                })
            }
            None => Ok(()),
        };
        if let Err(e) = kept {
            // An ended service's own error says more than its ending does.
            let why = match self.transfer.take().expect("only file transfer that runs fails").stop(!self.fastboot && !self.sideloading) {
                Err(cause) => cause,
                Ok(()) => e,
            };
            skyward_sys::kmsg(&format!("file transfer stops: {why:#}"))?;
            self.app.file_transfer_failed(false, &skyward_ui::fmt::transfer_stopped(self.app.state.lang, &format!("{why:#}")), now);
        }
        Ok(())
    }
}

/// What comes in to the pages, and what they ask for in answer.
impl Pages<'_> {
    /// Hand the pages what came in since the last turn: the first page
    /// showing, a USB drive plugged in or pulled out, what programs asked on
    /// the recovery socket, each answered as the recovery stands when its
    /// turn comes, and touches and keys. What the pages ask for is done in
    /// the order they ask.
    fn take_in(&mut self, drive_told: Option<Option<UsbDrive>>, now: u64) -> Result<()> {
        let requests = match &self.control {
            Some(c) => c.requests()?,
            None => Vec::new(),
        };
        let incoming = self
            .arriving
            .take()
            .into_iter()
            .chain(drive_told.map(Incoming::Drive))
            .chain(requests.into_iter().map(Incoming::Asked))
            .chain(self.inputs.read()?.into_iter().map(Incoming::Input));
        for came in incoming {
            let effects = match came {
                Incoming::Arrived => self.app.arrive(),
                Incoming::Drive(drive) => self.app.set_usb_drive(drive),
                Incoming::Asked(asked) => {
                    skyward_sys::kmsg(&format!("asked on the recovery socket: {asked:?}"))?;
                    answer(asked, self.fastboot)
                }
                Incoming::Input(Event::Touch(t, at)) => self.app.touch(t, &self.fonts, at),
                Incoming::Input(Event::Key(k, true)) => self.app.key(k, now),
                Incoming::Input(Event::Key(k, false)) => self.app.key_up(k, now),
            };
            // What the pages ask for in answer to the work done goes on
            // after it.
            let mut queue = VecDeque::from(effects);
            while let Some(fx) = queue.pop_front() {
                eprintln!("effect {fx:?}");
                queue.extend(self.effect(fx, now)?);
            }
        }
        Ok(())
    }

    /// Do what a page asks for, and give what the pages ask for in answer
    /// to it.
    fn effect(&mut self, fx: Effect, now: u64) -> Result<Vec<Effect>> {
        let cfg = self.cfg;
        match fx {
            // Leaving is the user's answer to whatever the request
            // still kept in misc asked.
            Effect::Reboot(target) => {
                self.done_with_request()?;
                power(target)?
            }
            Effect::Dismiss => self.done_with_request()?,
            Effect::FactoryReset => self.reset_asked(now)?,
            Effect::Wipe(chosen) => self.wipe(chosen, now)?,
            Effect::Install => self.installer.as_ref().context("no installer holds an update")?.install(),
            Effect::StartShell(folder) => self.start_shell(&folder, now)?,
            // A shell that will not take keys is the page's to report; the
            // recovery carries on.
            Effect::TermInput(bytes) => {
                let sh = self.shell.as_mut().context("keys went to a shell that has ended")?;
                if let Err(e) = sh.write(&bytes) {
                    self.app.report(&format!("{e:#}"), now);
                }
            }
            Effect::ShellTo(folder) => self.shell_to(&folder, now)?,
            Effect::Haptic => self.click()?,
            Effect::Ticks => self.ticks()?,
            // The panel shows the last frame drawn, which is what is
            // saved; one that cannot be is the page's to report.
            Effect::Screenshot { storage } => {
                let t0 = Instant::now();
                let saved = save_screenshot(&self.screen, storage, self.app.state.time_zone).map_err(|e| format!("{e:#}"));
                eprintln!("screenshot {saved:?} in {} ms", t0.elapsed().as_millis());
                self.app.screenshot_saved(saved, now);
            }
            Effect::Sideload => {
                if self.may_install(now)? {
                    self.installer = Some(Installer::start(detach(self.programs.to_vec()), install_options(&self.app))?);
                    self.sideloading = true;
                }
            }
            Effect::InstallFile(place, path) => {
                if self.may_install(now)? {
                    self.installer = Some(Installer::open(in_place(place, &path), detach(self.programs.to_vec()), install_options(&self.app))?);
                }
            }
            Effect::InstallPath(path) => {
                if self.may_install(now)? {
                    self.installer = Some(Installer::open(PathBuf::from(path), detach(self.programs.to_vec()), install_options(&self.app))?);
                }
            }
            Effect::Flash { partition, pick } => {
                let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                self.installer.as_ref().context("no installer holds an image")?.flash(partition, pick.slots(slot));
            }
            Effect::Cancel => self.cancel_install(now)?,
            Effect::ReadMounts => self.app.set_mounts(self.mounts()),
            Effect::ReadSlots => self.app.set_boot_slots(read_slots()),
            Effect::SetSlot(slot) => self.app.slot_switched(set_slot(slot), now),
            Effect::ReadSystems => self.systems()?.send(systems::Task::Read),
            // An install still under way may yet write the boot
            // partitions; a switch waits for it.
            Effect::SwitchSystem(to) => {
                if self.installer.is_some() {
                    self.app.systems_work_failed(self.app.state.lang.pick(INSTALL_UNFINISHED), now);
                } else {
                    skyward_sys::kmsg(&format!("switching to the {to:?} system, as the systems page asked"))?;
                    self.systems()?.send(systems::Task::Switch(to));
                }
            }
            Effect::RemoveSystem => {
                if self.installer.is_some() {
                    self.app.systems_work_failed(self.app.state.lang.pick(INSTALL_UNFINISHED), now);
                } else {
                    skyward_sys::kmsg("removing the second system, as the systems page asked")?;
                    self.systems()?.send(systems::Task::Remove);
                }
            }
            Effect::InstallSystem { size } => self.installer.as_ref().context("no installer holds a system")?.install_system(size),
            // The decrypt page shows meanwhile that the work is
            // under way, and takes nothing more until it is done.
            Effect::OpenData => {
                ensure!(self.decryptor.is_none(), "the data partition is being opened already");
                self.decryptor = Some(Decryptor::start(decrypt::Job::Open)?);
            }
            Effect::Unlock { user, code } => {
                ensure!(self.decryptor.is_none(), "the data partition is being opened already");
                let job = decrypt::Job::Unlock { closed: self.closed.clone(), user, credential: code.into_bytes() };
                self.decryptor = Some(Decryptor::start(job)?);
            }
            Effect::ReadDir(folder) => list(&mut self.app, folder)?,
            Effect::ReadPlace(place, folder) => list_place(&mut self.app, place, folder)?,
            Effect::Paste { op, path, into, clash } => self.paste(op, path, into, clash, now)?,
            Effect::Delete(path) => {
                ensure!(self.worker.is_none(), "work on files is under way already");
                self.worker = Some(filework::Worker::start(filework::Task::Delete { path: PathBuf::from(&path) })?);
                self.app.file_job_started(FileTask::Delete { path });
            }
            Effect::StopJob => self.worker.as_ref().context("there is no work on files to stop")?.stop(),
            Effect::Relabel => self.relabel(now)?,
            Effect::StopRelabel => self.relabeler.as_ref().context("there is no relabeling to stop")?.stop(),
            // Partitions that cannot be read are the page's to say.
            Effect::ReadFilesystems => self.app.set_filesystems(read_filesystems().map_err(|e| format!("{e:#}"))),
            Effect::Filesystem { name, action } => self.filesystem(&name, action, now)?,
            Effect::Backup { parts, place, compress, verify } => self.back_up(parts, place, compress, verify, now)?,
            Effect::Restore { place, name, parts } => self.restore(place, &name, parts, now)?,
            Effect::FindBackups { storage, usb } => find_backups(&mut self.app, storage, usb, now),
            Effect::DeleteBackup { place, name } => {
                let deleted = delete_backup(place, &backups_in(place).join(&name)).map_err(|e| format!("{e:#}"));
                return Ok(self.app.backup_deleted(deleted, now));
            }
            Effect::StopBackup => self.backuper.as_ref().context("there is no backup or restore to stop")?.stop(),
            Effect::ReadUpdate => {
                let slot = self.info.slot.context("only an A/B phone has an update to merge")?;
                // A merge followed tells where it stands itself.
                if self.merger.is_none() {
                    self.app.set_update(skyward_vab::standing(slot)?);
                }
                // An update in the other slot starts with that slot,
                // which a system's updater may not have set yet.
                self.app.set_boot_slots(read_slots());
            }
            Effect::Merge => {
                ensure!(self.merger.is_none(), "the update is being merged already");
                let slot = self.info.slot.context("only an A/B phone merges an update")?;
                skyward_sys::kmsg("merging the update, as the update page asked")?;
                self.merger = Some(merge::Merger::start(slot, true)?);
            }
            Effect::Wlan(on) => wlan_worker(&self.wlan)?.send(if on { wlan::Command::On } else { wlan::Command::Off }),
            Effect::ScanWlan => {
                self.scanned_at = Instant::now();
                wlan_worker(&self.wlan)?.send(wlan::Command::Scan);
            }
            Effect::JoinWlan { ssid, name, security, password } => {
                wlan_worker(&self.wlan)?.send(wlan::Command::Join { ssid, name, security, password: password.0 })
            }
            Effect::LeaveWlan => wlan_worker(&self.wlan)?.send(wlan::Command::Leave),
            Effect::CheckUpdate => self.check_update()?,
            Effect::Download(release) => self.download(release, now)?,
            Effect::StopDownload => self.downloader.as_ref().context("there is no download to stop")?.stop(),
            Effect::CheckSkyward => {
                ensure!(self.skyward_checker.is_none(), "GitHub is being asked already");
                let probe = cfg.wlan.as_ref().context("the phone's WLAN is not brought up")?.probe.clone();
                let task = updates::Task::CheckSkyward { probe, codename: cfg.device.codename.clone() };
                self.skyward_checker = Some(updates::Worker::start(task, self.servers.clone())?);
            }
            // The zip goes into the recovery's own folder, which is
            // in memory, before its image goes into the recovery
            // partitions.
            Effect::UpdateSkyward(build) => {
                ensure!(self.skyward_updater.is_none(), "Skyward is being updated already");
                ensure!(self.installer.is_none(), "an installer is running already");
                skyward_sys::kmsg(&format!("updating Skyward to {}", build.name))?;
                let path = Path::new(skyward_sys::RUN).join(&build.name);
                let task = updates::Task::UpdateSkyward { build, codename: cfg.device.codename.clone(), path };
                self.skyward_updater = Some(updates::Worker::start(task, self.servers.clone())?);
            }
            Effect::StopSkywardUpdate => self.skyward_updater.as_ref().context("Skyward is not being updated")?.stop(),
            // Logs that cannot be saved are the page's to report;
            // the recovery carries on.
            Effect::ExportLogs { recovery, kernel, storage } => {
                let into = match storage {
                    true => Path::new(skyward_crypto::data::SDCARD).join(STORAGE_LOGS),
                    false => PathBuf::from(TMP_LOGS),
                };
                match export_logs(recovery, kernel, &into, self.app.state.time_zone) {
                    Ok(folder) => self.app.report(&skyward_ui::fmt::exported(self.app.state.lang, &folder.display().to_string()), now),
                    Err(e) => self.app.report(&skyward_ui::fmt::export_failed(self.app.state.lang, &format!("{e:#}")), now),
                }
            }
            Effect::SetMode { path, mode } => {
                let outcome = skyward_files::set_mode(Path::new(&path), mode).map_err(|e| format!("{e:#}"));
                self.app.mode_set(mode, outcome, now);
            }
            Effect::OpenText(path) => {
                let read = skyward_files::read_up_to(Path::new(&path), skyward_ui::editor::MAX_BYTES).map_err(|e| format!("{e:#}"));
                self.app.text_opened(&self.fonts, path, read, now);
            }
            // The folder is listed again, for the file's new size
            // and time.
            Effect::SaveText { path, text } => match skyward_files::replace(Path::new(&path), text.as_bytes()) {
                Ok(()) => {
                    self.app.text_saved(Ok(()), now);
                    let folder = self.app.state.files.path.clone();
                    list(&mut self.app, folder)?;
                }
                Err(e) => self.app.text_saved(Err(format!("{e:#}")), now),
            },
            Effect::Rename { path, name } => match skyward_files::rename(Path::new(&path), &name) {
                Ok(_) => {
                    self.app.renamed(Ok(()), now);
                    let folder = self.app.state.files.path.clone();
                    list(&mut self.app, folder)?;
                }
                Err(e) if e.is::<skyward_files::Taken>() => self.app.renamed(Err(RenameRefused::Taken), now),
                Err(e) => self.app.renamed(Err(RenameRefused::Failed(format!("{e:#}"))), now),
            },
            // A partition that will not mount, or is in use, is the
            // page's to report; the recovery carries on.
            Effect::Mount(base) => {
                let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                if let Err(e) = skyward_vab::mount_one(slot, &base) {
                    self.app.mount_failed(MountFailure { mounting: true, base, detail: format!("{e:#}") });
                }
                self.app.set_mounts(self.mounts());
            }
            Effect::Unmount(base) => {
                let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
                if let Err(e) = skyward_vab::unmount_one(slot, &base) {
                    self.app.mount_failed(MountFailure { mounting: false, base, detail: format!("{e:#}") });
                }
                self.app.set_mounts(self.mounts());
            }
            Effect::EnterRecovery => self.enter_recovery()?,
            Effect::FileTransfer(on) => self.file_transfer(on, now)?,
        }
        Ok(Vec::new())
    }

    /// The second system's work, which only a phone that can hold a second
    /// system has.
    fn systems(&mut self) -> Result<&mut systems::Worker> {
        self.systems_worker.as_mut().context("this phone holds no second system")
    }

    /// Whether an install may start: none runs, and file transfer has let
    /// internal storage go, which the page is told otherwise.
    fn may_install(&mut self, now: u64) -> Result<bool> {
        ensure!(self.installer.is_none(), "an installer is running already");
        withhold(&mut self.transfer, &mut self.app, now)
    }

    /// Erase all user data, as the page the system's request showed asks.
    fn reset_asked(&mut self, now: u64) -> Result<()> {
        ensure!(self.eraser.is_none(), "erasing is under way already");
        if !withhold(&mut self.transfer, &mut self.app, now)? {
            return Ok(());
        }
        self.eraser = Some(factory_reset(&mut self.app, self.request.keep_memtag_mode, true, None)?);
        self.erase_leaves = true;
        Ok(())
    }

    /// Erase what the wipe page chose. Opening the data partition and
    /// erasing it are not to cross; the page says to wait for the one under
    /// way.
    fn wipe(&mut self, chosen: Vec<Wipe>, now: u64) -> Result<()> {
        ensure!(self.eraser.is_none(), "erasing is under way already");
        if self.decryptor.is_some() {
            self.app.report(self.app.state.lang.pick(DATA_OPENING), now);
            return Ok(());
        }
        if !withhold(&mut self.transfer, &mut self.app, now)? {
            return Ok(());
        }
        self.erase_leaves = false;
        if chosen.contains(&Wipe::All) {
            self.eraser = Some(factory_reset(&mut self.app, false, false, None)?);
        } else if chosen.iter().any(|&w| w != Wipe::Cache) && self.app.state.data == Data::Erased {
            self.app.erase_started(false, false, Vec::new());
            self.app.erase_ended(Err(EraseStop::Erased), now);
        } else {
            let steps = chosen.into_iter().map(step).collect();
            self.eraser = Some(parts_wipe(&mut self.app, steps, &self.cfg.wipe.logs, false, None)?);
        }
        Ok(())
    }

    /// Start the terminal's shell in `folder`. A shell that will not start
    /// is the page's to report; the recovery carries on.
    fn start_shell(&mut self, folder: &str, now: u64) -> Result<()> {
        ensure!(self.shell.is_none(), "a shell runs already");
        let (rows, cols) = self.app.term_size(&self.fonts);
        match pty::Shell::start(rows, cols, &self.cfg.device.codename, Path::new(folder)) {
            Ok(sh) => {
                self.shell = Some(sh);
                self.app.term_started(rows, cols);
            }
            Err(e) => {
                skyward_sys::kmsg(&format!("no shell for the terminal: {e:#}"))?;
                self.app.report(&format!("{e:#}"), now);
            }
        }
        Ok(())
    }

    /// Have the terminal's shell go to `folder`, while it waits for a
    /// command; one that will not is the page's to report.
    fn shell_to(&mut self, folder: &str, now: u64) -> Result<()> {
        let sh = self.shell.as_mut().context("a folder went to a shell that has ended")?;
        let went = match sh.waiting() {
            Ok(true) => sh.change_to(folder),
            Ok(false) => {
                self.app.report(self.app.state.lang.pick(skyward_ui::fmt::TERMINAL_BUSY), now);
                Ok(())
            }
            Err(e) => Err(e),
        };
        if let Err(e) = went {
            self.app.report(&format!("{e:#}"), now);
        }
        Ok(())
    }

    /// Stop waiting for the computer, for where an image goes, or for an
    /// update to be agreed to.
    fn cancel_install(&mut self, now: u64) -> Result<()> {
        match &self.installer {
            Some(i) => i.cancel()?,
            // The update the system asked for, waiting on the
            // lock of the user whose files hold it, is given up
            // as its page is left, and is done with.
            None => {
                ensure!(self.pending.take().is_some(), "no installer to cancel");
                skyward_sys::kmsg("the update asked for is given up, its page left")?;
                self.done_with_request()?;
                let cancelled = skyward_install::Event::Finished(skyward_install::Outcome::Cancelled);
                self.app.install(cancelled, now);
            }
        }
        Ok(())
    }

    /// Copy or move `path` into the folder `into`. A name taken where an
    /// entry goes is the user's to decide on before anything is written.
    fn paste(&mut self, op: FileOp, path: String, into: String, clash: Option<ClashChoice>, now: u64) -> Result<()> {
        ensure!(self.worker.is_none(), "work on files is under way already");
        let clash = match clash {
            Some(choice) => clash_rule(choice),
            None => match skyward_files::clashes(&[PathBuf::from(&path)], Path::new(&into)) {
                Ok(taken) if taken.is_empty() => skyward_files::Clash::Refuse,
                Ok(taken) => {
                    let names = taken.iter().map(|t| t.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).collect();
                    self.app.file_clash(op, path, into, names);
                    return Ok(());
                }
                Err(e) => {
                    self.app.report(&format!("{e:#}"), now);
                    return Ok(());
                }
            },
        };
        let ownership = match ownership(Path::new(&into)) {
            Ok(ownership) => ownership,
            Err(e) => {
                self.app.report(&format!("{e:#}"), now);
                return Ok(());
            }
        };
        let (from, to) = (PathBuf::from(&path), PathBuf::from(&into));
        let (task, shown) = match op {
            FileOp::Copy => (filework::Task::Copy { from, into: to, clash, ownership }, FileTask::Copy { path, into }),
            FileOp::Move => (filework::Task::Move { from, into: to, clash, ownership }, FileTask::Move { path, into }),
        };
        self.worker = Some(filework::Worker::start(task)?);
        self.app.file_job_started(shown);
        Ok(())
    }

    /// Give the data partition's files the labels the installed system's
    /// rules give them. Rules kept on a partition that is not mounted are
    /// the page's to report, with what to do first.
    fn relabel(&mut self, now: u64) -> Result<()> {
        ensure!(self.relabeler.is_none(), "relabeling is under way already");
        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
        match relabel::rule_files(slot) {
            Ok(rules) => {
                self.relabeler = Some(filework::Worker::start(filework::Task::Relabel { rules })?);
                self.app.relabel_started();
            }
            Err(e) => match e.downcast_ref::<relabel::Unmounted>() {
                Some(relabel::Unmounted(base)) => self.app.report(&skyward_ui::fmt::mount_first(self.app.state.lang, base), now),
                None => self.app.report(&skyward_ui::fmt::rules_unread(self.app.state.lang, &format!("{e:#}")), now),
            },
        }
        Ok(())
    }

    /// Repair the filesystem of the partition `name`, or make it afresh.
    /// The data partition made afresh as another filesystem is a factory
    /// reset, as the system asks for one; the rest is work on one
    /// partition. Opening the data partition and working on it are not to
    /// cross, and file transfer lets internal storage go first.
    fn filesystem(&mut self, name: &str, action: FsAction, now: u64) -> Result<()> {
        ensure!(self.eraser.is_none() && self.part_worker.is_none(), "work on partitions is under way already");
        if name == DATA_PARTITION && self.decryptor.is_some() {
            self.app.report(self.app.state.lang.pick(DATA_OPENING), now);
            return Ok(());
        }
        if name == DATA_PARTITION && !withhold(&mut self.transfer, &mut self.app, now)? {
            return Ok(());
        }
        let part = fs_part(name)?;
        let (kind, task) = match action {
            FsAction::Remake(filesystem) if name == DATA_PARTITION => {
                self.erase_leaves = false;
                self.eraser = Some(factory_reset(&mut self.app, false, false, Some(filesystem))?);
                return Ok(());
            }
            FsAction::Repair => (PartKind::Repair, partwork::Task::Repair(part)),
            FsAction::Remake(filesystem) => (PartKind::Remake(filesystem.clone()), partwork::Task::Remake(part, filesystem)),
        };
        skyward_sys::kmsg(&format!("{kind:?} the filesystem of {name}, as the filesystem page asked"))?;
        self.app.part_work_started(kind, false);
        self.part_worker = Some(partwork::Worker::start(task)?);
        Ok(())
    }

    /// Back the parts chosen up into `place`. An install still stopping may
    /// yet write; a backup waits for it.
    fn back_up(&mut self, parts: [bool; 4], place: Place, compress: bool, verify: bool, now: u64) -> Result<()> {
        ensure!(self.backuper.is_none(), "a backup or restore is under way already");
        if self.installer.is_some() {
            self.app.report(self.app.state.lang.pick(INSTALL_UNFINISHED), now);
            return Ok(());
        }
        let into = backups_in(place);
        let there = into.ancestors().find(|p| p.exists()).context("the root is always there")?;
        let ownership = match ownership(there) {
            Ok(ownership) => ownership,
            Err(e) => {
                self.app.report(&format!("{e:#}"), now);
                return Ok(());
            }
        };
        let made = unix_now()?;
        let task = backup::BackupTask {
            parts,
            stamp: skyward_ui::fmt::stamp(made, self.app.state.time_zone),
            into,
            ownership,
            compress,
            verify,
            remount: (place == Place::UsbDrive).then(|| PathBuf::from(USB_DRIVE)),
            made,
            device: self.cfg.device.codename.clone(),
            system: self.info.os_version.clone(),
            efs: self.cfg.backup.efs.clone(),
            skipped: backup::data_skipped(&self.cfg.backup.data_skipped),
        };
        self.backuper = Some(backup::Worker::start(backup::Task::Backup(task))?);
        self.app.backup_started(false, place, verify);
        Ok(())
    }

    /// Restore the parts chosen of the backup `name` in `place`. An install
    /// still stopping may yet write; a restore waits for it.
    fn restore(&mut self, place: Place, name: &str, parts: [bool; 4], now: u64) -> Result<()> {
        ensure!(self.backuper.is_none(), "a backup or restore is under way already");
        if self.installer.is_some() {
            self.app.report(self.app.state.lang.pick(INSTALL_UNFINISHED), now);
            return Ok(());
        }
        let task = backup::RestoreTask {
            folder: backups_in(place).join(name),
            parts,
            device: self.cfg.device.codename.clone(),
            skipped: backup::data_skipped(&self.cfg.backup.data_skipped),
            detach: detach(self.programs.to_vec()),
        };
        self.backuper = Some(backup::Worker::start(backup::Task::Restore(task))?);
        self.app.backup_started(true, place, true);
        Ok(())
    }

    /// Ask the update server for the newest system. What the installed
    /// system is called is the page's to report when it cannot be read.
    fn check_update(&mut self) -> Result<()> {
        ensure!(self.checker.is_none(), "the update server is being asked already");
        let server = self.cfg.update.as_ref().context("the phone's update server is not known")?.server.clone();
        let probe = self.cfg.wlan.as_ref().context("the phone's WLAN is not brought up")?.probe.clone();
        match installed(self.cfg, &self.info) {
            Ok(installed) => {
                self.checker = Some(updates::Worker::start(updates::Task::Check { server, probe, installed }, self.servers.clone())?)
            }
            Err(e) => self.app.update_checked(Err(Unanswered::Failed(format!("{e:#}"))), Have::Nothing),
        }
        Ok(())
    }

    /// Download `release`'s full package into internal storage.
    fn download(&mut self, release: Release, now: u64) -> Result<()> {
        ensure!(self.downloader.is_none(), "the full package is being downloaded already");
        let server = self.cfg.update.as_ref().context("the phone's update server is not known")?.server.clone();
        let path = downloaded(&release);
        match installed(self.cfg, &self.info) {
            Ok(installed) => {
                let task = updates::Task::Download { server, installed, release, path };
                self.downloader = Some(updates::Worker::start(task, self.servers.clone())?);
            }
            Err(e) => {
                let have = have(&self.app, &release);
                self.app.download_ended(DownloadEnd::Failed(Failure::Other(format!("{e:#}"))), have, now);
            }
        }
        Ok(())
    }

    /// Leave fastboot for the recovery's pages. init stops fastbootd as the
    /// port goes through "none"; then the slot's partitions come up as at
    /// boot.
    fn enter_recovery(&mut self) -> Result<()> {
        skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT)?;
        let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
        crate::prepare(self.cfg, slot)?;
        self.read_at = Instant::now();
        read_details(self.cfg, &mut self.info, &mut self.unread.details)?;
        self.fastboot = false;
        self.app.enter_recovery();
        if let Some(w) = self.systems_worker.as_mut() {
            w.send(systems::Task::Resume);
        }
        Ok(())
    }

    /// Turn file transfer on or off. A port that something else has takes
    /// file transfer on, or leaves it, when it comes back to rest.
    fn file_transfer(&mut self, on: bool, now: u64) -> Result<()> {
        let rests = !self.fastboot && !self.sideloading;
        match (on, self.transfer.take()) {
            (true, None) => match Transfer::start(self.cfg, &self.info, self.app.state.time_zone) {
                Ok(t) => {
                    self.transfer = Some(t);
                    if rests
                        && let Err(e) = skyward_sys::usb::switch(skyward_sys::usb::resting(), USB_TIMEOUT)
                    {
                        skyward_sys::kmsg(&format!("the port did not take file transfer: {e:#}"))?;
                        self.app.report(&format!("{e:#}"), now);
                    }
                }
                Err(e) => {
                    skyward_sys::kmsg(&format!("file transfer did not start: {e:#}"))?;
                    self.app.file_transfer_failed(false, &skyward_ui::fmt::transfer_failed(self.app.state.lang, &format!("{e:#}")), now);
                }
            },
            (false, Some(t)) => {
                if let Err(e) = t.stop(rests) {
                    skyward_sys::kmsg(&format!("file transfer did not stop cleanly: {e:#}"))?;
                    self.app.report(&skyward_ui::fmt::transfer_left_on(self.app.state.lang, &format!("{e:#}")), now);
                }
            }
            (on, t) => unreachable!("the switch turned file transfer {on} while it was {}", t.is_some()),
        }
        Ok(())
    }
}
