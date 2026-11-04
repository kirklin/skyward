//! The modules a root manager has installed, and what can be done with them:
//! each is a directory under the manager's module folder with a
//! `module.prop` beside it. A `disable` file in a module's directory keeps it
//! from loading, which is what turning one off writes.

use crate::{cmd, Manager};
use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Where managers keep their modules.
pub const MODULES: &str = "/data/adb/modules";

/// A module the phone has: what its own `module.prop` says of it, and whether
/// it loads and is on its way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// It loads: no `disable` file in its directory.
    pub enabled: bool,
    /// The manager takes it away when the phone next starts: a `remove` file
    /// is in its directory.
    pub removed: bool,
}

/// The modules under `dir`, by name.
pub fn list(dir: &Path) -> Result<Vec<Module>> {
    let mut found = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        let at = entry.path();
        if !at.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        // A directory with no `module.prop` is not a module the manager sees.
        let Some(prop) = read_prop(&at.join("module.prop"))? else { continue };
        let field = |name: &str| prop.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone()).unwrap_or_default();
        let name = field("name");
        found.push(Module {
            name: if name.is_empty() { id.clone() } else { name },
            id,
            version: field("version"),
            description: field("description"),
            enabled: !at.join("disable").exists(),
            removed: at.join("remove").exists(),
        });
    }
    found.sort_by_key(|module| module.name.to_lowercase());
    Ok(found)
}

/// The `key=value` lines of a `module.prop`, or none where there is no such
/// file.
fn read_prop(at: &Path) -> Result<Option<Vec<(String, String)>>> {
    let text = match fs::read_to_string(at) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", at.display())),
    };
    let mut fields = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    Ok(Some(fields))
}

/// Turn the module `id` on or off: a `disable` file in its directory is what
/// keeps one from loading, so it is written to turn one off and removed to
/// turn one back on.
pub fn set_enabled(dir: &Path, id: &str, on: bool) -> Result<()> {
    let at = module(dir, id)?;
    let flag = at.join("disable");
    if on {
        match fs::remove_file(&flag) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("remove {}", flag.display())),
        }
    } else {
        fs::write(&flag, b"").with_context(|| format!("write {}", flag.display()))
    }
}

/// Have the manager take the module `id` away when the phone next starts:
/// removing it now would leave what its scripts did behind, which the
/// manager's own removal step undoes first.
pub fn uninstall(dir: &Path, id: &str) -> Result<()> {
    let at = module(dir, id)?;
    let flag = at.join("remove");
    fs::write(&flag, b"").with_context(|| format!("write {}", flag.display()))
}

/// Take back a removal that was asked for: the module stays, and loads as it
/// did.
pub fn restore(dir: &Path, id: &str) -> Result<()> {
    let at = module(dir, id)?;
    let flag = at.join("remove");
    match fs::remove_file(&flag) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", flag.display())),
    }
}

/// The directory of the module `id`, which has to be one the page lists: a
/// name that would walk out of `dir` is not.
fn module(dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || id.contains('/') || id == "." || id == ".." {
        bail!("{id:?} is not a module's name");
    }
    let at = dir.join(id);
    if !at.is_dir() {
        bail!("{} is not a module", at.display());
    }
    Ok(at)
}

/// Install the module zip at `zip` with the manager that takes it, as the
/// manager's own installer would.
pub fn install(manager: &Manager, zip: &Path) -> Result<()> {
    let zip = zip.as_os_str();
    match manager {
        // KernelSU and its variants install a module through ksud.
        Manager::KernelSu(k) => {
            let args = [OsStr::new("module"), OsStr::new("install"), zip];
            cmd::run(&k.ksud, &args, Path::new("/"), &[]).context("install the module with ksud")?;
        }
        Manager::Magisk(m) => {
            let magisk = m.dir.join("magisk");
            let args = [OsStr::new("--install-module"), zip];
            cmd::run(&magisk, &args, Path::new("/"), &[]).context("install the module with Magisk")?;
        }
        // APatch installs a module through its own program, which this
        // recovery does not know how to run.
        Manager::Apatch(_) => bail!("APatch takes modules through its own app, which a recovery does not run"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_at(root: &Path, id: &str, prop: &str) {
        let at = root.join(id);
        fs::create_dir_all(&at).unwrap();
        fs::write(at.join("module.prop"), prop).unwrap();
    }

    #[test]
    fn a_module_is_read_from_its_own_prop() {
        let dir = tempfile::tempdir().unwrap();
        module_at(dir.path(), "tricky_store", "id=tricky_store\nname=Tricky Store\nversion=1.2\n");
        let found = list(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Tricky Store");
        assert_eq!(found[0].version, "1.2");
        assert!(found[0].enabled && !found[0].removed);
    }

    #[test]
    fn a_directory_with_no_prop_is_not_a_module() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("not_a_module")).unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn a_disable_file_is_what_turns_one_off() {
        let dir = tempfile::tempdir().unwrap();
        module_at(dir.path(), "one", "id=one\nname=One\n");
        set_enabled(dir.path(), "one", false).unwrap();
        assert!(!list(dir.path()).unwrap()[0].enabled);
        set_enabled(dir.path(), "one", true).unwrap();
        assert!(list(dir.path()).unwrap()[0].enabled);
    }

    #[test]
    fn a_module_marked_for_removal_can_be_kept() {
        let dir = tempfile::tempdir().unwrap();
        module_at(dir.path(), "one", "id=one\nname=One\n");
        uninstall(dir.path(), "one").unwrap();
        assert!(list(dir.path()).unwrap()[0].removed);
        restore(dir.path(), "one").unwrap();
        assert!(!list(dir.path()).unwrap()[0].removed, "kept, and it loads again");
        restore(dir.path(), "one").unwrap();
    }

    #[test]
    fn a_name_that_walks_out_of_the_module_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(module(dir.path(), "..").is_err());
        assert!(module(dir.path(), "one/../two").is_err());
    }
}
