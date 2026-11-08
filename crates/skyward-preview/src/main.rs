use anyhow::{anyhow, Result};
use skyward_config::DeviceConfig;
use skyward_info::{Battery, Boot, Charge, Cpu, DeviceInfo, PatchLevel, Storage};
use skyward_install::{Failure, ImageKind, Outcome, Ramdisk, Reflash, Refusal, Stage};
use skyward_net::download::{Failure as DownloadFailure, Progress};
use skyward_net::github::{Build, Newest};
use skyward_net::reach::Reach;
use skyward_net::release::{Release, Unanswered};
use skyward_net::supplicant::Security;
use skyward_net::wlan::{Bands, Joined, Link, Network, Power, Stage as Joining, Traffic, Wlan};
use skyward_systems::{Named, Other, Overview, Who};
use skyward_ui::app::{
    Action, Adding, App, Asked, BackupAt, BackupEnd, BackupEntry, BackupJob, BackupRefusal, Choice, Clip, Damage, Data, DownloadEnd,
    DownloadJob, EraseStop, Field, FileKind, FileOp, FileRow, FileTask, FsAction, FsRow, Have, Held, Install, ItemEnd, JobEnd, Key, Keys,
    MountFailure, MountRow, Part, PartKind, PartStop, Pick, Place, Screen, SkywardEnd, SkywardJob, SkywardStage, Source, SystemChoice,
    SystemsWork, TermEnd, Touch, UsbDrive, UsbLink, SCREEN_OFF,
};
use skyward_ui::widgets::{self, Bar, Bolt, Cap, Layer};
use skyward_ui::{editor, fmt, pages, Canvas, Ctx, Fonts, Lang, Theme};
use skyward_vab::{BootSlots, FailureCode, Hold, Slot, SnapshotState, Standing};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

mod matcher;

const CLOCK: &str = "23:46";

/// The language the pages are drawn in, as `--lang` names its locale;
/// simplified Chinese where it names none.
static LANG: OnceLock<Lang> = OnceLock::new();

fn lang() -> Lang {
    *LANG.get().expect("the language is chosen before any page is drawn")
}

/// What a shell in the recovery writes for a few commands, colours and all.
const SHELL_SAMPLE: &[u8] = b"ishtar:/ # getprop ro.product.device\r\nishtar\r\n\
ishtar:/ # ls --color=always /\r\n\
\x1b[1;34macct\x1b[0m  \x1b[1;34mapex\x1b[0m  \x1b[1;36mbin\x1b[0m  \x1b[1;34mconfig\x1b[0m  \x1b[1;34mdata\x1b[0m  \x1b[1;34mdev\x1b[0m\r\n\
\x1b[1;36metc\x1b[0m  \x1b[1;32minit\x1b[0m  \x1b[1;34mmetadata\x1b[0m  \x1b[1;34mmnt\x1b[0m  \x1b[1;34modm\x1b[0m\r\n\
\x1b[1;34mproc\x1b[0m  \x1b[1;34msys\x1b[0m  \x1b[1;34msystem\x1b[0m  \x1b[1;34mtmp\x1b[0m  \x1b[1;34mvendor\x1b[0m\r\n\
ishtar:/ # df -h /metadata\r\n\
Filesystem      Size Used Avail Use% Mounted on\r\n\
/dev/block/sda9  59M 7.0M   52M  12% /metadata\r\n\
ishtar:/ # ";

/// A script as the editor shows it: an indented line, a comment in
/// Chinese, and lines long enough to wrap.
const EDITOR_SAMPLE: &str = "#!/system/bin/sh\n\
# 开机完成后调整调度器与刷新率\n\
for cpu in /sys/devices/system/cpu/cpu[0-7]; do\n\
\techo schedutil > $cpu/cpufreq/scaling_governor\n\
done\n\
setprop persist.vendor.display.enable_optimize_refresh 1\n\
echo \"调度器已调整，刷新率设置写入 /data/local/tmp/refresh.log\" >> /data/local/tmp/refresh.log\n";

/// What fsck.f2fs says as it repairs a data partition with one check that
/// failed and was fixed.
const FSCK_SAMPLE: &str = "Info: Force to fix corruption\n\
Info: total FS sectors = 57982464 (226494 MB)\n\
Info: checkpoint state = 285 :  allow_nocrc nat_bits compacted_summary orphan_inodes umount\n\
\n\
[FSCK] Unreachable nat entries                       [Ok..] [0x0]\n\
[FSCK] SIT valid block bitmap checking                [Ok..]\n\
[FSCK] Hard link checking for regular file           [Ok..] [0x2b]\n\
[FSCK] valid_block_count matching with CP            [Fail] [0x2f0b1d3]\n\
[FSCK] valid_node_count matching with CP (de lookup) [Ok..] [0x3c4a1]\n\
[FSCK] valid_inode_count matched with CP             [Ok..] [0x3b2f0]\n\
[FSCK] free segment_count matched with CP            [Ok..] [0x6b17]\n\
[FSCK] other corrupted bugs                          [Ok..]\n\
\n\
Done: 41.618725 secs\n";

/// What fsck.f2fs says as it gives up on a data partition it could not
/// mend.
const FSCK_FAILED_SAMPLE: &str = "Info: Force to fix corruption\n\
Info: total FS sectors = 57982464 (226494 MB)\n\
Info: checkpoint state = 46 :  crc compacted_summary orphan_inodes sudden-power-off\n\
\n\
[FSCK] Unreachable nat entries                       [Fail] [0x1c]\n\
[FSCK] SIT valid block bitmap checking                [Fail]\n\
[FSCK] Hard link checking for regular file           [Ok..] [0x2b]\n\
[FSCK] valid_block_count matching with CP            [Fail] [0x2f0b1d3]\n\
[FSCK] other corrupted bugs                          [Fail]\n\
\n\
Done: 38.204116 secs\n";

/// The 13 Ultra's partitions as the filesystem page lists them, its data
/// partition open or closed.
fn phone_filesystems(open: bool) -> Vec<FsRow> {
    vec![
        FsRow {
            name: "data".into(),
            mount_point: "/data".into(),
            filesystem: open.then(|| "f2fs".into()),
            size: 237_498_597_376,
            others: Vec::new(),
        },
        FsRow { name: "metadata".into(), mount_point: "/metadata".into(), filesystem: Some("f2fs".into()), size: 64 << 20, others: Vec::new() },
    ]
}

/// The systems of a 17 Ultra with a second system installed, HyperOS
/// starting.
fn two_systems() -> Overview {
    Overview {
        installed: Named { name: "HyperOS 3".into(), version: Some("OS3.0.308.0.WMACNXM".into()) },
        other: Some(Other { id: "ubuntu-touch".into(), name: "Ubuntu Touch".into(), version: "24.04-1.x".into(), size: 32_000_000_000 }),
        other_starts: false,
        changed: false,
    }
}

/// File name, page, dark theme, readings, where the data partition stands,
/// pattern drawn.
type Shot<'a> = (&'a str, Screen, bool, &'a DeviceInfo, Data, &'a [usize]);

/// File name, dark theme, readings, a mount that failed, and the partitions
/// left unmounted.
type MountShot<'a> = (&'a str, bool, &'a DeviceInfo, Option<MountFailure>, &'a [&'a str]);

/// File name, page, dark theme, and what to set before the page is drawn.
type PageShot<'a> = (&'a str, Screen, bool, &'a dyn Fn(&mut App));

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("preview-out"));
    std::fs::create_dir_all(&out)?;
    let lang = match args.iter().position(|a| a == "--lang") {
        Some(k) => Lang::for_locale(args.get(k + 1).ok_or_else(|| anyhow!("--lang needs a locale"))?)?,
        None => Lang::Hans,
    };
    LANG.set(lang).expect("the language is chosen once");

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let font_path = font_path()?;
    let fonts = Fonts::load(&font_path, 0, root.join("resources/fonts/KirkMono-Regular.ttf"))?;
    eprintln!("font: {}", font_path.display());
    for (tag, min, def, max) in fonts.axes() {
        eprintln!("axis {tag}: {min}..{max} (default {def})");
    }

    let cfg = DeviceConfig::load(root.join("devices/nezha.toml"))?;
    let (w, h) = (cfg.display.width, cfg.display.height);
    let ppd = cfg.display.density as f32 / 160.0;
    let light = Theme::light(w, h, ppd);

    match args.get(2).map(String::as_str) {
        Some("--calibrate") => {
            let path = out.join("calibrate.png");
            write_png(&path, &calibrate(&fonts, &light)?)?;
            println!("wrote {}", path.display());
            return Ok(());
        }
        Some("--match") => {
            let probes = args.get(3).ok_or_else(|| anyhow!("--match needs a probe file"))?;
            return matcher::run(&fonts, Path::new(probes));
        }
        _ => {}
    }

    let info = nezha(&cfg);
    let pending = DeviceInfo { snapshot: SnapshotState::Snapshotted, ..info.clone() };
    let pattern = Data::Locked { user: 0, keys: Keys::Pattern, checking: false };
    // Whole pages, the way a long screenshot shows them. The About page's
    // field is drawn only on screen, so it appears among the moments below.
    let shots: [Shot; 12] = [
        ("01-decrypt-dark", Screen::Decrypt, true, &info, pattern, &[0, 3, 4, 5, 8]),
        ("02-home-light", Screen::Home, false, &info, Data::Open, &[]),
        ("03-home-dark-update-pending", Screen::Home, true, &pending, Data::Open, &[]),
        ("04-settings-light", Screen::Settings, false, &info, Data::Open, &[]),
        ("05-settings-dark", Screen::Settings, true, &info, Data::Open, &[]),
        ("06-reboot-dark", Screen::Reboot, true, &info, Data::Open, &[]),
        ("07-wipe-light", Screen::Wipe, false, &info, Data::Open, &[]),
        ("09-specs-light", Screen::Specs, false, &info, Data::Open, &[]),
        ("10-specs-dark", Screen::Specs, true, &info, Data::Open, &[]),
        ("11-install-light", Screen::Install, false, &info, Data::Open, &[]),
        ("12-install-dark-update-pending", Screen::Install, true, &pending, Data::Open, &[]),
        ("13-screen-off-light", Screen::ScreenOff, false, &info, Data::Open, &[]),
    ];
    for (name, screen, dark, info, data, pattern) in shots {
        let mut app = App::new(screen, w, h, ppd);
        app.state.lang = lang;
        app.state.dark = dark;
        app.state.data = data;
        app.state.pattern = pattern.to_vec();
        let path = out.join(format!("{name}.png"));
        write_png(&path, &whole_page(&mut app, &fonts, info))?;
        println!("wrote {}", path.display());
    }

    // The screen kept on, which warns about burn-in.
    let mut app = opened(Screen::ScreenOff, w, h, ppd);
    app.state.dark = true;
    app.state.screen_off = SCREEN_OFF.iter().position(Option::is_none).expect("one choice keeps the screen on");
    let path = out.join("14-screen-off-never-dark.png");
    write_png(&path, &whole_page(&mut app, &fonts, &info))?;
    println!("wrote {}", path.display());

    // The sideload page through an install, as the installer tells it.
    let recovery = ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery };
    let installing: [(&str, bool, Install); 17] = [
        ("30-sideload-waiting", false, Install::Waiting),
        ("31-sideload-checking", false, Install::Checking { size: 7_103_043_868, stage: None }),
        (
            "31-sideload-verifying-dark",
            true,
            Install::Checking { size: 25_166_592, stage: Some(Stage::Signature { done: 10_485_760, total: 25_164_934 }) },
        ),
        (
            "32-sideload-choosing",
            false,
            Install::Choosing(Choice { kind: recovery, size: 43_257_856, partitions: vec!["recovery"], partition: 0, pick: Pick::Current }),
        ),
        (
            "33-sideload-writing-dark",
            true,
            Install::Updating {
                version: Some("OS4.0.5.0.XPACNXM".into()),
                stage: Some(Stage::Writing { done: 2_934_000_000, total: 7_103_041_280 }),
                overwriting: true,
            },
        ),
        (
            "34-sideload-verifying",
            false,
            Install::Updating {
                version: Some("OS4.0.5.0.XPACNXM".into()),
                stage: Some(Stage::Verifying { index: 6, count: 21, partition: "system".into() }),
                overwriting: false,
            },
        ),
        ("35-sideload-flashing", false, Install::Flashing { partition: "boot", stage: Some(Stage::Flashing { done: 300, total: 800 }) }),
        ("36-sideload-installed", false, Install::Finished { outcome: Outcome::Installed { slot: Slot::A, wipe: false }, damage: None }),
        ("96-sideload-light-wipes", false, Install::Confirming { version: Some("OS3.0.306.0.WMACNXM".into()), wipe: true }),
        ("18-install-ready", false, Install::Confirming { version: Some("OS3.0.306.0.WMACNXM".into()), wipe: false }),
        (
            "37-sideload-flashed-recovery-dark",
            true,
            Install::Finished { outcome: Outcome::Flashed { partition: "recovery", slots: vec![Slot::A, Slot::B] }, damage: None },
        ),
        (
            "38-sideload-refused",
            false,
            Install::Finished { outcome: Outcome::Refused(Refusal::OtherDevice("ishtar".into())), damage: None },
        ),
        (
            "39-sideload-failed",
            false,
            Install::Finished { outcome: Outcome::Failed(Failure::Engine(9)), damage: Some(Damage::System) },
        ),
        (
            "40-sideload-error-dark",
            true,
            Install::Finished {
                outcome: Outcome::Failed(Failure::Error("set USB up for sideload: open ep0: Device or resource busy (os error 16)".into())),
                damage: None,
            },
        ),
        (
            "199-sideload-system-choosing",
            false,
            Install::SystemChoosing(SystemChoice {
                name: "Ubuntu Touch".into(),
                version: "24.04-1.x".into(),
                sizes: [5, 8, 16, 32, 64].map(|gb: u64| gb * 1_000_000_000).to_vec(),
                size: 3,
            }),
        ),
        (
            "200-sideload-system-writing-dark",
            true,
            Install::Systeming { name: "Ubuntu Touch".into(), stage: Some(Stage::Root { done: 1_610_612_736, total: 9_663_676_416 }) },
        ),
        ("201-sideload-system-installed", false, Install::Finished { outcome: Outcome::SystemInstalled { name: "Ubuntu Touch".into() }, damage: None }),
    ];
    for (name, dark, install) in installing {
        let mut app = opened(Screen::Installing, w, h, ppd);
        app.state.dark = dark;
        app.state.install = install;
        let path = out.join(format!("{name}.png"));
        write_png(&path, &whole_page(&mut app, &fonts, &info))?;
        println!("wrote {}", path.display());
    }

    // The mount page as it opens after boot, after an unmount that failed,
    // and while an update waits to merge, with some partitions unmounted.
    let busy = MountFailure {
        mounting: false,
        base: "vendor".into(),
        detail: "unmount /vendor: Device or resource busy (os error 16)".into(),
    };
    let mounting: [MountShot; 3] = [
        ("15-mount-light", false, &info, None, &[]),
        ("16-mount-dark-unmount-failed", true, &info, Some(busy), &[]),
        ("17-mount-light-update-pending", false, &pending, None, &["product", "system_ext"]),
    ];
    for (name, dark, info, failure, unmounted) in mounting {
        let mut app = opened(Screen::Mount, w, h, ppd);
        app.state.dark = dark;
        app.set_mounts(Ok(sample_mounts(unmounted)));
        if let Some(failure) = failure {
            app.mount_failed(failure);
        }
        let path = out.join(format!("{name}.png"));
        write_png(&path, &whole_page(&mut app, &fonts, info))?;
        println!("wrote {}", path.display());
    }

    // The pages the remaining features open, each with what it would show.
    let made = |place, made, size, parts, system: &str| {
        let held = Held { made, size, parts, device: cfg.device.codename.clone(), system: Some(system.into()), slot: Some(Slot::B) };
        BackupEntry { place, name: fmt::stamp(made, 0), held: Ok(held) }
    };
    let backups = vec![
        made(Place::Storage, 1_792_348_260, 6_214_000_000, [true, true, false, true], "OS4.0.4.0.XPACNXM"),
        made(Place::UsbDrive, 1_791_830_000, 141_000_000, [true, false, false, false], "OS4.0.3.0.XPACNXM"),
        BackupEntry {
            place: Place::Storage,
            name: "20261020-221500".into(),
            held: Err("read /sdcard/Skyward/backups/20261020-221500/backup.toml: No such file or directory (os error 2)".into()),
        },
    ];
    // What a backup of the slot's partitions and the baseband goes through.
    let plan = || -> Vec<(String, u64)> {
        [
            ("boot", 100_663_296),
            ("init_boot", 8_388_608),
            ("vendor_boot", 100_663_296),
            ("dtbo", 25_165_824),
            ("vbmeta", 65_536),
            ("vbmeta_system", 65_536),
            ("recovery", 104_857_600),
            ("system", 1_253_212_160),
            ("system_ext", 702_984_192),
            ("product", 4_312_031_232),
            ("vendor", 2_101_317_632),
            ("odm", 1_420_455_936),
            ("vendor_dlkm", 104_177_664),
            ("system_dlkm", 26_177_536),
            ("modemst1", 2_097_152),
            ("modemst2", 2_097_152),
            ("fsg", 8_388_608),
            ("fsc", 131_072),
            ("persist", 67_108_864),
        ]
        .map(|(name, size)| (name.to_string(), size))
        .to_vec()
    };
    let job = |restore, plan: Vec<(String, u64)>, at, outcome| {
        Some(BackupJob { restore, place: Place::Storage, checks: true, plan, at, outcome, stopping: false })
    };
    let reset_asked = || Asked { reason: Some("factroy_reset(1217)".into()), ..Asked::default() };
    let in_download = |app: &mut App, name: &str| {
        app.set_files("/sdcard/Download".into(), Ok(sample_download()), LISTED);
        app.state.file = sample_download().into_iter().find(|e| e.name == name);
    };
    let ota = "/sdcard/Download/OTA_ishtar_OS3.0.305.0.WMACNXM_20260412_full_package_signed_for_recovery_install.zip";
    let copying = || FileTask::Copy { path: ota.into(), into: "/sdcard/Skyward".into() };
    let from_storage = |app: &mut App, install: Install| {
        app.state.source = Source::File(Place::Storage);
        app.state.install = install;
    };
    // What a ROM's install script shows as it runs, in the ways scripts lay
    // their lines out; the failing one stops on writing vendor.
    let said = |failed: bool| -> Vec<String> {
        let mut lines = vec![
            " ",
            "******************************",
            "  Example ROM 2.1 for ishtar",
            "******************************",
            " ",
            "- 正在检查设备型号",
            "- Mounting system and vendor",
            "- Flashing boot to /dev/block/by-name/boot_b",
            "- Flashing dtbo and vendor_boot to the inactive slot, which takes a few seconds on most devices",
            "  Partition      Size       State",
            "  system_b       1.17 GiB   written",
        ];
        lines.extend(match failed {
            false => ["  vendor_b       1.96 GiB   written", "- Cleaning up", "- Done"].as_slice(),
            true => ["! Could not write vendor_b", "! Installation aborted"].as_slice(),
        });
        lines.into_iter().map(String::from).collect()
    };
    let script = |app: &mut App, install: Install, failed: bool| {
        from_storage(app, install);
        app.state.package_name = Some("ExampleROM-2.1-ishtar.zip".into());
        app.state.script_output = said(failed);
    };
    let usb_page = Screen::Choose(Place::UsbDrive);
    let adding = |app: &mut App, name: &str, security: Security, field: Field, choosing: bool| {
        app.state.adding = Some(Adding { name: name.into(), security, field, choosing });
    };
    let pages: [PageShot; 178] = [
        ("11-install-light-locked", Screen::Install, false, &|app| {
            app.state.data = Data::Locked { user: 0, keys: Keys::Digits, checking: false };
        }),
        ("11-install-light-dual-boot", Screen::Install, false, &|app| app.state.dual_boot = true),
        ("11-install-dark-usb", Screen::Install, true, &|app| {
            app.set_usb_drive(Some(UsbDrive::Shown));
        }),
        ("19-choose-usb-light-absent", usb_page, false, &|_| {}),
        ("19-choose-usb-dark-unreadable", usb_page, true, &|app| {
            let why = "sdg holds no filesystem the kernel reads: sdg1 as vfat: Invalid argument (os error 22); sdg1 as exfat: Invalid argument (os error 22); sdg1 as ntfs3: No such device (os error 19); sdg1 as ext4: Invalid argument (os error 22); sdg1 as f2fs: Invalid argument (os error 22)";
            app.set_usb_drive(Some(UsbDrive::Unreadable(why.into())));
        }),
        ("19-choose-usb-light", usb_page, false, &|app| {
            app.set_usb_drive(Some(UsbDrive::Shown));
            let made = 1_790_380_800;
            let rows = vec![folder("ROMs", 4, made), file("boot.img", 100_663_296, made), file("ishtar-ota-OS3.0.306.0.WMACNXM.zip", 7_103_043_868, made)];
            app.set_place(Place::UsbDrive, String::new(), Ok(rows), LISTED);
        }),
        ("20-install-usb-checking", Screen::Installing, false, &|app| {
            app.state.source = Source::File(Place::UsbDrive);
            app.state.install = Install::Checking { size: 7_103_043_868, stage: None };
        }),
        ("19-choose-storage-light", Screen::Choose(Place::Storage), false, &|app| {
            app.set_place(Place::Storage, "Download".into(), Ok(sample_download()), LISTED)
        }),
        ("19-choose-storage-dark-empty", Screen::Choose(Place::Storage), true, &|app| {
            app.set_place(Place::Storage, "Download/Skyward".into(), Ok(vec![file("notes.txt", 2_048, LISTED - 600)]), LISTED)
        }),
        ("20-install-storage-checking", Screen::Installing, false, &|app| {
            from_storage(app, Install::Checking { size: 7_014_220_000, stage: None })
        }),
        ("20-install-storage-writing-dark", Screen::Installing, true, &|app| {
            let stage = Some(Stage::Writing { done: 2_934_000_000, total: 7_014_220_000 });
            from_storage(app, Install::Updating { version: Some("OS3.0.305.0.WMACNXM".into()), stage, overwriting: false })
        }),
        ("20-install-storage-refused", Screen::Installing, false, &|app| {
            from_storage(app, Install::Finished { outcome: Outcome::Refused(Refusal::Unknown), damage: None })
        }),
        ("20-install-storage-failed", Screen::Installing, false, &|app| {
            let gone = "read /sdcard/Download/OTA_ishtar_OS3.0.305.0.WMACNXM.zip: Input/output error (os error 5)";
            from_storage(app, Install::Finished { outcome: Outcome::Failed(Failure::Unreadable(gone.into())), damage: Some(Damage::System) })
        }),
        ("20-install-script-confirming", Screen::Installing, false, &|app| {
            script(app, Install::ConfirmingScript, false);
            app.state.script_output.clear();
        }),
        ("20-install-script-running-dark", Screen::Installing, true, &|app| {
            script(app, Install::Scripting { stage: Some(Stage::Script { thousandths: 420 }), running: true }, false);
            app.state.script_output.truncate(9);
        }),
        ("20-install-script-ran", Screen::Installing, false, &|app| {
            script(app, Install::Finished { outcome: Outcome::Ran { wipe_cache: false }, damage: None }, false)
        }),
        ("20-install-script-failed-dark", Screen::Installing, true, &|app| {
            script(app, Install::Finished { outcome: Outcome::Failed(Failure::Script(1)), damage: Some(Damage::Script) }, true)
        }),
        ("20-install-script-reflashing", Screen::Installing, false, &|app| {
            let stage = Some(Stage::Reflashing { done: 52_428_800, total: 209_715_200 });
            script(app, Install::Scripting { stage, running: true }, false)
        }),
        ("20-install-script-ran-reflashed-dark", Screen::Installing, true, &|app| {
            script(app, Install::Finished { outcome: Outcome::Ran { wipe_cache: false }, damage: None }, false);
            app.state.reflash = Some(Reflash::Written(vec![Slot::A, Slot::B]));
        }),
        ("20-install-script-reflash-failed", Screen::Installing, false, &|app| {
            script(app, Install::Finished { outcome: Outcome::Failed(Failure::Script(1)), damage: Some(Damage::Script) }, true);
            let why = "write /dev/block/by-name/recovery_a: Input/output error (os error 5)";
            app.state.reflash = Some(Reflash::Failed(why.into()));
        }),
        ("20-install-script-verifying", Screen::Installing, false, &|app| {
            let stage = Some(Stage::Signature { done: 10_485_760, total: 25_164_934 });
            script(app, Install::Checking { size: 25_166_592, stage }, false);
            app.state.script_output.clear();
        }),
        ("20-install-script-unsigned-dark", Screen::Installing, true, &|app| {
            script(app, Install::Finished { outcome: Outcome::Refused(Refusal::Unsigned), damage: None }, false);
            app.state.script_output.clear();
        }),
        ("20-install-script-unverified", Screen::Installing, false, &|app| {
            script(app, Install::Finished { outcome: Outcome::Refused(Refusal::Unverified), damage: None }, false);
            app.state.script_output.clear();
        }),
        ("11-install-dark-signature", Screen::Install, true, &|app| {
            app.state.recovery_partitions = true;
            app.state.check_signature = true;
        }),
        ("20-install-storage-installed-reflashed", Screen::Installing, false, &|app| {
            from_storage(app, Install::Finished { outcome: Outcome::Installed { slot: Slot::A, wipe: false }, damage: None });
            app.state.reflash = Some(Reflash::Written(vec![Slot::A]));
        }),
        ("11-install-light-recovery-partitions", Screen::Install, false, &|app| app.state.recovery_partitions = true),
        ("41-search-light", Screen::Search, false, &|_| {}),
        ("42-search-light-found", Screen::Search, false, &|app| app.state.search = "b".into()),
        ("43-search-dark-pinyin", Screen::Search, true, &|app| app.state.search = "zd".into()),
        ("44-backup-light", Screen::Backup, false, &|_| {}),
        ("45-backup-dark", Screen::Backup, true, &|app| {
            app.set_usb_drive(Some(UsbDrive::Shown));
            app.state.backup_to = Place::UsbDrive;
        }),
        ("46-restore-light-none", Screen::Restore, false, &|_| {}),
        ("47-restore-light", Screen::Restore, false, &|app| app.set_backups(backups.clone())),
        ("48-restore-backup-light", Screen::RestoreBackup, false, &|app| {
            app.state.restore = Some((backups[0].clone(), [true, false, false, true]));
        }),
        ("48-restore-backup-dark-deleting", Screen::RestoreBackup, true, &|app| {
            app.state.restore = Some((backups[0].clone(), [true, true, false, true]));
            app.state.asking_delete = true;
        }),
        ("48-restore-backup-light-other-version", Screen::RestoreBackup, false, &|app| {
            app.state.restore = Some((backups[1].clone(), [true, false, false, false]));
        }),
        ("48-restore-backup-light-unreadable", Screen::RestoreBackup, false, &|app| {
            app.state.restore = Some((backups[2].clone(), [false; 4]));
        }),
        ("100-backup-job-light-measuring", Screen::BackupJob, false, &|app| app.state.backup_job = job(false, Vec::new(), None, None)),
        ("101-backup-job-light", Screen::BackupJob, false, &|app| {
            app.state.backup_job = job(false, plan(), Some(BackupAt { item: 9, done: 1_820_000_000, of: 4_312_031_232, checking: false }), None)
        }),
        ("102-backup-job-dark-checking", Screen::BackupJob, true, &|app| {
            app.state.backup_job = job(false, plan(), Some(BackupAt { item: 11, done: 300_000_000, of: 1_420_455_936, checking: true }), None)
        }),
        ("103-backup-job-light-saved", Screen::BackupJob, false, &|app| {
            app.state.backup_job = job(false, plan(), None, Some(BackupEnd::Saved("20261021-200110".into())))
        }),
        ("104-backup-job-light-room", Screen::BackupJob, false, &|app| {
            let room = BackupRefusal::Room { needed: 52_400_000_000, free: 31_200_000_000 };
            app.state.backup_job = job(false, Vec::new(), None, Some(BackupEnd::Refused(room)))
        }),
        ("105-restore-job-light-checking", Screen::BackupJob, false, &|app| {
            app.state.backup_job = job(true, plan(), Some(BackupAt { item: 3, done: 12_000_000, of: 25_165_824, checking: true }), None)
        }),
        ("106-restore-job-dark-writing", Screen::BackupJob, true, &|app| {
            app.state.backup_job = job(true, plan(), Some(BackupAt { item: 10, done: 2_600_000_000, of: 4_202_635_264, checking: false }), None)
        }),
        ("107-restore-job-light-done", Screen::BackupJob, false, &|app| {
            let unmade = vec!["/data/user/10".into(), "/data/user_de/10".into(), "/data/misc_ce/10".into()];
            app.state.backup_job = job(true, plan(), None, Some(BackupEnd::Restored { unmade }))
        }),
        ("108-restore-job-light-failed", Screen::BackupJob, false, &|app| {
            let at = Some(BackupAt { item: 8, done: 400_000_000, of: 1_405_968_384, checking: false });
            app.state.backup_job = job(true, plan(), at, Some(BackupEnd::Failed("write /dev/block/dm-61: Input/output error (os error 5)".into())))
        }),
        ("109-restore-job-dark-refused", Screen::BackupJob, true, &|app| {
            app.state.backup_job = job(true, Vec::new(), None, Some(BackupEnd::Refused(BackupRefusal::DataReset)))
        }),
        ("49-advanced-light", Screen::Advanced, false, &|_| {}),
        ("50-files-light", Screen::Files, false, &|app| app.set_files("/".into(), Ok(sample_root()), LISTED)),
        ("51-files-dark", Screen::Files, true, &|app| app.set_files("/".into(), Ok(sample_root()), LISTED)),
        ("52-file-info-light", Screen::FileInfo, false, &|app| app.state.file = sample_root().into_iter().find(|e| e.name == "init.environ.rc")),
        ("52-file-permissions-dark", Screen::Permissions, true, &|app| {
            app.state.file = sample_root().into_iter().find(|e| e.name == "init.environ.rc")
        }),
        ("61-files-names-light", Screen::Files, false, &|app| app.set_files("/sdcard/Download".into(), Ok(sample_download()), LISTED)),
        ("110-files-types-light", Screen::Files, false, &|app| app.set_files("/sdcard/Download".into(), Ok(sample_types()), LISTED)),
        ("110-files-types-dark", Screen::Files, true, &|app| app.set_files("/sdcard/Download".into(), Ok(sample_types()), LISTED)),
        ("62-files-empty-light", Screen::Files, false, &|app| app.set_files("/cust".into(), Ok(Vec::new()), LISTED)),
        ("63-files-unreadable-dark", Screen::Files, true, &|app| {
            app.set_files("/sys/kernel/debug".into(), Err("read /sys/kernel/debug: Permission denied (os error 13)".into()), LISTED)
        }),
        ("53-terminal-light", Screen::Terminal, false, &|app| {
            let (rows, cols) = app.term_size(&fonts);
            app.term_started(rows, cols);
            app.term_output(SHELL_SAMPLE);
        }),
        ("54-terminal-dark-symbols", Screen::Terminal, true, &|app| {
            let (rows, cols) = app.term_size(&fonts);
            app.term_started(rows, cols);
            app.term_output(SHELL_SAMPLE);
            app.state.layer = Layer::Symbols;
        }),
        ("124-editor-light", Screen::Files, false, &|app| {
            app.text_opened(&fonts, "/data/adb/service.d/tune.sh".into(), Ok(Some(EDITOR_SAMPLE.into())), 0);
        }),
        ("125-editor-dark-symbols", Screen::Files, true, &|app| {
            let path = "/vendor/etc/init/hw/init.target.performance.rc";
            app.text_opened(&fonts, path.into(), Ok(Some(EDITOR_SAMPLE.into())), 0);
            app.state.layer = Layer::Symbols;
        }),
        ("127-relabel-light", Screen::Relabel, false, &|_| {}),
        ("128-relabel-dark-closed", Screen::Relabel, true, &|app| app.state.data = Data::Closed),
        ("129-relabel-light-running", Screen::Relabel, false, &|app| {
            app.relabel_started();
            app.relabel_progress(48_213, 131_908, 6);
        }),
        ("130-relabel-dark-done", Screen::Relabel, true, &|app| {
            app.relabel_started();
            app.relabel_progress(131_908, 131_908, 37);
            app.relabel_ended(JobEnd::Done, 0);
        }),
        ("131-home-light-wlan", Screen::Home, false, &|app| app.state.wlan = Some(joined(Joining::Connected(router_link())))),
        ("132-wlan-light-off", Screen::Wlan, false, &|app| app.state.wlan = Some(Wlan::default())),
        ("133-wlan-dark-starting", Screen::Wlan, true, &|app| app.state.wlan = Some(Wlan { power: Power::Starting, ..Wlan::default() })),
        ("134-wlan-light-nearby", Screen::Wlan, false, &|app| app.state.wlan = Some(Wlan { power: Power::On, networks: nearby(), joined: None })),
        ("135-wlan-light-joined", Screen::Wlan, false, &|app| app.state.wlan = Some(joined(Joining::Connected(router_link())))),
        ("136-wlan-dark-wrong-password", Screen::Wlan, true, &|app| app.state.wlan = Some(joined(Joining::WrongPassword))),
        ("137-wlan-light-addressing", Screen::Wlan, false, &|app| app.state.wlan = Some(joined(Joining::Addressing))),
        ("138-wlan-light-failed", Screen::Wlan, false, &|app| {
            let why = "wpa_supplicant ended as it started: exit status: 255".into();
            app.state.wlan = Some(Wlan { power: Power::Failed(why), ..Wlan::default() });
        }),
        ("139-wlan-password-light", Screen::WlanPassword, false, &|app| {
            app.state.joining = Some(nearby()[0].clone());
            app.state.password = "skyward2024".into();
        }),
        ("140-wlan-password-dark-shown", Screen::WlanPassword, true, &|app| {
            app.state.joining = Some(nearby()[2].clone());
            app.state.password = "skyward2024".into();
            app.state.show_password = true;
        }),
        ("141-wlan-network-light", Screen::WlanNetwork, false, &|app| {
            app.state.wlan = Some(joined(Joining::Connected(router_link())));
            app.state.details = Some(nearby()[0].clone());
        }),
        ("142-wlan-network-dark", Screen::WlanNetwork, true, &|app| {
            app.state.wlan = Some(joined(Joining::Connected(router_link())));
            app.state.details = Some(nearby()[0].clone());
        }),
        ("152-wlan-network-light-nearby", Screen::WlanNetwork, false, &|app| {
            app.state.wlan = Some(joined(Joining::Connected(router_link())));
            app.state.details = Some(nearby()[7].clone());
        }),
        ("153-wlan-light-searching", Screen::Wlan, false, &|app| app.state.wlan = Some(Wlan { power: Power::On, ..Wlan::default() })),
        ("154-wlan-add-light", Screen::WlanAdd, false, &|app| adding(app, "", Security::Open, Field::Name, false)),
        ("155-wlan-add-light-password", Screen::WlanAdd, false, &|app| {
            adding(app, "Skyward-Attic", Security::Psk, Field::Password, false);
            app.state.password = "skyward2024".into();
        }),
        ("156-wlan-add-light-choosing", Screen::WlanAdd, false, &|app| adding(app, "Skyward-Attic", Security::Psk, Field::Name, true)),
        ("157-wlan-add-dark-choosing", Screen::WlanAdd, true, &|app| {
            adding(app, "Skyward-Attic", Security::Open, Field::Name, true);
            app.state.held = Some(Action::PickSecurity(Security::Sae));
        }),
        ("158-wlan-light-traffic", Screen::Wlan, false, &|app| {
            app.state.wlan = Some(joined(Joining::Connected(router_link())));
            app.state.traffic = Traffic { rx: 5000, tx: 2188 };
        }),
        ("182-wlan-light-login", Screen::Wlan, false, &|app| {
            let login = Reach::Login(Some("http://192.168.64.254:2060/redirect/?pip=192.168.72.76".into()));
            app.state.wlan = Some(joined(Joining::Connected(Link { reach: Some(login), ..router_link() })));
        }),
        ("159-filesystems-light", Screen::Filesystems, false, &|app| app.set_filesystems(Ok(phone_filesystems(true)))),
        ("160-filesystems-dark-closed", Screen::Filesystems, true, &|app| {
            app.state.data = Data::Closed;
            app.set_filesystems(Ok(phone_filesystems(false)));
        }),
        ("161-filesystem-light-data", Screen::Filesystem, false, &|app| app.set_filesystems(Ok(phone_filesystems(true)))),
        ("162-filesystem-dark-cache", Screen::Filesystem, true, &|app| {
            let mut rows = phone_filesystems(true);
            rows.push(FsRow {
                name: "cache".into(),
                mount_point: "/cache".into(),
                filesystem: Some("ext4".into()),
                size: 512 << 20,
                others: vec!["f2fs".into()],
            });
            app.set_filesystems(Ok(rows));
            app.state.cache_partition = true;
            app.state.fs_chosen = 2;
            app.state.fs_action = FsAction::Remake("f2fs".into());
        }),
        ("163-part-work-light-repairing", Screen::Filesystem, false, &|app| {
            app.set_filesystems(Ok(phone_filesystems(true)));
            app.part_work_started(PartKind::Repair, false);
            app.part_listed(vec!["data".into()]);
            app.part_at(0);
        }),
        ("164-part-work-dark-repaired", Screen::Filesystem, true, &|app| {
            app.set_filesystems(Ok(phone_filesystems(true)));
            app.part_work_started(PartKind::Repair, false);
            app.part_listed(vec!["data".into()]);
            app.part_ended(0, ItemEnd::Fixed, FSCK_SAMPLE);
            app.part_work_ended(Ok(()), 0);
        }),
        ("165-part-work-light-unshare-done", Screen::Home, false, &|app| {
            app.part_work_started(PartKind::Unshare, true);
            app.part_listed(["system", "system_ext", "product", "vendor", "odm"].map(String::from).to_vec());
            for k in 0..5 {
                app.part_ended(k, ItemEnd::NotShared("erofs".into()), "");
            }
            app.part_work_ended(Ok(()), 0);
        }),
        ("166-part-work-dark-disable-refused", Screen::Home, true, &|app| {
            app.set_asked(Asked { reason: Some("wipe_ab".into()), ..Asked::default() });
            app.part_work_started(PartKind::Disable, true);
            app.part_work_ended(Err(PartStop::Refused(Refusal::Unverified)), 0);
        }),
        ("167-part-work-light-disable-erasing", Screen::Home, false, &|app| {
            app.part_work_started(PartKind::Disable, true);
            app.part_listed(["userdata", "metadata", "super"].map(String::from).to_vec());
            app.part_ended(0, ItemEnd::Erased, "");
            app.part_at(1);
        }),
        ("168-installing-light-asked-locked", Screen::Home, false, &|app| {
            app.start_asked(false, "/sdcard/Download/ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0.zip".into());
            app.asked_locked(0, 0);
        }),
        ("169-installing-dark-asked-opening", Screen::Home, true, &|app| {
            app.set_asked(Asked { reason: Some("update".into()), ..Asked::default() });
            app.start_asked(true, "@/cache/recovery/block.map".into());
            app.asked_opening();
        }),
        ("170-installing-light-rescue-waiting", Screen::Home, false, &|app| {
            app.start_rescue();
            app.install(skyward_install::Event::Waiting, 0);
        }),
        ("171-installing-dark-rescue-wiping", Screen::Home, true, &|app| {
            app.start_rescue();
            app.rescue_wiping();
        }),
        ("172-installing-light-rescue-installed", Screen::Home, false, &|app| {
            app.start_rescue();
            app.install(skyward_install::Event::Done(Outcome::Installed { slot: Slot::B, wipe: false }), 0);
        }),
        ("173-installing-light-asked-installed", Screen::Home, false, &|app| {
            app.start_asked(false, "@/cache/recovery/block.map".into());
            app.install(skyward_install::Event::Finished(Outcome::Installed { slot: Slot::B, wipe: false }), 0);
        }),
        ("174-installing-dark-asked-panicked", Screen::Home, true, &|app| {
            app.start_asked(false, "@/cache/recovery/block.map".into());
            app.install(skyward_install::Event::Finished(Outcome::Refused(Refusal::Panicked)), 0);
        }),
        ("175-installing-light-asked-image", Screen::Home, false, &|app| {
            app.start_asked(false, "/data/ota_package/boot.img".into());
            app.install(skyward_install::Event::Finished(Outcome::Refused(Refusal::Image)), 0);
        }),
        ("176-part-work-light-repair-failed", Screen::Filesystem, false, &|app| {
            app.set_filesystems(Ok(phone_filesystems(false)));
            app.part_work_started(PartKind::Repair, false);
            app.part_listed(vec!["data".into()]);
            app.part_ended(0, ItemEnd::Failed(None), FSCK_FAILED_SAMPLE);
            app.part_work_ended(Ok(()), 0);
        }),
        ("177-part-work-dark-repair-unreached", Screen::Filesystem, true, &|app| {
            app.set_filesystems(Ok(phone_filesystems(false)));
            app.part_work_started(PartKind::Repair, false);
            app.part_listed(vec!["data".into()]);
            let why = "android.hardware.security.keymint.IKeyMintDevice/default is not running; the phone's key service has to be started first";
            app.part_ended(0, ItemEnd::Failed(Some(why.into())), "");
            app.part_work_ended(Ok(()), 0);
        }),
        ("178-file-info-light-package", Screen::FileInfo, false, &|app| {
            in_download(app, "OTA_ishtar_OS3.0.305.0.WMACNXM_20260412_full_package_signed_for_recovery_install.zip")
        }),
        ("179-file-info-dark-image", Screen::FileInfo, true, &|app| in_download(app, "skyward-recovery-ishtar.img")),
        ("180-install-opened-light-checking", Screen::Installing, false, &|app| {
            app.state.source = Source::Opened { drive: false };
            app.state.install = Install::Checking { size: 7_014_220_000, stage: None };
        }),
        ("181-install-opened-dark-usb", Screen::Installing, true, &|app| {
            app.state.source = Source::Opened { drive: true };
            app.state.install = Install::Checking { size: 104_857_600, stage: None };
        }),
        ("143-update-light-offline", Screen::Update, false, &|app| {
            app.state.update_server = true;
            app.state.wlan = Some(Wlan::default());
        }),
        ("144-update-light-newer", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Nothing);
        }),
        ("145-update-dark-downloading", Screen::Update, true, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Nothing);
            app.state.download = Some(DownloadJob {
                progress: Some(Progress::Downloading { done: 2_843_210_752, total: 7_572_375_802 }),
                rate: 23_400_000,
                outcome: None,
                stopping: false,
            });
        }),
        ("146-update-light-checking-part", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Part(2_843_210_752));
            let progress = Some(Progress::Checking { done: 1_022_000_000, total: 2_843_210_752 });
            app.state.download = Some(DownloadJob { progress, rate: 0, outcome: None, stopping: false });
        }),
        ("147-update-light-paused", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Part(2_843_210_752));
            app.state.download = Some(DownloadJob { progress: None, rate: 0, outcome: Some(DownloadEnd::Stopped), stopping: false });
        }),
        ("148-update-light-no-room", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Nothing);
            let full = DownloadFailure::NoRoom { needed: 7_572_375_802, free: 3_104_000_000 };
            app.state.download = Some(DownloadJob { progress: None, rate: 0, outcome: Some(DownloadEnd::Failed(full)), stopping: false });
        }),
        ("149-update-dark-downloaded", Screen::Update, true, &|app| {
            online(app);
            app.update_checked(Ok(Some(newest())), Have::Whole(7_572_375_802));
        }),
        ("150-update-light-latest", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Ok(Some(Release { version: "OS4.0.4.0.XPACNXM".into(), ..newest() })), Have::Nothing);
        }),
        ("151-update-light-unanswered", Screen::Update, false, &|app| {
            online(app);
            let why = "ask https://update.miui.com/updates/miotaV3.php: io: the DNS server at 192.168.1.1:53 did not answer";
            app.update_checked(Err(Unanswered::Failed(why.into())), Have::Nothing);
        }),
        ("183-update-light-login", Screen::Update, false, &|app| {
            online(app);
            app.update_checked(Err(Unanswered::Login), Have::Nothing);
        }),
        ("190-skyward-update-light-offline", Screen::SkywardUpdate, false, &|app| {
            app.state.recovery_partitions = true;
            app.state.wlan = Some(Wlan::default());
        }),
        ("191-skyward-update-light-newer", Screen::SkywardUpdate, false, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
        }),
        ("192-skyward-update-dark-downloading", Screen::SkywardUpdate, true, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
            let stage = Some(SkywardStage::Download(Progress::Downloading { done: 17_104_000, total: 43_145_196 }));
            app.state.skyward_job = Some(SkywardJob { stage, rate: 3_260_000, outcome: None, stopping: false });
        }),
        ("193-skyward-update-light-writing", Screen::SkywardUpdate, false, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
            let stage = Some(SkywardStage::Write { done: 140_000_000, total: 419_430_400 });
            app.state.skyward_job = Some(SkywardJob { stage, rate: 0, outcome: None, stopping: false });
        }),
        ("194-skyward-update-light-updated", Screen::SkywardUpdate, false, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
            let outcome = Some(SkywardEnd::Updated(vec![Slot::A, Slot::B]));
            app.state.skyward_job = Some(SkywardJob { stage: None, rate: 0, outcome, stopping: false });
        }),
        ("195-skyward-update-dark-latest", Screen::SkywardUpdate, true, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(Newest { version: "0.1.3".into(), newer: false, ..skyward_newest() }));
        }),
        ("196-skyward-update-light-write-failed", Screen::SkywardUpdate, false, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
            let why = "/dev/block/by-name/recovery_a reads back different from what was written";
            let outcome = Some(SkywardEnd::WriteFailed(vec![Slot::A], why.into()));
            app.state.skyward_job = Some(SkywardJob { stage: None, rate: 0, outcome, stopping: false });
        }),
        ("197-skyward-update-light-unanswered", Screen::SkywardUpdate, false, &|app| {
            skyward_online(app);
            let why = "GitHub answered 403 Forbidden: API rate limit exceeded for 203.0.113.7. (But here's the good news: Authenticated requests get a higher rate limit.)";
            app.skyward_checked(Err(Unanswered::Failed(why.into())));
        }),
        ("198-settings-light-skyward-newer", Screen::Settings, false, &|app| {
            skyward_online(app);
            app.skyward_checked(Ok(skyward_newest()));
        }),
        ("75-terminal-light-ended", Screen::Terminal, false, &|app| {
            let (rows, cols) = app.term_size(&fonts);
            app.term_started(rows, cols);
            app.term_output(b"ishtar:/ # exit\r\n");
            app.term_ended(TermEnd::Exited(0));
        }),
        ("55-language-light", Screen::Language, false, &|_| {}),
        ("56-time-zone-light", Screen::TimeZone, false, &|_| {}),
        ("56-time-zone-dark-london", Screen::TimeZone, true, &|app| app.state.time_zone = 5),
        ("57-logs-light", Screen::Logs, false, &|_| {}),
        ("113-update-light-waiting", Screen::Update, false, &|app| app.set_update(Standing::Waiting)),
        ("114-update-dark-merging", Screen::Update, true, &|app| app.set_update(Standing::Merging(0.37))),
        ("115-update-light-installed", Screen::Update, false, &|app| app.set_update(Standing::Installed(Slot::A))),
        // Installed by an updater that has slot A start next only as it
        // restarts the phone itself.
        ("118-update-light-installed-unswitched", Screen::Update, false, &|app| {
            app.set_update(Standing::Installed(Slot::A));
            app.set_boot_slots(Ok(BootSlots { active: Slot::B, bootable: [false, true], successful: [false, true] }));
        }),
        ("116-update-dark-failed", Screen::Update, true, &|app| app.set_update(Standing::Failed(FailureCode::ExpectedMergeTarget))),
        ("117-advanced-light-merged", Screen::Advanced, false, &|app| app.set_update(Standing::Merged)),
        ("58-license-light", Screen::License, false, &|_| {}),
        ("59-font-license-light", Screen::FontLicense, false, &|_| {}),
        ("60-slot-dark", Screen::Slot, true, &|app| {
            app.set_boot_slots(Ok(BootSlots { active: Slot::B, bootable: [false, true], successful: [true, true] }));
            app.state.next_slot = Some(Slot::A);
        }),
        ("60-slot-light-unread", Screen::Slot, false, &|app| {
            app.set_boot_slots(Err("android.hardware.boot.IBootControl/default is not running; the phone's boot control has to be started first".into()))
        }),
        ("202-systems-light", Screen::Systems, false, &|app| app.set_systems(Ok(two_systems()))),
        ("202-systems-light-picked", Screen::Systems, false, &|app| {
            app.set_systems(Ok(two_systems()));
            app.state.system_pick = Some(Who::Other);
        }),
        ("202-systems-dark", Screen::Systems, true, &|app| app.set_systems(Ok(two_systems()))),
        ("202-systems-dark-second-starts", Screen::Systems, true, &|app| {
            app.set_systems(Ok(Overview { other_starts: true, changed: true, ..two_systems() }));
            app.state.asking_delete = true;
        }),
        ("202-systems-light-empty", Screen::Systems, false, &|app| app.set_systems(Ok(Overview { other: None, ..two_systems() }))),
        ("202-systems-dark-switching", Screen::Systems, true, &|app| {
            app.set_systems(Ok(two_systems()));
            app.state.systems_work = Some(SystemsWork::Switching(Who::Other));
        }),
        ("202-systems-light-stopped", Screen::Systems, false, &|app| {
            app.set_systems(Ok(Overview { other_starts: true, ..two_systems() }));
            app.state.system_notice = Some("the root holds no program /sbin/init to start".into());
            app.state.data = Data::Closed;
        }),
        ("203-home-light-systems", Screen::Home, false, &|app| {
            app.state.dual_boot = true;
            app.set_systems(Ok(Overview { other_starts: true, ..two_systems() }));
            app.state.system_notice = Some("the root holds no program /sbin/init to start".into());
        }),
        ("70-erase-light", Screen::Erase, false, &|app| {
            app.set_asked(reset_asked());
            app.erase_started(true, true, vec![Part::Data, Part::Metadata]);
            app.erase_at(Part::Metadata);
        }),
        ("71-erase-dark-failed", Screen::Erase, true, &|app| {
            app.set_asked(reset_asked());
            app.erase_started(true, true, vec![Part::Data, Part::Metadata]);
            app.erase_at(Part::Metadata);
            app.erase_ended(
                Err(EraseStop::Failed(
                    Part::Metadata,
                    "/system/bin/make_f2fs -g android -O project_quota,extra_attr /dev/block/by-name/metadata ended with exit status: 1: Error: Device size is not sufficient for F2FS volume".into(),
                )),
                0,
            );
        }),
        ("72-erase-light-no-cache", Screen::Erase, false, &|app| {
            app.erase_started(false, true, Vec::new());
            app.erase_ended(Err(EraseStop::NoCache), 0);
        }),
        ("73-cant-start-light", Screen::CantStart, false, &|app| {
            app.set_asked(Asked { reason: Some("RescueParty".into()), ..Asked::default() })
        }),
        ("74-unhandled-dark", Screen::Unhandled, true, &|app| {
            app.set_asked(Asked { unhandled: Some("--update_package=/data/ota_package/update.zip".into()), ..Asked::default() })
        }),
        ("76-files-picked-light", Screen::Files, false, &|app| {
            app.set_files("/sdcard/Download".into(), Ok(sample_download()), LISTED);
            app.state.clip = Some(Clip { op: FileOp::Move, path: "/sdcard/README.txt".into() });
        }),
        ("77-folder-info-dark", Screen::FileInfo, true, &|app| in_download(app, "安装包")),
        ("78-file-info-light-deleting", Screen::FileInfo, false, &|app| {
            in_download(app, "README.txt");
            app.state.asking_delete = true;
        }),
        ("79-rename-light", Screen::Rename, false, &|app| {
            in_download(app, "README.txt");
            app.state.new_name = "README-old.txt".into();
        }),
        ("80-clash-dark", Screen::Clash, true, &|app| {
            app.file_clash(FileOp::Copy, "/sdcard/README.txt".into(), "/sdcard/Download".into(), vec!["README.txt".into()])
        }),
        ("81-file-job-light", Screen::Files, false, &|app| {
            app.file_job_started(copying());
            app.file_progress((2_934_000_000, 7_014_220_000), (0, 1));
        }),
        ("82-file-job-dark-measuring", Screen::Files, true, &|app| app.file_job_started(FileTask::Delete { path: "/sdcard/DCIM".into() })),
        ("83-file-job-light-stopped", Screen::Files, false, &|app| {
            app.file_job_started(copying());
            app.file_progress((2_934_000_000, 7_014_220_000), (0, 1));
            app.file_job_ended(JobEnd::Stopped, 0);
        }),
        ("84-file-job-dark-failed", Screen::Files, true, &|app| {
            app.file_job_started(FileTask::Move { path: "/tmp/skyward.log".into(), into: "/sdcard/Download".into() });
            app.file_progress((0, 18_000), (0, 1));
            app.file_job_ended(JobEnd::Failed("make /sdcard/Download/skyward.log: Read-only file system (os error 30)".into()), 0);
        }),
        ("85-decrypt-light-opening", Screen::Decrypt, false, &|app| app.state.data = Data::Opening),
        ("86-decrypt-light-digits", Screen::Decrypt, false, &|app| {
            app.data_waiting(Some((0, Keys::Digits)));
            app.state.code = "123".into();
        }),
        ("87-decrypt-dark-letters", Screen::Decrypt, true, &|app| app.data_waiting(Some((10, Keys::Letters)))),
        ("88-decrypt-light-checking", Screen::Decrypt, false, &|app| {
            app.state.data = Data::Locked { user: 0, keys: Keys::Digits, checking: true };
        }),
        ("89-decrypt-light-open", Screen::Decrypt, false, &|app| app.data_waiting(None)),
        ("90-decrypt-dark-failed", Screen::Decrypt, true, &|app| {
            app.data_failed("android.hardware.security.keymint.IKeyMintDevice/default is not running".into())
        }),
        ("91-wipe-light-all", Screen::Wipe, false, &|app| app.state.wipe[0] = true),
        ("92-wipe-dark-parts-cache", Screen::Wipe, true, &|app| {
            app.state.cache_partition = true;
            app.state.phone_logs = 7;
            app.state.wipe = [false, false, true, true, true, false, true, false, true];
        }),
        ("93-erase-light-media", Screen::Erase, false, &|app| {
            app.erase_started(false, false, vec![Part::Media, Part::AppCaches, Part::SystemCaches, Part::Logs]);
            app.erase_at(Part::AppCaches);
            app.erase_progress(41_380, 128_906);
        }),
        ("94-erase-light-reset-done", Screen::Erase, false, &|app| {
            app.erase_started(true, false, vec![Part::Data, Part::Metadata]);
            app.erase_ended(Ok(()), 0);
        }),
        ("95-erase-dark-merging", Screen::Erase, true, &|app| {
            app.erase_started(true, false, Vec::new());
            app.erase_ended(Err(EraseStop::Update(Hold::Merging)), 0);
        }),
        ("97-decrypt-light-erased", Screen::Decrypt, false, &|app| app.state.data = Data::Erased),
        ("98-erase-light-closed", Screen::Erase, false, &|app| {
            app.erase_started(false, false, vec![Part::Art]);
            app.erase_ended(Err(EraseStop::Closed("android.hardware.security.keymint.IKeyMintDevice/default is not running".into())), 0);
        }),
        ("99-erase-dark-locked", Screen::Erase, true, &|app| {
            app.erase_started(false, false, vec![Part::AppData, Part::AppCaches]);
            app.erase_ended(Err(EraseStop::Locked(vec![0, 999])), 0);
        }),
    ];
    for (name, screen, dark, set) in pages {
        let mut app = opened(screen, w, h, ppd);
        app.state.dark = dark;
        set(&mut app);
        let path = out.join(format!("{name}.png"));
        write_png(&path, &whole_page(&mut app, &fonts, &info))?;
        println!("wrote {}", path.display());
    }

    // The panel mid-gesture, to set beside recordings of the same moves.
    let moments: [Moment; 27] = [
        ("08-about-light", Screen::About, false, &|_, _| 2000),
        ("08-about-dark", Screen::About, true, &|_, _| 2000),
        ("08-about-light-later", Screen::About, false, &|_, _| 9000),
        ("21-settings-scrolled", Screen::Settings, false, &|app, f| drag(app, f, 1900.0, 1300.0)),
        ("22-settings-scrolled-dark", Screen::Settings, true, &|app, f| drag(app, f, 1900.0, 1300.0)),
        ("23-specs-title-fading", Screen::Specs, false, &|app, f| drag(app, f, 1900.0, 1870.0)),
        ("24-specs-title-gone", Screen::Specs, false, &|app, f| drag(app, f, 1900.0, 1820.0)),
        ("25-specs-under-bar", Screen::Specs, false, &|app, f| drag(app, f, 1900.0, 1450.0)),
        ("26-reboot-pulled-down", Screen::Reboot, false, &|app, f| hold(app, f, 900.0, 1700.0)),
        ("27-back-cue-short", Screen::Home, false, &|app, f| swipe_in(app, f, &info, 133.0)),
        ("28-back-cue-armed", Screen::Home, false, &|app, f| swipe_in(app, f, &info, 323.0)),
        ("29-about-scrolled", Screen::About, false, &|app, f| drag(app, f, 1900.0, 1500.0)),
        ("64-files-scrolled", Screen::Files, false, &|app, f| {
            app.set_files("/system/bin".into(), Ok(sample_bin()), LISTED);
            app.frame(f, &info);
            drag(app, f, 2000.0, 700.0)
        }),
        ("65-files-scrolled-dark", Screen::Files, true, &|app, f| {
            app.set_files("/system/bin".into(), Ok(sample_bin()), LISTED);
            app.frame(f, &info);
            drag(app, f, 2000.0, 700.0)
        }),
        // The fastboot page starts worked with the keys, its first choice lit.
        ("68-fastboot-light", Screen::Fastboot, false, &|app, _| {
            app.set_fastboot(true, UsbLink::Connected);
            0
        }),
        ("69-fastboot-dark-unplugged", Screen::Fastboot, true, &|app, f| {
            app.set_fastboot(true, UsbLink::Unplugged);
            keys_down(app, f, &info, 1)
        }),
        // The home page's search bar folding away under a finger: its words
        // fading, then the bar, folded whole once let go past halfway, and
        // past that the large title going.
        ("184-home-search-folding-light", Screen::Home, false, &|app, f| hold(app, f, 1900.0, 1834.0)),
        ("185-home-search-folding-dark-late", Screen::Home, true, &|app, f| hold(app, f, 1900.0, 1790.0)),
        ("186-home-search-folded-light", Screen::Home, false, &|app, f| drag(app, f, 1900.0, 1750.0)),
        ("187-home-title-going-dark", Screen::Home, true, &|app, f| {
            // The WLAN's row and a restart into the other slot make the page
            // long enough to go on past the fold.
            app.state.wlan = Some(joined(Joining::Connected(router_link())));
            app.set_boot_slots(Ok(BootSlots { active: Slot::A, bootable: [true, true], successful: [true, true] }));
            app.frame(f, &info);
            hold(app, f, 1900.0, 1700.0)
        }),
        ("111-home-keys-light", Screen::Home, false, &|app, f| keys_down(app, f, &info, 3)),
        ("112-home-keys-dark", Screen::Home, true, &|app, f| keys_down(app, f, &info, 9)),
        // A row held down until dark, then let go: the page it opens slides
        // in over it.
        ("119-home-row-held-light", Screen::Home, false, &|app, f| {
            finger_on(app, f, &info, Action::Open(Screen::Settings), 1000);
            frames(app, f, &info, 1000, 1400);
            1400
        }),
        ("120-home-row-opens-light", Screen::Home, false, &|app, f| {
            finger_on(app, f, &info, Action::Open(Screen::Settings), 1000);
            frames(app, f, &info, 1000, 1400);
            app.touch(Touch::Up, f, 1400);
            frames(app, f, &info, 1400, 1440);
            1450
        }),
        // `<` held down, then let go: the page slides off over the one gone
        // back to.
        ("121-settings-back-held-dark", Screen::Home, true, &|app, f| {
            finger_on(app, f, &info, Action::Open(Screen::Settings), 1000);
            app.touch(Touch::Up, f, 1050);
            frames(app, f, &info, 1050, 2000);
            finger_on(app, f, &info, Action::Back, 2000);
            frames(app, f, &info, 2000, 2100);
            2100
        }),
        ("122-settings-goes-back-dark", Screen::Home, true, &|app, f| {
            finger_on(app, f, &info, Action::Open(Screen::Settings), 1000);
            app.touch(Touch::Up, f, 1050);
            frames(app, f, &info, 1050, 2000);
            finger_on(app, f, &info, Action::Back, 2000);
            frames(app, f, &info, 2000, 2100);
            app.touch(Touch::Up, f, 2100);
            frames(app, f, &info, 2100, 2160);
            2170
        }),
        // A tap past the end of the first line, then " -x" typed there: the
        // round button to save comes up, and the cursor stands after it.
        ("126-editor-changed-light", Screen::Files, false, &|app, f| {
            app.text_opened(f, "/data/adb/service.d/tune.sh".into(), Ok(Some(EDITOR_SAMPLE.into())), 0);
            let l = editor::layout(&app.theme(), f);
            app.touch(Touch::Down(l.x0 + 30.0 * l.cell, l.y0 + l.pitch / 2.0), f, 100);
            app.touch(Touch::Up, f, 150);
            let keys = [Cap::Space, Cap::Show(Layer::Digits), Cap::Char('-'), Cap::Show(Layer::Letters), Cap::Char('x')];
            type_keys(app, f, &info, &keys, 300)
        }),
    ];
    for (name, screen, dark, act) in moments {
        let mut app = opened(screen, w, h, ppd);
        app.set_screen_corners(cfg.display.corner_radius as f32);
        app.state.dark = dark;
        app.frame(&fonts, &info);
        app.tick(0);
        let now = act(&mut app, &fonts);
        // Time passes frame by frame, as on the panel.
        for ms in (0..now).step_by(16) {
            app.tick(ms);
        }
        app.tick(now);
        let mut c = Canvas::new(w, h);
        app.compose(&mut c, &fonts, &info, CLOCK, now);
        let path = out.join(format!("{name}.png"));
        write_png(&path, &c)?;
        println!("wrote {}", path.display());
    }

    // The home page's card with each notice it gives: a battery too low to
    // install on, an update waiting to merge, and a restart into the other
    // slot; the data partition still shut.
    let battery = info.battery.clone().map(|b| Battery { level: 12, charge: Charge::Discharging, ..b });
    let wanting = DeviceInfo { battery, snapshot: SnapshotState::Snapshotted, ..info.clone() };
    for (name, dark) in [("188-home-notices-light", false), ("189-home-notices-dark", true)] {
        let mut app = App::new(Screen::Home, w, h, ppd);
        app.state.lang = lang;
        app.state.dark = dark;
        app.set_boot_slots(Ok(BootSlots { active: Slot::A, bootable: [true, true], successful: [true, true] }));
        let path = out.join(format!("{name}.png"));
        write_png(&path, &whole_page(&mut app, &fonts, &wanting))?;
        println!("wrote {}", path.display());
    }

    // The status bar's battery in each state, light and dark, one bar a row.
    for (name, theme) in [("66-battery-light", light), ("67-battery-dark", Theme::dark(w, h, ppd))] {
        let path = out.join(format!("{name}.png"));
        write_png(&path, &battery_states(&fonts, &theme))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Status bars over the page colour, one for each state the battery shows:
/// charging, quickly too, plain levels with the cable out, low ones, full.
fn battery_states(f: &Fonts, t: &Theme) -> Canvas {
    let (charging, quick) = (Some(Bolt::Charging), Some(Bolt::Quick));
    let states: [(u8, Option<Bolt>); 9] =
        [(50, charging), (50, quick), (15, charging), (100, charging), (100, None), (50, None), (30, None), (15, None), (5, None)];
    let row = t.m.dp(52.0).round() as u32;
    let mut sheet = Canvas::new(t.m.w, row * states.len() as u32);
    for (k, &battery) in states.iter().enumerate() {
        let mut bar = Canvas::new(t.m.w, row);
        bar.clear(t.pal.bg);
        widgets::status_bar(&mut bar, t, f, &Bar { clock: CLOCK, battery: Some(battery), wlan: None, data: None, slot: None, temp: None }, t.pal.status);
        sheet.put(&bar, (row as usize * k) as i32);
    }
    sheet
}

/// File name, page, dark theme, and the touches leading up to the moment,
/// which return the time it is shown at.
type Moment<'a> = (&'a str, Screen, bool, &'a dyn Fn(&mut App, &Fonts) -> u64);

/// Drag from `y0` to `y1` at x = 600 and rest before lifting, so the page
/// stays where the finger left it; returns a time after the bar has
/// settled.
fn drag(app: &mut App, f: &Fonts, y0: f32, y1: f32) -> u64 {
    app.touch(Touch::Down(600.0, y0), f, 0);
    for k in 1..=10u64 {
        app.touch(Touch::Move(600.0 + (k % 2) as f32, y0 + (y1 - y0) * k as f32 / 10.0), f, 40 * k);
    }
    app.touch(Touch::Up, f, 800);
    for ms in (800..2000).step_by(8) {
        app.tick(ms);
    }
    2000
}

/// Put a finger down at `at` on the control doing `action`, on the page
/// drawn: `<`, or one of the page's own.
fn finger_on(app: &mut App, f: &Fonts, info: &DeviceInfo, action: Action, at: u64) {
    let t = app.theme();
    let rect = match action {
        Action::Back => widgets::back_rect(&t),
        _ => app.frame(f, info).hits.iter().find(|h| h.action == action).expect("the control is on the page").rect,
    };
    let (x, y) = rect.centre();
    app.touch(Touch::Down(x, y), f, at);
}

/// Tap each of `keys` on the page's keyboard, 120 ms apart from `from`;
/// returns a time just after the last.
fn type_keys(app: &mut App, f: &Fonts, info: &DeviceInfo, keys: &[Cap], from: u64) -> u64 {
    let mut at = from;
    for &cap in keys {
        finger_on(app, f, info, Action::Type(cap), at);
        app.touch(Touch::Up, f, at + 60);
        at += 120;
    }
    at
}

/// Draw a frame every 8 ms from `from` to `to`, as the panel does.
fn frames(app: &mut App, f: &Fonts, info: &DeviceInfo, from: u64, to: u64) {
    let t = app.theme();
    let mut c = Canvas::new(t.m.w, t.m.h);
    for ms in (from..=to).step_by(8) {
        app.tick(ms);
        app.compose(&mut c, f, info, CLOCK, ms);
    }
}

/// Press the volume key that moves the light down the page `times` times,
/// a second apart, on the page drawn; returns a time after the page has
/// come to rest.
fn keys_down(app: &mut App, f: &Fonts, info: &DeviceInfo, times: u64) -> u64 {
    for k in 0..times {
        app.frame(f, info);
        app.key(Key::VolumeDown, k * 1000);
        app.key_up(Key::VolumeDown, k * 1000 + 100);
        for ms in (k * 1000..(k + 1) * 1000).step_by(16) {
            app.tick(ms);
        }
    }
    times * 1000
}

/// Pull from `y0` to `y1` at x = 600 and keep the finger down.
fn hold(app: &mut App, f: &Fonts, y0: f32, y1: f32) -> u64 {
    app.touch(Touch::Down(600.0, y0), f, 0);
    for k in 1..=10u64 {
        app.touch(Touch::Move(600.0, y0 + (y1 - y0) * k as f32 / 10.0), f, 40 * k);
    }
    400
}

/// Open the reboot page from the home page, which leaves a page to go back
/// to, then swipe in from the left edge at y = 1300 to `x` and keep the
/// finger down, the way the 17U recordings were made.
fn swipe_in(app: &mut App, f: &Fonts, info: &DeviceInfo, x: f32) -> u64 {
    let hits = &app.frame(f, info).hits;
    let (rx, ry) = hits.iter().find(|h| h.action == Action::Open(Screen::Reboot)).expect("the home page opens the reboot page").rect.centre();
    app.touch(Touch::Down(rx, ry), f, 0);
    app.touch(Touch::Up, f, 0);
    app.touch(Touch::Down(3.0, 1300.0), f, 0);
    for k in 1..=8u64 {
        app.touch(Touch::Move(3.0 + (x - 3.0) * k as f32 / 8.0, 1300.0 + k as f32), f, 8 * k);
    }
    64
}

/// The slot's partitions on a Xiaomi 13 Ultra, in the order super lists
/// them and where the recovery mounts them, all mounted but `unmounted`.
/// The touch programs run from odm.
fn sample_mounts(unmounted: &[&str]) -> Vec<MountRow> {
    let partitions = [
        ("odm", "/odm"),
        ("product", "/mnt/product"),
        ("system", "/mnt/system"),
        ("system_dlkm", "/system_dlkm"),
        ("system_ext", "/mnt/system_ext"),
        ("vendor", "/vendor"),
        ("vendor_dlkm", "/vendor_dlkm"),
        ("mi_ext", "/mnt/mi_ext"),
    ];
    partitions
        .iter()
        .map(|&(base, point)| {
            let mounted = !unmounted.contains(&base);
            let holders = if mounted && base == "odm" { vec!["touch_report".into(), "touchsensor".into()] } else { Vec::new() };
            MountRow { base: base.into(), point: point.into(), mounted, holders }
        })
        .collect()
}

/// When the previews' folders are listed: 2026-10-19 23:46 in Beijing, the
/// time their status bar shows.
const LISTED: i64 = 1_792_424_760;

/// The ramdisk's timestamp on every file a recovery image carries.
const RAMDISK_TIME: i64 = 1_230_768_000;

fn link(name: &str, target: &str, folder: bool) -> FileRow {
    let kind = FileKind::Link { target: target.into(), folder };
    FileRow { name: name.into(), kind, size: target.len() as u64, modified: RAMDISK_TIME, mode: 0o120_777, uid: 0, gid: 0 }
}

fn folder(name: &str, items: usize, modified: i64) -> FileRow {
    FileRow { name: name.into(), kind: FileKind::Folder { items: Some(items) }, size: 4096, modified, mode: 0o040_755, uid: 0, gid: 0 }
}

fn file(name: &str, size: u64, modified: i64) -> FileRow {
    FileRow { name: name.into(), kind: FileKind::File, size, modified, mode: 0o100_644, uid: 0, gid: 0 }
}

/// Part of the root folder of a Xiaomi 13 Ultra, as `ls -la /` lists it,
/// in the file manager's order.
fn sample_root() -> Vec<FileRow> {
    let made = RAMDISK_TIME;
    vec![
        folder("apex", 31, made),
        link("bin", "/system/bin", true),
        link("cache", "/data/cache", true),
        folder("data", 0, made),
        folder("dev", 142, LISTED - 3600),
        link("etc", "/system/etc", true),
        folder("metadata", 9, made),
        folder("mnt", 14, LISTED - 3600),
        folder("odm", 7, made),
        folder("proc", 612, LISTED - 3600),
        folder("product", 0, made),
        link("sdcard", "/storage/self/primary", true),
        folder("system", 9, made),
        folder("vendor", 11, made),
        link("init", "/system/bin/init", false),
        FileRow { name: "init.environ.rc".into(), kind: FileKind::File, size: 621, modified: made, mode: 0o100_750, uid: 0, gid: 2000 },
    ]
}

/// A download folder whose names take one line, two, or more than two.
fn sample_download() -> Vec<FileRow> {
    let (today, this_year, earlier) = (LISTED - 4 * 3600, 1_790_380_800, 1_667_692_800);
    vec![
        folder("Skyward", 3, today),
        folder("安装包", 68, this_year),
        file("1d9bf122-130e-4db7-ae55-460c84819f0e.pdf", 1_090_000, this_year),
        file("2021一带一路暨金砖大赛之物联网技术及其在智慧城市与工业互联网领域的应用创新赛表彰文件.pdf", 634_960, earlier),
        file("OTA_ishtar_OS3.0.305.0.WMACNXM_20260412_full_package_signed_for_recovery_install.zip", 7_014_220_000, this_year),
        file("README.txt", 900, today),
        file("skyward-recovery-ishtar.img", 104_857_600, today),
    ]
}

/// A folder holding a file of each type the list draws an icon of its own
/// for, and one it does not know, in the file manager's order.
fn sample_types() -> Vec<FileRow> {
    let made = LISTED - 2 * 3600;
    let mut rows: Vec<FileRow> = [
        ("boot.img", 100_663_296),
        ("IMG_20261021_143005.jpg", 4_812_000),
        ("update-ishtar.zip", 1_402_000_000),
        ("app-release.apk", 11_300_000),
        ("recovery.log", 486_000),
        ("VID_20261020_201512.mp4", 182_400_000),
        ("会议录音.m4a", 8_900_000),
        ("build.prop", 3_200),
        ("年度报告.docx", 268_000),
        ("预算.xlsx", 54_000),
        ("方案汇报.pptx", 6_100_000),
        ("用户手册.pdf", 2_300_000),
        ("persist.bak", 33_554_432),
    ]
    .into_iter()
    .map(|(name, size)| file(name, size, made))
    .collect();
    rows.sort_by_key(|r| r.name.to_lowercase());
    rows
}

/// A Xiaomi 13 Ultra's /system/bin: programs, and the links to toybox that
/// stand for its commands, in the file manager's order.
fn sample_bin() -> Vec<FileRow> {
    let programs = [
        "abb", "am", "app_process32", "app_process64", "applypatch", "appops", "appwidget", "atrace", "audioserver", "auditctl",
        "blkid", "bmgr", "bootanimation", "bootstat", "bpfloader", "bugreport", "bugreportz", "cameraserver", "cmd", "content",
        "crash_dump32", "crash_dump64", "debuggerd", "device_config", "dmctl", "dpm", "dumpstate", "dumpsys", "e2fsck", "e2fsdroid",
        "fsck.f2fs", "fsck_msdos", "gatekeeperd", "gpuservice", "hid", "idmap2", "ime", "incident", "incidentd", "init", "input",
        "installd", "ip", "iptables", "keystore2", "lmkd", "locksettings", "logcat", "logd", "lpdump", "lshal", "make_f2fs",
        "mediaserver", "mke2fs", "monkey", "netd", "odsign", "perfetto", "ping", "pm", "reboot", "resize2fs", "run-as",
        "screencap", "screenrecord", "sdcard", "sensorservice", "service", "servicemanager", "settings", "sgdisk", "sh",
        "simpleperf", "sm", "snapshotctl", "storaged", "surfaceflinger", "svc", "tombstoned", "toolbox", "toybox", "traced",
        "tune2fs", "uiautomator", "uncrypt", "update_engine", "update_verifier", "vdc", "vold", "wificond", "wm",
    ];
    let commands = [
        "base64", "basename", "cat", "chcon", "chmod", "chown", "cmp", "cp", "cut", "date", "dd", "df", "diff", "dirname", "dmesg",
        "du", "echo", "env", "expr", "find", "free", "getprop", "grep", "head", "hostname", "id", "ifconfig", "insmod", "kill",
        "ln", "ls", "lsmod", "lsof", "md5sum", "mkdir", "mount", "mv", "netstat", "nohup", "pgrep", "pidof", "pkill", "printf",
        "ps", "pwd", "readlink", "rm", "rmdir", "sed", "sendevent", "seq", "setprop", "sleep", "sort", "start", "stat", "stop",
        "tail", "tar", "tee", "top", "touch", "tr", "umount", "uname", "uptime", "wc", "which", "xargs", "xxd",
    ];
    let mut rows = vec![folder("hw", 34, RAMDISK_TIME)];
    let mut rest: Vec<FileRow> = programs
        .iter()
        .enumerate()
        .map(|(k, name)| FileRow { mode: 0o100_755, gid: 2000, ..file(name, 18_000 + 7_919 * k as u64 % 900_000, RAMDISK_TIME) })
        .chain(commands.iter().map(|name| link(name, "toybox", false)))
        .collect();
    rest.sort_by_key(|r| r.name.to_lowercase());
    rows.extend(rest);
    rows
}

/// Networks in range, as the 13 Ultra's supplicant found them on
/// 2026-10-22, the strongest of each.
fn nearby() -> Vec<Network> {
    let n = |name: &str, signal: i32, frequency: u32, also: Option<u32>, security: Security| {
        let b = Bands::of(frequency);
        let bands = match also.map(Bands::of) {
            Some(o) => Bands { ghz2: b.ghz2 || o.ghz2, ghz5: b.ghz5 || o.ghz5, ghz6: b.ghz6 || o.ghz6 },
            None => b,
        };
        Network { ssid: name.as_bytes().to_vec(), name: name.into(), signal, frequency, bands, security }
    };
    vec![
        n("Skyward", -30, 2462, Some(5805), Security::Psk),
        n("Skyward-Guest", -36, 2447, Some(5180), Security::Psk),
        n("Skyward-IoT", -38, 2447, None, Security::Psk),
        n("Skyward-Lab", -45, 2452, None, Security::Psk),
        n("Skyward客厅", -50, 5180, None, Security::Psk),
        n("Skyward-Mesh", -62, 2452, None, Security::Psk),
        n("Skyward-Office", -68, 2412, None, Security::Psk),
        n("Skyward-5G", -68, 5745, None, Security::Psk),
        n("Skyward-Family", -71, 2462, None, Security::Sae),
        n("Skyward-Garage", -77, 2437, None, Security::Psk),
        n("Skyward书房", -80, 2437, None, Security::Psk),
        n("Skyward-Open", -84, 2412, None, Security::Open),
    ]
}

/// The first of [`nearby`] chosen, and how joining it goes.
fn joined(stage: Joining) -> Wlan {
    let n = nearby();
    let j = Joined { ssid: n[0].ssid.clone(), name: n[0].name.clone(), security: n[0].security, stage };
    Wlan { power: Power::On, networks: n, joined: Some(j) }
}

/// What a home router gave the 13 Ultra.
fn router_link() -> Link {
    use std::net::Ipv4Addr;
    Link {
        address: Ipv4Addr::new(192, 168, 1, 23),
        mask: Ipv4Addr::new(255, 255, 255, 0),
        router: Some(Ipv4Addr::new(192, 168, 1, 1)),
        dns: vec![Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(114, 114, 114, 114)],
        signal: -31,
        frequency: 5805,
        speed: Some(866),
        generation: Some(5),
        mac: "02:00:00:00:00:04".into(),
        reach: Some(Reach::Online),
    }
}

/// A newer system than the 17 Ultra's own, as the update server tells of
/// one.
fn newest() -> Release {
    Release {
        version: "OS4.0.6.0.XPACNXM".into(),
        name: "nezha-ota_full-OS4.0.6.0.XPACNXM-user-17.0-3f2a9c81d4.zip".into(),
        urls: vec!["https://ultimateota.d.miui.com/OS4.0.6.0.XPACNXM/nezha-ota_full-OS4.0.6.0.XPACNXM-user-17.0-3f2a9c81d4.zip".into()],
        md5: "3f2a9c81d4e05b7a6c1e9d2f8b7a6c5d".into(),
        size: "7.1G".into(),
        length: Some(7_572_375_802),
    }
}

/// A phone whose update server is known, on a network joined.
fn online(app: &mut App) {
    app.state.update_server = true;
    app.state.wlan = Some(joined(Joining::Connected(router_link())));
}

/// A newer release of Skyward than the one running, as GitHub tells of it,
/// with its zip for the 17 Ultra.
fn skyward_newest() -> Newest {
    Newest {
        version: "0.1.4".into(),
        newer: true,
        build: Some(Build {
            name: "skyward-v0.1.4-nezha-OS4.0.4.0.XPACNXM.zip".into(),
            url: "https://api.github.com/repos/kirklin/skyward/releases/assets/719751747".into(),
            link: "https://github.com/kirklin/skyward/releases/download/v0.1.4/skyward-v0.1.4-nezha-OS4.0.4.0.XPACNXM.zip".into(),
            size: 43_145_196,
            sha256: "9f6165079529418d8f6635beddfdd9abf9a70c89d69eb467a9922b0db69bfaf1".into(),
        }),
    }
}

/// A phone that keeps its recovery in a partition of each slot, on a
/// network joined.
fn skyward_online(app: &mut App) {
    app.state.recovery_partitions = true;
    app.state.wlan = Some(joined(Joining::Connected(router_link())));
}

/// A page as most previews show it: on a phone whose data partition is
/// open, its owner's own files with it.
fn opened(screen: Screen, w: u32, h: u32, ppd: f32) -> App {
    let mut app = App::new(screen, w, h, ppd);
    app.state.lang = lang();
    app.state.data = Data::Open;
    app
}

/// The full length of a page with the status bar over its top, the way a
/// long screenshot shows it: a long list's rows and a pinned strip in the
/// places an unscrolled page gives them.
fn whole_page(app: &mut App, f: &Fonts, info: &DeviceInfo) -> Canvas {
    let t = app.theme();
    let (screen, state) = (app.screen(), app.state.clone());
    let frame = app.whole_frame(f, info);
    let mut c = Canvas::new(frame.canvas.w, frame.height(&t) as u32);
    c.clear(frame.bg);
    c.put(&frame.canvas, 0);
    if let Some(fold) = &frame.fold {
        fold.draw(&mut c, &t, &widgets::search_words(&t, f, fold.bar, fold.hint), 0.0);
    }
    if let Some(rows) = &frame.rows {
        let cx = Ctx { t: &t, f, info, state: &state, term: None, band: None };
        for k in 0..rows.count() {
            let mut row = Canvas::new(c.w, (rows.edges[k + 1] - rows.edges[k]) as u32);
            pages::row(screen, &cx, k, &mut row);
            c.put(&row, rows.edges[k] as i32);
        }
    }
    if let Some(p) = &frame.pinned {
        c.put(&p.canvas, p.top.round() as i32);
    }
    if let Some(d) = &frame.dock {
        c.put(&d.canvas, (c.h - d.canvas.h) as i32);
    }
    widgets::status_bar(&mut c, &t, f, &app.bar(CLOCK, info), t.pal.status);
    c
}

/// Xiaomi 17 Ultra by Leica as `skyward info` read it on 2026-10-18 (the
/// config supplies the SoC name, panel and screen size).
fn nezha(cfg: &DeviceConfig) -> DeviceInfo {
    DeviceInfo {
        model: "Xiaomi 17 Ultra by Leica".into(),
        codename: cfg.device.codename.clone(),
        soc: cfg.device.soc.clone(),
        cpu: Cpu { cores: 8, max_khz: Some(4_608_000) },
        ram_kib: 15_472_464,
        panel: Some((cfg.display.width, cfg.display.height)),
        screen: cfg.display.diagonal,
        os_version: Some("OS4.0.4.0.XPACNXM".into()),
        android: Some("17".into()),
        locale: Some("zh-CN".into()),
        security_patch: Some(PatchLevel { year: 2026, month: 10, day: 1 }),
        kernel: "6.12.69-android16-6-g43b5ab7e247d-abogki561813157-4k".into(),
        storage: Storage { total: 1_039_535_157_248, free: Some(620_652_802_048) },
        battery: Some(Battery {
            level: 95,
            charge: Charge::Charging,
            temp: Some(350),
            health: Some("Good".into()),
            cycles: Some(300),
            design: Some(6_800_000),
            full: Some(6_543_000),
            voltage: Some(4_442_000),
            // The recovery has no fast charging handshake with the charger.
            quick: false,
            plugged: true,
        }),
        slot: Some(Slot::B),
        snapshot: SnapshotState::None,
        boot: Boot { unlocked: Some(true), verified: Some("orange".into()) },
    }
}

/// Weight ladders for matching stroke widths against device screenshots: the
/// settings strings at their sizes, and a capital H at the wordmark size.
fn calibrate(fonts: &Fonts, t: &Theme) -> Result<Canvas> {
    let m = &t.m;
    let weights = [330.0, 380.0, 420.0, 450.0, 480.0, 520.0, 560.0, 600.0, 620.0, 650.0, 700.0];
    let mut c = Canvas::new(m.w, 200 * weights.len() as u32);
    c.clear(t.pal.bg);
    let ink = t.pal.text;
    for (i, w) in weights.iter().enumerate() {
        let face = fonts.face(*w)?;
        let top = 10.0 + 200.0 * i as f32;
        c.text_top(&face, "设置", 10.0, top, m.title_px, ink, None);
        c.text_top(&face, "蓝牙", 210.0, top, m.row_title_px, ink, None);
        c.text_top(&face, "亮度", 340.0, top, m.sub_px, ink, None);
        c.text_top(&face, "登录小米账号", 440.0, top, m.account_title_px, ink, None);
        let b = top + 110.0;
        c.text_top(&face, "享受更多小米服务", 10.0, b, m.sub_px, ink, None);
        c.text_top(&face, "已开启", 340.0, b, m.sub_px, ink, None);
        c.text_top(&face, "搜索系统设置项", 480.0, b, m.row_title_px, ink, None);
        c.text(&face, "15:29", 840.0, b + m.status_px * 0.72, m.status_px, ink, None);
        c.text_top(&face, "新版本", 1010.0, b, m.badge_px, ink, None);
        // The stock wordmark is set at a 136.5 px em on the 3.5 px/dp
        // reference panel.
        c.text(&face, "H", 1100.0, top + 105.0, 136.5, ink, None);
    }
    Ok(c)
}

fn font_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("SKYWARD_FONT") {
        return Ok(PathBuf::from(p));
    }
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/fonts/MiSansVF.ttf");
    if bundled.exists() {
        return Ok(bundled);
    }
    for p in ["/System/Library/Fonts/STHeiti Medium.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"] {
        if Path::new(p).exists() {
            return Ok(PathBuf::from(p));
        }
    }
    Err(anyhow!("no font found; set SKYWARD_FONT to a .ttf/.ttc path"))
}

fn write_png(path: &Path, canvas: &Canvas) -> Result<()> {
    let file = File::create(path)?;
    let w = BufWriter::new(file);
    let mut enc = png::Encoder::new(w, canvas.w, canvas.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&canvas.data)?;
    Ok(())
}
