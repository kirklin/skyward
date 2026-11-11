use anyhow::{bail, ensure, Context, Result};
use std::fs::{File, OpenOptions};
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

/// The bootloader message at the start of misc: command[32], status[32],
/// recovery[768], stage[32] and reserved[1184].
const SIZE: usize = 2048;
const COMMAND: Range<usize> = 0..32;
const RECOVERY: Range<usize> = 64..832;

/// Where Android keeps the memory tagging the bootloader is to turn on: in
/// the system's own space of misc, which starts at 32 KiB, after the 64
/// bytes of the virtual A/B message.
const MEMTAG_MESSAGE: Range<u64> = 32 * 1024 + 64..32 * 1024 + 128;

/// Where the system of an A/B phone leaves the wipe package it asks the
/// recovery to erase the phone by: from 16 KiB up to the system's own space.
const WIPE_PACKAGE: Range<u64> = 16 * 1024..32 * 1024;

/// The filesystems a request may have the data partition formatted as, the
/// two Android's own recovery formats it as.
pub const REFORMATS: [&str; 2] = ["ext4", "f2fs"];

/// The misc partition, shared by the bootloader, Android and recovery.
pub fn misc() -> PathBuf {
    Path::new(skyward_block::BY_NAME).join("misc")
}

/// What the bootloader control block asks for: the command the bootloader
/// acts on, such as "boot-recovery", and the arguments recovery should run
/// with, such as "--wipe_data".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bcb {
    pub command: String,
    pub args: Vec<String>,
}

impl Bcb {
    pub fn parse(raw: &[u8]) -> Result<Bcb> {
        if raw.len() < SIZE {
            bail!("a bootloader message takes {SIZE} bytes, got {}", raw.len());
        }
        let command = text(&raw[COMMAND]).context("misc command")?.to_string();
        let recovery = text(&raw[RECOVERY]).context("misc recovery message")?;
        let mut lines = recovery.split('\n');
        let args = match lines.next() {
            None | Some("") => Vec::new(),
            Some("recovery") => lines.filter(|l| !l.is_empty()).map(str::to_string).collect(),
            Some(_) => bail!("misc holds an unknown recovery message {recovery:?}"),
        };
        Ok(Bcb { command, args })
    }

    /// Whether the message asks the bootloader for the recovery: for work of
    /// its own, its fastboot, or rescue.
    pub fn asks_for_recovery(&self) -> bool {
        matches!(self.command.as_str(), "boot-recovery" | "boot-fastboot" | "boot-rescue")
    }

    /// What the message asks the recovery to do, read the way Android's
    /// own recovery reads it: of several pieces of work it does the first
    /// in this order, fastboot, switching systems and showing why the
    /// second system's start stopped, which are Skyward's own, an update
    /// package, erasing data, asking whether to erase data, erasing the
    /// cache, erasing by a wipe package, sideload, rescue, unsharing
    /// blocks, leaving at once. A command with no arguments can ask for
    /// fastboot or rescue by itself. An option given twice counts as it
    /// was given last.
    pub fn request(&self) -> Request {
        let mut r = Request::default();
        let (mut fastboot, mut wipe_data, mut prompt, mut wipe_cache, mut wipe_ab) = (false, false, false, false, false);
        let (mut rescue, mut unshare, mut exit, mut security, mut stopped) = (false, false, false, false, false);
        let (mut sideload, mut switch) = (None, None);
        let (mut update, mut reformat, mut retry, mut wipe_package) = (None, None, 0, 0);
        for arg in &self.args {
            let Some(option) = arg.strip_prefix("--") else {
                r.unknown.push(arg.clone());
                continue;
            };
            let (name, value) = match option.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (option, None),
            };
            // An option that takes a value is one the recovery does not
            // know without it, or with one it cannot read.
            let mut unread = || r.unknown.push(arg.clone());
            match name {
                "fastboot" => fastboot = true,
                // The second is how the stock system of some phones asks
                // for a factory reset.
                "wipe_data" | "format_data" => wipe_data = true,
                "prompt_and_wipe_data" => prompt = true,
                "wipe_cache" => wipe_cache = true,
                "wipe_ab" => wipe_ab = true,
                "sideload" => sideload = sideload.or(Some(false)),
                "sideload_auto_reboot" => sideload = Some(true),
                "rescue" => rescue = true,
                "fsck_unshare_blocks" => unshare = true,
                "just_exit" => exit = true,
                "security" => security = true,
                "reason" => r.reason = value.map(str::to_string),
                "locale" => r.locale = value.map(str::to_string),
                "shutdown_after" => r.shutdown_after = true,
                "keep_memtag_mode" => r.keep_memtag_mode = true,
                "update_package" => match value.filter(|v| !v.is_empty()) {
                    Some(path) => update = Some(path.to_string()),
                    None => unread(),
                },
                "reformat_data" => match value.filter(|v| !v.is_empty()) {
                    Some(filesystem) => reformat = Some((filesystem.to_string(), arg.clone())),
                    None => unread(),
                },
                "retry_count" => match value.and_then(|v| v.parse().ok()) {
                    Some(count) => retry = count,
                    None => unread(),
                },
                "wipe_package_size" => match value.and_then(|v| v.parse().ok()) {
                    Some(size) => wipe_package = size,
                    None => unread(),
                },
                "switch_system" => match value.filter(|v| !v.is_empty()) {
                    Some(to) => switch = Some(to.to_string()),
                    None => unread(),
                },
                "system_stopped" => stopped = true,
                // How Android's own recovery reads a package, and how its
                // text shows.
                "install_with_fuse" | "update_wipe" | "export_validate" | "show_text" => {}
                _ => unread(),
            }
        }
        // Formatting as another filesystem shapes erasing data or the cache;
        // as one the recovery cannot make, it leaves that work undone.
        let reformat = match reformat {
            Some((filesystem, _)) if REFORMATS.contains(&filesystem.as_str()) => Ok(Some(filesystem)),
            Some((_, arg)) => Err(arg),
            None => Ok(None),
        };
        r.work = if fastboot {
            Work::Fastboot
        } else if let Some(to) = switch {
            Work::SwitchSystem { to }
        } else if stopped {
            Work::SystemStopped
        } else if let Some(package) = update {
            Work::Update(Update { package, retry, wipe_cache, security })
        } else if wipe_data {
            reformat.map_or_else(Work::Unsupported, |reformat| Work::WipeData { reformat })
        } else if prompt {
            Work::PromptWipeData
        } else if wipe_cache {
            reformat.map_or_else(Work::Unsupported, |reformat| Work::WipeCache { reformat })
        } else if wipe_ab {
            Work::WipeAb { package_size: wipe_package }
        } else if let Some(auto_reboot) = sideload {
            Work::Sideload { auto_reboot }
        } else if rescue {
            Work::Rescue
        } else if unshare {
            Work::UnshareBlocks
        } else if exit {
            Work::Exit
        } else if !r.unknown.is_empty() {
            Work::Unsupported(r.unknown.join(" "))
        } else if self.args.is_empty() && self.command == "boot-fastboot" {
            Work::Fastboot
        } else if self.args.is_empty() && self.command == "boot-rescue" {
            Work::Rescue
        } else {
            Work::Menu
        };
        r
    }

    /// The message to write back as an update the system asked for starts
    /// for the `count`th time, as Android's own recovery counts them: the
    /// count it had taken out, the new one put in.
    pub fn with_retry(&self, count: u32) -> Bcb {
        let mut args: Vec<String> = self.args.iter().filter(|a| !a.starts_with("--retry_count")).cloned().collect();
        args.push(format!("--retry_count={count}"));
        Bcb { command: "boot-recovery".into(), args }
    }

    /// Lay the message out over `raw`, a bootloader message read from misc:
    /// the command, and the arguments under the line that says they are
    /// the recovery's, with the rest of the fields left as they were.
    fn lay_over(&self, raw: &mut [u8; SIZE]) -> Result<()> {
        let recovery = match self.args.is_empty() {
            true => String::new(),
            false => self.args.iter().fold(String::from("recovery\n"), |text, arg| text + arg + "\n"),
        };
        // Each field ends in a NUL, as the bootloader reads it.
        ensure!(self.command.len() < COMMAND.len(), "the command {:?} does not fit in misc", self.command);
        ensure!(recovery.len() < RECOVERY.len(), "{} bytes of arguments do not fit in misc", recovery.len());
        for (field, text) in [(COMMAND, self.command.as_bytes()), (RECOVERY, recovery.as_bytes())] {
            let field = &mut raw[field];
            field.fill(0);
            field[..text.len()].copy_from_slice(text);
        }
        Ok(())
    }
}

/// What a boot message asks the recovery to do: one piece of work, and how
/// to go about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub work: Work,
    /// Why the system asked, as it says it, such as RescueParty.
    pub reason: Option<String>,
    /// The language the system runs in.
    pub locale: Option<String>,
    /// Power off once the work is done, instead of starting the system.
    pub shutdown_after: bool,
    /// Erasing data leaves alone the memory tagging asked of the
    /// bootloader.
    pub keep_memtag_mode: bool,
    /// Arguments the recovery knows nothing of.
    pub unknown: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Work {
    /// Nothing: the recovery's own pages.
    #[default]
    Menu,
    /// The recovery's userspace fastboot.
    Fastboot,
    /// Install an update package, as the system's updater asks.
    Update(Update),
    /// Erase all user data, as a factory reset does; with `reformat`, the
    /// data partition, the cache partition and /metadata are each made
    /// afresh as that filesystem.
    WipeData { reformat: Option<String> },
    /// Say that Android cannot start, and offer to erase all user data.
    PromptWipeData,
    /// Erase the cache partition; with `reformat`, made afresh as that
    /// filesystem.
    WipeCache { reformat: Option<String> },
    /// Erase the partitions a wipe package signed with the phone's update
    /// keys lists, as a phone to be disabled for good is erased. The
    /// package is the `package_size` bytes the system left in misc.
    WipeAb { package_size: u64 },
    /// Wait for `adb sideload`; with `auto_reboot`, start the system once
    /// the package is in or has failed.
    Sideload { auto_reboot: bool },
    /// Wait for `adb rescue` on a computer: to install a package, read
    /// what the phone is, or erase all user data.
    Rescue,
    /// Undo the sharing of blocks in the slot's ext4 partitions that hold
    /// shared ones, so they can be written to.
    UnshareBlocks,
    /// Have the bootloader start another system: `installed` for the one in
    /// the phone's own partitions, or the id of the one Skyward installed
    /// beside it, as that system asks.
    SwitchSystem { to: String },
    /// Say why the second system's start stopped, as Skyward's program that
    /// starts it tells: the phone stays in the recovery, and the same
    /// system starts next.
    SystemStopped,
    /// Start the system at once.
    Exit,
    /// Work the recovery does not do, as it was asked.
    Unsupported(String),
}

/// An update package the system asks the recovery to install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    /// The package's path, as the system names it; or `@` and the path of
    /// a block map, which lists where the package's blocks are on a
    /// partition, for a package the system kept in the data partition.
    pub package: String,
    /// How many times the install has been started before: once a fresh
    /// install has started, a restart part way through comes back to it as
    /// a retry.
    pub retry: u32,
    /// Erase the cache partition once the package is in.
    pub wipe_cache: bool,
    /// The package is a security update.
    pub security: bool,
}

impl Work {
    /// Whether the work goes on past the recovery's start. Its request
    /// then stays in misc until it is done, so a restart halfway comes back
    /// to it, as Android's own recovery does.
    pub fn lasts(&self) -> bool {
        !matches!(self, Work::Menu | Work::Fastboot | Work::Exit | Work::SystemStopped | Work::Unsupported(_))
    }
}

/// A NUL-terminated field; bytes after the first NUL are leftovers of an
/// earlier, longer value.
fn text(field: &[u8]) -> Result<&str> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).context("not UTF-8")
}

pub fn read(misc: &Path) -> Result<Bcb> {
    let mut raw = [0u8; SIZE];
    File::open(misc)
        .and_then(|f| f.read_exact_at(&mut raw, 0))
        .with_context(|| format!("read {}", misc.display()))?;
    Bcb::parse(&raw)
}

/// Zero the bootloader message so the next boot follows its reboot reason
/// instead of a pending command.
pub fn clear(misc: &Path) -> Result<()> {
    let f = OpenOptions::new().write(true).open(misc).with_context(|| format!("open {}", misc.display()))?;
    f.write_all_at(&[0u8; SIZE], 0).with_context(|| format!("write {}", misc.display()))?;
    f.sync_all().with_context(|| format!("sync {}", misc.display()))
}

/// Write `bcb` over the bootloader message in misc, its status and stage
/// left as they are.
pub fn write(misc: &Path, bcb: &Bcb) -> Result<()> {
    let f = OpenOptions::new().read(true).write(true).open(misc).with_context(|| format!("open {}", misc.display()))?;
    let mut raw = [0u8; SIZE];
    f.read_exact_at(&mut raw, 0).with_context(|| format!("read {}", misc.display()))?;
    bcb.lay_over(&mut raw)?;
    f.write_all_at(&raw, 0).with_context(|| format!("write {}", misc.display()))?;
    f.sync_all().with_context(|| format!("sync {}", misc.display()))
}

/// The wipe package the system of an A/B phone left in misc, `size` bytes,
/// for erasing the phone by.
pub fn wipe_package(misc: &Path, size: u64) -> Result<Vec<u8>> {
    ensure!(size > 0, "the request gives the wipe package no size");
    let room = WIPE_PACKAGE.end - WIPE_PACKAGE.start;
    ensure!(size <= room, "a wipe package of {size} bytes is more than the {room} misc keeps one in");
    let mut package = vec![0u8; size as usize];
    File::open(misc)
        .and_then(|f| f.read_exact_at(&mut package, WIPE_PACKAGE.start))
        .with_context(|| format!("read the wipe package from {}", misc.display()))?;
    Ok(package)
}

/// Leave the memory tagging message empty, as erasing data does unless it
/// is asked to keep it: the bootloader then turns on no tagging it was
/// asked for before.
pub fn forget_memtag_mode(misc: &Path) -> Result<()> {
    let f = OpenOptions::new().write(true).open(misc).with_context(|| format!("open {}", misc.display()))?;
    let empty = [0u8; (MEMTAG_MESSAGE.end - MEMTAG_MESSAGE.start) as usize];
    f.write_all_at(&empty, MEMTAG_MESSAGE.start).with_context(|| format!("write {}", misc.display()))?;
    f.sync_all().with_context(|| format!("sync {}", misc.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(command: &[u8], recovery: &[u8]) -> Vec<u8> {
        let mut raw = vec![0u8; SIZE];
        raw[..command.len()].copy_from_slice(command);
        raw[RECOVERY.start..RECOVERY.start + recovery.len()].copy_from_slice(recovery);
        raw
    }

    #[test]
    fn reads_recovery_arguments() {
        let raw = message(b"boot-recovery", b"recovery\n--wipe_data\n--locale=zh_CN\n");
        let bcb = Bcb::parse(&raw).unwrap();
        assert_eq!(bcb.command, "boot-recovery");
        assert_eq!(bcb.args, ["--wipe_data", "--locale=zh_CN"]);
    }

    #[test]
    fn ignores_leftovers_after_nul() {
        // The 13 Ultra's misc after the bootloader wrote boot-recovery over
        // bootonce-bootloader.
        let bcb = Bcb::parse(&message(b"boot-recovery\0oader", b"")).unwrap();
        assert_eq!(bcb, Bcb { command: "boot-recovery".into(), args: Vec::new() });
    }

    #[test]
    fn empty_message_asks_for_nothing() {
        assert_eq!(Bcb::parse(&[0u8; SIZE]).unwrap(), Bcb::default());
    }

    #[test]
    fn refuses_unknown_recovery_message() {
        assert!(Bcb::parse(&message(b"boot-recovery", b"--wipe_data\n")).is_err());
    }

    fn asked(command: &str, args: &[&str]) -> Request {
        Bcb { command: command.into(), args: args.iter().map(|a| a.to_string()).collect() }.request()
    }

    const WIPE_DATA: Work = Work::WipeData { reformat: None };

    #[test]
    fn a_factory_reset_from_the_settings_erases_data() {
        let r = asked("boot-recovery", &["--format_data", "--locale=zh-CN", "--reason=factroy_reset(1217)"]);
        assert_eq!(r.work, WIPE_DATA);
        assert_eq!(r.locale.as_deref(), Some("zh-CN"));
        assert_eq!(r.reason.as_deref(), Some("factroy_reset(1217)"));
        assert!(!r.shutdown_after && !r.keep_memtag_mode && r.unknown.is_empty());
    }

    #[test]
    fn a_wipe_asked_by_the_system_says_how_to_end() {
        let reason = "--reason=MasterClearConfirm,2026-10-20T01:00:00Z";
        let r = asked("boot-recovery", &["--wipe_data", "--shutdown_after", reason, "--keep_memtag_mode", "--locale=en-US"]);
        assert_eq!(r.work, WIPE_DATA);
        assert!(r.shutdown_after && r.keep_memtag_mode);
        assert_eq!(r.reason.as_deref(), Some("MasterClearConfirm,2026-10-20T01:00:00Z"));
    }

    #[test]
    fn each_kind_of_work_is_read() {
        assert_eq!(asked("boot-recovery", &["--prompt_and_wipe_data", "--reason=RescueParty"]).work, Work::PromptWipeData);
        assert_eq!(asked("boot-recovery", &["--wipe_cache"]).work, Work::WipeCache { reformat: None });
        assert_eq!(asked("boot-recovery", &["--sideload"]).work, Work::Sideload { auto_reboot: false });
        assert_eq!(asked("boot-recovery", &["--sideload_auto_reboot"]).work, Work::Sideload { auto_reboot: true });
        assert_eq!(asked("boot-recovery", &["--rescue"]).work, Work::Rescue);
        assert_eq!(asked("boot-recovery", &["--fsck_unshare_blocks"]).work, Work::UnshareBlocks);
        assert_eq!(asked("boot-recovery", &["--fastboot"]).work, Work::Fastboot);
        assert_eq!(asked("boot-recovery", &["--just_exit"]).work, Work::Exit);
        assert_eq!(asked("boot-recovery", &[]).work, Work::Menu);
        assert_eq!(asked("boot-recovery", &["--locale=zh-CN"]).work, Work::Menu, "a language alone asks for nothing");
    }

    #[test]
    fn a_switch_of_systems_names_the_system() {
        let r = asked("boot-recovery", &["--switch_system=installed"]);
        assert_eq!(r.work, Work::SwitchSystem { to: "installed".into() });
        assert!(r.work.lasts(), "a switch cut off is taken up again");
        assert_eq!(asked("boot-recovery", &["--switch_system=ubuntu", "--wipe_data"]).work, Work::SwitchSystem { to: "ubuntu".into() });
        assert_eq!(asked("boot-recovery", &["--switch_system="]).work, Work::Unsupported("--switch_system=".into()), "a switch to no system");
    }

    #[test]
    fn a_stopped_start_says_why_and_is_shown_once() {
        let r = asked("boot-recovery", &["--system_stopped", "--reason=no partition userdata showed within 15s"]);
        assert_eq!(r.work, Work::SystemStopped);
        assert_eq!(r.reason.as_deref(), Some("no partition userdata showed within 15s"));
        assert!(!r.work.lasts());
        assert_eq!(asked("boot-recovery", &["--system_stopped", "--switch_system=installed"]).work, Work::SwitchSystem { to: "installed".into() }, "a switch comes first");
        let written = Bcb { command: "boot-recovery".into(), args: vec!["--system_stopped".into(), "--reason=x".into()] };
        let mut raw = [0u8; SIZE];
        written.lay_over(&mut raw).unwrap();
        assert_eq!(Bcb::parse(&raw).unwrap(), written, "as the system's first program writes it");
    }

    #[test]
    fn the_commands_that_ask_for_the_recovery() {
        let command = |c: &str| Bcb { command: c.into(), args: Vec::new() };
        for asking in ["boot-recovery", "boot-fastboot", "boot-rescue"] {
            assert!(command(asking).asks_for_recovery(), "{asking}");
        }
        for not in ["", "bootonce-bootloader"] {
            assert!(!command(not).asks_for_recovery(), "{not:?}");
        }
    }

    #[test]
    fn the_first_work_in_order_is_the_one_done() {
        assert_eq!(asked("boot-recovery", &["--sideload", "--wipe_data"]).work, WIPE_DATA);
        assert_eq!(asked("boot-recovery", &["--wipe_data", "--fastboot"]).work, Work::Fastboot);
        assert_eq!(
            asked("boot-recovery", &["--wipe_data", "--update_package=/data/ota_package/update.zip"]).work,
            Work::Update(Update { package: "/data/ota_package/update.zip".into(), ..Update::default() })
        );
        assert_eq!(asked("boot-recovery", &["--wipe_ab", "--wipe_package_size=4096", "--sideload"]).work, Work::WipeAb { package_size: 4096 });
        assert_eq!(asked("boot-recovery", &["--wipe_cache", "--wipe_ab"]).work, Work::WipeCache { reformat: None });
        assert_eq!(asked("boot-recovery", &["--fsck_unshare_blocks", "--rescue"]).work, Work::Rescue);
        assert_eq!(asked("boot-recovery", &["--just_exit", "--fsck_unshare_blocks"]).work, Work::UnshareBlocks);
    }

    #[test]
    fn an_update_the_system_asks_for_says_how_to_install_it() {
        let fresh = asked("boot-recovery", &["--update_package=@/cache/recovery/block.map", "--locale=zh-CN", "--security"]);
        assert_eq!(fresh.work, Work::Update(Update { package: "@/cache/recovery/block.map".into(), retry: 0, wipe_cache: false, security: true }));
        let again = asked("boot-recovery", &["--update_package=/sdcard/update.zip", "--wipe_cache", "--retry_count=2", "--install_with_fuse"]);
        assert_eq!(again.work, Work::Update(Update { package: "/sdcard/update.zip".into(), retry: 2, wipe_cache: true, security: false }));
        assert!(again.unknown.is_empty());
        let last = asked("boot-recovery", &["--update_package=/a.zip", "--update_package=/b.zip"]);
        assert_eq!(last.work, Work::Update(Update { package: "/b.zip".into(), ..Update::default() }), "as given last");
    }

    #[test]
    fn an_option_without_the_value_it_takes_is_not_known() {
        assert_eq!(asked("boot-recovery", &["--update_package"]).work, Work::Unsupported("--update_package".into()));
        assert_eq!(asked("boot-recovery", &["--update_package="]).work, Work::Unsupported("--update_package=".into()));
        let unread = asked("boot-recovery", &["--update_package=/a.zip", "--retry_count=twice"]);
        assert_eq!(unread.work, Work::Update(Update { package: "/a.zip".into(), ..Update::default() }));
        assert_eq!(unread.unknown, ["--retry_count=twice"]);
        assert_eq!(asked("boot-recovery", &["--wipe_ab", "--wipe_package_size=-1"]).unknown, ["--wipe_package_size=-1"]);
    }

    #[test]
    fn formatting_as_another_filesystem_shapes_erasing() {
        let data = asked("boot-recovery", &["--wipe_data", "--reformat_data=ext4", "--reason=toggle16k"]);
        assert_eq!(data.work, Work::WipeData { reformat: Some("ext4".into()) });
        assert_eq!(asked("boot-recovery", &["--reformat_data=f2fs", "--wipe_cache"]).work, Work::WipeCache { reformat: Some("f2fs".into()) });
        assert_eq!(asked("boot-recovery", &["--reformat_data=ext4"]).work, Work::Menu, "alone, it shapes nothing");
        assert_eq!(
            asked("boot-recovery", &["--wipe_data", "--reformat_data=btrfs"]).work,
            Work::Unsupported("--reformat_data=btrfs".into()),
            "a filesystem the recovery cannot make leaves the erasing undone"
        );
        assert_eq!(
            asked("boot-recovery", &["--reformat_data=ext4", "--update_package=/a.zip"]).work,
            Work::Update(Update { package: "/a.zip".into(), ..Update::default() })
        );
    }

    #[test]
    fn a_wipe_package_comes_with_its_size() {
        assert_eq!(asked("boot-recovery", &["--wipe_ab", "--wipe_package_size=1834", "--reason=wipe_ab"]).work, Work::WipeAb { package_size: 1834 });
        assert_eq!(asked("boot-recovery", &["--wipe_ab"]).work, Work::WipeAb { package_size: 0 }, "erasing says the size is missing");
    }

    #[test]
    fn a_command_alone_can_ask_for_fastboot_or_rescue() {
        assert_eq!(asked("boot-fastboot", &[]).work, Work::Fastboot);
        assert_eq!(asked("boot-fastboot", &["--wipe_data"]).work, WIPE_DATA, "arguments come first");
        assert_eq!(asked("boot-rescue", &[]).work, Work::Rescue);
    }

    #[test]
    fn arguments_the_recovery_does_not_know() {
        let alone = asked("boot-recovery", &["--wipe_efs"]);
        assert_eq!(alone.work, Work::Unsupported("--wipe_efs".into()), "asked for on their own, they are shown");
        let beside = asked("boot-recovery", &["--wipe_data", "--keep_photos", "stray"]);
        assert_eq!(beside.work, WIPE_DATA, "beside work the recovery knows, they change nothing");
        assert_eq!(beside.unknown, ["--keep_photos", "stray"]);
    }

    #[test]
    fn work_that_goes_on_keeps_its_request() {
        let lasting = [
            Work::Update(Update::default()),
            WIPE_DATA,
            Work::PromptWipeData,
            Work::WipeCache { reformat: None },
            Work::WipeAb { package_size: 1 },
            Work::Sideload { auto_reboot: true },
            Work::Rescue,
            Work::UnshareBlocks,
        ];
        for work in lasting {
            assert!(work.lasts(), "{work:?}");
        }
        for work in [Work::Menu, Work::Fastboot, Work::Exit, Work::SystemStopped, Work::Unsupported("--wipe_efs".into())] {
            assert!(!work.lasts(), "{work:?}");
        }
    }

    #[test]
    fn a_retry_takes_the_place_of_the_count_before_it() {
        let bcb = Bcb { command: "boot-recovery".into(), args: vec!["--update_package=/a.zip".into(), "--retry_count=1".into(), "--locale=zh-CN".into()] };
        assert_eq!(bcb.with_retry(2).args, ["--update_package=/a.zip", "--locale=zh-CN", "--retry_count=2"]);
        let fresh = Bcb { command: String::new(), args: vec!["--update_package=/a.zip".into()] };
        assert_eq!(fresh.with_retry(1), Bcb { command: "boot-recovery".into(), args: vec!["--update_package=/a.zip".into(), "--retry_count=1".into()] });
    }

    fn scratch(name: &str, contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("skyward-bcb-{name}-{}", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_message_written_reads_back_with_the_rest_of_misc_kept() {
        let mut before = vec![0xffu8; 64 * 1024];
        before[..SIZE].copy_from_slice(&message(b"bootonce-bootloader", b"recovery\n--wipe_data\n--locale=zh-CN-with-a-long-tail\n"));
        let path = scratch("write", &before);
        let bcb = Bcb { command: "boot-recovery".into(), args: vec!["--update_package=/a.zip".into(), "--retry_count=1".into()] };
        write(&path, &bcb).unwrap();
        assert_eq!(read(&path).unwrap(), bcb);
        let rescue = Bcb { command: "boot-rescue".into(), args: Vec::new() };
        write(&path, &rescue).unwrap();
        let after = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(Bcb::parse(&after).unwrap(), rescue);
        assert!(after[RECOVERY].iter().all(|&b| b == 0), "no arguments leave the field empty");
        assert!(after[32..64].iter().all(|&b| b == 0), "the status was zero");
        assert!(after[SIZE..].iter().all(|&b| b == 0xff), "nothing past the message is touched");
    }

    #[test]
    fn a_message_too_long_for_misc_is_not_written() {
        let path = scratch("long", &[0u8; SIZE]);
        let long = Bcb { command: "boot-recovery".into(), args: vec!["--update_package=/".to_string() + &"a".repeat(800)] };
        let written = write(&path, &long);
        let after = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(written.is_err());
        assert!(after.iter().all(|&b| b == 0), "misc is left as it was");
    }

    #[test]
    fn the_wipe_package_is_read_from_sixteen_kibibytes() {
        let mut misc = vec![0u8; 64 * 1024];
        misc[16 * 1024..16 * 1024 + 4].copy_from_slice(b"PK\x03\x04");
        let path = scratch("wipe", &misc);
        let read = wipe_package(&path, 4);
        let none = wipe_package(&path, 0);
        let too_big = wipe_package(&path, 16 * 1024 + 1);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read.unwrap(), b"PK\x03\x04");
        assert!(none.is_err() && too_big.is_err());
    }

    #[test]
    fn forgetting_memtag_mode_empties_only_its_message() {
        let path = std::env::temp_dir().join(format!("skyward-bcb-memtag-{}", std::process::id()));
        std::fs::write(&path, [0xffu8; 64 * 1024]).unwrap();
        forget_memtag_mode(&path).unwrap();
        let after = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let (start, end) = (MEMTAG_MESSAGE.start as usize, MEMTAG_MESSAGE.end as usize);
        assert_eq!((start, end - start), (32832, 64));
        assert!(after[start..end].iter().all(|&b| b == 0));
        assert!(after[..start].iter().chain(&after[end..]).all(|&b| b == 0xff));
    }

    #[test]
    fn clear_zeroes_only_the_message() {
        let path = std::env::temp_dir().join(format!("skyward-bcb-clear-{}", std::process::id()));
        std::fs::write(&path, [0xffu8; 2 * SIZE]).unwrap();
        clear(&path).unwrap();
        let after = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(after[..SIZE].iter().all(|&b| b == 0));
        assert!(after[SIZE..].iter().all(|&b| b == 0xff));
    }
}
