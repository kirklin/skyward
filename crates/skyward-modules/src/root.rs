//! Recovery-side management of directory-based KernelSU and Magisk modules.

use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const PROP_LIMIT: u64 = 64 * 1024;

/// A module and the status its manager will use the next time Android starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub disabled: bool,
    pub remove: bool,
    pub editable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Enable,
    Disable,
    Remove,
    UndoRemove,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id != "." && id != ".." && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Directory descriptors pin the module tree across all marker operations.
/// Neither module directories, property files nor markers follow symlinks.
struct Dir(File);

impl Dir {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::options()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self(file))
    }

    fn file(&self, name: &str, flags: i32) -> std::io::Result<File> {
        let name = CString::new(name).map_err(std::io::Error::other)?;
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn child(&self, name: &str) -> std::io::Result<Self> {
        self.file(name, libc::O_RDONLY | libc::O_DIRECTORY).map(Self)
    }

    fn flag(&self, name: &str) -> Result<bool> {
        match self.file(name, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(file) => {
                ensure!(file.metadata()?.is_file(), "{name} is not a regular marker file");
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("read module marker {name}")),
        }
    }

    fn mark(&self, name: &str, present: bool) -> Result<()> {
        if self.flag(name)? == present {
            return Ok(());
        }
        if present {
            let file = self.file(name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
            file.sync_all()?;
        } else {
            let name = CString::new(name)?;
            ensure!(
                unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) } == 0,
                "remove marker: {}",
                std::io::Error::last_os_error()
            );
        }
        self.0.sync_all()?;
        Ok(())
    }

    fn properties(&self) -> Result<BTreeMap<String, String>> {
        let file = self.file("module.prop", libc::O_RDONLY | libc::O_NONBLOCK)?;
        ensure!(file.metadata()?.is_file(), "module.prop is not a regular file");
        let mut text = String::new();
        file.take(PROP_LIMIT + 1).read_to_string(&mut text)?;
        ensure!(text.len() as u64 <= PROP_LIMIT, "module.prop is too large");
        Ok(text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with('#') {
                    return None;
                }
                let (name, value) = line.split_once('=')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect())
    }

    fn names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", self.0.as_raw_fd()))? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if valid_id(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}

fn optional_child(dir: &Dir, name: &str) -> Result<Option<Dir>> {
    match dir.child(name) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("open {name} without following links")),
    }
}

/// A module whose early-boot files are cached may still run after a marker
/// changes. Recovery leaves those caches and the modules they affect alone.
fn has_boot_cache(metadata: &Path) -> Result<bool> {
    for root in ["vold/ksu", "watchdog/ksu", "ksu"] {
        let dir = match Dir::open(&metadata.join(root)) {
            Ok(dir) => dir,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e).context("open KernelSU boot cache"),
        };
        match dir.file("modules.rc", libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(file) => {
                let data = file.metadata()?;
                ensure!(data.is_file(), "KernelSU modules.rc is not a regular file");
                // The manager can leave an empty or whitespace-only file.
                // A larger file is treated as an active cache without reading it all.
                let mut contents = Vec::new();
                file.take(PROP_LIMIT + 1).read_to_end(&mut contents)?;
                if contents.len() as u64 > PROP_LIMIT || contents.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("read KernelSU modules.rc"),
        }
        if let Some(preinit) = optional_child(&dir, "preinit")? && !preinit.names()?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn editable(adb: &Path, root: &Dir, dir: &Dir, id: &str, has_cache: bool) -> Result<bool> {
    if has_cache || ["ksu/modules.img", "ksu/modules_update.img", "modules.img"].iter().any(|p| adb.join(p).exists()) {
        return Ok(false);
    }
    let pending = match optional_child(root, "modules_update")? {
        Some(staged) => optional_child(&staged, id)?.is_some(),
        None => false,
    };
    Ok(!pending && !dir.flag("update")? && optional_child(dir, "initrc")?.is_none() && optional_child(dir, "preinit")?.is_none())
}

fn read_module(adb: &Path, root: &Dir, dir: &Dir, id: &str, cached: bool) -> Result<Module> {
    let props = dir.properties()?;
    ensure!(props.get("id").is_some_and(|value| value == id), "module.prop id differs from directory {id}");
    Ok(Module {
        id: id.to_owned(),
        name: props.get("name").filter(|name| !name.is_empty()).map(|name| name.chars().take(128).collect()).unwrap_or_else(|| id.to_owned()),
        disabled: dir.flag("disable")?,
        remove: dir.flag("remove")?,
        editable: editable(adb, root, dir, id, cached)?,
    })
}

/// List the active module directories under a mounted data partition.
pub fn list(adb: &Path, metadata: &Path) -> Result<Vec<Module>> {
    let root = match Dir::open(adb) {
        Ok(dir) => dir,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("open the root managers' directory"),
    };
    let Some(modules) = optional_child(&root, "modules")? else { return Ok(Vec::new()) };
    let cached = has_boot_cache(metadata)?;
    let mut ids = modules.names()?;
    ids.sort_unstable();
    let mut catalog = Vec::new();
    for id in ids {
        // A damaged entry should not prevent the user from managing other modules.
        let entry = modules.child(&id).map_err(anyhow::Error::from).and_then(|dir| read_module(adb, &root, &dir, &id, cached));
        if let Ok(module) = entry {
            catalog.push(module);
        }
    }
    Ok(catalog)
}

/// Change one manager marker, without running any installed module code.
pub fn change(adb: &Path, metadata: &Path, id: &str, choice: Change) -> Result<()> {
    ensure!(valid_id(id), "invalid module id");
    let root = Dir::open(adb)?;
    let dir = root.child("modules")?.child(id)?;
    let module = read_module(adb, &root, &dir, id, has_boot_cache(metadata)?)?;
    ensure!(module.editable, "module {id} has an update or early-boot state managed by Android");
    ensure!(choice != Change::Enable || !module.remove, "cancel the pending removal before enabling {id}");
    let (marker, present) = match choice {
        Change::Enable => ("disable", false),
        Change::Disable => ("disable", true),
        Change::Remove => ("remove", true),
        Change::UndoRemove => ("remove", false),
    };
    dir.mark(marker, present).with_context(|| format!("change {marker} for {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let adb = temp.path().join("adb");
        let module = adb.join("modules/demo");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=demo\nname=Demo module\n").unwrap();
        let metadata = temp.path().join("metadata");
        (temp, adb, metadata)
    }

    #[test]
    fn markers_are_reversible_and_do_not_remove_module_files() {
        let (_temp, adb, metadata) = fixture();
        let modules = adb.join("modules/demo");
        assert_eq!(list(&adb, &metadata).unwrap()[0].name, "Demo module");
        change(&adb, &metadata, "demo", Change::Disable).unwrap();
        change(&adb, &metadata, "demo", Change::Disable).unwrap();
        assert!(list(&adb, &metadata).unwrap()[0].disabled);
        change(&adb, &metadata, "demo", Change::Remove).unwrap();
        assert!(list(&adb, &metadata).unwrap()[0].remove);
        assert!(change(&adb, &metadata, "demo", Change::Enable).is_err());
        change(&adb, &metadata, "demo", Change::UndoRemove).unwrap();
        change(&adb, &metadata, "demo", Change::Enable).unwrap();
        assert!(!modules.join("disable").exists() && !modules.join("remove").exists());
        assert!(modules.join("module.prop").is_file());
    }

    #[test]
    fn a_replaced_or_cached_module_is_listed_but_not_written() {
        let (_temp, adb, metadata) = fixture();
        let pending = adb.join("modules_update/demo");
        fs::create_dir_all(&pending).unwrap();
        assert!(!list(&adb, &metadata).unwrap()[0].editable);
        assert!(change(&adb, &metadata, "demo", Change::Disable).is_err());
        fs::remove_dir(pending).unwrap();
        let cache = metadata.join("vold/ksu");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("modules.rc"), "service demo /example").unwrap();
        assert!(!list(&adb, &metadata).unwrap()[0].editable);
        assert!(change(&adb, &metadata, "demo", Change::Remove).is_err());
        assert!(!adb.join("modules/demo/remove").exists());
    }

    #[test]
    fn module_directories_and_marker_files_do_not_follow_symlinks() {
        let (temp, adb, metadata) = fixture();
        let target = temp.path().join("outside");
        fs::write(&target, "keep").unwrap();
        symlink(&target, adb.join("modules/demo/disable")).unwrap();
        symlink(adb.join("modules/demo"), adb.join("modules/linked")).unwrap();
        assert!(change(&adb, &metadata, "linked", Change::Disable).is_err());
        assert!(change(&adb, &metadata, "demo", Change::Enable).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
        assert!(change(&adb, &metadata, "../outside", Change::Disable).is_err());
    }
}
