//! Patching a phone's boot or init_boot so a root manager starts with the
//! system, as each manager's own patcher does it: the manager's tools are run
//! over a copy of the stock image, and the image that comes back is what the
//! partition gets.

pub mod managers;
// The managers keep their own names at the crate's root, as they had them
// before they were gathered into one file.
pub use managers::{apatch, apk, cmd, detect, installed, ksu, magisk};
pub mod modules;
pub mod partitions;
pub mod releases;
pub mod rooted;

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub use apatch::Apatch;
pub use detect::{detect, Manager};
pub use ksu::KernelSu;
pub use magisk::Magisk;

/// Where a phone keeps its partitions by name.
pub const BY_NAME: &str = "/dev/block/by-name";

/// What a patch keeps of the phone as it is, as the manager's own patcher
/// takes it: the filesystem checks of the system it boots, and whether the
/// vbmeta the patched image would need signed again is left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Keep the phone's dm-verity checks on the system.
    pub keep_verity: bool,
    /// Keep the phone's forced encryption.
    pub keep_forceencrypt: bool,
    /// Patch the flags of the vbmeta the patched image carries, as a phone
    /// whose bootloader verifies it needs.
    pub patch_vbmeta: bool,
}

impl Default for Options {
    fn default() -> Options {
        // What an update may change: a patched image is not the phone's own,
        // so the checks that would fail on it go, and the phone keeps
        // encrypting as it did.
        Options { keep_verity: false, keep_forceencrypt: true, patch_vbmeta: false }
    }
}

/// The manager the phone has, with what it patches with copied into `into`:
/// an install lets go of the data partition, so the tools it keeps there are
/// taken out first and the patch runs from the copies.
pub fn stage(into: &Path) -> anyhow::Result<Manager> {
    use anyhow::Context;
    let manager = detect().context("no root manager on the phone")?;
    std::fs::create_dir_all(into).with_context(|| format!("create {}", into.display()))?;
    Ok(match &manager {
        Manager::Magisk(m) => {
            copy_tree(&m.dir, into)?;
            Manager::Magisk(magisk::Magisk { dir: into.to_path_buf(), version: m.version.clone() })
        }
        Manager::KernelSu(k) => {
            let name = k.ksud.file_name().context("ksud has no name")?;
            let at = into.join(name);
            std::fs::copy(&k.ksud, &at).with_context(|| format!("copy {}", k.ksud.display()))?;
            Manager::KernelSu(ksu::KernelSu { ksud: at, version: k.version.clone() })
        }
        Manager::Apatch(a) => {
            copy_tree(&a.dir, &into.join("ap"))?;
            let apd = detect::ADB; let apd = Path::new(apd).join("apd");
            if apd.is_file() {
                std::fs::copy(&apd, into.join("apd")).with_context(|| format!("copy {}", apd.display()))?;
            }
            Manager::Apatch(apatch::Apatch { dir: into.join("ap"), version: a.version.clone() })
        }
    })
}

/// What stands in `from`, copied into `into`.
fn copy_tree(from: &Path, into: &Path) -> anyhow::Result<()> {
    use anyhow::Context;
    std::fs::create_dir_all(into).with_context(|| format!("create {}", into.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("read {}", from.display()))?.flatten() {
        let at = entry.path();
        let to = into.join(entry.file_name());
        if at.is_dir() {
            copy_tree(&at, &to)?;
        } else {
            std::fs::copy(&at, &to).with_context(|| format!("copy {}", at.display()))?;
        }
    }
    Ok(())
}


/// The partition a root manager patches, of the slot `slot` runs from: the
/// generic ramdisk lives in `init_boot` where the phone has one, and in
/// `boot` where it does not.
pub fn target(dir: &Path, slot: skyward_vab::Slot) -> &'static str {
    let name = |partition: &str| dir.join(format!("{partition}{}", slot.suffix()));
    if name("init_boot").exists() {
        "init_boot"
    } else {
        "boot"
    }
}

/// The whole of the partition `partition` of `slot`, read into `into`, which
/// is also given back.
pub fn read(dir: &Path, partition: &str, slot: skyward_vab::Slot, into: &Path) -> Result<PathBuf> {
    let device = dir.join(format!("{partition}{}", slot.suffix()));
    let mut from = File::open(&device).with_context(|| format!("open {}", device.display()))?;
    let mut to = File::create(into).with_context(|| format!("create {}", into.display()))?;
    std::io::copy(&mut from, &mut to).with_context(|| format!("read {}", device.display()))?;
    Ok(into.to_path_buf())
}

/// The block layer's call that says whether a device is read-only.
const BLKROSET: u64 = 0x125d;

/// Write `image` over the partition `partition` of `slot`, then read the
/// partition back and compare it with what was written.
pub fn write(image: &Path, dir: &Path, partition: &str, slot: skyward_vab::Slot) -> Result<()> {
    let device = dir.join(format!("{partition}{}", slot.suffix()));
    let mut from = File::open(image).with_context(|| format!("open {}", image.display()))?;
    let written = from.metadata().with_context(|| format!("look at {}", image.display()))?.len();
    let mut to = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&device)
        .with_context(|| format!("open {}", device.display()))?;
    // A partition an install wrote over, or mapped for one, comes back
    // read-only: the flag is cleared before the image goes over it, as the
    // block layer asks, and the write below says what came of it.
    {
        use std::os::fd::AsRawFd;
        let mut read_only: libc::c_int = 0;
        let at = &mut read_only as *mut libc::c_int as *mut libc::c_void;
        if unsafe { libc::ioctl(to.as_raw_fd(), BLKROSET as _, at) } != 0 {
            let e = std::io::Error::last_os_error();
            // A file that is not a block device has no such flag to clear.
            if !matches!(e.raw_os_error(), Some(libc::ENOTTY) | Some(libc::EINVAL)) {
                return Err(e).with_context(|| format!("clear the read-only flag of {}", device.display()));
            }
        }
    }
    std::io::copy(&mut from, &mut to).with_context(|| format!("write {}", device.display()))?;
    to.sync_all().with_context(|| format!("flush {}", device.display()))?;
    let mut back = Vec::with_capacity(written as usize);
    File::open(&device)
        .with_context(|| format!("open {} to read it back", device.display()))?
        .read_to_end(&mut back)
        .with_context(|| format!("read {} back", device.display()))?;
    if back.len() < written as usize {
        bail!("{} read back {} bytes of {written}", device.display(), back.len());
    }
    let mut want = vec![0u8; written as usize];
    File::open(image)?.read_exact(&mut want)?;
    if back[..want.len()] != want[..] {
        bail!("{} does not hold what was written to it", device.display());
    }
    Ok(())
}
