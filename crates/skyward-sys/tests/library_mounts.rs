#![cfg(target_os = "linux")]

use skyward_sys::library_mounts::{process_path, private_namespace, LibraryMounts};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{symlink, MetadataExt};
use std::os::unix::process::CommandExt;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("skyward-library-mounts-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn validates_before_fork() {
    let scratch = Scratch::new();
    let source = scratch.0.join("new.so");
    let target = scratch.0.join("old.so");
    fs::write(&source, b"new").unwrap();
    fs::write(&target, b"old").unwrap();
    assert!(!LibraryMounts::prepare([(&source, &target)]).unwrap().is_empty());
    assert!(LibraryMounts::prepare([(Path::new("relative"), &target)]).is_err());
    assert!(LibraryMounts::prepare([(&scratch.0, &target)]).is_err());
    assert!(LibraryMounts::prepare([(&source, &scratch.0)]).is_err());
    assert!(LibraryMounts::prepare([(&source, &scratch.0.join("missing"))]).is_err());
    assert!(LibraryMounts::prepare([(&source, &target), (&source, &target)]).is_err());
    let link = scratch.0.join("link.so");
    symlink(&source, &link).unwrap();
    assert!(LibraryMounts::prepare([(&link, &target)]).is_err());
    assert!(LibraryMounts::prepare([(&source, &link)]).is_err());
    assert!(process_path(1, Path::new("/tmp/../escape")).is_err());
}

struct Probe {
    child: Child,
    release: PathBuf,
}

impl Probe {
    fn start(root: &Path, role: &str, mounts: Option<Arc<LibraryMounts>>) -> Self {
        let ready = root.join(format!("ready-{role}"));
        let release = root.join(format!("release-{role}"));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--ignored", "--exact", "private_bind_mounts_are_isolated", "--test-threads=1", "--nocapture"])
            .env("SKYWARD_MOUNT_PROBE", role)
            .env("SKYWARD_MOUNT_ROOT", root);
        if let Some(mounts) = mounts {
            // This is the same order used by services::command: establish
            // one namespace, then apply only the prepared syscall plan.
            unsafe {
                command.pre_exec(move || {
                    private_namespace()?;
                    mounts.apply()
                });
            }
        }
        let child = command.spawn().expect("spawn probe (root with CAP_SYS_ADMIN is required for a mount namespace)");
        let mut probe = Self { child, release };
        for _ in 0..500 {
            if ready.is_file() {
                return probe;
            }
            assert!(probe.child.try_wait().unwrap().is_none(), "probe exited before reporting its view");
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("probe never reported its view");
    }

    fn finish(&mut self) {
        fs::write(&self.release, b"done").unwrap();
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        // Only our own child is reaped, including when an assertion fails.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn child_probe(role: &str, root: &Path) {
    let expected = if role == "old" { b"old" } else { b"new" };
    assert_eq!(fs::read(root.join("target.so")).unwrap(), expected);
    if role != "old" {
        let error = OpenOptions::new().write(true).open(root.join("target.so")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EROFS), "the bind itself must be read-only even to root");
    }
    let file = File::open(root.join("target.so")).unwrap();
    let mapped = unsafe { libc::mmap(std::ptr::null_mut(), 3, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    assert_ne!(mapped, libc::MAP_FAILED);
    assert_eq!(unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), 3) }, expected);
    fs::write(root.join(format!("ready-{role}")), b"ready").unwrap();
    let release = root.join(format!("release-{role}"));
    while !release.is_file() {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unsafe { libc::munmap(mapped, 3) }, 0);
}

/// Build normally, then run this test binary via sudo with --ignored.
/// It does not modify the parent mount namespace or any system path.
#[test]
#[ignore = "requires root and CAP_SYS_ADMIN; performs actual private file bind mounts"]
fn private_bind_mounts_are_isolated() {
    if let Ok(role) = std::env::var("SKYWARD_MOUNT_PROBE") {
        child_probe(&role, &PathBuf::from(std::env::var_os("SKYWARD_MOUNT_ROOT").unwrap()));
        return;
    }
    assert_eq!(unsafe { libc::geteuid() }, 0, "run the test binary via sudo");
    let scratch = Scratch::new();
    let (source, target) = (scratch.0.join("source.so"), scratch.0.join("target.so"));
    fs::write(&source, b"new").unwrap();
    fs::write(&target, b"old").unwrap();
    let original = fs::metadata(&target).unwrap();
    let mounts = Arc::new(LibraryMounts::prepare([(&source, &target)]).unwrap());
    // Replacing the pathname must not replace the already prepared inode.
    fs::rename(&source, scratch.0.join("held-source.so")).unwrap();
    fs::write(&source, b"replacement").unwrap();

    let mut old = Probe::start(&scratch.0, "old", None);
    let error = mounts.verify_process(old.child.id()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "an unmodified old instance must be rejected for adoption");

    let mut replaced = Command::new(std::env::current_exe().unwrap());
    let changed = Arc::clone(&mounts);
    unsafe {
        replaced.pre_exec(move || {
            private_namespace()?;
            changed.apply()
        });
    }
    assert_eq!(replaced.spawn().unwrap_err().raw_os_error(), Some(libc::ESTALE), "a replaced source inode must fail before exec");
    fs::rename(&source, scratch.0.join("rejected-source.so")).unwrap();
    fs::rename(scratch.0.join("held-source.so"), &source).unwrap();

    for role in ["new", "restarted"] {
        let mut new = Probe::start(&scratch.0, role, Some(Arc::clone(&mounts)));
        mounts.verify_process(new.child.id()).expect("a correctly covered instance can be adopted");
        assert_eq!(fs::read(process_path(new.child.id(), &target).unwrap()).unwrap(), b"new");
        let actual = fs::metadata(process_path(new.child.id(), &target).unwrap()).unwrap();
        assert_ne!(actual.ino(), original.ino());
        let maps = fs::read_to_string(format!("/proc/{}/maps", new.child.id())).unwrap();
        assert!(maps.lines().any(|line| line.split_whitespace().nth(4).and_then(|n| n.parse::<u64>().ok()) == Some(actual.ino())), "pin's child-view file must be an actually mapped inode");
        assert_eq!(fs::read(process_path(old.child.id(), &target).unwrap()).unwrap(), b"old", "a sibling retains the original view");
        assert_eq!(fs::read(&target).unwrap(), b"old", "the parent retains the original view");
        assert_eq!(fs::read_link(format!("/proc/{}/exe", new.child.id())).unwrap(), std::env::current_exe().unwrap());
        new.finish();
    }
    old.finish();
    let after = fs::metadata(&target).unwrap();
    assert_eq!((after.dev(), after.ino()), (original.dev(), original.ino()));
    assert_eq!(fs::read(&target).unwrap(), b"old", "the underlying target was never overwritten");
    assert_eq!(fs::read(&source).unwrap(), b"new");
    assert_eq!(fs::read(scratch.0.join("rejected-source.so")).unwrap(), b"replacement");
}
