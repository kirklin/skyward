//! What a phone's own partitions say of the programs they carry, taken out
//! of the stock build's full update package: the init scripts that say how
//! each is started, the descriptions of the services each registers, the
//! accounts the scripts name and the build's properties, and for each
//! program a script names, whether its partition carries it. They are what
//! the recovery reads on the phone to start the phone's own programs, kept
//! here for every phone so that reading them is tested against each.
//!
//! A partition is several gigabytes and these files a megabyte of it, so
//! the partition is not taken out whole: its filesystem is read where it
//! lies in the package on its server, and only the parts of the package
//! that hold what is read are downloaded, each checked against the
//! package's own sum of it.

use anyhow::{bail, ensure, Context, Result};
use fs_erofs::{FileType, Filesystem, Inode};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use unpayload::{ImageOptions, PartitionImage, Payload, ReadAt};

/// What is taken out of each where it has it: the init scripts and the
/// service descriptions, each a folder taken whole, the accounts its
/// scripts name, and the build's properties, which the vendor partition
/// keeps at its top and the others under etc.
const TAKEN: [&str; 6] = ["etc/init", "etc/vintf", "etc/passwd", "etc/group", "build.prop", "etc/build.prop"];

/// A partition in the package, read where the filesystem asks for it.
struct Partition(PartitionImage);

impl fs_core::BlockRead for Partition {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> fs_core::Result<()> {
        self.0.read_exact_at(buf, offset).map_err(fs_core::Error::Io)
    }

    fn size_bytes(&self) -> u64 {
        self.0.size()
    }
}

/// Take [`TAKEN`] of each of `partitions` out of the full update package at
/// `package`, and write it under `out` as the partition and then the path
/// in it.
///
/// A script may have init put a file in the place of another as the phone
/// starts, by mounting the one on the other, which is how a phone built in
/// more than one way swaps a service's description for the one of the
/// build it is. Each file put in another's place so is taken as well,
/// where it is on one of `partitions`.
///
/// A script may also name a program its partition does not carry, which
/// init leaves unstarted. Each program a script names that is on one of
/// `partitions` is written as an empty file where the partition has it:
/// that it is there is all the recovery asks of it before it starts it.
pub fn take(package: &str, partitions: &[String], out: &Path) -> Result<()> {
    let payload = Payload::open_url(package).with_context(|| format!("open {package}"))?;
    let mut read = Vec::new();
    for name in partitions {
        let to = out.join(name);
        ensure!(!to.exists(), "{} is there already; take it away to write it afresh", to.display());
        let image = payload.open_image(name, &ImageOptions::default()).with_context(|| format!("open the {name} partition of {package}"))?;
        let filesystem = Filesystem::open(Arc::new(Partition(image))).with_context(|| format!("read the filesystem of the {name} partition"))?;
        let mut written = 0;
        for path in TAKEN {
            written += take_path(&filesystem, path, &to).with_context(|| format!("take /{path} out of the {name} partition"))?;
        }
        ensure!(written > 0, "the {name} partition of {package} holds none of {TAKEN:?}");
        println!("wrote {written} files of the {name} partition under {}", to.display());
        read.push((name.as_str(), filesystem, to));
    }
    let mut named = Named::default();
    for (_, _, to) in &read {
        named.in_scripts(&to.join(SCRIPTS))?;
    }
    let on = |path: &str| -> Option<(&(&str, Filesystem, std::path::PathBuf), String)> {
        let (top, within) = skyward_programs::phone::partition_of(read.iter().map(|(name, _, _)| *name), path)?;
        Some((read.iter().find(|(name, _, _)| *name == top)?, within.to_string()))
    };
    for path in &named.put {
        let Some(((name, filesystem, to), within)) = on(path) else { continue };
        if to.join(&within).exists() {
            continue;
        }
        if take_path(filesystem, &within, to).with_context(|| format!("take {path} out of the {name} partition"))? > 0 {
            println!("wrote {path}, which the phone puts in the place of another file");
        }
    }
    let mut carried = 0;
    for path in &named.programs {
        let Some(((name, filesystem, to), within)) = on(path) else { continue };
        match filesystem.lookup_path(&within) {
            Ok(_) => {}
            // A path through a link that leads off the partition is not
            // one the partition carries either.
            Err(fs_erofs::Error::NotFound | fs_erofs::Error::NotADirectory) => continue,
            Err(e) => return Err(e).with_context(|| format!("look for {path} on the {name} partition")),
        }
        let at = to.join(&within);
        let folder = at.parent().context("a program is in a folder")?;
        fs::create_dir_all(folder).with_context(|| format!("make {}", folder.display()))?;
        fs::write(&at, b"").with_context(|| format!("write {}", at.display()))?;
        carried += 1;
    }
    println!("wrote {carried} of the {} programs the scripts name as empty files: those their partitions carry", named.programs.len());
    Ok(())
}

/// Where a partition keeps its init scripts.
const SCRIPTS: &str = "etc/init";

/// Take the file or the folder at `path` of `filesystem` and write it at
/// that path under `to`; gives how many files that was, none where the
/// filesystem has no such path.
fn take_path(filesystem: &Filesystem, path: &str, to: &Path) -> Result<usize> {
    let inode = match filesystem.lookup_path(path) {
        Ok(inode) => inode,
        Err(fs_erofs::Error::NotFound) => return Ok(0),
        Err(e) => return Err(e).context("look for it"),
    };
    copy(filesystem, &inode, &to.join(path))
}

/// The files the scripts taken name, each once, as the scripts name them.
#[derive(Default)]
struct Named {
    /// Each file a script has init mount on another.
    put: Vec<String>,
    /// Each program a script has init run.
    programs: Vec<String>,
}

impl Named {
    /// Add what the scripts in `folder`, and in the folders in it, name.
    fn in_scripts(&mut self, folder: &Path) -> Result<()> {
        let listing = match fs::read_dir(folder) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("list {}", folder.display())),
        };
        for entry in listing {
            let path = entry.with_context(|| format!("list {}", folder.display()))?.path();
            if path.is_dir() {
                self.in_scripts(&path)?;
            } else if path.extension().is_some_and(|e| e == "rc") {
                let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
                let script = skyward_programs::script::read(&text).with_context(|| format!("read {}", path.display()))?;
                for command in script.actions.iter().flat_map(|a| &a.commands) {
                    if let [mount, _, from, _, options @ ..] = &command[..]
                        && mount == "mount"
                        && options.iter().any(|o| o == "bind")
                        && !self.put.contains(from)
                    {
                        self.put.push(from.clone());
                    }
                }
                for program in script.programs {
                    if !self.programs.contains(&program.path) {
                        self.programs.push(program.path);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Write what `inode` holds at `to`, a file or a folder with all in it,
/// and give how many files that was. The folders taken hold files and
/// folders alone; anything else in one is refused, as nothing here would
/// read it.
fn copy(filesystem: &Filesystem, inode: &Inode, to: &Path) -> Result<usize> {
    match inode.file_type() {
        FileType::RegularFile => {
            let size = usize::try_from(inode.size).context("a file too large to hold")?;
            let mut data = vec![0u8; size];
            filesystem.read_file(inode, 0, &mut data).context("read it")?;
            let parent = to.parent().context("what is taken out goes into a folder")?;
            fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
            fs::write(to, data).with_context(|| format!("write {}", to.display()))?;
            Ok(1)
        }
        FileType::Dir => {
            fs::create_dir_all(to).with_context(|| format!("make {}", to.display()))?;
            let mut written = 0;
            for entry in filesystem.read_dir(inode).context("list it")? {
                if entry.name == b"." || entry.name == b".." {
                    continue;
                }
                let name = std::str::from_utf8(&entry.name).with_context(|| format!("a name that is not text: {:?}", String::from_utf8_lossy(&entry.name)))?;
                let child = filesystem.read_inode(entry.nid).with_context(|| format!("read {name}"))?;
                written += copy(filesystem, &child, &to.join(name)).with_context(|| format!("in {name}"))?;
            }
            Ok(written)
        }
        other => bail!("{} is a {other:?}, neither a file nor a folder", to.display()),
    }
}
