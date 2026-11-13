//! Temporarily permit writes to selected block devices left read-only by
//! update_engine's verifier. Ordinary files never receive block ioctls.

use anyhow::{ensure, Context, Result};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

const BLKROSET: u32 = 0x125d;
const BLKROGET: u32 = 0x125e;

fn ioctl(file: &File, request: u32, flag: &mut libc::c_int) -> io::Result<()> {
    // libc::Ioctl is c_int on bionic, c_ulong on glibc.
    if unsafe { libc::ioctl(file.as_raw_fd(), request as libc::Ioctl, flag) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn read_only(file: &File) -> io::Result<bool> {
    let mut flag = 0;
    ioctl(file, BLKROGET, &mut flag)?;
    Ok(flag != 0)
}

pub(super) fn set_read_only(file: &File, on: bool) -> io::Result<()> {
    ioctl(file, BLKROSET, &mut libc::c_int::from(on))
}

pub(super) struct ReadOnly {
    path: PathBuf,
    held: Option<File>,
}

impl ReadOnly {
    pub fn clear(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {} read-only", path.display()))?;
        let block = file.metadata().with_context(|| format!("stat {}", path.display()))?.file_type().is_block_device();
        let mut guard = Self { path: path.to_path_buf(), held: None };
        if block && read_only(&file).with_context(|| format!("read the read-only flag of {}", path.display()))? {
            // Arm before changing anything: even a failed verification of
            // the change must put the original flag back.
            guard.held = Some(file);
            let file = guard.held.as_ref().unwrap();
            set_read_only(file, false).with_context(|| format!("clear the read-only flag of {}", path.display()))?;
            ensure!(!read_only(file)?, "{} stays read-only", path.display());
        }
        Ok(guard)
    }

    pub fn restore(&mut self) -> Result<()> {
        if let Some(file) = &self.held {
            set_read_only(file, true).with_context(|| format!("restore the read-only flag of {}", self.path.display()))?;
            ensure!(read_only(file)?, "{} did not become read-only again", self.path.display());
            // Keep the descriptor on failure so Drop can retry and log.
            self.held = None;
        }
        Ok(())
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            let _ = skyward_sys::kmsg(&format!("install: {e:#}"));
        }
    }
}
