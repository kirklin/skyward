//! Work on whole partitions on a thread of its own, one after another:
//! erasing the partitions a wipe package lists and undoing shared blocks,
//! which the system asks for before the recovery starts, and repairing or
//! making afresh the filesystem of one partition, which the filesystem
//! page asks for.

use anyhow::{Context, Result};
use skyward_block::erase;
use skyward_block::fsck::{self, Fixed};
use skyward_block::fstab::{self, Entry};
use skyward_block::{mount, probe, tools};
use skyward_crypto::data;
use skyward_install::package::Phone;
use skyward_install::wipe::{self as package, Wipe};
use skyward_install::Refusal;
use skyward_sys::{bcb, prop};
use skyward_vab::Slot;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

/// Where e2fsck is put to run once the installed system it comes from is
/// let go, as Android's own recovery puts it, with the libraries of its own
/// it loads, which the system keeps newer than the recovery's: those whose
/// names start so. The rest it loads are the recovery's.
const E2FSCK_COPY: &str = "/tmp/e2fsck";
const EXT2_LIBRARIES: &str = "libext2";

/// The name a partition is mapped under while it is looked at or worked
/// on, beside any mapping of it the recovery keeps.
const WORK_MAPPING: &str = "skyward-partwork";

pub enum Task {
    /// Erase the partitions the wipe package of `package_size` bytes the
    /// system left in misc lists, once it is found signed with the phone's
    /// update keys and made for this phone.
    Disable { package_size: u64 },
    /// Undo the sharing of blocks in the slot's ext4 partitions.
    Unshare { slot: Slot },
    /// Check the whole of a filesystem and fix what is wrong.
    Repair(Part),
    /// Make a partition's filesystem afresh as this one.
    Remake(Part, String),
}

/// A partition the filesystem page works on.
pub enum Part {
    /// The data partition, reached through its decryption under the key
    /// the phone keeps for it.
    Data,
    /// A partition of the phone's table, by its entry.
    Entry(Entry),
}

/// How one partition ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Erased,
    Unshared,
    /// It holds this filesystem, which shares no blocks.
    NotShared(String),
    Fixed(Fixed),
    Made,
    Failed(String),
}

/// Why the work stopped before going through the partitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The wipe package is not one this phone may be erased by.
    Refused(Refusal),
    /// The program it takes is only the installed system's, and that
    /// system is not mounted.
    Unmounted(&'static str),
    Failed(String),
}

pub enum Event {
    /// The partitions the work goes through, in order.
    Listed(Vec<String>),
    /// The partition at this index is under way.
    At(usize),
    /// The partition at this index ended this way, the program working on
    /// it having said what follows.
    Ended(usize, Item, String),
    /// Every partition has ended, or the work stopped before them.
    Done(Result<(), Stop>),
}

pub struct Worker {
    events: Receiver<Event>,
    /// Readable while events wait.
    wake: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(task: Task) -> Result<Worker> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error()).context("make the partition worker's wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (tx, events) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("partitions".into())
            .spawn(move || {
                let mut tell = |event: Event| {
                    // The screen only stops listening once it has let the
                    // worker go.
                    let _ = tx.send(event);
                    unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
                };
                let done = match task {
                    Task::Disable { package_size } => disable(package_size, &mut tell),
                    Task::Unshare { slot } => unshare(slot, &mut tell),
                    Task::Repair(part) => repair(part, &mut tell),
                    Task::Remake(part, filesystem) => remake(part, &filesystem, &mut tell),
                };
                tell(Event::Done(done));
            })
            .context("start the partition worker")?;
        Ok(Worker { events, wake, thread: Some(thread) })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.events.try_iter().collect()
    }
}

impl Drop for Worker {
    /// The screen lets the worker go once its work has ended.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the partition worker does not panic");
        }
    }
}

/// Why something failed, as the page shows it.
fn said(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

/// Erase the phone for good, as Android's own recovery does for a wipe
/// package: every partition it lists is discarded whole, and one that
/// fails does not keep the rest from being discarded.
fn disable(package_size: u64, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
    let read = (|| -> Result<Wipe> {
        let bytes = bcb::wipe_package(&bcb::misc(), package_size)?;
        let phone = Phone {
            device: prop("ro.product.device").context("ro.product.device is unset")?,
            serial: prop("ro.serialno").unwrap_or_default(),
            installed: None,
        };
        package::read(bytes, &phone, &prop("ro.build.tags").unwrap_or_default())
    })();
    let partitions = match read {
        Ok(Wipe::Partitions(partitions)) => partitions,
        Ok(Wipe::Refused(refusal)) => return Err(Stop::Refused(refusal)),
        Err(e) => return Err(Stop::Failed(said(&e))),
    };
    // What is erased of the boot partitions is the installed system's,
    // whose boot set goes back into them first.
    skyward_systems::install::ensure_installed().map_err(|e| Stop::Failed(said(&e)))?;
    tell(Event::Listed(partitions.clone()));
    for (index, partition) in partitions.iter().enumerate() {
        tell(Event::At(index));
        let item = match erase::discard(Path::new(partition)) {
            Ok(()) => Item::Erased,
            Err(e) => Item::Failed(said(&e)),
        };
        tell(Event::Ended(index, item, String::new()));
    }
    Ok(())
}

/// Undo the sharing of blocks in the slot's partitions, as Android's own
/// recovery did for `adb remount`: each one is looked at, and each that
/// holds ext4 has e2fsck give its files blocks of their own. The recovery
/// runs this with none of the slot's partitions mounted, so where it
/// carries no e2fsck of its own, the installed system's is copied out
/// first.
fn unshare(slot: Slot, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
    let bases = skyward_vab::slot_bases(slot).map_err(|e| Stop::Failed(said(&e)))?;
    tell(Event::Listed(bases.clone()));
    let mut held = Vec::new();
    for (index, base) in bases.iter().enumerate() {
        tell(Event::At(index));
        match filesystem_of(slot, base) {
            Ok(Some(filesystem)) if filesystem == "ext4" => held.push(index),
            Ok(Some(filesystem)) => tell(Event::Ended(index, Item::NotShared(filesystem), String::new())),
            Ok(None) => tell(Event::Ended(index, Item::NotShared(String::new()), String::new())),
            Err(e) => tell(Event::Ended(index, Item::Failed(said(&e)), String::new())),
        }
    }
    if held.is_empty() {
        return Ok(());
    }
    let e2fsck = e2fsck(slot)?;
    for index in held {
        tell(Event::At(index));
        let base = &bases[index];
        let ran = (|| -> Result<(Fixed, tools::Ran)> {
            let whole = skyward_vab::map_whole(slot, base, WORK_MAPPING, false)?;
            let ran = fsck::unshare_blocks(&e2fsck, &whole.device);
            skyward_vab::unmap_whole(&whole)?;
            ran
        })();
        let (item, words) = match ran {
            Ok((Fixed::Failed, ran)) => (Item::Fixed(Fixed::Failed), ran.said),
            Ok((_, ran)) => (Item::Unshared, ran.said),
            Err(e) => (Item::Failed(said(&e)), String::new()),
        };
        tell(Event::Ended(index, item, words));
    }
    Ok(())
}

/// What the slot's partition `base` holds, looked at through a mapping of
/// its own.
fn filesystem_of(slot: Slot, base: &str) -> Result<Option<String>> {
    let whole = skyward_vab::map_whole(slot, base, WORK_MAPPING, true)?;
    let held = probe::filesystem(&whole.device);
    skyward_vab::unmap_whole(&whole)?;
    held
}

/// e2fsck as it runs once the slot's partitions are let go: the recovery's
/// own as it is, or the installed system's, copied out of it.
fn e2fsck(slot: Slot) -> Result<tools::Tool, Stop> {
    match tools::find(fsck::E2FSCK) {
        Ok(own) if own.libraries.is_empty() => Ok(own),
        _ => copy_e2fsck(slot),
    }
}

/// Copy e2fsck and its own libraries out of the installed system, mounted
/// for the moment, to where they run from once the system is let go.
fn copy_e2fsck(slot: Slot) -> Result<tools::Tool, Stop> {
    let copied = (|| -> Result<Result<tools::Tool, Stop>> {
        skyward_vab::mount_one(slot, tools::SYSTEM)?;
        let found = tools::find(fsck::E2FSCK);
        let copied = match &found {
            Ok(tool) => copy_with_libraries(&tool.program),
            Err(_) => Ok(PathBuf::new()),
        };
        skyward_vab::unmount_one(slot, tools::SYSTEM)?;
        let program = copied?;
        Ok(match found {
            Ok(_) => Ok(tools::Tool { program, libraries: vec![E2FSCK_COPY.into(), tools::RECOVERY_LIB64.into()] }),
            Err(tools::Missing::Unmounted(base)) => Err(Stop::Unmounted(base)),
            Err(tools::Missing::Absent(name)) => Err(Stop::Failed(format!("the installed system has no {name}"))),
        })
    })();
    copied.unwrap_or_else(|e| Err(Stop::Failed(said(&e))))
}

/// Copy `program` and the installed system's ext2 libraries into
/// [`E2FSCK_COPY`], and give where the program is now.
fn copy_with_libraries(program: &Path) -> Result<PathBuf> {
    let dir = Path::new(E2FSCK_COPY);
    std::fs::create_dir_all(dir).with_context(|| format!("make {E2FSCK_COPY}"))?;
    let name = program.file_name().with_context(|| format!("{} names no program", program.display()))?;
    let copy = dir.join(name);
    std::fs::copy(program, &copy).with_context(|| format!("copy {} to {}", program.display(), copy.display()))?;
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).with_context(|| format!("make {} runnable", copy.display()))?;
    for entry in std::fs::read_dir(tools::SYSTEM_LIB64).with_context(|| format!("read {}", tools::SYSTEM_LIB64))? {
        let library = entry.with_context(|| format!("read {}", tools::SYSTEM_LIB64))?.path();
        let own = library.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(EXT2_LIBRARIES) && n.ends_with(".so"));
        if own {
            let to = dir.join(library.file_name().expect("listed with a name"));
            std::fs::copy(&library, &to).with_context(|| format!("copy {} to {}", library.display(), to.display()))?;
        }
    }
    Ok(copy)
}

/// Check the whole of the filesystem a partition holds, found as it is
/// reached, and fix what is wrong, telling what the program said. The data
/// partition is closed first and reached through a mapping of its own,
/// which goes again once the work is done; another partition is unmounted
/// for it and mounted again read-only after.
fn repair(part: Part, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
    tell(Event::Listed(vec![name(&part)]));
    tell(Event::At(0));
    let done = reach(&part, |device| {
        let filesystem = probe::filesystem(device)?.context("the partition holds no filesystem blkid knows")?;
        let (fixed, ran) = fsck::repair(device, &filesystem)?;
        Ok((Item::Fixed(fixed), ran.said))
    });
    let (item, words) = done.unwrap_or_else(|e| (Item::Failed(said(&e)), String::new()));
    tell(Event::Ended(0, item, words));
    Ok(())
}

/// Make the partition afresh as `filesystem`, with nothing mounted from it.
fn remake(part: Part, filesystem: &str, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
    tell(Event::Listed(vec![name(&part)]));
    tell(Event::At(0));
    let made = match &part {
        Part::Entry(entry) if entry.mount_point == "/cache" => crate::wipe::remake_cache(entry, filesystem),
        Part::Entry(entry) => crate::wipe::remake(entry, filesystem),
        Part::Data => Err(anyhow::anyhow!("the data partition is made afresh as another filesystem by a factory reset")),
    };
    let item = match made {
        Ok(()) => Item::Made,
        Err(e) => Item::Failed(said(&e)),
    };
    tell(Event::Ended(0, item, String::new()));
    Ok(())
}

/// The partition's name, as the pages list it.
fn name(part: &Part) -> String {
    match part {
        Part::Data => "data".into(),
        Part::Entry(entry) => entry.mount_point.trim_start_matches('/').to_string(),
    }
}

/// Do `work` with the device the partition's filesystem is reached through,
/// nothing mounted from it.
fn reach<T>(part: &Part, work: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    match part {
        Part::Data => {
            crate::wipe::release_data()?;
            let mapped = crate::decrypt::with_key_service(data::map_unmounted).map_err(anyhow::Error::msg)?;
            let done = work(&mapped);
            data::unmap()?;
            done
        }
        Part::Entry(entry) => {
            let at = Path::new(&entry.mount_point);
            let mounted = mount::is_mount_point(at)?;
            if mounted {
                mount::umount(at)?;
            }
            let device = PathBuf::from(fstab::device(entry)?);
            let done = work(&device);
            if mounted {
                mount::mount_ro(&device, at)?;
            }
            done
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_the_phone_a_partition_of_the_table_is_repaired_and_mounted_again_read_only() {
        let root = std::env::temp_dir().join(format!("skyward-partwork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let at = root.join("mnt");
        std::fs::create_dir_all(&at).unwrap();
        let image = root.join("part.img");
        std::fs::File::create(&image).unwrap().set_len(128 << 20).unwrap();
        let out = std::process::Command::new("/system/bin/losetup").args(["-f", "--show"]).arg(&image).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let device = String::from_utf8(out.stdout).unwrap().trim().to_string();
        // Held open throughout, so nothing lets the device go before the
        // test does.
        let held = std::fs::File::open(&device).unwrap();
        erase::make(Path::new(&device), "f2fs", &[], false).unwrap();
        mount::mount_ro(Path::new(&device), &at).unwrap();
        let mount_point = at.to_string_lossy().into_owned();
        let entry = Entry {
            device: device.clone(),
            mount_point: mount_point.clone(),
            filesystem: "f2fs".into(),
            options: Vec::new(),
            extras: Vec::new(),
        };

        let mut told = Vec::new();
        assert_eq!(repair(Part::Entry(entry), &mut |event| told.push(event)), Ok(()));

        let [Event::Listed(names), Event::At(0), Event::Ended(0, item, said)] = &told[..] else {
            panic!("told {} events", told.len())
        };
        assert_eq!(names, &[mount_point.trim_start_matches('/').to_string()]);
        assert_eq!(item, &Item::Fixed(Fixed::Clean), "{said}");
        assert!(mount::is_mount_point(&at).unwrap(), "mounted again");
        let written = std::fs::write(at.join("note"), b"x").unwrap_err();
        assert_eq!(written.raw_os_error(), Some(libc::EROFS), "read-only");
        mount::umount(&at).unwrap();
        drop(held);
        let detached = std::process::Command::new("/system/bin/losetup").args(["-d", &device]).status().unwrap();
        assert!(detached.success(), "let {device} go");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
