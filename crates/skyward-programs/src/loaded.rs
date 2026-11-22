//! The libraries a program asks its loader for by name as it starts, and
//! where the loader finds each when the program may name it no folders.
//! The kernel has the loader take none from whoever starts a program that
//! starts with more than they have: one whose file carries capabilities of
//! its own, run as another account than root.

use anyhow::{Context, Result};
use object::elf::{DT_NEEDED, DT_RUNPATH};
use object::read::elf::ElfFile64;
use object::Endianness;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// The folders Android's loader looks in when nothing names it others, in
/// the order it looks in them.
pub const OWN_FOLDERS: [&str; 3] = ["/system/lib64", "/odm/lib64", "/vendor/lib64"];

/// The name the loader answers to itself, whatever files there are.
const LOADER: &str = "ld-android.so";

/// What a file may write for the folder it is in and for the name of the
/// folders of libraries, in the folders it has the loader look in.
const ORIGIN: &str = "ORIGIN";
const LIBRARIES: (&str, &str) = ("LIB", "lib64");

/// What a file asks of the loader: the libraries it names, and the folders
/// it has the loader look in for them before the loader's own.
#[derive(Debug, Default, PartialEq, Eq)]
struct Asks {
    libraries: Vec<String>,
    folders: Vec<String>,
}

/// What the program or the library at `file` asks of the loader.
fn asks(file: &Path) -> Result<Asks> {
    let data = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let read = || -> object::Result<Asks> {
        let table = ElfFile64::<Endianness>::parse(data.as_slice())?.elf_dynamic_table()?;
        let mut asks = Asks::default();
        for entry in table.iter() {
            if entry.tag == DT_NEEDED {
                asks.libraries.push(String::from_utf8_lossy(table.string(entry)?).into_owned());
            } else if entry.tag == DT_RUNPATH {
                let folders = String::from_utf8_lossy(table.string(entry)?).into_owned();
                asks.folders.extend(folders.split(':').filter(|folder| !folder.is_empty()).map(str::to_string));
            }
        }
        Ok(asks)
    };
    read().with_context(|| format!("read what {} asks its loader for", file.display()))
}

/// `folder` as a file in `origin` names it: with the folder the file is in
/// and the name of the folders of libraries written out.
fn named_from(folder: &str, origin: &Path) -> PathBuf {
    let mut folder = folder.to_string();
    for (name, value) in [(ORIGIN, origin.to_string_lossy().as_ref()), LIBRARIES] {
        folder = folder.replace(&format!("${{{name}}}"), value).replace(&format!("${name}"), value);
    }
    PathBuf::from(folder)
}

/// Where the libraries a program asks for are, of those the loader finds
/// in none of the folders it looks in.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Beyond {
    /// Those another folder has, each as the file in the first that has
    /// it.
    pub elsewhere: Vec<PathBuf>,
    /// The names of those no folder has.
    pub nowhere: Vec<String>,
}

/// The libraries `program` asks for, by itself and by each library it
/// loads, that a loader looking in the `loader` folders finds in none of
/// them, as the first of the `others` folders that has each gives it. The
/// loader looks first in the folders the file that asks names; a library
/// found among `others` asks as it will from `placed`, the one of the
/// loader's folders it is to be put in.
pub fn beyond(program: &Path, loader: &[PathBuf], others: &[PathBuf], placed: &Path) -> Result<Beyond> {
    let mut out = Beyond::default();
    let mut asked: HashSet<String> = HashSet::from([LOADER.to_string()]);
    let origin = program.parent().with_context(|| format!("{} is in no folder", program.display()))?;
    let mut loading = VecDeque::from([(program.to_path_buf(), origin.to_path_buf())]);
    let has = |folder: PathBuf, name: &str| Some(folder.join(name)).filter(|file| file.is_file());
    while let Some((file, origin)) = loading.pop_front() {
        let asks = asks(&file)?;
        for name in asks.libraries {
            if !asked.insert(name.clone()) {
                continue;
            }
            let named = asks.folders.iter().map(|folder| named_from(folder, &origin));
            if let Some(found) = named.chain(loader.iter().cloned()).find_map(|folder| has(folder, &name)) {
                let folder = found.parent().expect("a file in a folder").to_path_buf();
                loading.push_back((found, folder));
            } else if let Some(found) = others.iter().cloned().find_map(|folder| has(folder, &name)) {
                out.elsewhere.push(found.clone());
                loading.push_back((found, placed.to_path_buf()));
            } else {
                out.nowhere.push(name);
            }
        }
    }
    Ok(out)
}
