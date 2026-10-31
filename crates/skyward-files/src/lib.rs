//! Copying, moving, renaming and deleting files and folders the way a file
//! manager does, and making new ones: folders whole, links as links, times
//! kept, each new entry owned as asked, by what the original had or by
//! what the folder it goes into gives its own, and how far the work has
//! come told as it goes.

use anyhow::{bail, ensure, Context, Result};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// How much of a file is copied between two reports of how far it is.
const CHUNK: usize = 1 << 20;

/// Bytes as lowercase hex digits, as a digest or a key is written.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What to do when an entry of the same name is already where one goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// Write nothing, and say which name is taken.
    Refuse,
    /// Put the new entry in place of the old one, which goes first.
    Replace,
    /// Leave the old entry, and leave out the new one.
    Skip,
    /// Give the new entry a name of its own, as "name (1).txt".
    KeepBoth,
}

/// How far a piece of work has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub bytes: u64,
    pub total_bytes: u64,
    pub entries: u64,
    pub total_entries: u64,
}

/// The work was stopped, as asked, before it was done.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stopped before it was done")
    }
}

impl std::error::Error for Cancelled {}

/// The name is another entry's in the folder already. A folder that
/// takes names in any case counts one that differs only in case.
#[derive(Debug)]
pub struct Taken(pub PathBuf);

impl std::fmt::Display for Taken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is already there", self.0.display())
    }
}

impl std::error::Error for Taken {}

/// What a copy, or a move to another filesystem, gives each entry it
/// makes beyond its contents and times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    /// What the original has: owner, group, permissions and extended
    /// attributes, as far as the filesystem the entry goes to keeps them.
    Kept,
    /// The owner, group and SELinux label of the folder it goes into, as
    /// the entries the system makes there have them, and its own
    /// permissions.
    Folder,
    /// The owner, group, label and permissions of the folder it goes into,
    /// a file's without execute: shared storage, where the system gives
    /// every entry the same.
    Shared,
}

/// What a filesystem keeps of an entry beyond its contents and times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Keeps {
    owner: bool,
    mode: bool,
    xattrs: bool,
}

/// What the folder a copy goes into gives each entry made in it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Like {
    uid: u32,
    gid: u32,
    /// The folder's SELinux label as stored, on the phone.
    label: Option<Vec<u8>>,
    /// The folder's permissions, where entries take those as well.
    mode: Option<u32>,
}

/// What each entry a copy makes is given.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Give {
    Original(Keeps),
    Folder(Like),
}

/// Whether the work goes on, or was asked to stop.
fn going(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    Ok(())
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("look at {}", path.display())),
    }
}

fn name(path: &Path) -> Result<&OsStr> {
    path.file_name().with_context(|| format!("{} names no entry", path.display()))
}

/// Where an entry really is: its folder with every link in the way
/// followed, and its own name, the entry itself not followed if a link.
fn real(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let folder = fs::canonicalize(parent).with_context(|| format!("find {}", parent.display()))?;
    Ok(folder.join(name(path)?))
}

/// Whether an entry is where a filesystem, or a file, is mounted: it sits
/// on another device than the folder it is in.
fn mounted_at(path: &Path, meta: &fs::Metadata) -> Result<bool> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else { return Ok(true) };
    let up = fs::metadata(parent).with_context(|| format!("look at {}", parent.display()))?;
    Ok(up.dev() != meta.dev())
}

/// The bytes the files of `paths` hold and how many entries they are,
/// folders whole. With `one_filesystem`, a folder that has another
/// filesystem mounted on or inside it is refused: the work would reach
/// into that filesystem.
fn measure(paths: &[PathBuf], one_filesystem: bool) -> Result<(u64, u64)> {
    fn walk(path: &Path, dev: Option<u64>, one_filesystem: bool, sum: &mut (u64, u64)) -> Result<()> {
        let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
        if one_filesystem && meta.is_dir() && dev.is_some_and(|d| d != meta.dev()) {
            bail!("another filesystem is mounted at {}; unmount it first", path.display());
        }
        sum.1 += 1;
        if meta.is_file() {
            sum.0 += meta.len();
        } else if meta.is_dir() {
            for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
                let entry = entry.with_context(|| format!("read {}", path.display()))?;
                walk(&entry.path(), Some(meta.dev()), one_filesystem, sum)?;
            }
        }
        Ok(())
    }
    let mut sum = (0, 0);
    for path in paths {
        if one_filesystem {
            let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
            if meta.is_dir() && mounted_at(path, &meta)? {
                bail!("a filesystem is mounted at {}; unmount it first", path.display());
            }
        }
        walk(path, None, one_filesystem, &mut sum)?;
    }
    Ok(sum)
}

/// The names that are taken: where entries of `sources` would go in the
/// folder `into` and an entry of that name is already there.
pub fn clashes(sources: &[PathBuf], into: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for source in sources {
        let target = into.join(name(source)?);
        if exists(&target)? && real(&target)? != real(source)? {
            out.push(target);
        }
    }
    Ok(out)
}

/// A name in `into` for a new entry named `name` that is not taken:
/// "name (1).txt", "name (2).txt" and on.
fn free_name(into: &Path, name: &OsStr) -> Result<PathBuf> {
    let as_path = Path::new(name);
    let stem = as_path.file_stem().unwrap_or(name).to_string_lossy();
    let extension = as_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let candidate = into.join(format!("{stem} ({n}){extension}"));
        if !exists(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!("some number gives a name that is not taken")
}

/// Where each of `sources` goes in the folder `into` under `clash`, with
/// what cannot be done refused before anything is written. An entry moved
/// to the folder it is in already stays as it is; one copied there is a
/// second one beside it, under a name of its own.
fn plan(sources: &[PathBuf], into: &Path, clash: Clash, moving: bool) -> Result<Vec<(PathBuf, PathBuf)>> {
    let folder = fs::metadata(into).with_context(|| format!("look at {}", into.display()))?;
    ensure!(folder.is_dir(), "{} is not a folder", into.display());
    let into_real = fs::canonicalize(into).with_context(|| format!("find {}", into.display()))?;
    let mut out = Vec::new();
    for source in sources {
        let meta = fs::symlink_metadata(source).with_context(|| format!("look at {}", source.display()))?;
        let source_real = real(source)?;
        if meta.is_dir() && into_real.starts_with(&source_real) {
            bail!("{} cannot go inside itself", source.display());
        }
        let mut target = into.join(name(source)?);
        let itself = into_real.join(name(source)?) == source_real;
        if moving && itself {
            continue;
        }
        if itself {
            target = free_name(into, name(source)?)?;
        } else if exists(&target)? {
            match clash {
                Clash::Refuse => return Err(Taken(target).into()),
                Clash::Replace => {}
                Clash::Skip => continue,
                Clash::KeepBoth => target = free_name(into, name(source)?)?,
            }
        }
        out.push((source.clone(), target));
    }
    Ok(out)
}

/// What the filesystem holding the folder `into` keeps of an entry: one
/// made for other systems keeps neither owners, permissions nor extended
/// attributes.
#[cfg(target_os = "android")]
fn keeps(into: &Path) -> Result<Keeps> {
    use std::os::unix::ffi::OsStrExt;
    const EXFAT_SUPER_MAGIC: u64 = 0x2011_bab0;
    const SDCARDFS_SUPER_MAGIC: u64 = 0x5dca_2df5;
    let c = std::ffi::CString::new(into.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", into.display()))?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("look at the filesystem of {}", into.display()));
    }
    let foreign = [libc::MSDOS_SUPER_MAGIC as u64, EXFAT_SUPER_MAGIC, libc::FUSE_SUPER_MAGIC as u64, SDCARDFS_SUPER_MAGIC];
    let native = !foreign.contains(&st.f_type);
    Ok(Keeps { owner: native, mode: native, xattrs: native })
}

/// A computer the tests run on keeps owners and permissions; extended
/// attributes are copied only on the phone.
#[cfg(not(target_os = "android"))]
fn keeps(_into: &Path) -> Result<Keeps> {
    Ok(Keeps { owner: true, mode: true, xattrs: false })
}

/// What each entry a copy into the folder `into` makes is given under
/// `ownership`.
fn give(into: &Path, ownership: Ownership) -> Result<Give> {
    let keeps = keeps(into)?;
    if ownership == Ownership::Kept {
        return Ok(Give::Original(keeps));
    }
    ensure!(keeps.owner, "{} is on a filesystem that keeps no owners to give what is made there", into.display());
    let folder = fs::metadata(into).with_context(|| format!("look at {}", into.display()))?;
    let mode = (ownership == Ownership::Shared).then_some(folder.mode() & 0o7777);
    Ok(Give::Folder(Like { uid: folder.uid(), gid: folder.gid(), label: label(into)?, mode }))
}

/// The SELinux label of the folder `into`, as stored.
#[cfg(target_os = "android")]
fn label(into: &Path) -> Result<Option<Vec<u8>>> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(into.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", into.display()))?;
    let mut value = vec![0u8; 256];
    let len = unsafe { libc::getxattr(c.as_ptr(), c"security.selinux".as_ptr(), value.as_mut_ptr().cast(), value.len()) };
    if len < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("read the label of {}", into.display()));
    }
    value.truncate(len as usize);
    Ok(Some(value))
}

/// A computer the tests run on labels nothing.
#[cfg(not(target_os = "android"))]
fn label(_into: &Path) -> Result<Option<Vec<u8>>> {
    Ok(None)
}

/// Copy each of `sources` into the folder `into`, each new entry given
/// what `ownership` says.
pub fn copy(
    sources: &[PathBuf],
    into: &Path,
    clash: Clash,
    ownership: Ownership,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let plan = plan(sources, into, clash, false)?;
    let froms: Vec<PathBuf> = plan.iter().map(|(from, _)| from.clone()).collect();
    let (total_bytes, total_entries) = measure(&froms, false)?;
    let mut p = Progress { total_bytes, total_entries, ..Progress::default() };
    progress(p);
    let give = give(into, ownership)?;
    for (from, to) in plan {
        if exists(&to)? {
            remove(&to, cancel, &mut Progress::default(), &mut |_| {})?;
        }
        copy_entry(&from, &to, &give, cancel, &mut p, progress)?;
    }
    Ok(())
}

/// Move each of `sources` into the folder `into`. Within one filesystem an
/// entry is moved at once, keeping all it has; to another, it is copied,
/// each new entry given what `ownership` says, and then deleted.
pub fn move_into(
    sources: &[PathBuf],
    into: &Path,
    clash: Clash,
    ownership: Ownership,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let plan = plan(sources, into, clash, true)?;
    let froms: Vec<PathBuf> = plan.iter().map(|(from, _)| from.clone()).collect();
    let (total_bytes, total_entries) = measure(&froms, true)?;
    let mut p = Progress { total_bytes, total_entries, ..Progress::default() };
    progress(p);
    let give = give(into, ownership)?;
    for (from, to) in plan {
        going(cancel)?;
        if exists(&to)? {
            remove(&to, cancel, &mut Progress::default(), &mut |_| {})?;
        }
        let (bytes, entries) = measure(std::slice::from_ref(&from), true)?;
        match fs::rename(&from, &to) {
            Ok(()) => {
                p.bytes += bytes;
                p.entries += entries;
                progress(p);
            }
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                copy_entry(&from, &to, &give, cancel, &mut p, progress)?;
                remove(&from, cancel, &mut Progress::default(), &mut |_| {})?;
            }
            Err(e) => return Err(e).with_context(|| format!("move {} to {}", from.display(), to.display())),
        }
    }
    Ok(())
}

/// Make the folder `path` and each folder it is in that is not there yet,
/// each new one given what `ownership` says, as a copy into the folder it
/// is in would be.
pub fn make_folders(path: &Path, ownership: Ownership) -> Result<()> {
    if exists(path)? {
        return Ok(());
    }
    let into = path.parent().filter(|p| !p.as_os_str().is_empty()).with_context(|| format!("{} is in no folder", path.display()))?;
    make_folders(into, ownership)?;
    let give = give(into, ownership)?;
    fs::create_dir(path).with_context(|| format!("make {}", path.display()))?;
    given(path, &give, true)
}

/// Make the file `path`, which is not there yet, given what `ownership`
/// says as a copy into its folder would be, and open it to be written.
pub fn make_file(path: &Path, ownership: Ownership) -> Result<File> {
    let into = path.parent().filter(|p| !p.as_os_str().is_empty()).with_context(|| format!("{} is in no folder", path.display()))?;
    let give = give(into, ownership)?;
    let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).with_context(|| format!("make {}", path.display()))?;
    given(path, &give, false)?;
    Ok(file)
}

/// Give an entry just made what `give` says, where it is the folder's: its
/// label and owner and, in shared storage, its permissions, a file's
/// without execute. One that keeps an original's keeps what it was made
/// with, as there is no original.
fn given(path: &Path, give: &Give, folder: bool) -> Result<()> {
    let Give::Folder(like) = give else { return Ok(()) };
    if let Some(label) = &like.label {
        set_label(path, label)?;
    }
    std::os::unix::fs::lchown(path, Some(like.uid), Some(like.gid)).with_context(|| format!("give {} its owner", path.display()))?;
    if let Some(mode) = like.mode {
        let mode = if folder { mode } else { mode & 0o666 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).with_context(|| format!("give {} its permissions", path.display()))?;
    }
    Ok(())
}

/// Give the entry at `path` the name `new`, in the folder it is in, and
/// say where it now is. A folder that takes names in any case finds the
/// entry itself under a name that differs from its own only in case; the
/// entry then goes through a free name to its new one, as the kernel
/// leaves an entry renamed to itself as it is.
pub fn rename(path: &Path, new: &str) -> Result<PathBuf> {
    ensure!(
        !new.is_empty() && new != "." && new != ".." && !new.contains(['/', '\0']),
        "{new:?} cannot be the name of a file or folder"
    );
    let folder = path.parent().with_context(|| format!("{} is in no folder", path.display()))?;
    let to = folder.join(new);
    if to == path {
        return Ok(to);
    }
    if exists(&to)? {
        let mut named = false;
        for entry in fs::read_dir(folder).with_context(|| format!("read {}", folder.display()))? {
            named |= entry.with_context(|| format!("read {}", folder.display()))?.file_name() == new;
        }
        let own = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
        let found = fs::symlink_metadata(&to).with_context(|| format!("look at {}", to.display()))?;
        if named || (own.dev(), own.ino()) != (found.dev(), found.ino()) {
            return Err(Taken(to).into());
        }
        let step = free_name(folder, name(path)?)?;
        fs::rename(path, &step).with_context(|| format!("rename {} to {}", path.display(), step.display()))?;
        fs::rename(&step, &to).with_context(|| format!("rename {} to {new}", step.display()))?;
        return Ok(to);
    }
    fs::rename(path, &to).with_context(|| format!("rename {} to {new}", path.display()))?;
    Ok(to)
}

/// Give the entry at `path`, a link's target, the permission bits `mode`,
/// the set-ID and sticky bits among them, and say which it has after: a
/// filesystem that keeps none of its own, as FAT keeps none, leaves them as
/// they were.
pub fn set_mode(path: &Path, mode: u32) -> Result<u32> {
    let mode = mode & 0o7777;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).with_context(|| format!("give {} the permissions {mode:04o}", path.display()))?;
    Ok(fs::metadata(path).with_context(|| format!("look at {}", path.display()))?.mode() & 0o7777)
}

/// What the file at `path` holds, if that is no more than `most` bytes.
pub fn read_up_to(path: &Path, most: u64) -> Result<Option<Vec<u8>>> {
    // Opened without waiting, as a pipe waits for a writer, to be turned
    // down for what it is.
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path).with_context(|| format!("open {}", path.display()))?;
    ensure!(file.metadata().with_context(|| format!("look at {}", path.display()))?.is_file(), "{} is not a file", path.display());
    let mut bytes = Vec::new();
    file.take(most + 1).read_to_end(&mut bytes).with_context(|| format!("read {}", path.display()))?;
    Ok((bytes.len() as u64 <= most).then_some(bytes))
}

/// Write `bytes` as the contents of the file at `path`, which keeps its
/// owner, permissions and extended attributes, the SELinux label among
/// them, as far as its filesystem keeps them. The new contents are written
/// whole beside the file and then take its place at once, so that a write
/// cut short leaves it as it was; a file with other names sharing its
/// contents, or one mounted over, is written where it is.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
    ensure!(meta.is_file(), "{} is not a file", path.display());
    if meta.nlink() > 1 || mounted_at(path, &meta)? {
        let mut file = OpenOptions::new().write(true).truncate(true).open(path).with_context(|| format!("open {}", path.display()))?;
        file.write_all(bytes).with_context(|| format!("write {}", path.display()))?;
        return file.sync_all().with_context(|| format!("write {}", path.display()));
    }
    let folder = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temp, mut file) = beside(path)?;
    let written = (|| -> Result<()> {
        file.write_all(bytes).with_context(|| format!("write {}", temp.display()))?;
        file.sync_all().with_context(|| format!("write {}", temp.display()))?;
        attributes(path, &temp, &meta, &Give::Original(keeps(folder)?))?;
        fs::rename(&temp, path).with_context(|| format!("put {} in place of {}", temp.display(), path.display()))
    })();
    if written.is_err() {
        drop(file);
        fs::remove_file(&temp).with_context(|| format!("take away {}", temp.display()))?;
        return written;
    }
    File::open(folder).and_then(|f| f.sync_all()).with_context(|| format!("write {}", folder.display()))
}

/// A new file beside the file at `path`, hidden and named after it, to be
/// written and then put in its place.
fn beside(path: &Path) -> Result<(PathBuf, File)> {
    let folder = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    for n in 0.. {
        let mut temp = std::ffi::OsString::from(".");
        temp.push(name(path)?);
        temp.push(format!(".{n}.saving"));
        let temp = folder.join(temp);
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("make {}", temp.display())),
        }
    }
    unreachable!("some number gives a name that is not taken")
}

/// How far giving the data partition's entries their labels has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Relabeling {
    /// Entries looked at, of how many; the total is 0 while they are still
    /// counted.
    pub checked: u64,
    pub total: u64,
    /// Entries that took a new label.
    pub changed: u64,
}

/// What relabeling does with an entry of the data partition, by where it
/// is, as Android's own restorecon does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Treat {
    /// Labeled by the rules, and so is all in it.
    Label,
    /// An app's data, which the installer labels by the app: left as it is,
    /// with all in it.
    Leave,
    /// The system's files kept encrypted for each user, which vold and the
    /// programs writing them label, some by the user: the folder is
    /// labeled, and nothing in it.
    Alone,
    /// Each user's folder of app profiles, which vold labels: what is in it
    /// is labeled, and not the folder.
    Within,
}

/// What relabeling does with the entry at `rel` from the top of the data
/// partition.
fn treat(rel: &Path) -> Treat {
    let names: Vec<&OsStr> = rel.iter().collect();
    match names.as_slice() {
        [top, _, ..] if ["data", "user", "user_de", "storage_area"].iter().any(|n| top == n) => Treat::Leave,
        [misc, _, sandbox, ..] if *misc == "misc_de" && *sandbox == "sdksandbox" => Treat::Leave,
        [top, ..] if ["system_ce", "vendor_ce", "misc_ce"].iter().any(|n| top == n) => Treat::Alone,
        [misc, profiles, cur, _] if *misc == "misc" && *profiles == "profiles" && *cur == "cur" => Treat::Within,
        _ => Treat::Label,
    }
}

/// Give each entry of the data partition mounted at `root` the label the
/// system's rules give it, as Android's own restorecon does as the system
/// starts, but looking at every entry: an app's data is left to the
/// installer, which labels it by the app; of the system's files kept
/// encrypted for each user only their folders are labeled; another
/// filesystem mounted inside is labeled where it shows, and not gone into.
/// `label` gives the entry at a path, by its path from the top of the
/// partition, its label, and says whether that changed it. The entries are
/// counted first, for how far the work has come to tell.
pub fn relabel(
    root: &Path,
    label: &mut dyn FnMut(&Path, &Path, &fs::Metadata) -> Result<bool>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Relabeling),
) -> Result<()> {
    let dev = fs::symlink_metadata(root).with_context(|| format!("look at {}", root.display()))?.dev();
    let mut p = Relabeling::default();
    progress(p);
    walk_labeled(root, Path::new(""), dev, cancel, &mut |_, _, _| {
        p.total += 1;
        Ok(())
    })?;
    progress(p);
    walk_labeled(root, Path::new(""), dev, cancel, &mut |path, rel, meta| {
        p.changed += u64::from(label(path, rel, meta)?);
        p.checked += 1;
        progress(p);
        Ok(())
    })
}

/// Visit each entry relabeling labels from `path`, at `rel` from the top of
/// the partition: the entry, then what is in it.
fn walk_labeled(
    path: &Path,
    rel: &Path,
    dev: u64,
    cancel: &AtomicBool,
    visit: &mut dyn FnMut(&Path, &Path, &fs::Metadata) -> Result<()>,
) -> Result<()> {
    going(cancel)?;
    let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
    let treat = match treat(rel) {
        Treat::Alone | Treat::Within if !meta.is_dir() => Treat::Label,
        t => t,
    };
    if treat == Treat::Leave {
        return Ok(());
    }
    if treat != Treat::Within {
        visit(path, rel, &meta)?;
    }
    if treat == Treat::Alone || !meta.is_dir() || meta.dev() != dev {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry.with_context(|| format!("read {}", path.display()))?;
        walk_labeled(&entry.path(), &rel.join(entry.file_name()), dev, cancel, visit)?;
    }
    Ok(())
}

/// Delete each of `paths`, folders with all they hold. A folder that has a
/// filesystem mounted on or inside it is refused before anything goes.
pub fn delete(paths: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    let (total_bytes, total_entries) = measure(paths, true)?;
    let mut p = Progress { total_bytes, total_entries, ..Progress::default() };
    progress(p);
    for path in paths {
        remove(path, cancel, &mut p, progress)?;
    }
    Ok(())
}

/// Delete every file under each of `folders`, leaving the folders and all
/// the folders, links, pipes and sockets in them: the programs that made
/// those keep them where they left them. A folder that is not there holds
/// nothing to delete; one with another filesystem mounted inside it is
/// refused before anything goes.
pub fn delete_files(folders: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    let mut files = Vec::new();
    for folder in folders {
        if exists(folder)? {
            files_in(folder, None, &mut files)?;
        }
    }
    let total_bytes = files.iter().map(|(_, len)| len).sum();
    let mut p = Progress { total_bytes, total_entries: files.len() as u64, ..Progress::default() };
    progress(p);
    for (file, len) in &files {
        going(cancel)?;
        fs::remove_file(file).with_context(|| format!("delete {}", file.display()))?;
        p.bytes += len;
        p.entries += 1;
        progress(p);
    }
    Ok(())
}

/// The files under `folder` and how many bytes each holds, added to `out`;
/// `dev` is the device of the folder it is in, which it has to be on.
fn files_in(folder: &Path, dev: Option<u64>, out: &mut Vec<(PathBuf, u64)>) -> Result<()> {
    let meta = fs::symlink_metadata(folder).with_context(|| format!("look at {}", folder.display()))?;
    ensure!(meta.is_dir(), "{} is not a folder", folder.display());
    if dev.is_some_and(|d| d != meta.dev()) {
        bail!("another filesystem is mounted at {}; unmount it first", folder.display());
    }
    for entry in fs::read_dir(folder).with_context(|| format!("read {}", folder.display()))? {
        let path = entry.with_context(|| format!("read {}", folder.display()))?.path();
        let inside = fs::symlink_metadata(&path).with_context(|| format!("look at {}", path.display()))?;
        if inside.is_dir() {
            files_in(&path, Some(meta.dev()), out)?;
        } else if inside.is_file() {
            out.push((path, inside.len()));
        }
    }
    Ok(())
}

fn remove(path: &Path, cancel: &AtomicBool, p: &mut Progress, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    going(cancel)?;
    let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
    if meta.is_dir() {
        for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry.with_context(|| format!("read {}", path.display()))?;
            remove(&entry.path(), cancel, p, progress)?;
        }
        fs::remove_dir(path).with_context(|| format!("delete {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("delete {}", path.display()))?;
    }
    if meta.is_file() {
        p.bytes += meta.len();
    }
    p.entries += 1;
    progress(*p);
    Ok(())
}

fn copy_entry(from: &Path, to: &Path, give: &Give, cancel: &AtomicBool, p: &mut Progress, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    going(cancel)?;
    let meta = fs::symlink_metadata(from).with_context(|| format!("look at {}", from.display()))?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(from).with_context(|| format!("read the link {}", from.display()))?;
        std::os::unix::fs::symlink(&target, to).with_context(|| format!("make the link {}", to.display()))?;
    } else if kind.is_dir() {
        fs::create_dir(to).with_context(|| format!("make {}", to.display()))?;
        let mut entries: Vec<PathBuf> = fs::read_dir(from)
            .with_context(|| format!("read {}", from.display()))?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()
            .with_context(|| format!("read {}", from.display()))?;
        entries.sort();
        for entry in entries {
            copy_entry(&entry, &to.join(name(&entry)?), give, cancel, p, progress)?;
        }
    } else if kind.is_file() {
        copy_file(from, to, cancel, p, progress)?;
    } else if kind.is_block_device() || kind.is_char_device() || kind.is_fifo() {
        make_node(to, meta.mode(), meta.rdev())?;
    } else {
        bail!("{} is a socket, which only the program that made it can answer", from.display());
    }
    keep(from, to, &meta, give)?;
    p.entries += 1;
    progress(*p);
    Ok(())
}

/// Copy a file's contents, telling how far each chunk takes the work. A
/// file stopped halfway is not left behind.
fn copy_file(from: &Path, to: &Path, cancel: &AtomicBool, p: &mut Progress, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    let mut source = File::open(from).with_context(|| format!("open {}", from.display()))?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)
        .with_context(|| format!("make {}", to.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let copied = (|| -> Result<()> {
        loop {
            going(cancel)?;
            let n = source.read(&mut buf).with_context(|| format!("read {}", from.display()))?;
            if n == 0 {
                return target.sync_all().with_context(|| format!("write {}", to.display()));
            }
            target.write_all(&buf[..n]).with_context(|| format!("write {}", to.display()))?;
            p.bytes += n as u64;
            progress(*p);
        }
    })();
    if copied.is_err() {
        drop(target);
        fs::remove_file(to).with_context(|| format!("take away the part of {} written", to.display()))?;
    }
    copied
}

fn make_node(to: &Path, mode: u32, rdev: u64) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(to.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", to.display()))?;
    if unsafe { libc::mknod(c.as_ptr(), mode as libc::mode_t, rdev as libc::dev_t) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("make {}", to.display()));
    }
    Ok(())
}

/// Give a copy what `give` says beyond its contents, and the original's
/// times, which go last, as writing the rest would change them.
fn keep(from: &Path, to: &Path, meta: &fs::Metadata, give: &Give) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    attributes(from, to, meta, give)?;
    let c = std::ffi::CString::new(to.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", to.display()))?;
    let times = [
        libc::timespec { tv_sec: meta.atime() as _, tv_nsec: meta.atime_nsec() as _ },
        libc::timespec { tv_sec: meta.mtime() as _, tv_nsec: meta.mtime_nsec() as _ },
    ];
    if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("give {} its times", to.display()));
    }
    Ok(())
}

/// Give a copy what `give` says beyond its contents and times: its owner,
/// its permissions and its extended attributes or label. The owner goes
/// before the permissions, as changing it clears the set-ID bits.
fn attributes(from: &Path, to: &Path, meta: &fs::Metadata, give: &Give) -> Result<()> {
    let link = meta.file_type().is_symlink();
    let (owner, mode) = match give {
        Give::Original(keeps) => {
            if keeps.xattrs {
                copy_xattrs(from, to)?;
            }
            (keeps.owner.then_some((meta.uid(), meta.gid())), keeps.mode.then_some(meta.mode() & 0o7777))
        }
        Give::Folder(like) => {
            if let Some(label) = &like.label {
                set_label(to, label)?;
            }
            let mode = match like.mode {
                Some(folder) if meta.is_dir() => folder,
                Some(folder) => folder & 0o666,
                None => meta.mode() & 0o7777,
            };
            (Some((like.uid, like.gid)), Some(mode))
        }
    };
    if let Some((uid, gid)) = owner {
        std::os::unix::fs::lchown(to, Some(uid), Some(gid)).with_context(|| format!("give {} its owner", to.display()))?;
    }
    if let Some(mode) = mode.filter(|_| !link) {
        fs::set_permissions(to, fs::Permissions::from_mode(mode)).with_context(|| format!("give {} its permissions", to.display()))?;
    }
    Ok(())
}

/// Copy every extended attribute, the SELinux label among them, from one
/// entry to another, a link's own rather than its target's, as far as the
/// filesystem of `to` keeps them: one that keeps no attribute of a kind,
/// as the recovery's own root keeps no access lists, says so for each.
#[cfg(target_os = "android")]
fn copy_xattrs(from: &Path, to: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let cstr = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", p.display()));
    let (f, t) = (cstr(from)?, cstr(to)?);
    let size = unsafe { libc::llistxattr(f.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("list the attributes of {}", from.display()));
    }
    let mut names = vec![0u8; size as usize];
    let size = unsafe { libc::llistxattr(f.as_ptr(), names.as_mut_ptr().cast(), names.len()) };
    if size < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("list the attributes of {}", from.display()));
    }
    for attr in names[..size as usize].split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let n = std::ffi::CString::new(attr).expect("a name split at NULs holds none");
        let len = unsafe { libc::lgetxattr(f.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0) };
        if len < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("read {:?} of {}", n, from.display()));
        }
        let mut value = vec![0u8; len as usize];
        let len = unsafe { libc::lgetxattr(f.as_ptr(), n.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        if len < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("read {:?} of {}", n, from.display()));
        }
        if unsafe { libc::lsetxattr(t.as_ptr(), n.as_ptr(), value.as_ptr().cast(), len as usize, 0) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EOPNOTSUPP) {
                continue;
            }
            return Err(e).with_context(|| format!("give {} {:?}", to.display(), n));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn copy_xattrs(_from: &Path, _to: &Path) -> Result<()> {
    unreachable!("only the phone copies extended attributes")
}

/// Give an entry an SELinux label, a link its own.
#[cfg(target_os = "android")]
fn set_label(to: &Path, label: &[u8]) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(to.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", to.display()))?;
    if unsafe { libc::lsetxattr(c.as_ptr(), c"security.selinux".as_ptr(), label.as_ptr().cast(), label.len(), 0) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("label {}", to.display()));
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn set_label(_to: &Path, _label: &[u8]) -> Result<()> {
    unreachable!("only the phone labels entries")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A folder of its own for a test, empty, under the system's temporary
    /// folder.
    fn scratch(test: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-files-{test}-{}", std::process::id()));
        if exists(&at).unwrap() {
            fs::remove_dir_all(&at).unwrap();
        }
        fs::create_dir_all(&at).unwrap();
        at
    }

    /// A tree to copy: a file, a folder with a file in it, and a link.
    fn tree(at: &Path) -> PathBuf {
        let root = at.join("photos");
        fs::create_dir_all(root.join("2026")).unwrap();
        fs::write(root.join("a.jpg"), vec![7u8; 3 * CHUNK + 5]).unwrap();
        fs::write(root.join("2026/b.txt"), b"hello").unwrap();
        symlink("2026/b.txt", root.join("latest")).unwrap();
        fs::set_permissions(root.join("2026/b.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        root
    }

    fn quiet() -> impl FnMut(Progress) {
        |_| {}
    }

    #[test]
    fn a_folder_is_copied_whole_with_links_as_links() {
        let at = scratch("copy");
        let root = tree(&at);
        fs::create_dir(at.join("into")).unwrap();
        let mut last = Progress::default();
        copy(std::slice::from_ref(&root), &at.join("into"), Clash::Refuse, Ownership::Kept, &AtomicBool::new(false), &mut |p| last = p).unwrap();
        let copied = at.join("into/photos");
        assert_eq!(fs::read(copied.join("a.jpg")).unwrap(), vec![7u8; 3 * CHUNK + 5]);
        assert_eq!(fs::read(copied.join("2026/b.txt")).unwrap(), b"hello");
        assert_eq!(fs::read_link(copied.join("latest")).unwrap(), Path::new("2026/b.txt"));
        assert_eq!(fs::metadata(copied.join("2026/b.txt")).unwrap().mode() & 0o777, 0o640, "permissions are kept");
        let (before, after) = (fs::metadata(root.join("a.jpg")).unwrap(), fs::metadata(copied.join("a.jpg")).unwrap());
        assert_eq!((after.mtime(), after.mtime_nsec()), (before.mtime(), before.mtime_nsec()), "times are kept");
        assert_eq!(last, Progress { bytes: 3 * CHUNK as u64 + 10, total_bytes: 3 * CHUNK as u64 + 10, entries: 5, total_entries: 5 });
        assert!(root.join("a.jpg").exists(), "the original stays");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_taken_name_is_refused_replaced_skipped_or_kept_beside() {
        let at = scratch("clash");
        fs::create_dir(at.join("into")).unwrap();
        fs::write(at.join("note.txt"), b"new").unwrap();
        fs::write(at.join("into/note.txt"), b"old").unwrap();
        let (cancel, sources) = (AtomicBool::new(false), [at.join("note.txt")]);
        assert_eq!(clashes(&sources, &at.join("into")).unwrap(), [at.join("into/note.txt")]);
        let refused = copy(&sources, &at.join("into"), Clash::Refuse, Ownership::Kept, &cancel, &mut quiet());
        assert!(refused.unwrap_err().is::<Taken>());
        copy(&sources, &at.join("into"), Clash::Skip, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        assert_eq!(fs::read(at.join("into/note.txt")).unwrap(), b"old");
        copy(&sources, &at.join("into"), Clash::KeepBoth, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        copy(&sources, &at.join("into"), Clash::KeepBoth, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        assert_eq!(fs::read(at.join("into/note (1).txt")).unwrap(), b"new");
        assert_eq!(fs::read(at.join("into/note (2).txt")).unwrap(), b"new");
        copy(&sources, &at.join("into"), Clash::Replace, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        assert_eq!(fs::read(at.join("into/note.txt")).unwrap(), b"new");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_copy_into_its_own_folder_is_a_second_one() {
        let at = scratch("duplicate");
        fs::write(at.join(".profile"), b"x").unwrap();
        let cancel = AtomicBool::new(false);
        assert!(clashes(&[at.join(".profile")], &at).unwrap().is_empty(), "an entry does not clash with itself");
        for clash in [Clash::Refuse, Clash::Replace] {
            copy(&[at.join(".profile")], &at, clash, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        }
        assert_eq!(fs::read(at.join(".profile")).unwrap(), b"x", "the original stays");
        assert_eq!(fs::read(at.join(".profile (1)")).unwrap(), b"x");
        assert_eq!(fs::read(at.join(".profile (2)")).unwrap(), b"x", "whatever is chosen for names taken by others");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn folders_and_files_are_made_where_they_are_missing() {
        let at = scratch("make");
        let deep = at.join("Skyward").join("logs").join("20261021-133000");
        make_folders(&deep, Ownership::Kept).unwrap();
        assert!(deep.is_dir(), "with every folder on the way");
        make_folders(&deep, Ownership::Kept).unwrap();
        let mut log = make_file(&deep.join("kernel.log"), Ownership::Kept).unwrap();
        log.write_all(b"x").unwrap();
        assert_eq!(fs::read(deep.join("kernel.log")).unwrap(), b"x");
        assert!(make_file(&deep.join("kernel.log"), Ownership::Kept).is_err(), "a file there already is not made over");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_folder_cannot_go_inside_itself() {
        let at = scratch("inside");
        let root = tree(&at);
        let cancel = AtomicBool::new(false);
        assert!(copy(std::slice::from_ref(&root), &root.join("2026"), Clash::Refuse, Ownership::Kept, &cancel, &mut quiet()).is_err());
        assert!(move_into(std::slice::from_ref(&root), &root, Clash::Refuse, Ownership::Kept, &cancel, &mut quiet()).is_err());
        assert!(!root.join("2026/photos").exists(), "nothing was written");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn moving_takes_the_entries_away_from_where_they_were() {
        let at = scratch("move");
        let root = tree(&at);
        fs::create_dir(at.join("into")).unwrap();
        let cancel = AtomicBool::new(false);
        let mut last = Progress::default();
        move_into(std::slice::from_ref(&root), &at.join("into"), Clash::Refuse, Ownership::Kept, &cancel, &mut |p| last = p).unwrap();
        assert!(!root.exists());
        assert_eq!(fs::read(at.join("into/photos/2026/b.txt")).unwrap(), b"hello");
        assert_eq!((last.entries, last.total_entries), (5, 5));
        move_into(&[at.join("into/photos")], &at.join("into"), Clash::Refuse, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        assert!(at.join("into/photos").exists(), "moving to the folder it is in leaves it as it is");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn renaming_keeps_to_names_that_can_be() {
        let at = scratch("rename");
        fs::write(at.join("a.txt"), b"a").unwrap();
        fs::write(at.join("b.txt"), b"b").unwrap();
        assert_eq!(rename(&at.join("a.txt"), "c.txt").unwrap(), at.join("c.txt"));
        assert_eq!(fs::read(at.join("c.txt")).unwrap(), b"a");
        assert!(rename(&at.join("c.txt"), "b.txt").unwrap_err().is::<Taken>(), "a name taken stays with its own entry");
        for bad in ["", ".", "..", "x/y"] {
            assert!(rename(&at.join("c.txt"), bad).is_err(), "{bad:?}");
        }
        assert_eq!(rename(&at.join("c.txt"), "c.txt").unwrap(), at.join("c.txt"), "the same name changes nothing");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn permissions_are_set_and_read_back() {
        let at = scratch("mode");
        fs::write(at.join("run.sh"), b"#!/system/bin/sh\n").unwrap();
        fs::set_permissions(at.join("run.sh"), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(set_mode(&at.join("run.sh"), 0o755).unwrap(), 0o755);
        assert_eq!(fs::metadata(at.join("run.sh")).unwrap().mode() & 0o7777, 0o755);
        assert_eq!(set_mode(&at.join("run.sh"), 0o100_600).unwrap(), 0o600, "the bits of the type are not permissions");
        fs::create_dir(at.join("shared")).unwrap();
        assert_eq!(set_mode(&at.join("shared"), 0o1777).unwrap(), 0o1777, "a folder takes the sticky bit");
        std::os::unix::fs::symlink("run.sh", at.join("link")).unwrap();
        assert_eq!(set_mode(&at.join("link"), 0o700).unwrap(), 0o700, "a link's target is changed");
        assert_eq!(fs::metadata(at.join("run.sh")).unwrap().mode() & 0o7777, 0o700);
        assert!(set_mode(&at.join("gone"), 0o644).is_err());
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_file_is_read_only_up_to_the_bytes_asked_for() {
        let at = scratch("read");
        fs::write(at.join("a.txt"), b"12345").unwrap();
        assert_eq!(read_up_to(&at.join("a.txt"), 5).unwrap(), Some(b"12345".to_vec()));
        assert_eq!(read_up_to(&at.join("a.txt"), 4).unwrap(), None, "a byte more than asked for");
        assert!(read_up_to(&at, 5).is_err(), "a folder is no file");
        let pipe = std::ffi::CString::new(at.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(pipe.as_ptr(), 0o600) }, 0, "make a pipe");
        assert!(read_up_to(&at.join("pipe"), 5).is_err(), "a pipe is turned down rather than waited on");
        assert!(read_up_to(&at.join("gone"), 5).is_err());
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_file_written_anew_takes_its_own_place_keeping_its_permissions() {
        let at = scratch("replace");
        let path = at.join("run.sh");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        let before = fs::metadata(&path).unwrap();
        replace(&path, b"#!/system/bin/sh\necho new\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"#!/system/bin/sh\necho new\n");
        let after = fs::metadata(&path).unwrap();
        assert_eq!(after.mode() & 0o7777, 0o750, "permissions are kept");
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_ne!(after.ino(), before.ino(), "a new file took its place");
        let names: Vec<_> = fs::read_dir(&at).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["run.sh"], "nothing is left beside it");
        fs::write(at.join(".run.sh.0.saving"), b"left by a save cut short").unwrap();
        replace(&path, b"again").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"again", "a name taken beside it is passed over");
        fs::remove_file(at.join(".run.sh.0.saving")).unwrap();
        fs::hard_link(&path, at.join("same.sh")).unwrap();
        let shared = fs::metadata(&path).unwrap().ino();
        replace(&path, b"both").unwrap();
        assert_eq!(fs::read(at.join("same.sh")).unwrap(), b"both", "a file with another name is written where it is");
        assert_eq!(fs::metadata(&path).unwrap().ino(), shared);
        assert!(replace(&at, b"x").is_err(), "a folder is no file");
        assert!(replace(&at.join("gone"), b"x").is_err());
        fs::remove_dir_all(&at).unwrap();
    }

    /// The top of a data partition as Android lays it out, with an entry or
    /// two in each kind of place relabeling treats its own way.
    fn data_tree(at: &Path) -> PathBuf {
        let root = at.join("data");
        let folders = [
            "data/com.app/files",
            "user",
            "user_de/0/com.app",
            "storage_area/com.app",
            "misc_ce/0/sdksandbox/com.app",
            "misc_ce/0/apexdata",
            "system_ce/0/accounts",
            "misc/profiles/cur/0/com.app",
            "local/tmp",
            "media/0/DCIM",
        ];
        for folder in folders {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        let files =
            ["data/com.app/files/a", "misc_ce/0/apexdata/f", "system_ce/0/accounts/db", "misc/profiles/cur/0/com.app/primary.prof", "local/tmp/run.sh"];
        for file in files {
            fs::write(root.join(file), b"x").unwrap();
        }
        symlink("/data/data", root.join("user/0")).unwrap();
        fs::write(root.join("media/0/DCIM/p.jpg"), b"jpg").unwrap();
        root
    }

    #[test]
    fn relabeling_goes_where_androids_own_restorecon_goes() {
        let at = scratch("relabel");
        let root = data_tree(&at);
        fs::create_dir_all(root.join("misc_de/0/sdksandbox/com.app")).unwrap();
        fs::create_dir_all(root.join("misc_de/0/vold")).unwrap();
        let mut seen = Vec::new();
        let mut last = Relabeling::default();
        let mut label = |_: &Path, rel: &Path, _: &fs::Metadata| {
            seen.push(rel.to_string_lossy().into_owned());
            Ok(rel.starts_with("local"))
        };
        relabel(&root, &mut label, &AtomicBool::new(false), &mut |p| last = p).unwrap();
        seen.sort();
        let labeled = [
            "",
            "data",
            "local",
            "local/tmp",
            "local/tmp/run.sh",
            "media",
            "media/0",
            "media/0/DCIM",
            "media/0/DCIM/p.jpg",
            "misc",
            "misc/profiles",
            "misc/profiles/cur",
            "misc/profiles/cur/0/com.app",
            "misc/profiles/cur/0/com.app/primary.prof",
            "misc_ce",
            "misc_de",
            "misc_de/0",
            "misc_de/0/vold",
            "storage_area",
            "system_ce",
            "user",
            "user_de",
        ];
        assert_eq!(seen, labeled, "apps' data is the installer's, and each user's encrypted system files are left in their folders");
        assert_eq!(last, Relabeling { checked: 22, total: 22, changed: 3 }, "each entry under local took a new label");
        let stopped = relabel(&root, &mut |_, _, _| Ok(false), &AtomicBool::new(true), &mut |_| {});
        assert!(stopped.unwrap_err().is::<Cancelled>());
        fs::remove_dir_all(&at).unwrap();
    }

    /// Whether `at` takes names in any case, as a Mac's own disk does.
    fn any_case(at: &Path) -> bool {
        fs::write(at.join("probe"), b"").unwrap();
        let found = exists(&at.join("PROBE")).unwrap();
        fs::remove_file(at.join("probe")).unwrap();
        found
    }

    #[test]
    fn a_name_can_change_only_in_case_wherever_names_are_taken_in_any_case() {
        let at = scratch("case");
        fs::write(at.join("readme.txt"), b"r").unwrap();
        fs::write(at.join("notes.txt"), b"n").unwrap();
        rename(&at.join("readme.txt"), "README.txt").unwrap();
        let names: Vec<_> = fs::read_dir(&at).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.contains(&"README.txt".into()) && !names.contains(&"readme.txt".into()), "{names:?}");
        assert_eq!(fs::read(at.join("README.txt")).unwrap(), b"r");
        assert_eq!(names.len(), 2, "no name is left over from the way there");
        let other = rename(&at.join("README.txt"), "NOTES.txt");
        match any_case(&at) {
            true => assert!(other.unwrap_err().is::<Taken>(), "another entry's name in another case is taken"),
            false => assert_eq!(fs::read(other.unwrap()).unwrap(), b"r"),
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn entries_made_in_a_folder_can_take_what_its_own_have() {
        let at = scratch("ownership");
        let root = tree(&at);
        fs::set_permissions(root.join("a.jpg"), fs::Permissions::from_mode(0o604)).unwrap();
        let cancel = AtomicBool::new(false);
        for (folder, ownership) in [("folder", Ownership::Folder), ("shared", Ownership::Shared)] {
            let into = at.join(folder);
            fs::create_dir(&into).unwrap();
            fs::set_permissions(&into, fs::Permissions::from_mode(0o750)).unwrap();
            copy(std::slice::from_ref(&root), &into, Clash::Refuse, ownership, &cancel, &mut quiet()).unwrap();
            let mode = |p: &str| fs::symlink_metadata(into.join(p)).unwrap().mode() & 0o7777;
            let own = fs::metadata(&into).unwrap();
            for p in ["photos", "photos/a.jpg", "photos/2026", "photos/2026/b.txt", "photos/latest"] {
                let m = fs::symlink_metadata(into.join(p)).unwrap();
                assert_eq!((m.uid(), m.gid()), (own.uid(), own.gid()), "{folder}: {p} is owned as the folder is");
            }
            match ownership {
                Ownership::Folder => assert_eq!((mode("photos/a.jpg"), mode("photos/2026/b.txt")), (0o604, 0o640), "their own permissions"),
                _ => assert_eq!((mode("photos"), mode("photos/2026"), mode("photos/a.jpg"), mode("photos/2026/b.txt")), (0o750, 0o750, 0o640, 0o640)),
            }
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn deleting_takes_folders_whole_and_counts_each_entry() {
        let at = scratch("delete");
        let root = tree(&at);
        fs::write(at.join("loose.txt"), b"x").unwrap();
        let mut seen = Vec::new();
        delete(&[root.clone(), at.join("loose.txt")], &AtomicBool::new(false), &mut |p| seen.push(p.entries)).unwrap();
        assert!(!root.exists() && !at.join("loose.txt").exists());
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6], "every entry is told as it goes");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn deleting_files_leaves_every_folder_link_and_pipe() {
        let at = scratch("delete-files");
        let root = tree(&at);
        let pipe = std::ffi::CString::new(root.join("2026/pipe").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(pipe.as_ptr(), 0o600) }, 0, "make a pipe");
        fs::create_dir(root.join("empty")).unwrap();
        let mut last = Progress::default();
        let folders = [root.clone(), at.join("absent")];
        delete_files(&folders, &AtomicBool::new(false), &mut |p| last = p).unwrap();
        assert!(!root.join("a.jpg").exists() && !root.join("2026/b.txt").exists(), "the files are gone");
        assert!(root.join("2026").is_dir() && root.join("empty").is_dir(), "the folders stay");
        assert!(fs::symlink_metadata(root.join("latest")).unwrap().file_type().is_symlink(), "the link stays");
        assert!(fs::symlink_metadata(root.join("2026/pipe")).unwrap().file_type().is_fifo(), "the pipe stays");
        let bytes = 3 * CHUNK as u64 + 5 + 5;
        assert_eq!(last, Progress { bytes, total_bytes: bytes, entries: 2, total_entries: 2 }, "only files are counted");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn deleting_files_stops_when_asked() {
        let at = scratch("delete-files-stop");
        let root = tree(&at);
        let stop = AtomicBool::new(true);
        let stopped = delete_files(std::slice::from_ref(&root), &stop, &mut quiet());
        assert!(stopped.unwrap_err().is::<Cancelled>());
        assert!(root.join("a.jpg").exists(), "nothing went");
        fs::remove_dir_all(&at).unwrap();
    }

    #[cfg(target_os = "android")]
    fn label(path: &Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let (p, n) = (std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap(), c"security.selinux");
        let mut value = vec![0u8; 256];
        let len = unsafe { libc::lgetxattr(p.as_ptr(), n.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        assert!(len > 0, "{} has a label", path.display());
        String::from_utf8_lossy(&value[..len as usize]).trim_end_matches('\0').to_string()
    }

    /// On the phone, as root: labels go with a copy, and a move to another
    /// filesystem, /dev beside the temporary folder, copies and deletes.
    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_labels_go_with_a_copy_and_moves_cross_filesystems() {
        use std::os::unix::ffi::OsStrExt;
        let at = scratch("phone");
        let root = tree(&at);
        let file = root.join("2026/b.txt");
        let (p, n, v) = (std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap(), c"security.selinux", b"u:object_r:system_file:s0\0");
        assert_eq!(unsafe { libc::lsetxattr(p.as_ptr(), n.as_ptr(), v.as_ptr().cast(), v.len(), 0) }, 0, "{}", std::io::Error::last_os_error());
        fs::create_dir(at.join("into")).unwrap();
        let cancel = AtomicBool::new(false);
        copy(std::slice::from_ref(&root), &at.join("into"), Clash::Refuse, Ownership::Kept, &cancel, &mut quiet()).unwrap();
        assert_eq!(label(&at.join("into/photos/2026/b.txt")), "u:object_r:system_file:s0");
        let other = Path::new("/dev").join(format!("skyward-files-{}", std::process::id()));
        fs::create_dir(&other).unwrap();
        assert_ne!(fs::metadata(&other).unwrap().dev(), fs::metadata(&at).unwrap().dev(), "two filesystems");
        let mut last = Progress::default();
        move_into(std::slice::from_ref(&root), &other, Clash::Refuse, Ownership::Kept, &cancel, &mut |p| last = p).unwrap();
        assert!(!root.exists(), "the original goes once copied");
        assert_eq!(fs::read(other.join("photos/a.jpg")).unwrap(), vec![7u8; 3 * CHUNK + 5]);
        assert_eq!(label(&other.join("photos/2026/b.txt")), "u:object_r:system_file:s0");
        assert_eq!((last.bytes, last.entries), (last.total_bytes, last.total_entries));
        assert!(delete(&[Path::new("/dev").to_path_buf()], &cancel, &mut quiet()).is_err(), "a filesystem's own folder is not deleted");
        delete(&[other], &cancel, &mut quiet()).unwrap();
        fs::remove_dir_all(&at).unwrap();
    }

    /// On the phone, as root, with the temporary folder on the data
    /// partition: an access list, which the recovery's own root keeps none
    /// of, is left behind, and the copy is made all the same.
    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_what_a_filesystem_keeps_none_of_is_left_behind() {
        use std::os::unix::ffi::OsStrExt;
        let at = scratch("phone-acl");
        let file = at.join("listed.txt");
        fs::write(&file, b"x").unwrap();
        // Version 2, then the entries of the owner, the group, group 1000,
        // the mask and everyone else: tag, permissions, id.
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, perm, id) in [(0x01u16, 6u16, u32::MAX), (0x04, 4, u32::MAX), (0x08, 4, 1000), (0x10, 4, u32::MAX), (0x20, 0, u32::MAX)] {
            acl.extend(tag.to_le_bytes());
            acl.extend(perm.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        let p = std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap();
        let set = unsafe { libc::lsetxattr(p.as_ptr(), c"system.posix_acl_access".as_ptr(), acl.as_ptr().cast(), acl.len(), 0) };
        assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
        let into = Path::new("/").join(format!("skyward-files-acl-{}", std::process::id()));
        fs::create_dir(&into).unwrap();
        copy(&[file], &into, Clash::Refuse, Ownership::Kept, &AtomicBool::new(false), &mut quiet()).unwrap();
        assert_eq!(fs::read(into.join("listed.txt")).unwrap(), b"x");
        fs::remove_dir_all(&into).unwrap();
        fs::remove_dir_all(&at).unwrap();
    }

    /// On the phone, as root: what a folder gives its own entries includes
    /// its label, whatever the originals had.
    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_entries_made_in_a_folder_take_its_label() {
        use std::os::unix::ffi::OsStrExt;
        let at = scratch("phone-label");
        let root = tree(&at);
        let into = at.join("into");
        fs::create_dir(&into).unwrap();
        let (p, n, v) = (std::ffi::CString::new(into.as_os_str().as_bytes()).unwrap(), c"security.selinux", b"u:object_r:system_file:s0\0");
        assert_eq!(unsafe { libc::lsetxattr(p.as_ptr(), n.as_ptr(), v.as_ptr().cast(), v.len(), 0) }, 0, "{}", std::io::Error::last_os_error());
        assert_ne!(label(&root.join("2026/b.txt")), "u:object_r:system_file:s0");
        let cancel = AtomicBool::new(false);
        copy(std::slice::from_ref(&root), &into, Clash::Refuse, Ownership::Shared, &cancel, &mut quiet()).unwrap();
        for entry in ["photos", "photos/a.jpg", "photos/2026/b.txt", "photos/latest"] {
            assert_eq!(label(&into.join(entry)), "u:object_r:system_file:s0", "{entry}");
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn work_stopped_halfway_leaves_no_part_of_a_file() {
        let at = scratch("cancel");
        let root = tree(&at);
        fs::create_dir(at.join("into")).unwrap();
        let cancel = AtomicBool::new(false);
        let stopped = copy(&[root.join("a.jpg")], &at.join("into"), Clash::Refuse, Ownership::Kept, &cancel, &mut |p| {
            if p.bytes >= CHUNK as u64 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(stopped.unwrap_err().is::<Cancelled>());
        assert!(!at.join("into/a.jpg").exists());
        fs::remove_dir_all(&at).unwrap();
    }
}
