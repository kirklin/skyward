//! A folder's files as an archive, and back: the data partition's files
//! for a backup, each with what the system gave it beyond its contents.
//! That is its owner, permissions and times, its SELinux label and its
//! other extended attributes, the project its storage is counted in and,
//! for a folder, whether a key of its own begins there. Only the system can
//! make such a folder, so one gone is not made again.

use anyhow::{bail, ensure, Context, Result};
use skyward_files::Cancelled;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tar::{Archive, Builder, EntryType, Header};

/// How the archive's extended headers name what a tar header leaves out.
const XATTR: &str = "SCHILY.xattr.";
const PROJECT: &str = "SKYWARD.project";
const INHERITS: &str = "SKYWARD.project-inherit";
const KEY_ROOT: &str = "SKYWARD.key-root";
const MTIME: &str = "mtime";

/// Paths in a folder left out of its archive and left alone when an
/// archive is unpacked into it, each from the folder, a `*` in it standing
/// for any one name.
#[derive(Debug, Clone)]
pub struct Skipped(Vec<Vec<String>>);

impl Skipped {
    pub fn new(paths: &[String]) -> Skipped {
        Skipped(paths.iter().map(|p| p.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect()).collect())
    }

    /// Whether `rel` itself is skipped.
    fn holds(&self, rel: &Path) -> bool {
        let names: Vec<&str> = rel.components().map(|c| c.as_os_str().to_str().unwrap_or_default()).collect();
        self.0.iter().any(|p| p.len() == names.len() && p.iter().zip(&names).all(|(want, name)| want == "*" || want == name))
    }

    /// Whether `rel` is skipped or inside what is.
    fn covers(&self, rel: &Path) -> bool {
        rel.ancestors().filter(|a| !a.as_os_str().is_empty()).any(|a| self.holds(a))
    }
}

/// How many bytes of file contents `pack` would archive, each file with
/// more than one name once, and how many entries there are; stopping when
/// asked to.
pub fn measure(root: &Path, skipped: &Skipped, cancel: &AtomicBool) -> Result<(u64, u64)> {
    fn walk(
        folder: &Path,
        rel: &Path,
        dev: u64,
        skipped: &Skipped,
        cancel: &AtomicBool,
        seen: &mut HashSet<(u64, u64)>,
        sum: &mut (u64, u64),
    ) -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        for (name, path, meta) in children(folder)? {
            let rel = rel.join(&name);
            if skipped.holds(&rel) || meta.file_type().is_socket() || (meta.is_dir() && meta.dev() != dev) {
                continue;
            }
            sum.1 += 1;
            if meta.is_file() && (meta.nlink() == 1 || seen.insert((meta.dev(), meta.ino()))) {
                sum.0 += meta.len();
            } else if meta.is_dir() {
                walk(&path, &rel, dev, skipped, cancel, seen, sum)?;
            }
        }
        Ok(())
    }
    let dev = fs::symlink_metadata(root).with_context(|| format!("look at {}", root.display()))?.dev();
    let mut sum = (0, 0);
    walk(root, Path::new(""), dev, skipped, cancel, &mut HashSet::new(), &mut sum)?;
    Ok(sum)
}

/// The entries of a folder in the order of their names: each name, where it
/// is, and what it is, not following links.
fn children(folder: &Path) -> Result<Vec<(std::ffi::OsString, PathBuf, fs::Metadata)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(folder).with_context(|| format!("read {}", folder.display()))? {
        let entry = entry.with_context(|| format!("read {}", folder.display()))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).with_context(|| format!("look at {}", path.display()))?;
        out.push((entry.file_name(), path, meta));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Write the files of `root` to `out` as an archive: all but what is
/// `skipped`, sockets, which only the program that made them answers, and
/// what another filesystem mounted inside holds. A folder comes before what
/// it holds. Tells how many bytes of file contents are archived so far.
pub fn pack(root: &Path, skipped: &Skipped, out: impl Write, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let mut b = Builder::new(out);
    let meta = fs::symlink_metadata(root).with_context(|| format!("look at {}", root.display()))?;
    let mut packing = Packing { links: HashMap::new(), done: 0, cancel, progress };
    packing.folder(&mut b, root, Path::new(""), meta.dev(), skipped, key(root)?.as_deref())?;
    b.into_inner().context("finish the archive")?.flush().context("finish the archive")?;
    Ok(())
}

struct Packing<'a> {
    /// The first path archived of each file with more than one name, by its
    /// device and inode; the others are archived as links to it.
    links: HashMap<(u64, u64), PathBuf>,
    done: u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(u64),
}

impl Packing<'_> {
    /// Archive what the folder at `path` holds; `rel` is where it is in the
    /// archive and `above` the key its own entries are encrypted with.
    fn folder<W: Write>(&mut self, b: &mut Builder<W>, path: &Path, rel: &Path, dev: u64, skipped: &Skipped, above: Option<&[u8]>) -> Result<()> {
        for (name, path, meta) in children(path)? {
            let rel = rel.join(&name);
            if skipped.holds(&rel) || meta.file_type().is_socket() || (meta.is_dir() && meta.dev() != dev) {
                continue;
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let own = if meta.is_dir() { key(&path)? } else { None };
            if let Err(e) = self.entry(b, &path, &rel, &meta, meta.is_dir() && own.as_deref() != above) {
                // A file stopped partway through comes back as an error of
                // the archive's writing; it was stopped all the same.
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(Cancelled.into());
                }
                return Err(e.context(format!("archive {}", path.display())));
            }
            if meta.is_dir() {
                self.folder(b, &path, &rel, dev, skipped, own.as_deref())?;
            }
        }
        Ok(())
    }

    fn entry<W: Write>(&mut self, b: &mut Builder<W>, path: &Path, rel: &Path, meta: &fs::Metadata, key_root: bool) -> Result<()> {
        let kind = meta.file_type();
        let mut header = Header::new_gnu();
        header.set_mode(meta.mode() & 0o7777);
        header.set_uid(u64::from(meta.uid()));
        header.set_gid(u64::from(meta.gid()));
        header.set_mtime(meta.mtime().max(0) as u64);
        header.set_size(0);
        let mut records = beyond(path, meta)?;
        records.push((MTIME.into(), format!("{}.{:09}", meta.mtime(), meta.mtime_nsec()).into_bytes()));
        if key_root {
            records.push((KEY_ROOT.into(), b"1".to_vec()));
        }
        b.append_pax_extensions(records.iter().map(|(k, v)| (k.as_str(), v.as_slice())))?;
        if kind.is_dir() {
            header.set_entry_type(EntryType::Directory);
            b.append_data(&mut header, rel, io::empty())?;
        } else if kind.is_symlink() {
            header.set_entry_type(EntryType::Symlink);
            b.append_link(&mut header, rel, fs::read_link(path)?)?;
        } else if kind.is_file() {
            let id = (meta.dev(), meta.ino());
            if let Some(first) = self.links.get(&id).filter(|_| meta.nlink() > 1) {
                header.set_entry_type(EntryType::Link);
                b.append_link(&mut header, rel, first)?;
                return Ok(());
            }
            if meta.nlink() > 1 {
                self.links.insert(id, rel.to_path_buf());
            }
            header.set_entry_type(EntryType::Regular);
            header.set_size(meta.len());
            let file = File::open(path)?;
            let feed = Feed { file, left: meta.len(), done: &mut self.done, cancel: self.cancel, progress: &mut *self.progress };
            b.append_data(&mut header, rel, feed)?;
        } else {
            header.set_entry_type(match kind {
                k if k.is_char_device() => EntryType::Char,
                k if k.is_block_device() => EntryType::Block,
                _ => EntryType::Fifo,
            });
            header.set_device_major(major(meta.rdev()))?;
            header.set_device_minor(minor(meta.rdev()))?;
            b.append_data(&mut header, rel, io::empty())?;
        }
        Ok(())
    }
}

/// A file's contents as the archive takes them: as many bytes as it held
/// when it was looked at, telling the bytes archived so far, and stopping
/// when asked to.
struct Feed<'a> {
    file: File,
    left: u64,
    done: &'a mut u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(u64),
}

impl Read for Feed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other(Cancelled));
        }
        if self.left == 0 {
            return Ok(0);
        }
        let want = buf.len().min(self.left as usize);
        let n = self.file.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(ErrorKind::UnexpectedEof, "the file got shorter while it was archived"));
        }
        self.left -= n as u64;
        *self.done += n as u64;
        (self.progress)(*self.done);
        Ok(n)
    }
}

/// What unpacking left out: the folders a key of their own began in that
/// are gone, with what the archive held in them; only the system makes
/// such a folder, for a user it knows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unpacked {
    pub unmade: Vec<PathBuf>,
}

/// What an entry's extended header held.
#[derive(Default)]
struct Records {
    xattrs: Vec<(String, Vec<u8>)>,
    project: Option<u32>,
    inherits: bool,
    key_root: bool,
    mtime: Option<(i64, u32)>,
}

/// Make the files of `root` what the archive from `input` holds: each entry
/// as it was archived, with all it had beyond its contents, a folder that
/// is there already kept as it is; then take away what the archive does
/// not hold, but for what is `skipped` and the folders a key of their own
/// begins in. Tells how many bytes of file contents are written so far.
pub fn unpack(input: impl Read, root: &Path, skipped: &Skipped, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<Unpacked> {
    ensure!(fs::symlink_metadata(root).with_context(|| format!("look at {}", root.display()))?.is_dir(), "the restore root is not a directory");
    let mut archive = Archive::new(input);
    let mut held: HashSet<PathBuf> = HashSet::new();
    let mut folders: Vec<(PathBuf, (i64, u32))> = Vec::new();
    let mut unpacked = Unpacked::default();
    let mut done = 0u64;
    for entry in archive.entries().context("read the archive")? {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let mut entry = entry.context("read the archive")?;
        let rel = entry.path().context("read the archive")?.into_owned();
        ensure!(!rel.as_os_str().is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_))), "the archive names {} outside its folder", rel.display());
        if skipped.covers(&rel) || unpacked.unmade.iter().any(|u| rel.starts_with(u)) {
            continue;
        }
        let records = records(&mut entry).with_context(|| format!("read what the archive holds of {}", rel.display()))?;
        let header = entry.header().clone();
        let path = inside(root, &rel)?;
        let mtime = records.mtime.unwrap_or((header.mtime()? as i64, 0));
        let made = (|| -> Result<bool> {
            match header.entry_type() {
                EntryType::Directory => {
                    match fs::symlink_metadata(&path) {
                        Ok(there) if there.is_dir() => {}
                        Ok(_) => remove(&path)?,
                        Err(e) if e.kind() == ErrorKind::NotFound => {}
                        Err(e) => return Err(e).context("look at it"),
                    }
                    if !path.exists() {
                        if records.key_root {
                            return Ok(false);
                        }
                        fs::create_dir(&path).context("make it")?;
                    }
                    give(&path, &header, &records, false)?;
                    folders.push((rel.clone(), mtime));
                }
                EntryType::Regular => {
                    let name = path.file_name().context("it has no name")?.to_string_lossy().into_owned();
                    let temp = path.with_file_name(format!(".{name}.skyward-restore"));
                    // One a restore that stopped left behind goes first.
                    remove_any(&temp)?;
                    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp).context("make it")?;
                    let mut buf = vec![0u8; 1 << 20];
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            drop(file);
                            fs::remove_file(&temp).context("take away the part written")?;
                            return Err(Cancelled.into());
                        }
                        let n = entry.read(&mut buf).context("read it from the archive")?;
                        if n == 0 {
                            break;
                        }
                        file.write_all(&buf[..n]).context("write it")?;
                        done += n as u64;
                        progress(done);
                    }
                    file.sync_all().context("write it")?;
                    drop(file);
                    give(&temp, &header, &records, false)?;
                    times(&temp, mtime)?;
                    if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
                        remove(&path)?;
                    }
                    fs::rename(&temp, &path).context("put it in place")?;
                }
                EntryType::Symlink => {
                    let target = entry.link_name().context("read where it points")?.context("it points nowhere")?.into_owned();
                    remove_any(&path)?;
                    std::os::unix::fs::symlink(&target, &path).context("make it")?;
                    give(&path, &header, &records, true)?;
                    times(&path, mtime)?;
                }
                EntryType::Link => {
                    let first = entry.link_name().context("read what it is another name of")?.context("it names no file")?.into_owned();
                    let source = inside(root, &first)?;
                    ensure!(source != path, "a hard link cannot name itself");
                    ensure!(fs::symlink_metadata(&source).context("look at its original file")?.is_file(), "a hard link must name a regular file inside the restore root");
                    remove_any(&path)?;
                    fs::hard_link(&source, &path).context("make it")?;
                }
                EntryType::Char | EntryType::Block | EntryType::Fifo => {
                    remove_any(&path)?;
                    let c = cstr(&path)?;
                    let mode = (header.mode()? & 0o7777) as libc::mode_t;
                    let made = match header.entry_type() {
                        EntryType::Fifo => unsafe { libc::mkfifo(c.as_ptr(), mode) },
                        kind => {
                            let kind = if kind == EntryType::Char { libc::S_IFCHR } else { libc::S_IFBLK };
                            let dev = libc::makedev(header.device_major()?.unwrap_or(0) as _, header.device_minor()?.unwrap_or(0) as _);
                            unsafe { libc::mknod(c.as_ptr(), kind | mode, dev) }
                        }
                    };
                    if made != 0 {
                        return Err(io::Error::last_os_error()).context("make it");
                    }
                    give(&path, &header, &records, false)?;
                    times(&path, mtime)?;
                }
                other => bail!("the archive holds a {other:?} there"),
            }
            Ok(true)
        })()
        .with_context(|| format!("restore {}", path.display()))?;
        if !made {
            unpacked.unmade.push(rel);
            continue;
        }
        held.insert(rel);
    }
    let dev = fs::symlink_metadata(root).with_context(|| format!("look at {}", root.display()))?.dev();
    sweep(root, Path::new(""), dev, &held, skipped, key(root)?.as_deref())?;
    // A folder's time goes last, its contents having changed it, the
    // deepest first.
    for (rel, mtime) in folders.iter().rev() {
        times(&root.join(rel), *mtime)?;
    }
    Ok(unpacked)
}

/// Archive entries may restore symbolic links, but may never traverse one
/// as a parent. A lexical relative path alone does not keep a write below
/// root when an earlier entry or the existing tree supplies a symlink.
fn inside(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(!relative.as_os_str().is_empty() && relative.components().all(|c| matches!(c, Component::Normal(_))), "the archive names {} outside its folder", relative.display());
    let mut parent = root.to_path_buf();
    for component in relative.parent().expect("a relative entry has a parent").components() {
        parent.push(component);
        let meta = fs::symlink_metadata(&parent).with_context(|| format!("look at restore parent {}", parent.display()))?;
        ensure!(meta.is_dir(), "restore parent {} is not a plain directory", parent.display());
    }
    Ok(root.join(relative))
}

/// Take away what the folder at `rel` in `root` holds that `held` does
/// not: all but what is `skipped`, what another filesystem mounted inside
/// holds, and the folders a key of their own begins in, which the system
/// made; `above` is the key the folder's own entries are encrypted with.
fn sweep(root: &Path, rel: &Path, dev: u64, held: &HashSet<PathBuf>, skipped: &Skipped, above: Option<&[u8]>) -> Result<()> {
    for (name, path, meta) in children(&root.join(rel))? {
        let rel = rel.join(&name);
        if skipped.holds(&rel) || (meta.is_dir() && meta.dev() != dev) {
            continue;
        }
        let own = if meta.is_dir() { key(&path)? } else { None };
        if held.contains(&rel) {
            if meta.is_dir() {
                sweep(root, &rel, dev, held, skipped, own.as_deref())?;
            }
        } else if !(meta.is_dir() && own.as_deref() != above) {
            remove(&path)?;
        }
    }
    Ok(())
}

/// Take away whatever is at `path`, a folder whole.
fn remove(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
    match meta.is_dir() {
        true => fs::remove_dir_all(path),
        false => fs::remove_file(path),
    }
    .with_context(|| format!("take away {}", path.display()))
}

/// Take away whatever is at `path`, if anything is.
fn remove_any(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => remove(path),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("look at {}", path.display())),
    }
}

fn records(entry: &mut tar::Entry<impl Read>) -> Result<Records> {
    let mut r = Records::default();
    let Some(extensions) = entry.pax_extensions()? else { return Ok(r) };
    for ext in extensions {
        let ext = ext?;
        let key = ext.key()?;
        let value = ext.value_bytes();
        if let Some(name) = key.strip_prefix(XATTR) {
            r.xattrs.push((name.to_string(), value.to_vec()));
        } else if key == PROJECT {
            r.project = Some(std::str::from_utf8(value)?.parse()?);
        } else if key == INHERITS {
            r.inherits = value == b"1";
        } else if key == KEY_ROOT {
            r.key_root = value == b"1";
        } else if key == MTIME {
            r.mtime = Some(time(std::str::from_utf8(value)?)?);
        }
    }
    Ok(r)
}

/// A time as the extended header writes it: seconds, and nanoseconds after
/// a point.
fn time(text: &str) -> Result<(i64, u32)> {
    let (secs, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits: String = frac.chars().chain(std::iter::repeat('0')).take(9).collect();
    Ok((secs.parse().with_context(|| format!("read the time {text}"))?, digits.parse().with_context(|| format!("read the time {text}"))?))
}

/// Give an entry the owner, permissions and all else the archive held of
/// it. A link has no permissions of its own.
fn give(path: &Path, header: &Header, records: &Records, link: bool) -> Result<()> {
    let (uid, gid) = (header.uid()? as u32, header.gid()? as u32);
    std::os::unix::fs::lchown(path, Some(uid), Some(gid)).context("give it its owner")?;
    if !link {
        fs::set_permissions(path, fs::Permissions::from_mode(header.mode()? & 0o7777)).context("give it its permissions")?;
    }
    for (name, value) in &records.xattrs {
        set_xattr(path, name, value)?;
    }
    if !link && (records.project.is_some() || records.inherits) {
        set_project(path, records.project.unwrap_or(0), records.inherits)?;
    }
    Ok(())
}

/// Give an entry its times, a link its own: the time it changed, and the
/// same as the time it was read.
fn times(path: &Path, (secs, nanos): (i64, u32)) -> Result<()> {
    let c = cstr(path)?;
    let t = libc::timespec { tv_sec: secs as _, tv_nsec: nanos as _ };
    if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), [t, t].as_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(io::Error::last_os_error()).with_context(|| format!("give {} its times", path.display()));
    }
    Ok(())
}

fn cstr(path: &Path) -> Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", path.display()))
}

fn major(rdev: u64) -> u32 {
    ((rdev >> 32) & 0xffff_f000) as u32 | ((rdev >> 8) & 0xfff) as u32
}

fn minor(rdev: u64) -> u32 {
    ((rdev >> 12) & 0xffff_ff00) as u32 | (rdev & 0xff) as u32
}

/// The file's attributes on the filesystem, as the kernel hands them out
/// and takes them.
#[cfg(target_os = "android")]
#[repr(C)]
#[derive(Default)]
struct FsXattr {
    xflags: u32,
    extsize: u32,
    nextents: u32,
    projid: u32,
    cowextsize: u32,
    pad: [u8; 8],
}

#[cfg(target_os = "android")]
const FS_IOC_FSGETXATTR: libc::c_int = 0x801c_581f_u32 as libc::c_int;
#[cfg(target_os = "android")]
const FS_IOC_FSSETXATTR: libc::c_int = 0x401c_5820_u32 as libc::c_int;
/// New entries in a folder with this go in the folder's project.
#[cfg(target_os = "android")]
const FS_XFLAG_PROJINHERIT: u32 = 0x200;
#[cfg(target_os = "android")]
const FS_IOC_GET_ENCRYPTION_POLICY_EX: libc::c_int = 0xc009_6616_u32 as libc::c_int;

/// The extended attributes of an entry, a link's own, the SELinux label
/// among them, and the project its storage is counted in.
#[cfg(target_os = "android")]
fn beyond(path: &Path, meta: &fs::Metadata) -> Result<Vec<(String, Vec<u8>)>> {
    let c = cstr(path)?;
    let mut out = Vec::new();
    let size = unsafe { libc::llistxattr(c.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(io::Error::last_os_error()).context("list its attributes");
    }
    let mut names = vec![0u8; size as usize];
    let size = unsafe { libc::llistxattr(c.as_ptr(), names.as_mut_ptr().cast(), names.len()) };
    if size < 0 {
        return Err(io::Error::last_os_error()).context("list its attributes");
    }
    for raw in names[..size as usize].split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let name = std::ffi::CString::new(raw).expect("a name split at NULs holds none");
        let len = unsafe { libc::lgetxattr(c.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if len < 0 {
            return Err(io::Error::last_os_error()).with_context(|| format!("read its attribute {name:?}"));
        }
        let mut value = vec![0u8; len as usize];
        let len = unsafe { libc::lgetxattr(c.as_ptr(), name.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        if len < 0 {
            return Err(io::Error::last_os_error()).with_context(|| format!("read its attribute {name:?}"));
        }
        value.truncate(len as usize);
        out.push((format!("{XATTR}{}", String::from_utf8_lossy(raw)), value));
    }
    if meta.is_file() || meta.is_dir() {
        let fsx = fs_attributes(path)?;
        if fsx.projid != 0 {
            out.push((PROJECT.into(), fsx.projid.to_string().into_bytes()));
        }
        if fsx.xflags & FS_XFLAG_PROJINHERIT != 0 {
            out.push((INHERITS.into(), b"1".to_vec()));
        }
    }
    Ok(out)
}

#[cfg(not(target_os = "android"))]
fn beyond(_path: &Path, _meta: &fs::Metadata) -> Result<Vec<(String, Vec<u8>)>> {
    Ok(Vec::new())
}

/// A file or folder opened only to ask the kernel about it.
#[cfg(target_os = "android")]
fn open_bare(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

#[cfg(target_os = "android")]
fn fs_attributes(path: &Path) -> Result<FsXattr> {
    use std::os::fd::AsRawFd;
    let file = open_bare(path)?;
    let mut fsx = FsXattr::default();
    if unsafe { libc::ioctl(file.as_raw_fd(), FS_IOC_FSGETXATTR, &mut fsx) } != 0 {
        let e = io::Error::last_os_error();
        if matches!(e.raw_os_error(), Some(libc::ENOTTY | libc::EOPNOTSUPP)) {
            return Ok(FsXattr::default());
        }
        return Err(e).context("read its project");
    }
    Ok(fsx)
}

#[cfg(target_os = "android")]
fn set_project(path: &Path, project: u32, inherits: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    let file = open_bare(path)?;
    let mut fsx = fs_attributes(path)?;
    fsx.projid = project;
    fsx.xflags = if inherits { fsx.xflags | FS_XFLAG_PROJINHERIT } else { fsx.xflags & !FS_XFLAG_PROJINHERIT };
    if unsafe { libc::ioctl(file.as_raw_fd(), FS_IOC_FSSETXATTR, &fsx) } != 0 {
        return Err(io::Error::last_os_error()).context("give it its project");
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn set_project(_path: &Path, _project: u32, _inherits: bool) -> Result<()> {
    unreachable!("only the phone counts storage in projects")
}

/// Give an entry an extended attribute, a link its own. One the filesystem
/// keeps none of is let go.
#[cfg(target_os = "android")]
fn set_xattr(path: &Path, name: &str, value: &[u8]) -> Result<()> {
    let c = cstr(path)?;
    let n = std::ffi::CString::new(name).with_context(|| format!("the attribute {name} holds a NUL"))?;
    if unsafe { libc::lsetxattr(c.as_ptr(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0) } != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EOPNOTSUPP) {
            return Ok(());
        }
        return Err(e).with_context(|| format!("give it its attribute {name}"));
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn set_xattr(_path: &Path, _name: &str, _value: &[u8]) -> Result<()> {
    unreachable!("only the phone's archives hold extended attributes")
}

/// The key a folder's own entries are encrypted with, as the kernel tells
/// it; none for a folder of a filesystem that encrypts nothing, or one that
/// is not encrypted.
#[cfg(target_os = "android")]
fn key(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::os::fd::AsRawFd;
    // The size of the policy, which the kernel takes and sets, then the
    // policy, the largest the kernel writes.
    const POLICY: usize = 24;
    let file = open_bare(path)?;
    let mut arg = [0u8; 8 + POLICY];
    arg[..8].copy_from_slice(&(POLICY as u64).to_ne_bytes());
    if unsafe { libc::ioctl(file.as_raw_fd(), FS_IOC_GET_ENCRYPTION_POLICY_EX, arg.as_mut_ptr()) } != 0 {
        let e = io::Error::last_os_error();
        if matches!(e.raw_os_error(), Some(libc::ENODATA | libc::ENOTTY | libc::EOPNOTSUPP)) {
            return Ok(None);
        }
        return Err(e).with_context(|| format!("read how {} is encrypted", path.display()));
    }
    let size = u64::from_ne_bytes(arg[..8].try_into().expect("eight bytes")) as usize;
    Ok(Some(arg[8..8 + size.min(POLICY)].to_vec()))
}

#[cfg(not(target_os = "android"))]
fn key(_path: &Path) -> Result<Option<Vec<u8>>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn scratch(test: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-archive-{test}-{}", std::process::id()));
        if at.exists() {
            fs::remove_dir_all(&at).unwrap();
        }
        fs::create_dir_all(&at).unwrap();
        at
    }

    fn fifo(path: &Path) {
        let c = cstr(path).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o640) }, 0);
    }

    /// A folder as the data partition might hold one: apps' folders, a file
    /// with two names, links, a pipe, and what is left out.
    fn tree(at: &Path) {
        fs::create_dir_all(at.join("data/com.example/files")).unwrap();
        fs::write(at.join("data/com.example/files/notes.db"), vec![7u8; 300_000]).unwrap();
        fs::set_permissions(at.join("data/com.example/files/notes.db"), fs::Permissions::from_mode(0o660)).unwrap();
        fs::set_permissions(at.join("data/com.example"), fs::Permissions::from_mode(0o751)).unwrap();
        fs::write(at.join("data/com.example/empty"), b"").unwrap();
        fs::hard_link(at.join("data/com.example/files/notes.db"), at.join("data/com.example/notes-link.db")).unwrap();
        symlink("/data/user/0", at.join("data/self")).unwrap();
        fifo(&at.join("data/com.example/pipe"));
        fs::create_dir_all(at.join("system_de/0/spblob")).unwrap();
        fs::write(at.join("system_de/0/spblob/secret"), b"bound to the secure world").unwrap();
        fs::write(at.join("system_de/0/settings.xml"), b"<settings/>").unwrap();
        fs::create_dir_all(at.join("media/0/DCIM")).unwrap();
        fs::write(at.join("media/0/DCIM/photo.jpg"), b"jpeg").unwrap();
    }

    fn skipped() -> Skipped {
        Skipped::new(&["media".into(), "system_de/*/spblob".into()])
    }

    #[test]
    fn a_folder_comes_back_as_it_was_archived() {
        let at = scratch("round");
        let (from, to) = (at.join("from"), at.join("to"));
        tree(&from);
        #[cfg(target_os = "android")]
        set_xattr(&from.join("data/com.example/empty"), "user.skyward", b"kept").unwrap();
        let (bytes, entries) = measure(&from, &skipped(), &AtomicBool::new(false)).unwrap();
        assert_eq!((bytes, entries), (300_000 + 11, 11), "what is left out is not counted, nor a second name");
        let mut archived = Vec::new();
        let mut told = 0;
        pack(&from, &skipped(), &mut archived, &AtomicBool::new(false), &mut |n| told = n).unwrap();
        assert_eq!(told, 300_000 + 11, "a file with two names is archived once");
        // What is there before goes, but for what is left out.
        fs::create_dir_all(to.join("data/com.removed/cache")).unwrap();
        fs::write(to.join("data/com.removed/cache/x"), b"x").unwrap();
        fs::create_dir_all(to.join("data/com.example")).unwrap();
        fs::write(to.join("data/com.example/stale"), b"old").unwrap();
        fs::create_dir_all(to.join("system_de/0/spblob")).unwrap();
        fs::write(to.join("system_de/0/spblob/current"), b"the secure world's own").unwrap();
        fs::create_dir_all(to.join("media/0")).unwrap();
        fs::write(to.join("media/0/kept.jpg"), b"jpeg").unwrap();
        let unpacked = unpack(archived.as_slice(), &to, &skipped(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(unpacked, Unpacked::default());
        assert_eq!(fs::read(to.join("data/com.example/files/notes.db")).unwrap(), vec![7u8; 300_000]);
        assert_eq!(fs::metadata(to.join("data/com.example/files/notes.db")).unwrap().mode() & 0o7777, 0o660);
        assert_eq!(fs::metadata(to.join("data/com.example")).unwrap().mode() & 0o7777, 0o751);
        let (a, b) = (fs::metadata(to.join("data/com.example/files/notes.db")).unwrap(), fs::metadata(to.join("data/com.example/notes-link.db")).unwrap());
        assert_eq!(a.ino(), b.ino(), "two names of one file stay so");
        assert_eq!(fs::read_link(to.join("data/self")).unwrap(), Path::new("/data/user/0"));
        assert!(fs::symlink_metadata(to.join("data/com.example/pipe")).unwrap().file_type().is_fifo());
        assert_eq!(fs::read(to.join("data/com.example/empty")).unwrap(), b"");
        assert_eq!(fs::read(to.join("system_de/0/settings.xml")).unwrap(), b"<settings/>");
        assert!(!to.join("data/com.removed").exists(), "an app's folder the backup does not hold goes");
        assert!(!to.join("data/com.example/stale").exists());
        assert_eq!(fs::read(to.join("system_de/0/spblob/current")).unwrap(), b"the secure world's own", "what is left out stays");
        assert!(!to.join("system_de/0/spblob/secret").exists(), "and none of the backup's goes in");
        assert!(to.join("media/0/kept.jpg").exists());
        for rel in ["data/com.example/files/notes.db", "data/com.example", "data/self"] {
            let (was, now) = (fs::symlink_metadata(from.join(rel)).unwrap(), fs::symlink_metadata(to.join(rel)).unwrap());
            assert_eq!((was.mtime(), was.mtime_nsec()), (now.mtime(), now.mtime_nsec()), "{rel} keeps its time");
        }
        #[cfg(target_os = "android")]
        for rel in ["data/com.example/files/notes.db", "data/com.example/empty", "data/com.example", "data/self", "data/com.example/pipe"] {
            let (was, now) = (from.join(rel), to.join(rel));
            let held = |p: &Path| beyond(p, &fs::symlink_metadata(p).unwrap()).unwrap();
            assert_eq!(held(&was), held(&now), "{rel} keeps its label and all else beyond its contents");
            assert!(held(&now).iter().any(|(k, _)| k == "SCHILY.xattr.security.selinux"), "{rel} has a label");
        }
        fs::remove_dir_all(&at).unwrap();
    }

    fn test_header(kind: EntryType, bytes: usize) -> Header {
        let mut header = Header::new_gnu();
        header.set_entry_type(kind);
        header.set_mode(0o700);
        header.set_uid(u64::from(unsafe { libc::geteuid() }));
        header.set_gid(u64::from(unsafe { libc::getegid() }));
        header.set_size(bytes as u64);
        header
    }

    #[test]
    fn hard_link_sources_stay_inside_the_restore_root_and_are_regular_files() {
        let at = scratch("hard-link-boundary");
        let (root, outside) = (at.join("root"), at.join("outside"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        let protected = outside.join("protected");
        fs::write(&protected, b"outside stays unchanged").unwrap();
        fs::write(root.join("destination"), b"keep the previous destination").unwrap();
        symlink(&outside, root.join("parent-link")).unwrap();
        symlink(&protected, root.join("leaf-link")).unwrap();
        for target in [protected.clone(), PathBuf::from("../outside/protected"), PathBuf::from("parent-link/protected"),
                       PathBuf::from("leaf-link"), PathBuf::from("destination")] {
            let mut tar = Builder::new(Vec::new());
            tar.append_link(&mut test_header(EntryType::Link, 0), "destination", &target).unwrap();
            let bytes = tar.into_inner().unwrap();
            assert!(unpack(bytes.as_slice(), &root, &Skipped::new(&[]), &AtomicBool::new(false), &mut |_| {}).is_err(), "accepted {target:?}");
            assert_eq!(fs::read(root.join("destination")).unwrap(), b"keep the previous destination", "refuse before deleting the old destination");
            assert_eq!(fs::read(&protected).unwrap(), b"outside stays unchanged");
            assert_eq!(fs::metadata(&protected).unwrap().nlink(), 1, "never add a link to the outside file");
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn restoring_entries_never_traverses_existing_or_archived_symlink_parents() {
        let at = scratch("symlink-parents");
        for existing in [false, true] {
            for directory in [false, true] {
                let case = at.join(format!("{existing}-{directory}"));
                let (root, outside) = (case.join("root"), case.join("outside"));
                fs::create_dir_all(&root).unwrap();
                fs::create_dir(&outside).unwrap();
                fs::write(outside.join("changed"), b"protected outside data").unwrap();
                let mut tar = Builder::new(Vec::new());
                if existing {
                    symlink(&outside, root.join("alias")).unwrap();
                } else {
                    tar.append_link(&mut test_header(EntryType::Symlink, 0), "alias", &outside).unwrap();
                }
                let (kind, contents) = if directory { (EntryType::Directory, b"".as_slice()) } else { (EntryType::Regular, b"bad".as_slice()) };
                tar.append_data(&mut test_header(kind, contents.len()), "alias/changed", contents).unwrap();
                let bytes = tar.into_inner().unwrap();
                assert!(unpack(bytes.as_slice(), &root, &Skipped::new(&[]), &AtomicBool::new(false), &mut |_| {}).is_err());
                assert_eq!(fs::read(outside.join("changed")).unwrap(), b"protected outside data");
                assert!(!outside.join(".changed.skyward-restore").exists(), "no temporary file outside the restore root");
            }
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn an_entry_can_replace_a_leaf_symlink_without_following_it() {
        let at = scratch("replace-leaf-link");
        let (root, outside) = (at.join("root"), at.join("outside"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("kept"), b"outside").unwrap();
        symlink(&outside, root.join("folder")).unwrap();
        let mut tar = Builder::new(Vec::new());
        tar.append_data(&mut test_header(EntryType::Directory, 0), "folder", io::empty()).unwrap();
        tar.append_data(&mut test_header(EntryType::Regular, 8), "folder/file", b"restored".as_slice()).unwrap();
        let bytes = tar.into_inner().unwrap();
        let result = unpack(bytes.as_slice(), &root, &Skipped::new(&[]), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(result.unmade.is_empty());
        assert!(fs::symlink_metadata(root.join("folder")).unwrap().is_dir());
        assert_eq!(fs::read(root.join("folder/file")).unwrap(), b"restored");
        assert_eq!(fs::read(outside.join("kept")).unwrap(), b"outside");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn measuring_and_packing_stop_when_asked() {
        let at = scratch("stop");
        tree(&at.join("from"));
        let stop = AtomicBool::new(true);
        let e = measure(&at.join("from"), &skipped(), &stop).unwrap_err();
        assert!(e.is::<Cancelled>(), "{e:#}");
        let e = pack(&at.join("from"), &skipped(), io::sink(), &stop, &mut |_| {}).unwrap_err();
        assert!(e.is::<Cancelled>(), "{e:#}");
        let partway = AtomicBool::new(false);
        let e = pack(&at.join("from"), &skipped(), io::sink(), &partway, &mut |n| partway.store(n > 0, Ordering::Relaxed)).unwrap_err();
        assert!(e.is::<Cancelled>(), "stopped in the middle of a file: {e:#}");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn skipped_paths_match_name_by_name() {
        let s = Skipped::new(&["misc/vold".into(), "system_de/*/spblob".into()]);
        assert!(s.holds(Path::new("misc/vold")));
        assert!(s.holds(Path::new("system_de/999/spblob")));
        assert!(!s.holds(Path::new("misc/vold2")));
        assert!(!s.holds(Path::new("system_de/0")));
        assert!(s.covers(Path::new("misc/vold/user_keys/ce")));
        assert!(!s.covers(Path::new("misc/profiles")));
    }

    #[test]
    fn times_keep_their_nanoseconds() {
        assert_eq!(time("1792561872.000000123").unwrap(), (1_792_561_872, 123));
        assert_eq!(time("-5.5").unwrap(), (-5, 500_000_000));
        assert_eq!(time("86400").unwrap(), (86_400, 0));
    }
}
