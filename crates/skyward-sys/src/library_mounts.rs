//! Per-process library replacements without changing an executable or its
//! file capabilities. Preparing a plan opens its sources and allocates all
//! paths; applying it after fork uses only system calls and prepared data.
//! A bind keeps the source inode's SELinux label. This module never changes
//! labels or policy: the service's ability to map that label is a runtime gate.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(file: &File) -> io::Result<Self> {
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a library override must name an existing regular file"));
        }
        Ok(Self { device: meta.dev(), inode: meta.ino() })
    }
}

#[derive(Debug)]
struct Mount {
    // Keep this descriptor alive until exec. A later rename of the source
    // pathname must not silently change the component used on a restart.
    _source: File,
    // Reserve a descriptor number before fork so its /proc/self/fd path
    // is also prepared. In the child it is rebound to the same inode in
    // the child's new mount namespace, using open/fstat/dup3 only.
    mount_source: File,
    source: CString,
    source_path: CString,
    source_identity: Identity,
    target_path: CString,
    target: PathBuf,
    target_identity: Identity,
    remount_flags: libc::c_ulong,
}

/// Sources are opened read-only and CLOEXEC; the mount pins their inodes
/// after exec closes those descriptors. The supplier must keep their bytes
/// immutable, since holding an inode alone does not prevent in-place writes.
#[derive(Debug, Default)]
pub struct LibraryMounts {
    mounts: Vec<Mount>,
}

impl LibraryMounts {
    /// Resolve and validate everything before fork. Source and target must
    /// be absolute, existing regular files, not symbolic links.
    pub fn prepare<I, S, T>(overrides: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = (S, T)>,
        S: AsRef<Path>,
        T: AsRef<Path>,
    {
        let mut mounts = Vec::new();
        let mut targets = BTreeSet::new();
        for (source, target) in overrides {
            let (source, target) = (source.as_ref(), target.as_ref());
            absolute(source)?;
            absolute(target)?;
            if !targets.insert(target.to_path_buf()) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "duplicate library override target"));
            }
            let open = |path: &Path| OpenOptions::new().read(true).custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path);
            let source_name = CString::new(source.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in library source"))?;
            let source = open(source)?;
            let target_file = open(target)?;
            let source_identity = Identity::of(&source)?;
            let target_identity = Identity::of(&target_file)?;
            // A bind inherits the source mount's flags. Do not loosen the
            // security flags of either side when making that bind read-only.
            let security_flags = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
            let flags = (mount_flags(&source)? | mount_flags(&target_file)?) & security_flags;
            let mount_source = source.try_clone()?;
            mounts.push(Mount {
                source_path: CString::new(format!("/proc/self/fd/{}", mount_source.as_raw_fd())).unwrap(),
                _source: source,
                mount_source,
                source: source_name,
                source_identity,
                target_path: CString::new(target.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in library target"))?,
                target: target.to_path_buf(),
                target_identity,
                remount_flags: libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | flags,
            });
        }
        Ok(Self { mounts })
    }

    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    /// Apply after `private_namespace`, before dropping credentials. No
    /// allocation, filesystem lookup through Rust, logging or label changes
    /// are performed here. Any failure aborts this child before exec.
    pub fn apply(&self) -> io::Result<()> {
        for mount in &self.mounts {
            let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
            done(unsafe { libc::lstat(mount.target_path.as_ptr(), &mut stat) })?;
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG
                || stat.st_dev != mount.target_identity.device
                || stat.st_ino != mount.target_identity.inode
            {
                return Err(io::Error::from_raw_os_error(libc::ESTALE));
            }
            // A descriptor opened before unshare refers to a mount in
            // the parent namespace; Linux refuses it as an MS_BIND source.
            // Reopen in this namespace, then compare with the pinned inode
            // before using the reserved FD. A changed path fails closed.
            let fd = unsafe { libc::open(mount.source.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let result = match done(unsafe { libc::fstat(fd, &mut stat) }) {
                Err(error) => Err(error),
                Ok(()) if stat.st_mode & libc::S_IFMT != libc::S_IFREG
                    || stat.st_dev != mount.source_identity.device
                    || stat.st_ino != mount.source_identity.inode => Err(io::Error::from_raw_os_error(libc::ESTALE)),
                Ok(()) => {
                    if unsafe { libc::dup3(fd, mount.mount_source.as_raw_fd(), libc::O_CLOEXEC) } < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                }
            };
            unsafe { libc::close(fd) };
            result?;
            done(unsafe {
                libc::mount(mount.source_path.as_ptr(), mount.target_path.as_ptr(), std::ptr::null(), libc::MS_BIND, std::ptr::null())
            })?;
            done(unsafe {
                libc::mount(std::ptr::null(), mount.target_path.as_ptr(), std::ptr::null(), mount.remount_flags, std::ptr::null())
            })?;
        }
        Ok(())
    }

    /// An old process may have the right /proc/exe yet still see the stock
    /// library. Reject that collision rather than starting a duplicate or
    /// waiting on an instance with different code. Compare with the retained
    /// source inode, not a source pathname that may since have been replaced.
    pub fn verify_process(&self, pid: u32) -> io::Result<()> {
        for mount in &self.mounts {
            let actual = OpenOptions::new().read(true).custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(process_path(pid, &mount.target)?)?;
            if Identity::of(&actual)? != mount.source_identity {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("pid {pid} sees a different library at {}", mount.target.display()),
                ));
            }
        }
        Ok(())
    }
}

/// Enter once for all of a child's private mounts, including its RAM /data.
/// Mount events must not propagate back into the recovery or its siblings.
pub fn private_namespace() -> io::Result<()> {
    done(unsafe { libc::unshare(libc::CLONE_NEWNS) })?;
    done(unsafe {
        libc::mount(std::ptr::null(), c"/".as_ptr(), std::ptr::null(), libc::MS_REC | libc::MS_PRIVATE, std::ptr::null())
    })
}

/// Open a logical absolute pathname through a process's own mount view.
/// Keep the PID attached until the actual file has been opened; identical
/// path strings in two namespaces need not name the same inode.
pub fn process_path(pid: u32, path: &Path) -> io::Result<PathBuf> {
    absolute(path)?;
    Ok(PathBuf::from(format!("/proc/{pid}/root")).join(path.strip_prefix("/").unwrap()))
}

fn absolute(path: &Path) -> io::Result<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "a library path must be absolute without parent traversal"))
    } else {
        Ok(())
    }
}

fn mount_flags(file: &File) -> io::Result<libc::c_ulong> {
    let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
    done(unsafe { libc::fstatvfs(file.as_raw_fd(), &mut stat) })?;
    // Linux ST_NOSUID/NODEV/NOEXEC use the corresponding MS_* bit values.
    Ok(stat.f_flag)
}

fn done(result: libc::c_int) -> io::Result<()> {
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}
