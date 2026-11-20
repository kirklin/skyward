//! The entries of one storage the computer has been told of: a handle for
//! each file and folder, given as the computer reads folders and kept to
//! the same entry while it is renamed or moved; the folders read are
//! followed as other programs change them.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

/// The top of the storage, which the computer names by no handle.
pub const TOP: u32 = 0;

#[derive(Debug)]
pub enum Error {
    /// No entry has the handle.
    Unknown,
    /// The entry is no folder.
    NotFolder,
    Io(io::Error),
}

/// A change another program made in a folder the computer has read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added(u32),
    Removed(u32),
    /// A file written to and closed.
    Written(u32),
}

#[derive(Debug)]
struct Entry {
    parent: u32,
    name: OsString,
    /// What a folder holds; a file has none.
    folder: Option<Folder>,
}

#[derive(Debug, Default)]
struct Folder {
    entries: BTreeMap<OsString, u32>,
    /// Every entry of the folder is known.
    read: bool,
    /// The folder is followed, so what is known stays true; one that is
    /// not is read again each time it is asked for.
    watch: Option<i32>,
}

pub struct Store {
    top: PathBuf,
    entries: HashMap<u32, Entry>,
    /// The top folder's own entries.
    root: Folder,
    next: u32,
    watch: Watch,
    /// Changes found while reading folders again, told with the next ones.
    found: Vec<Change>,
}

impl Store {
    pub fn new(top: PathBuf) -> io::Result<Store> {
        Ok(Store { top, entries: HashMap::new(), root: Folder::default(), next: 1, watch: Watch::new()?, found: Vec::new() })
    }

    pub fn top(&self) -> &Path {
        &self.top
    }

    /// What to wait on for changes to the folders followed, where the
    /// system can follow them.
    pub fn fd(&self) -> Option<RawFd> {
        self.watch.fd()
    }

    pub fn contains(&self, h: u32) -> bool {
        self.entries.contains_key(&h)
    }

    pub fn path(&self, h: u32) -> Option<PathBuf> {
        let mut names = Vec::new();
        let mut at = h;
        while at != TOP {
            let e = self.entries.get(&at)?;
            names.push(e.name.as_os_str());
            at = e.parent;
        }
        let mut path = self.top.clone();
        path.extend(names.iter().rev());
        Some(path)
    }

    /// The path from the top of the storage, as the persistent ID takes it.
    pub fn relative(&self, h: u32) -> Option<PathBuf> {
        let path = self.path(h)?;
        Some(path.strip_prefix(&self.top).expect("every path starts at the top").to_path_buf())
    }

    pub fn name(&self, h: u32) -> Option<&OsStr> {
        self.entries.get(&h).map(|e| e.name.as_os_str())
    }

    /// The folder an entry is in: [`TOP`] for one at the top.
    pub fn parent(&self, h: u32) -> Option<u32> {
        self.entries.get(&h).map(|e| e.parent)
    }

    pub fn is_folder(&self, h: u32) -> bool {
        h == TOP || self.entries.get(&h).is_some_and(|e| e.folder.is_some())
    }

    /// The handle of the entry named `name` in `folder`, among those known.
    pub fn child(&self, folder: u32, name: &OsStr) -> Option<u32> {
        self.folder(folder).ok()?.entries.get(name).copied()
    }

    fn folder(&self, h: u32) -> Result<&Folder, Error> {
        if h == TOP {
            return Ok(&self.root);
        }
        self.entries.get(&h).ok_or(Error::Unknown)?.folder.as_ref().ok_or(Error::NotFolder)
    }

    fn folder_mut(&mut self, h: u32) -> Result<&mut Folder, Error> {
        if h == TOP {
            return Ok(&mut self.root);
        }
        self.entries.get_mut(&h).ok_or(Error::Unknown)?.folder.as_mut().ok_or(Error::NotFolder)
    }

    /// The handles of a folder's entries.
    pub fn children(&mut self, folder: u32) -> Result<Vec<u32>, Error> {
        let f = self.folder(folder)?;
        if !f.read || f.watch.is_none() {
            self.read(folder, false)?;
        }
        Ok(self.folder(folder)?.entries.values().copied().collect())
    }

    /// The handles of every entry under a folder, in every folder below it.
    pub fn descendants(&mut self, folder: u32) -> Result<Vec<u32>, Error> {
        let mut out = Vec::new();
        let mut folders = vec![folder];
        while let Some(f) = folders.pop() {
            for h in self.children(f)? {
                out.push(h);
                if self.is_folder(h) {
                    folders.push(h);
                }
            }
        }
        Ok(out)
    }

    /// Read a folder from the disk and follow it from then on. A known
    /// entry no longer there goes, and the computer, which may hold its
    /// handle, is told; one new there is told only with `tell_added`, as
    /// whoever asked for the folder learns of it with its handle.
    fn read(&mut self, folder: u32, tell_added: bool) -> Result<(), Error> {
        let path = self.path(folder).ok_or(Error::Unknown)?;
        if self.folder(folder)?.watch.is_none() {
            let watch = self.watch.add(&path, folder);
            self.folder_mut(folder)?.watch = watch;
        }
        let mut on_disk = BTreeMap::new();
        for entry in fs::read_dir(&path).map_err(Error::Io)? {
            let entry = entry.map_err(Error::Io)?;
            on_disk.insert(entry.file_name(), is_dir(&entry.path()));
        }
        let known: Vec<(OsString, u32)> = self.folder(folder)?.entries.iter().map(|(n, &h)| (n.clone(), h)).collect();
        for (name, h) in known {
            if on_disk.get(&name) != Some(&self.is_folder(h)) {
                self.remove(h);
                self.found.push(Change::Removed(h));
            }
        }
        for (name, dir) in on_disk {
            if self.child(folder, &name).is_none() {
                let h = self.add(folder, name, dir);
                if tell_added {
                    self.found.push(Change::Added(h));
                }
            }
        }
        self.folder_mut(folder)?.read = true;
        Ok(())
    }

    /// A new entry in `folder`, which takes the place of any of its name.
    pub fn add(&mut self, folder: u32, name: OsString, is_folder: bool) -> u32 {
        if let Some(old) = self.child(folder, &name) {
            self.remove(old);
        }
        let h = self.next;
        self.next = self.next.checked_add(1).expect("a session names fewer than four billion entries");
        self.folder_mut(folder).expect("entries go into a known folder").entries.insert(name.clone(), h);
        self.entries.insert(h, Entry { parent: folder, name, folder: is_folder.then(Folder::default) });
        h
    }

    /// Forget an entry, and everything in it.
    pub fn remove(&mut self, h: u32) {
        let Some(entry) = self.entries.remove(&h) else { return };
        if let Ok(parent) = self.folder_mut(entry.parent) {
            parent.entries.remove(&entry.name);
        }
        if let Some(folder) = entry.folder {
            self.forget(folder);
        }
    }

    fn forget(&mut self, folder: Folder) {
        if let Some(wd) = folder.watch {
            self.watch.remove(wd);
        }
        for h in folder.entries.into_values() {
            if let Some(Entry { folder: Some(inner), .. }) = self.entries.remove(&h) {
                self.forget(inner);
            }
        }
    }

    /// The entry has a new name in its folder, which takes the place of
    /// any entry of that name.
    pub fn rename(&mut self, h: u32, name: OsString) {
        let parent = self.parent(h).expect("a known entry is renamed");
        self.relocate(h, parent, name);
    }

    /// The entry is in another folder now, under `name`.
    pub fn relocate(&mut self, h: u32, folder: u32, name: OsString) {
        if let Some(old) = self.child(folder, &name).filter(|&old| old != h) {
            self.remove(old);
        }
        let entry = self.entries.get_mut(&h).expect("a known entry is moved");
        let (from, old) = (entry.parent, std::mem::replace(&mut entry.name, name.clone()));
        entry.parent = folder;
        if let Ok(parent) = self.folder_mut(from) {
            parent.entries.remove(&old);
        }
        self.folder_mut(folder).expect("entries go into a known folder").entries.insert(name, h);
    }

    /// What other programs changed in the folders followed since last
    /// asked, as the known entries now stand.
    pub fn changes(&mut self) -> Vec<Change> {
        for raw in self.watch.read() {
            self.apply(raw);
        }
        std::mem::take(&mut self.found)
    }

    fn apply(&mut self, raw: Raw) {
        match raw {
            // The system lost changes: every folder followed is read again.
            Raw::Overflow => {
                let followed: Vec<u32> = std::iter::once(TOP)
                    .chain(self.entries.iter().filter(|(_, e)| e.folder.as_ref().is_some_and(|f| f.read)).map(|(&h, _)| h))
                    .collect();
                for folder in followed {
                    if self.folder(folder).is_ok_and(|f| f.read) {
                        let _ = self.read(folder, true);
                    }
                }
            }
            Raw::Gone(wd) => {
                if let Some(folder) = self.watch.forget(wd)
                    && let Ok(f) = self.folder_mut(folder)
                    && f.watch == Some(wd)
                {
                    f.watch = None;
                }
            }
            Raw::Appeared(wd, name) => {
                let Some(folder) = self.watch.folder(wd) else { return };
                if self.child(folder, &name).is_some() {
                    return;
                }
                let Some(path) = self.path(folder) else { return };
                let path = path.join(&name);
                if fs::symlink_metadata(&path).is_ok() {
                    let h = self.add(folder, name, is_dir(&path));
                    self.found.push(Change::Added(h));
                }
            }
            Raw::Vanished(wd, name) => {
                let Some(folder) = self.watch.folder(wd) else { return };
                let Some(h) = self.child(folder, &name) else { return };
                let path = self.path(h).expect("a known entry has a path");
                if fs::symlink_metadata(&path).is_err() {
                    self.remove(h);
                    self.found.push(Change::Removed(h));
                }
            }
            Raw::Written(wd, name) => {
                if let Some(h) = self.watch.folder(wd).and_then(|folder| self.child(folder, &name)) {
                    self.found.push(Change::Written(h));
                }
            }
        }
    }
}

/// Whether `path` is a folder, through any link to one.
fn is_dir(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// What the system said happened in a folder followed. A system with no
/// inotify says nothing.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
enum Raw {
    /// Changes were lost.
    Overflow,
    /// The folder is followed no longer: it is gone, or its filesystem.
    Gone(i32),
    Appeared(i32, OsString),
    Vanished(i32, OsString),
    Written(i32, OsString),
}

/// Following folders with inotify.
#[cfg(any(target_os = "linux", target_os = "android"))]
struct Watch {
    fd: std::os::fd::OwnedFd,
    folders: HashMap<i32, u32>,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl Watch {
    const MASK: u32 = libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_CLOSE_WRITE | libc::IN_ONLYDIR;

    fn new() -> io::Result<Watch> {
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Watch { fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }, folders: HashMap::new() })
    }

    fn fd(&self) -> Option<RawFd> {
        use std::os::fd::AsRawFd;
        Some(self.fd.as_raw_fd())
    }

    /// Follow the folder at `path`; `None` where the system follows no
    /// more, and the folder is read again each time instead.
    fn add(&mut self, path: &Path, folder: u32) -> Option<i32> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let wd = unsafe { libc::inotify_add_watch(self.fd.as_raw_fd(), c.as_ptr(), Self::MASK) };
        if wd < 0 {
            return None;
        }
        self.folders.insert(wd, folder);
        Some(wd)
    }

    fn remove(&mut self, wd: i32) {
        use std::os::fd::AsRawFd;
        if self.folders.remove(&wd).is_some() {
            // Bionic takes the descriptor unsigned, glibc signed.
            unsafe { libc::inotify_rm_watch(self.fd.as_raw_fd(), wd as _) };
        }
    }

    fn folder(&self, wd: i32) -> Option<u32> {
        self.folders.get(&wd).copied()
    }

    fn forget(&mut self, wd: i32) -> Option<u32> {
        self.folders.remove(&wd)
    }

    /// What happened since last read, in order.
    fn read(&mut self) -> Vec<Raw> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStringExt;
        use zerocopy::FromBytes;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                return out;
            }
            let mut at = &buf[..n as usize];
            while let Ok((event, rest)) = InotifyEvent::read_from_prefix(at) {
                let (name, rest) = rest.split_at(event.len as usize);
                at = rest;
                let name = OsString::from_vec(name.iter().copied().take_while(|&b| b != 0).collect());
                let (wd, mask) = (event.wd, event.mask);
                // What happens to a folder itself comes without a name; only
                // its going away matters, which IN_IGNORED tells.
                let raw = if mask & libc::IN_Q_OVERFLOW != 0 {
                    Some(Raw::Overflow)
                } else if mask & libc::IN_IGNORED != 0 {
                    Some(Raw::Gone(wd))
                } else if name.is_empty() {
                    None
                } else if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
                    Some(Raw::Appeared(wd, name))
                } else if mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
                    Some(Raw::Vanished(wd, name))
                } else if mask & libc::IN_CLOSE_WRITE != 0 {
                    Some(Raw::Written(wd, name))
                } else {
                    None
                };
                out.extend(raw);
            }
        }
    }
}

/// struct inotify_event, before its name.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[derive(zerocopy::FromBytes, zerocopy::KnownLayout, zerocopy::Immutable)]
#[repr(C)]
struct InotifyEvent {
    wd: i32,
    mask: u32,
    cookie: u32,
    len: u32,
}

/// A system with no inotify follows nothing: each folder is read again
/// whenever it is asked for.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
struct Watch;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl Watch {
    fn new() -> io::Result<Watch> {
        Ok(Watch)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }

    fn add(&mut self, _path: &Path, _folder: u32) -> Option<i32> {
        None
    }

    fn remove(&mut self, _wd: i32) {}

    fn folder(&self, _wd: i32) -> Option<u32> {
        None
    }

    fn forget(&mut self, _wd: i32) -> Option<u32> {
        None
    }

    fn read(&mut self) -> Vec<Raw> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, empty, under `TMPDIR`.
    fn scratch(test: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-mtp-{test}-{}", std::process::id()));
        if at.exists() {
            fs::remove_dir_all(&at).unwrap();
        }
        fs::create_dir_all(&at).unwrap();
        at
    }

    fn names(store: &Store, handles: &[u32]) -> Vec<String> {
        let mut out: Vec<String> = handles.iter().map(|&h| store.name(h).unwrap().to_string_lossy().into_owned()).collect();
        out.sort();
        out
    }

    #[test]
    fn folders_are_read_with_handles_that_stay_their_entries() {
        let at = scratch("read");
        fs::create_dir_all(at.join("DCIM/Camera")).unwrap();
        fs::write(at.join("DCIM/Camera/IMG_1.jpg"), b"jpeg").unwrap();
        fs::write(at.join("notes.txt"), b"n").unwrap();
        let mut store = Store::new(at.clone()).unwrap();
        let top = store.children(TOP).unwrap();
        assert_eq!(names(&store, &top), ["DCIM", "notes.txt"]);
        assert_eq!(store.children(TOP).unwrap(), top, "the same handles each time");
        let dcim = store.child(TOP, OsStr::new("DCIM")).unwrap();
        assert!(store.is_folder(dcim) && !store.is_folder(store.child(TOP, OsStr::new("notes.txt")).unwrap()));
        let camera = store.children(dcim).unwrap()[0];
        let photo = store.children(camera).unwrap()[0];
        assert_eq!(store.path(photo).unwrap(), at.join("DCIM/Camera/IMG_1.jpg"));
        assert_eq!(store.relative(photo).unwrap(), Path::new("DCIM/Camera/IMG_1.jpg"));
        assert_eq!(store.parent(photo), Some(camera));
        assert_eq!(store.parent(dcim), Some(TOP));
        assert!(matches!(store.children(photo), Err(Error::NotFolder)));
        assert!(matches!(store.children(9999), Err(Error::Unknown)));
        let all = store.descendants(TOP).unwrap();
        assert_eq!(names(&store, &all), ["Camera", "DCIM", "IMG_1.jpg", "notes.txt"]);
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn entries_keep_their_handles_through_renames_and_moves() {
        let at = scratch("moves");
        fs::create_dir_all(at.join("a")).unwrap();
        fs::create_dir_all(at.join("b")).unwrap();
        fs::write(at.join("a/x.txt"), b"x").unwrap();
        let mut store = Store::new(at.clone()).unwrap();
        store.children(TOP).unwrap();
        let (a, b) = (store.child(TOP, OsStr::new("a")).unwrap(), store.child(TOP, OsStr::new("b")).unwrap());
        let x = store.children(a).unwrap()[0];
        fs::rename(at.join("a/x.txt"), at.join("a/y.txt")).unwrap();
        store.rename(x, "y.txt".into());
        assert_eq!(store.path(x).unwrap(), at.join("a/y.txt"));
        fs::rename(at.join("a/y.txt"), at.join("b/y.txt")).unwrap();
        store.relocate(x, b, "y.txt".into());
        assert_eq!(store.path(x).unwrap(), at.join("b/y.txt"));
        assert_eq!(store.children(b).unwrap(), [x], "a folder read after an entry moved in keeps its handle");
        assert!(store.children(a).unwrap().is_empty());
        assert!(store.changes().is_empty(), "nothing changed but what the session did");
        let made = store.add(a, "new".into(), true);
        fs::create_dir(at.join("a/new")).unwrap();
        assert!(store.children(made).unwrap().is_empty());
        store.remove(a);
        assert!(!store.contains(a) && !store.contains(made), "an entry goes with everything in it");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_folder_read_again_drops_what_is_gone_and_tells_the_computer() {
        let at = scratch("again");
        fs::write(at.join("a.txt"), b"a").unwrap();
        let mut store = Store::new(at.clone()).unwrap();
        let a = store.children(TOP).unwrap()[0];
        // Another program takes a.txt away and puts b.txt in, unfollowed:
        // reading the folder again finds both.
        fs::remove_file(at.join("a.txt")).unwrap();
        fs::write(at.join("b.txt"), b"b").unwrap();
        store.folder_mut(TOP).unwrap().watch = None;
        let now = store.children(TOP).unwrap();
        assert_eq!(names(&store, &now), ["b.txt"]);
        assert!(!store.contains(a));
        let changes = store.changes();
        assert!(changes.contains(&Change::Removed(a)), "{changes:?}");
        assert!(!changes.iter().any(|c| matches!(c, Change::Added(_))), "whoever asked learns of b.txt from the list: {changes:?}");
        fs::remove_dir_all(&at).unwrap();
    }

    /// On the phone: changes other programs make in folders the computer
    /// has read are told, and the session's own are not.
    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_other_programs_changes_are_told_once() {
        let at = scratch("watch");
        fs::create_dir_all(at.join("sub")).unwrap();
        let mut store = Store::new(at.clone()).unwrap();
        store.children(TOP).unwrap();
        let sub = store.child(TOP, OsStr::new("sub")).unwrap();
        fs::write(at.join("made.txt"), b"m").unwrap();
        let changes = store.changes();
        let made = store.child(TOP, OsStr::new("made.txt")).expect("a file another program made is known");
        assert_eq!(changes, [Change::Added(made), Change::Written(made)]);
        fs::remove_file(at.join("made.txt")).unwrap();
        assert_eq!(store.changes(), [Change::Removed(made)]);
        fs::rename(at.join("sub"), at.join("moved")).unwrap();
        let changes = store.changes();
        let moved = store.child(TOP, OsStr::new("moved")).unwrap();
        assert_eq!(changes, [Change::Removed(sub), Change::Added(moved)]);
        // What the session does itself it records first.
        let own = store.add(TOP, "own.txt".into(), false);
        fs::write(at.join("own.txt"), b"o").unwrap();
        assert_eq!(store.changes(), [Change::Written(own)], "only that it was written");
        fs::remove_file(at.join("own.txt")).unwrap();
        store.remove(own);
        assert!(store.changes().is_empty());
        fs::remove_dir_all(&at).unwrap();
    }
}
