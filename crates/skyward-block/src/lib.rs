use anyhow::{Context, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub mod area;
pub mod dm;
pub mod erase;
pub mod fsck;
pub mod fstab;
pub mod loopdev;
pub mod mount;
pub mod probe;
pub mod tools;
pub mod usb;

pub const BY_NAME: &str = "/dev/block/by-name";

/// A bootloader parameter such as `androidboot.slot_suffix`, from bootconfig
/// or, on kernels without it, the command line. Root tools can rewrite the
/// matching `ro.boot.*` properties; these files come from the kernel.
pub fn boot_param(key: &str) -> Result<Option<String>> {
    for src in ["/proc/bootconfig", "/proc/cmdline"] {
        match fs::read_to_string(src) {
            Ok(s) => {
                if let Some(v) = find_kv(&s, key) {
                    return Ok(Some(v));
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("read {src}")),
        }
    }
    Ok(None)
}

/// Active slot suffix (`_a` / `_b`), or `None` on devices that are not A/B.
pub fn slot_suffix() -> Result<Option<String>> {
    boot_param("androidboot.slot_suffix")
}

fn find_kv(haystack: &str, key: &str) -> Option<String> {
    // Kernel cmdline form: whitespace-separated `key=value` tokens.
    for tok in haystack.split_whitespace() {
        if let Some(val) = tok.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            let val = val.trim_matches('"').trim_end_matches(',');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    // bootconfig form: one `key = "value"` per line.
    for line in haystack.lines() {
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            let val = v.trim().trim_matches('"').trim_end_matches(',').trim_matches('"');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct Partition {
    pub name: String,
    pub path: PathBuf,
}

/// Enumerate the `by-name` partition links, resolving each to its block device.
pub fn list_by_name(dir: impl AsRef<Path>) -> Result<Vec<Partition>> {
    let dir = dir.as_ref();
    let mut out = Vec::new();
    for ent in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().into_owned();
        let path = fs::canonicalize(ent.path()).unwrap_or_else(|_| ent.path());
        out.push(Partition { name, path });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(target_os = "android")]
pub fn size_bytes(path: impl AsRef<Path>) -> Result<u64> {
    use std::os::unix::io::AsRawFd;
    // _IOR(0x12, 114, size_t) on aarch64 Linux. bionic's ioctl request arg is
    // a c_int, so the request is reinterpreted into it rather than widened.
    const BLKGETSIZE64: u32 = 0x8008_1272;
    let f = fs::File::open(path.as_ref())
        .with_context(|| format!("open {}", path.as_ref().display()))?;
    let mut size: u64 = 0;
    let rc = unsafe {
        libc::ioctl(f.as_raw_fd(), BLKGETSIZE64 as libc::c_int, &mut size as *mut u64)
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("BLKGETSIZE64");
    }
    Ok(size)
}

#[cfg(not(target_os = "android"))]
pub fn size_bytes(_path: impl AsRef<Path>) -> Result<u64> {
    anyhow::bail!("size_bytes is only implemented on android")
}

/// A block device's (major, minor).
#[cfg(target_os = "android")]
pub fn devnum(path: impl AsRef<Path>) -> Result<(u32, u32)> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let p = path.as_ref();
    let meta = fs::metadata(p).with_context(|| format!("stat {}", p.display()))?;
    anyhow::ensure!(meta.file_type().is_block_device(), "{} is not a block device", p.display());
    let rdev = meta.rdev() as libc::dev_t;
    Ok((libc::major(rdev) as u32, libc::minor(rdev) as u32))
}

#[cfg(not(target_os = "android"))]
pub fn devnum(path: impl AsRef<Path>) -> Result<(u32, u32)> {
    anyhow::bail!("this host has no block device {}", path.as_ref().display())
}

/// Drop the device's cached pages, so reading it back reads the flash.
#[cfg(target_os = "android")]
pub fn drop_cache(f: &fs::File) -> Result<()> {
    use std::os::fd::AsRawFd;
    let rc = unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    if rc != 0 {
        return Err(std::io::Error::from_raw_os_error(rc).into());
    }
    Ok(())
}

/// Hosts run the tests on plain files, whose reads come from the same
/// cache either way.
#[cfg(not(target_os = "android"))]
pub fn drop_cache(_: &fs::File) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slot_from_cmdline() {
        let line = "console=ttyMSM0 androidboot.slot_suffix=_b androidboot.hardware=qcom";
        assert_eq!(find_kv(line, "androidboot.slot_suffix").as_deref(), Some("_b"));
    }

    #[test]
    fn parses_slot_from_bootconfig() {
        let line = "androidboot.slot_suffix = \"_a\"\nandroidboot.force_normal_boot = \"1\"";
        assert_eq!(find_kv(line, "androidboot.slot_suffix").as_deref(), Some("_a"));
    }
}
