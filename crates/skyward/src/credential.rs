//! Where the credential that opened the data partition is kept, sealed: a
//! file of the recovery's own on /metadata, beside the settings, written only
//! where the Settings page's switch is on, and taken away as soon as it is
//! turned off.

use anyhow::{Context, Result};
use skyward_block::mount;
use skyward_crypto::credential::{open, seal};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::Path;

const METADATA: &str = "/metadata";
const FOLDER: &str = "/metadata/skyward";
const FILE: &str = "/metadata/skyward/credential";
/// What a write goes into first, to be moved over the file whole.
const WRITING: &str = "/metadata/skyward/credential.new";

/// This phone's serial number, which the key is derived from: a file copied
/// to another phone does not open there.
fn serial() -> Result<String> {
    skyward_sys::prop("ro.serialno")
        .or_else(|| skyward_sys::prop("ro.boot.serialno"))
        .filter(|s| !s.is_empty())
        .context("this phone's serial number is not known, so a credential cannot be sealed to it")
}

/// Whether a credential is kept. A file that is not there is no answer, not a
/// failure.
pub fn present() -> bool {
    Path::new(FILE).exists()
}

/// When what was kept was sealed, in whole seconds since 1970, or none when
/// nothing is kept. The file's own time is the seal's: it is written anew
/// each time a password replaces the one before.
pub fn saved_at() -> Option<i64> {
    let modified = fs::metadata(FILE).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since.as_secs() as i64)
}

/// Keep `secret`, sealed for this phone. Replaces what was kept before.
pub fn save(secret: &[u8]) -> Result<()> {
    let serial = serial()?;
    let sealed = seal(&serial, secret)?;
    with_metadata(|| {
        skyward_vab::metadata_writable(true)?;
        let wrote = write_file(&sealed);
        let back = skyward_vab::metadata_writable(false);
        wrote?;
        back
    })
}

/// What was kept, for the phone that kept it. A file that cannot be opened
/// says why rather than opening as something else.
pub fn load() -> Result<Vec<u8>> {
    let serial = serial()?;
    let file = with_metadata(|| fs::read(FILE).with_context(|| format!("read {FILE}")))?;
    open(&serial, &file)
}

/// Take the file away, as the switch being turned off asks. Says whether
/// there was one.
pub fn forget() -> Result<bool> {
    with_metadata(|| match fs::remove_file(FILE) {
        Ok(()) => {
            File::open(FOLDER).and_then(|folder| folder.sync_all()).with_context(|| format!("write {FOLDER}"))?;
            Ok(true)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("remove {FILE}")),
    })
}

fn write_file(sealed: &[u8]) -> Result<()> {
    fs::create_dir_all(FOLDER).with_context(|| format!("make {FOLDER}"))?;
    let mut file = File::create(WRITING).with_context(|| format!("make {WRITING}"))?;
    file.write_all(sealed).with_context(|| format!("write {WRITING}"))?;
    file.sync_all().with_context(|| format!("write {WRITING}"))?;
    fs::rename(WRITING, FILE).with_context(|| format!("move {WRITING} over {FILE}"))?;
    File::open(FOLDER).and_then(|folder| folder.sync_all()).with_context(|| format!("write {FOLDER}"))
}

/// Do `work` on /metadata, mounted read-only for the moment where it is not
/// mounted, as in fastboot, and taken away again after.
fn with_metadata<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    let at = Path::new(METADATA);
    if mount::is_mount_point(at)? {
        return work();
    }
    mount::mount_ro(&Path::new(skyward_block::BY_NAME).join("metadata"), at)?;
    let done = work();
    mount::umount(at)?;
    done
}
