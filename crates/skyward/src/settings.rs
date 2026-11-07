//! Where the settings the pages keep between starts live: a file of the
//! recovery's own on /metadata, which every phone here has, which needs no
//! screen lock to open, and which is there before the data partition opens,
//! in fastboot too. Skyward keeps /metadata read-only; it is writable only
//! while the file is written.

use anyhow::{Context, Result};
use skyward_block::mount;
use skyward_ui::settings::{Kept, Settings};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::Path;

const METADATA: &str = "/metadata";
const FOLDER: &str = "/metadata/skyward";
const FILE: &str = "/metadata/skyward/settings.json";
/// What a write goes into first, to be moved over the file whole.
const WRITING: &str = "/metadata/skyward/settings.json.new";

/// What the file holds. The recovery starts as well where it cannot be
/// read, as with /metadata damaged, so why goes to the kernel log.
pub fn read() -> Result<Kept> {
    match with_metadata(read_file) {
        Ok(kept) => Ok(kept),
        Err(e) => {
            skyward_sys::kmsg(&format!("the settings were not read: {e:#}"))?;
            Ok(Kept::Unreadable)
        }
    }
}

fn read_file() -> Result<Kept> {
    match fs::read_to_string(FILE) {
        Ok(text) => match Settings::read(&text) {
            Ok(settings) => Ok(Kept::Settings(settings)),
            Err(e) => {
                skyward_sys::kmsg(&format!("{FILE} does not read as settings, and is written afresh: {e:#}"))?;
                Ok(Kept::Nothing)
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Kept::Nothing),
        Err(e) => Err(e).with_context(|| format!("read {FILE}")),
    }
}

/// Write `settings` whole: into a file beside theirs, flushed, then moved
/// over it, with /metadata writable only meanwhile.
pub fn write(settings: &Settings) -> Result<()> {
    with_metadata(|| {
        skyward_vab::metadata_writable(true)?;
        let wrote = write_file(settings);
        let back = skyward_vab::metadata_writable(false);
        wrote?;
        back
    })
}

fn write_file(settings: &Settings) -> Result<()> {
    fs::create_dir_all(FOLDER).with_context(|| format!("make {FOLDER}"))?;
    let mut file = File::create(WRITING).with_context(|| format!("make {WRITING}"))?;
    file.write_all(settings.text().as_bytes()).with_context(|| format!("write {WRITING}"))?;
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
