//! The store of boot sets: after its header, three entries of the boot
//! partition's bytes and then the init_boot partition's. The installed
//! system's saves take turns between the first two, so that the last whole
//! one stays while the next is written; the other system's is the third.

use crate::state::{Saved, Store, Written};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use skyward_block::area::HEADER;
use skyward_vab::Slot;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const ENTRIES: u64 = 3;
/// The entry the other system's boot set is in.
pub const OTHER: u32 = 2;
/// The partitions a boot set fills.
pub const BOOT: &str = "boot";
pub const INIT_BOOT: &str = "init_boot";
const CHUNK: usize = 1 << 20;

impl Store {
    /// The bytes a store takes for partitions of these sizes.
    pub fn size_for(boot: u64, init_boot: u64) -> u64 {
        HEADER + ENTRIES * (boot + init_boot)
    }

    /// Where `entry` starts in the store.
    fn offset(&self, entry: u32) -> u64 {
        HEADER + u64::from(entry) * (self.boot + self.init_boot)
    }

    /// The entry the installed system's next save goes to: the one its last
    /// save is not in.
    pub fn next_save(last: Option<&Saved>) -> u32 {
        match last {
            Some(s) if s.entry == 0 => 1,
            _ => 0,
        }
    }
}

/// The partition `name` of `slot` among the links in `dir`.
pub fn partition(dir: &Path, name: &str, slot: Slot) -> PathBuf {
    dir.join(format!("{name}{}", slot.suffix()))
}

/// Copy `len` bytes from `from` to `to` where it stands, returning their
/// SHA-256.
fn copy(from: &mut dyn Read, to: &mut File, len: u64) -> Result<String> {
    let mut buf = vec![0u8; CHUNK];
    let mut sum = Sha256::new();
    let mut left = len;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        from.read_exact(&mut buf[..n]).context("read")?;
        to.write_all(&buf[..n]).context("write")?;
        sum.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hex::encode(sum.finalize()))
}

/// The SHA-256 of `len` bytes of `f` from `at`, read past any pages cached
/// of it.
fn sum(f: &mut File, at: u64, len: u64) -> Result<String> {
    skyward_block::drop_cache(f)?;
    f.seek(SeekFrom::Start(at))?;
    let mut buf = vec![0u8; CHUNK];
    let mut sum = Sha256::new();
    let mut left = len;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        f.read_exact(&mut buf[..n]).context("read")?;
        sum.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hex::encode(sum.finalize()))
}

fn open(path: &Path, write: bool) -> Result<File> {
    OpenOptions::new().read(true).write(write).open(path).with_context(|| format!("open {}", path.display()))
}

/// The partitions of `slot` a boot set fills, by name, with the bytes each
/// holds and where in an entry it goes.
fn parts(store: &Store, dir: &Path, slot: Slot) -> Vec<(&'static str, PathBuf, u64, u64)> {
    let mut parts = vec![(BOOT, partition(dir, BOOT, slot), store.boot, 0)];
    if store.init_boot > 0 {
        parts.push((INIT_BOOT, partition(dir, INIT_BOOT, slot), store.init_boot, store.boot));
    }
    parts
}

/// Copy the boot set in the partitions of `slot`, among the links in
/// `dir`, into `entry` of the store at `device`, and read the entry back.
pub fn save(store: &Store, device: &Path, entry: u32, dir: &Path, slot: Slot) -> Result<Saved> {
    let mut out = open(device, true)?;
    let mut sums = Vec::new();
    for (_, path, len, within) in parts(store, dir, slot) {
        let mut from = open(&path, false)?;
        skyward_block::drop_cache(&from)?;
        out.seek(SeekFrom::Start(store.offset(entry) + within))?;
        let sum = copy(&mut from, &mut out, len).with_context(|| format!("copy {} into the store", path.display()))?;
        sums.push(sum);
    }
    out.sync_all().with_context(|| format!("sync {}", device.display()))?;
    let saved = Saved { entry, boot: sums[0].clone(), init_boot: sums.get(1).cloned().unwrap_or_default() };
    check(store, device, &saved)?;
    Ok(saved)
}

/// Check that the entry `saved` names still holds what was saved in it.
pub fn check(store: &Store, device: &Path, saved: &Saved) -> Result<()> {
    let mut f = open(device, false)?;
    let at = store.offset(saved.entry);
    ensure!(sum(&mut f, at, store.boot)? == saved.boot, "the store's entry {} holds another boot image than was saved", saved.entry);
    if store.init_boot > 0 {
        ensure!(sum(&mut f, at + store.boot, store.init_boot)? == saved.init_boot, "the store's entry {} holds another init_boot image than was saved", saved.entry);
    }
    Ok(())
}

/// The init_boot image of the boot set `saved` names, once the store is
/// found to hold it whole, with the zeros after it to its partition's size.
pub fn init_boot(store: &Store, device: &Path, saved: &Saved) -> Result<Vec<u8>> {
    let mut f = open(device, false)?;
    skyward_block::drop_cache(&f)?;
    f.seek(SeekFrom::Start(store.offset(saved.entry) + store.boot))?;
    let mut image = vec![0u8; store.init_boot as usize];
    f.read_exact(&mut image).with_context(|| format!("read {}", device.display()))?;
    ensure!(hex::encode(Sha256::digest(&image)) == saved.init_boot, "the store's entry {} holds another init_boot image than was saved", saved.entry);
    Ok(image)
}

/// What the boot partitions of `slot`, among the links in `dir`, hold now:
/// the SHA-256 of each, whole.
pub fn held(store: &Store, dir: &Path, slot: Slot) -> Result<Written> {
    let mut sums = Vec::new();
    for (_, path, len, _) in parts(store, dir, slot) {
        sums.push(sum(&mut open(&path, false)?, 0, len).with_context(|| format!("read {}", path.display()))?);
    }
    Ok(Written { boot: sums[0].clone(), init_boot: sums.get(1).cloned().unwrap_or_default() })
}

/// Write the boot set `saved` names into the partitions of `slot`, once the
/// store is found to hold it whole, and read each back; with `init_boot`,
/// that image goes into the init_boot partition in place of the one stored.
/// Gives what the partitions hold once written.
pub fn restore(store: &Store, device: &Path, saved: &Saved, dir: &Path, slot: Slot, init_boot: Option<&[u8]>) -> Result<Written> {
    check(store, device, saved)?;
    if let Some(image) = init_boot {
        ensure!(store.init_boot > 0, "the phone has no init_boot partition for an init_boot image");
        ensure!(image.len() as u64 <= store.init_boot, "the init_boot image takes {} bytes, more than the {} of the init_boot partition", image.len(), store.init_boot);
    }
    let mut from = open(device, false)?;
    let mut sums = Vec::new();
    for ((name, path, len, within), want) in parts(store, dir, slot).into_iter().zip([&saved.boot, &saved.init_boot]) {
        let mut out = open(&path, true)?;
        let written = match init_boot.filter(|_| name == INIT_BOOT) {
            Some(image) => {
                let mut padded = image.chain(io::repeat(0).take(len - image.len() as u64));
                copy(&mut padded, &mut out, len).with_context(|| format!("write {}", path.display()))?
            }
            None => {
                from.seek(SeekFrom::Start(store.offset(saved.entry) + within))?;
                copy(&mut from, &mut out, len).with_context(|| format!("write {}", path.display()))?;
                want.clone()
            }
        };
        out.sync_all().with_context(|| format!("sync {}", path.display()))?;
        ensure!(sum(&mut out, 0, len)? == written, "{} reads back different from what was written", path.display());
        sums.push(written);
    }
    Ok(Written { boot: sums[0].clone(), init_boot: sums.get(1).cloned().unwrap_or_default() })
}

/// Write a boot set made up of `boot`, `boot_len` bytes, and `init_boot`
/// into `entry` of the store, each followed by zeros to its partition's
/// size, and read the entry back.
pub fn put(store: &Store, device: &Path, entry: u32, boot: &mut dyn Read, boot_len: u64, init_boot: &[u8]) -> Result<Saved> {
    ensure!(boot_len <= store.boot, "the boot image takes {boot_len} bytes, more than the {} of the boot partition", store.boot);
    ensure!(init_boot.len() as u64 <= store.init_boot, "the init_boot image takes {} bytes, more than the {} of the init_boot partition", init_boot.len(), store.init_boot);
    let mut out = open(device, true)?;
    out.seek(SeekFrom::Start(store.offset(entry)))?;
    let boot_sum = copy(&mut boot.take(boot_len).chain(io::repeat(0).take(store.boot - boot_len)), &mut out, store.boot)?;
    let init_sum = copy(&mut init_boot.chain(io::repeat(0).take(store.init_boot - init_boot.len() as u64)), &mut out, store.init_boot)?;
    out.sync_all().with_context(|| format!("sync {}", device.display()))?;
    let saved = Saved { entry, boot: boot_sum, init_boot: if store.init_boot > 0 { init_sum } else { String::new() } };
    check(store, device, &saved)?;
    Ok(saved)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::state::tests::area;

    /// A store over a file the size it takes, with partitions of 8 and 16
    /// KiB, and the two partitions of both slots in `dir`, each filled with
    /// one byte.
    pub fn bench(dir: &Path) -> (Store, PathBuf) {
        let store = Store { area: area("/data/skyward/systems/store", Store::size_for(8192, 16384)), boot: 8192, init_boot: 16384 };
        let device = dir.join("store");
        std::fs::write(&device, vec![0u8; store.area.size as usize]).unwrap();
        for (name, len, fill) in [("boot_a", 8192, 0xa1), ("init_boot_a", 16384, 0xa2), ("boot_b", 8192, 0xb1), ("init_boot_b", 16384, 0xb2)] {
            std::fs::write(dir.join(name), vec![fill; len]).unwrap();
        }
        (store, device)
    }

    #[test]
    fn a_boot_set_saved_goes_back_whole_into_the_partitions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, device) = bench(dir.path());
        let saved = save(&store, &device, 1, dir.path(), Slot::A).unwrap();
        assert_eq!(saved.entry, 1);
        std::fs::write(dir.path().join("boot_a"), vec![0u8; 8192]).unwrap();
        std::fs::write(dir.path().join("init_boot_a"), vec![0u8; 16384]).unwrap();
        restore(&store, &device, &saved, dir.path(), Slot::A, None).unwrap();
        assert_eq!(std::fs::read(dir.path().join("boot_a")).unwrap(), vec![0xa1; 8192]);
        assert_eq!(std::fs::read(dir.path().join("init_boot_a")).unwrap(), vec![0xa2; 16384]);
        assert_eq!(std::fs::read(dir.path().join("boot_b")).unwrap(), vec![0xb1; 8192], "the other slot left alone");
        let bytes = std::fs::read(&device).unwrap();
        assert!(bytes[..HEADER as usize + 24576].iter().all(|&b| b == 0), "entry 0 and the header untouched");
    }

    #[test]
    fn a_damaged_entry_is_never_written_into_the_partitions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, device) = bench(dir.path());
        let saved = save(&store, &device, 0, dir.path(), Slot::B).unwrap();
        let mut bytes = std::fs::read(&device).unwrap();
        bytes[HEADER as usize + 100] ^= 1;
        std::fs::write(&device, bytes).unwrap();
        assert!(restore(&store, &device, &saved, dir.path(), Slot::A, None).is_err());
        assert!(restore(&store, &device, &saved, dir.path(), Slot::A, Some(&[3u8; 200])).is_err());
        assert_eq!(std::fs::read(dir.path().join("boot_a")).unwrap(), vec![0xa1; 8192], "the partitions stay as they were");
    }

    #[test]
    fn an_init_boot_image_given_goes_in_place_of_the_one_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (store, device) = bench(dir.path());
        let saved = put(&store, &device, OTHER, &mut &[7u8; 8192][..], 8192, &[9u8; 100]).unwrap();
        let stored = init_boot(&store, &device, &saved).unwrap();
        assert_eq!((&stored[..100], stored[100..].iter().all(|&b| b == 0), stored.len()), (&[9u8; 100][..], true, 16384), "the image stored, whole");
        restore(&store, &device, &saved, dir.path(), Slot::A, Some(&[3u8; 200])).unwrap();
        assert_eq!(std::fs::read(dir.path().join("boot_a")).unwrap(), vec![7u8; 8192], "the boot image as stored");
        let init = std::fs::read(dir.path().join("init_boot_a")).unwrap();
        assert_eq!((&init[..200], init[200..].iter().all(|&b| b == 0)), (&[3u8; 200][..], true), "padded to its partition");
        check(&store, &device, &saved).unwrap();
        assert!(restore(&store, &device, &saved, dir.path(), Slot::B, Some(&[3u8; 16385])).is_err());
        assert_eq!((std::fs::read(dir.path().join("boot_b")).unwrap(), std::fs::read(dir.path().join("init_boot_b")).unwrap()), (vec![0xb1; 8192], vec![0xb2; 16384]), "nothing written for an image too large");
    }

    #[test]
    fn an_init_boot_image_damaged_in_the_store_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let (store, device) = bench(dir.path());
        let saved = put(&store, &device, OTHER, &mut &[7u8; 8192][..], 8192, &[9u8; 100]).unwrap();
        let mut bytes = std::fs::read(&device).unwrap();
        bytes[(store.offset(OTHER) + store.boot + 50) as usize] ^= 1;
        std::fs::write(&device, bytes).unwrap();
        assert!(init_boot(&store, &device, &saved).is_err());
    }

    #[test]
    fn a_new_boot_set_is_padded_to_its_partitions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, device) = bench(dir.path());
        let saved = put(&store, &device, OTHER, &mut &[7u8; 5000][..], 5000, &[9u8; 100]).unwrap();
        restore(&store, &device, &saved, dir.path(), Slot::B, None).unwrap();
        let boot = std::fs::read(dir.path().join("boot_b")).unwrap();
        assert_eq!((&boot[..5000], &boot[5000..]), (&[7u8; 5000][..], &[0u8; 3192][..]));
        let init = std::fs::read(dir.path().join("init_boot_b")).unwrap();
        assert_eq!((&init[..100], init[100..].iter().all(|&b| b == 0)), (&[9u8; 100][..], true));
        assert!(put(&store, &device, OTHER, &mut &[7u8; 9000][..], 9000, &[]).is_err(), "larger than the boot partition");
    }

    #[test]
    fn the_installed_systems_saves_take_turns() {
        assert_eq!(Store::next_save(None), 0);
        assert_eq!(Store::next_save(Some(&Saved { entry: 0, boot: String::new(), init_boot: String::new() })), 1);
        assert_eq!(Store::next_save(Some(&Saved { entry: 1, boot: String::new(), init_boot: String::new() })), 0);
    }
}
