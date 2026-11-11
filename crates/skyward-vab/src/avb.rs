//! The Android Verified Boot blocks a slot keeps, and the flags in them that
//! say whether the bootloader and the kernel check what they read.
//!
//! This only reads them. A flag changed under a signature no longer matches
//! it, and a page that shows the flags is about knowing, not about turning
//! the checks off, so nothing here writes.

use crate::Slot;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Where the phone names its partitions.
pub const BY_NAME: &str = "/dev/block/by-name";

/// The flags, as AVB 2.0 numbers them.
pub const HASHTREE_DISABLED: u32 = 1;
pub const VERIFICATION_DISABLED: u32 = 2;

/// A vbmeta image's header: 256 bytes starting with its magic, its
/// algorithm a big-endian 32-bit word 28 bytes in, its flags another one
/// 120 bytes in.
const MAGIC: &[u8; 4] = b"AVB0";
const ALGORITHM_AT: usize = 28;
const FLAGS_AT: usize = 120;
const HEADER: usize = 256;

/// What the header's algorithm field says.
pub fn algorithm_name(algorithm: u32) -> &'static str {
    match algorithm {
        0 => "none",
        1 => "sha256_rsa2048",
        2 => "sha256_rsa4096",
        3 => "sha256_rsa8192",
        _ => "other",
    }
}

/// What one vbmeta block says about the checks it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub name: String,
    pub flags: u32,
    pub algorithm: u32,
}

impl Reading {
    /// Whether the kernel checks the filesystems this block describes.
    pub fn verity(&self) -> bool {
        self.flags & HASHTREE_DISABLED == 0
    }

    /// Whether the bootloader checks what it starts.
    pub fn verification(&self) -> bool {
        self.flags & VERIFICATION_DISABLED == 0
    }

    /// Whether both checks are on, as the phone came from its maker.
    pub fn whole(&self) -> bool {
        self.verity() && self.verification()
    }
}

/// The vbmeta partitions this phone keeps for `slot`: its own, and those a
/// chain in it points at. A phone that keeps none of the others simply has
/// fewer entries, and a phone that is not an A/B phone names its blocks
/// without a slot suffix, which is what is read then.
pub fn partitions(slot: Slot) -> Vec<PathBuf> {
    partitions_in(Path::new(BY_NAME), slot)
}

/// [`partitions`], under `base`, where a test keeps stand-in partitions.
fn partitions_in(base: &Path, slot: Slot) -> Vec<PathBuf> {
    const NAMES: [&str; 3] = ["vbmeta", "vbmeta_system", "vbmeta_vendor"];
    let suffixed: Vec<PathBuf> = NAMES
        .iter()
        .map(|name| base.join(format!("{name}{}", slot.suffix())))
        .filter(|path| path.exists())
        .collect();
    if !suffixed.is_empty() {
        return suffixed;
    }
    NAMES.iter().map(|name| base.join(name)).filter(|path| path.exists()).collect()
}

/// Read what every vbmeta block of `slot` says about the checks it names.
pub fn read(slot: Slot) -> Result<Vec<Reading>> {
    read_in(Path::new(BY_NAME), slot)
}

/// [`read`], out of `base`, where a test keeps stand-in partitions.
fn read_in(base: &Path, slot: Slot) -> Result<Vec<Reading>> {
    let paths = partitions_in(base, slot);
    if paths.is_empty() {
        bail!("no vbmeta partition for slot {}", slot.letter());
    }
    paths.iter().map(|path| read_one(path)).collect()
}

/// One block's header, read whole so that a short or foreign block is told
/// apart before anything is taken from it.
fn read_one(path: &Path) -> Result<Reading> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < HEADER {
        bail!("{} holds {} bytes, too few for a vbmeta header", path.display(), bytes.len());
    }
    if &bytes[..4] != MAGIC {
        bail!("{} does not begin a vbmeta block: {:02x?}", path.display(), &bytes[..4]);
    }
    let word = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("vbmeta").to_string();
    Ok(Reading { name, flags: word(FLAGS_AT), algorithm: word(ALGORITHM_AT) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in vbmeta block of `len` bytes with the flags and algorithm
    /// asked for.
    fn block(flags: u32, algorithm: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[ALGORITHM_AT..ALGORITHM_AT + 4].copy_from_slice(&algorithm.to_be_bytes());
        bytes[FLAGS_AT..FLAGS_AT + 4].copy_from_slice(&flags.to_be_bytes());
        bytes
    }

    fn dir(tag: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-avb-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&at);
        fs::create_dir_all(&at).unwrap();
        at
    }

    #[test]
    fn every_flag_says_which_check_is_off() {
        let whole = Reading { name: "vbmeta_a".into(), flags: 0, algorithm: 2 };
        assert!(whole.verity() && whole.verification() && whole.whole(), "both checks on");
        let no_verity = Reading { flags: HASHTREE_DISABLED, ..whole.clone() };
        assert!(!no_verity.verity() && no_verity.verification(), "only the kernel's check is off");
        assert!(!no_verity.whole(), "and it is not whole");
        let none = Reading { flags: HASHTREE_DISABLED | VERIFICATION_DISABLED, ..whole };
        assert!(!none.verity() && !none.verification(), "both checks off");
    }

    #[test]
    fn the_blocks_the_slot_keeps_are_read_in_its_name() {
        let base = dir("read");
        fs::write(base.join("vbmeta_a"), block(0, 1)).unwrap();
        fs::write(base.join("vbmeta_system_a"), block(VERIFICATION_DISABLED, 2)).unwrap();
        fs::write(base.join("vbmeta_vendor_b"), block(0, 3)).unwrap();
        let a = read_in(&base, Slot::A).unwrap();
        assert_eq!(a, vec![
            Reading { name: "vbmeta_a".into(), flags: 0, algorithm: 1 },
            Reading { name: "vbmeta_system_a".into(), flags: VERIFICATION_DISABLED, algorithm: 2 },
        ], "the slot's own blocks, and none of the other slot's");
        assert!(!a[1].verification(), "the second block turned verification off");
        let b = read_in(&base, Slot::B).unwrap();
        assert_eq!(b.len(), 1, "the other slot holds what it holds");
        assert_eq!(b[0].name, "vbmeta_vendor_b");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_phone_that_names_no_slot_reads_its_blocks_without_a_suffix() {
        let base = dir("plain");
        fs::write(base.join("vbmeta"), block(0, 1)).unwrap();
        fs::write(base.join("vbmeta_system"), block(0, 2)).unwrap();
        let rows = read_in(&base, Slot::A).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["vbmeta", "vbmeta_system"], "the blocks as the phone names them");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_slot_that_keeps_no_block_says_so_and_a_foreign_block_is_refused() {
        let base = dir("none");
        let why = read_in(&base, Slot::A).err().unwrap().to_string();
        assert!(why.contains("no vbmeta partition"), "said plainly: {why}");
        fs::write(base.join("vbmeta_a"), vec![0u8; 64]).unwrap();
        let why = read_in(&base, Slot::A).err().unwrap().to_string();
        assert!(why.contains("too few"), "a short block is told apart: {why}");
        fs::write(base.join("vbmeta_a"), vec![b'X'; HEADER]).unwrap();
        let why = read_in(&base, Slot::A).err().unwrap().to_string();
        assert!(why.contains("does not begin a vbmeta block"), "as is one that is not AVB: {why}");
        let _ = fs::remove_dir_all(&base);
    }
}
