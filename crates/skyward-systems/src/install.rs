//! Installing the other system from its package, switching between the
//! systems on the phone, and taking the other system away again.

use crate::state::{State, Store};
use crate::{Named, Other, Overview};
use skyward_block::area::HEADER;

/// Where the areas' files are, in the data partition.
pub const FOLDER: &str = "/data/skyward/systems";
/// The devices the store and a root are laid out as while they are written.
pub const STORE_DEVICE: &str = "skyward-store";
pub const ROOT_DEVICE: &str = "skyward-root";
/// The gigabyte the pages count sizes in.
const GB: u64 = 1_000_000_000;
/// The sizes a root is offered in, in gigabytes, past what it least takes.
const OFFERED: [u64; 6] = [8, 16, 32, 64, 128, 256];

/// How far an install has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Setting room aside in the data partition: the bytes so far, of all.
    Allocating { done: u64, total: u64 },
    /// Writing the root image, then reading it back.
    Writing { done: u64, total: u64 },
    /// Putting the boot set in the store.
    Finishing,
}

/// The room the data partition has for a system's root, of `free` bytes
/// free: less the store where none is made yet, and less a super's worth,
/// which the installed system's updates keep for themselves, as Android
/// keeps it from its dynamic systems.
pub fn room(free: u64, store: u64, super_size: u64) -> u64 {
    free.saturating_sub(store + super_size + HEADER)
}

/// The sizes offered for a root that takes at least `least` bytes, with
/// `room` bytes to spare: what it least takes in whole gigabytes, and the
/// usual sizes past that, as far as there is room. The area made for one
/// is rounded up to its filesystem's blocks.
pub fn sizes(least: u64, room: u64) -> Vec<u64> {
    let first = least.div_ceil(GB).max(1);
    std::iter::once(first).chain(OFFERED.into_iter().filter(|&s| s > first)).map(|s| s * GB).filter(|&s| s <= room).collect()
}

/// What the pages show of the systems, from the state and the installed
/// system's name; `changed` says whether its vendor_boot or dtbo differ
/// from when the other system was installed.
pub fn overview(state: &State, installed: Named, changed: bool) -> Overview {
    Overview {
        installed,
        other: state.system.as_ref().map(|s| Other { id: s.id.clone(), name: s.name.clone(), version: s.version.clone(), size: s.root.size - HEADER }),
        other_starts: state.occupied.is_some(),
        changed,
    }
}

impl Store {
    /// A store's bytes for the phone's partitions, where the store is not
    /// made yet; none where it is.
    pub fn needed(state: &State, boot: u64, init_boot: u64) -> u64 {
        match state.store {
            Some(_) => 0,
            None => Store::size_for(boot, init_boot),
        }
    }
}

#[cfg(target_os = "android")]
pub use sys::*;

#[cfg(target_os = "android")]
mod sys {
    use super::*;
    use crate::boot::{self, Start};
    use crate::package::{Contents, BOOT, INIT_BOOT, ROOT};
    use anyhow::Result;
    use crate::state::{System, Who, PATH};
    use crate::store::{self, OTHER};
    use crate::switch::{self, Places};
    use anyhow::{ensure, Context};
    use sha2::{Digest, Sha256};
    use skyward_block::area::{self, Area};
    use skyward_block::{dm, BY_NAME};
    use skyward_vab::Slot;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use zip::ZipArchive;

    const CHUNK: usize = 1 << 20;

    fn by_name() -> &'static Path {
        Path::new(BY_NAME)
    }

    /// Where the data partition is mounted.
    const DATA: &std::ffi::CStr = c"/data";

    fn data() -> &'static Path {
        Path::new(DATA.to_str().expect("the data partition's mount point is written in ASCII"))
    }

    /// The bytes free in the data partition's filesystem.
    pub fn free() -> Result<u64> {
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(DATA.as_ptr(), &mut s) } != 0 {
            return Err(std::io::Error::last_os_error()).context("look at the room in the data partition");
        }
        Ok(s.f_bavail as u64 * s.f_frsize as u64)
    }

    /// The room for a root, in the data partition as it is now.
    pub fn room_now(slot: Slot) -> Result<u64> {
        let state = state()?;
        let size = |name: &str| skyward_block::size_bytes(store::partition(by_name(), name, slot));
        let needed = Store::needed(&state, size(store::BOOT)?, size(store::INIT_BOOT)?);
        Ok(room(free()?, needed, skyward_block::size_bytes(by_name().join("super"))?))
    }

    /// Lay the store out as a device, once it is found where it was, run
    /// `work` on it, and take the device away again.
    fn with_store<T>(state: &State, work: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        let store = state.store.as_ref().context("no store holds the boot sets")?;
        store.area.check_header()?;
        if dm::status(STORE_DEVICE)?.is_some() {
            dm::remove(STORE_DEVICE)?;
        }
        let node = store.area.map(STORE_DEVICE)?;
        let done = work(&node);
        let gone = dm::remove(STORE_DEVICE);
        let done = done?;
        gone?;
        Ok(done)
    }

    const METADATA: &str = "/metadata";

    /// Mount /metadata, where the state is kept, read-only where it is not
    /// mounted yet, as the recovery keeps it.
    pub fn metadata() -> Result<()> {
        let at = Path::new(METADATA);
        if !skyward_block::mount::is_mount_point(at)? {
            skyward_block::mount::mount_ro(&by_name().join("metadata"), at)?;
        }
        Ok(())
    }

    /// Run `work` with /metadata writable, and leave it read-only after.
    fn writable<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
        metadata()?;
        let at = Path::new(METADATA);
        skyward_block::mount::remount_writable(at, true)?;
        let done = work();
        let back = skyward_block::mount::remount_writable(at, false);
        let done = done?;
        back?;
        Ok(done)
    }

    /// The state as kept now.
    pub fn state() -> Result<State> {
        metadata()?;
        State::load(Path::new(PATH))
    }

    /// The program the other system starts with: this recovery's own.
    fn program() -> PathBuf {
        Path::new("/").join(skyward_config::ramdisk::SYSTEM_INIT)
    }

    /// Have the bootloader start `to`, writing its boot set into the boot
    /// partitions; see [`switch::switch`]. Switching to the other system
    /// waits for any update to be merged, and first checks that the other
    /// system's root is still where it was put.
    pub fn switch_to(next: Slot, to: Who) -> Result<()> {
        let state = state()?;
        if to == Who::Other {
            let update = skyward_vab::snapshot_state()?;
            ensure!(update == skyward_vab::SnapshotState::None, "an update is in flight ({update:?}), and the installed system has to start once more to merge it");
            state.system.as_ref().context("no other system is installed")?.root.check_header()?;
        }
        let (path, program) = (Path::new(PATH), program());
        writable(|| with_store(&state, |device| switch::switch(&Places { state: path, by_name: by_name(), store: device, program: &program }, next, to)))
    }

    /// Finish a switch cut off before, where one was; says whether there
    /// was one.
    pub fn resume() -> Result<bool> {
        let state = state()?;
        if state.switching.is_none() {
            return Ok(false);
        }
        let (path, program) = (Path::new(PATH), program());
        writable(|| with_store(&state, |device| switch::resume(&Places { state: path, by_name: by_name(), store: device, program: &program })))
    }

    /// Put the installed system's boot set back where the other system's
    /// is, as everything that writes or erases the installed system first
    /// does; says whether it had to.
    pub fn ensure_installed() -> Result<bool> {
        resume()?;
        let state = state()?;
        let Some(slot) = state.occupied.as_deref().map(|s| Slot::from_suffix(s).with_context(|| format!("the other system is in slot {s:?}"))).transpose()? else {
            return Ok(false);
        };
        switch_to(slot, Who::Installed)?;
        Ok(true)
    }

    /// The SHA-256 of all of a partition.
    fn sum_of(path: &Path) -> Result<String> {
        let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        skyward_block::drop_cache(&f)?;
        let mut sum = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
            if n == 0 {
                break;
            }
            sum.update(&buf[..n]);
        }
        Ok(sum.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Whether the installed system's vendor_boot or dtbo in `slot` differ
    /// from when the other system was installed.
    pub fn changed(state: &State, slot: Slot) -> Result<bool> {
        let Some(system) = &state.system else { return Ok(false) };
        Ok(sum_of(&store::partition(by_name(), "vendor_boot", slot))? != system.vendor_boot
            || sum_of(&store::partition(by_name(), "dtbo", slot))? != system.dtbo)
    }

    /// Install the system in the package `zip`, whose contents are
    /// `contents`, with a root of `size` bytes: its root written into a
    /// new area of the data partition, which has to be open, and its boot
    /// set, with `program` to start it, put in the store, made where there
    /// is none yet. `slot` is the slot the phone runs from, whose
    /// init_boot image lends its header.
    pub fn install<R: Read + Seek>(zip: &mut ZipArchive<R>, contents: &Contents, size: u64, program: &[u8], slot: Slot, progress: &mut dyn FnMut(Step)) -> Result<()> {
        writable(|| install_now(zip, contents, size, program, slot, progress))
    }

    fn install_now<R: Read + Seek>(zip: &mut ZipArchive<R>, contents: &Contents, size: u64, program: &[u8], slot: Slot, progress: &mut dyn FnMut(Step)) -> Result<()> {
        let path = Path::new(PATH);
        let mut state = State::load(path)?;
        ensure!(state.switching.is_none(), "a switch begun before is not done yet");
        ensure!(state.system.is_none(), "another system is installed already");
        ensure!(size >= contents.least(), "a root of {size} bytes is less than the {} the system takes", contents.least());
        let room = room_now(slot)?;
        ensure!(size <= room, "the data partition has room for {room} bytes, less than {size}");
        let folder = Path::new(FOLDER);
        fs::create_dir_all(folder).with_context(|| format!("make {}", folder.display()))?;
        if state.store.is_none() {
            let (boot, init_boot) = (skyward_block::size_bytes(store::partition(by_name(), store::BOOT, slot))?, skyward_block::size_bytes(store::partition(by_name(), store::INIT_BOOT, slot))?);
            let area = area::create(&folder.join("store"), Store::size_for(boot, init_boot), &mut |done, total| progress(Step::Allocating { done, total }))?;
            state.store = Some(Store { area, boot, init_boot });
            state.save(path)?;
        }
        let id = contents.manifest.id.clone();
        let file = folder.join(format!("{id}.img"));
        let root = area::create(&file, size + HEADER, &mut |done, total| progress(Step::Allocating { done, total }))?;
        let made = (|| -> Result<System> {
            write_root(zip, &root, contents.root, progress)?;
            progress(Step::Finishing);
            let start = Start { id: id.clone(), filesystem: contents.manifest.filesystem.clone(), init: contents.manifest.init.clone(), root: root.clone() };
            let (init_boot, own_ramdisk) = match contents.init_boot {
                true => {
                    let mut own = Vec::new();
                    zip.by_name(INIT_BOOT).context("open init_boot.img")?.read_to_end(&mut own).context("read init_boot.img")?;
                    let (image, own_ramdisk) = boot::with_start(&own, program, &start)?;
                    (image, Some(own_ramdisk))
                }
                false => (boot::init_boot(&fs::read(store::partition(by_name(), store::INIT_BOOT, slot)).context("read the phone's init_boot")?, program, &start)?, None),
            };
            let set = with_store(&state, |device| {
                let mut image = zip.by_name(BOOT).context("open boot.img")?;
                let store = state.store.as_ref().expect("made above");
                store::put(store, device, OTHER, &mut image, contents.boot, &init_boot)
            })?;
            Ok(System {
                id: id.clone(),
                name: contents.manifest.name.clone(),
                version: contents.manifest.version.clone(),
                filesystem: contents.manifest.filesystem.clone(),
                init: contents.manifest.init.clone(),
                root: root.clone(),
                set,
                own_ramdisk,
                vendor_boot: sum_of(&store::partition(by_name(), "vendor_boot", slot))?,
                dtbo: sum_of(&store::partition(by_name(), "dtbo", slot))?,
            })
        })();
        match made {
            Ok(system) => {
                state.system = Some(system);
                state.save(path)
            }
            Err(e) => match root.remove() {
                Ok(()) => Err(e),
                Err(r) => Err(e.context(format!("taking the root away afterwards failed too: {r:#}"))),
            },
        }
    }

    /// Write the root image from the package into the root area, after its
    /// header, then read it back.
    fn write_root<R: Read + Seek>(zip: &mut ZipArchive<R>, root: &Area, len: u64, progress: &mut dyn FnMut(Step)) -> Result<()> {
        ensure!(len <= root.size - HEADER, "the root image takes {len} bytes, more than its area's {}", root.size - HEADER);
        if dm::status(ROOT_DEVICE)?.is_some() {
            dm::remove(ROOT_DEVICE)?;
        }
        let node = root.map(ROOT_DEVICE)?;
        let written = (|| -> Result<()> {
            let mut out = OpenOptions::new().read(true).write(true).open(&node).with_context(|| format!("open {}", node.display()))?;
            let mut image = zip.by_name(ROOT).context("open rootfs.img")?;
            out.seek(SeekFrom::Start(HEADER))?;
            let mut buf = vec![0u8; CHUNK];
            let (mut sum, mut done) = (Sha256::new(), 0);
            while done < len {
                let n = (len - done).min(CHUNK as u64) as usize;
                image.read_exact(&mut buf[..n]).context("read rootfs.img")?;
                out.write_all(&buf[..n]).context("write the root")?;
                sum.update(&buf[..n]);
                done += n as u64;
                progress(Step::Writing { done, total: len * 2 });
            }
            out.sync_all().context("sync the root")?;
            skyward_block::drop_cache(&out)?;
            out.seek(SeekFrom::Start(HEADER))?;
            let (mut back, mut read) = (Sha256::new(), 0);
            while read < len {
                let n = (len - read).min(CHUNK as u64) as usize;
                out.read_exact(&mut buf[..n]).context("read the root back")?;
                back.update(&buf[..n]);
                read += n as u64;
                progress(Step::Writing { done: len + read, total: len * 2 });
            }
            ensure!(back.finalize() == sum.finalize(), "the root reads back different from what was written");
            Ok(())
        })();
        let gone = dm::remove(ROOT_DEVICE);
        written?;
        gone
    }

    /// Take the other system away: its boot set out of the partitions
    /// where it is there, its root and the store out of the data partition,
    /// which has to be open, and the state with them.
    pub fn remove() -> Result<()> {
        ensure!(skyward_block::mount::is_mount_point(data())?, "the data partition is not open");
        ensure_installed()?;
        writable(|| {
            let path = Path::new(PATH);
            let mut state = State::load(path)?;
            if let Some(system) = &state.system {
                system.root.remove()?;
                state.system = None;
                state.save(path)?;
            }
            if let Some(store) = &state.store {
                store.area.remove()?;
                state = State::default();
                state.save(path)?;
            }
            let folder = PathBuf::from(FOLDER);
            if folder.exists() && fs::read_dir(&folder)?.next().is_none() {
                fs::remove_dir(&folder).with_context(|| format!("delete {}", folder.display()))?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_for_a_root_keeps_a_supers_worth_for_updates() {
        assert_eq!(room(59 * GB, 600 << 20, 9 * GB), 59 * GB - (600 << 20) - 9 * GB - HEADER);
        assert_eq!(room(GB, 0, 9 * GB), 0, "none to spare");
    }

    #[test]
    fn the_sizes_offered_start_at_what_the_root_takes_and_stay_within_room() {
        assert_eq!(sizes(3 << 30, 50 * GB), [4 * GB, 8 * GB, 16 * GB, 32 * GB], "3 GiB is past 3 GB");
        assert_eq!(sizes(100 << 20, 9 * GB), [GB, 8 * GB]);
        assert_eq!(sizes(20 * GB, 300 * GB), [20 * GB, 32 * GB, 64 * GB, 128 * GB, 256 * GB]);
        assert!(sizes(20 * GB, 10 * GB).is_empty(), "no room for it at all");
    }
}
