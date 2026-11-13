use super::protection::{read_only, set_read_only};
use super::{flash, samples};
use crate::keep::Kept;
use skyward_vab::Slot;
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Command;

/// Only a newly allocated loop device backed by our own temporary file.
struct Loop {
    path: PathBuf,
    _backing: tempfile::TempDir,
}

impl Loop {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let backing = dir.path().join("partition.img");
        File::create(&backing).unwrap().set_len(4 << 20).unwrap();
        let output = Command::new("losetup").args(["--find", "--show", "--"]).arg(&backing).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        assert!(path.starts_with("/dev"));
        Self { path, _backing: dir }
    }

    fn ro(&self, on: bool) {
        set_read_only(&File::open(&self.path).unwrap(), on).unwrap();
        self.check_ro(on);
    }

    fn check_ro(&self, expected: bool) {
        assert_eq!(read_only(&File::open(&self.path).unwrap()).unwrap(), expected);
    }

    fn flash(&self, bytes: &[u8], len: u64) -> anyhow::Result<()> {
        flash(&mut Cursor::new(bytes), len, std::slice::from_ref(&self.path), &mut |_, _| {})
    }
}

impl Drop for Loop {
    fn drop(&mut self) {
        if let Ok(file) = File::open(&self.path) {
            let _ = set_read_only(&file, false);
        }
        let status = Command::new("losetup").arg("--detach").arg(&self.path).status();
        if !std::thread::panicking() {
            assert!(status.is_ok_and(|s| s.success()), "detach our disposable loop device");
        }
    }
}

#[test]
#[ignore = "requires root and a disposable loop device; never uses physical partitions"]
fn recovery_write_restores_block_flags_on_success_and_failure() {
    assert_eq!(unsafe { libc::geteuid() }, 0, "run the test binary with sudo");
    let device = Loop::new();
    let image: Vec<u8> = (0..100_000).map(|n| (n % 251) as u8).collect();
    device.ro(true);
    // This is the real pre-fix failure, not a mock of the writable policy.
    let blocked = File::options().write(true).open(&device.path).and_then(|mut file| file.write_all(&[0]));
    assert!(blocked.is_err());
    device.flash(&image, image.len() as u64).unwrap();
    device.check_ro(true);
    let back = fs::read(&device.path).unwrap();
    assert_eq!(&back[..image.len()], image);
    assert!(back[image.len()..].iter().all(|&b| b == 0));

    assert!(device.flash(&[], (4 << 20) + 1).is_err(), "oversized input");
    device.check_ro(true);
    assert!(device.flash(&[1; 100], 1 << 20).is_err(), "short input");
    device.check_ro(true);
    assert_eq!(fs::read(&device.path).unwrap(), back, "failed inputs did not write anything");

    let missing = device._backing.path().join("missing");
    assert!(flash(&mut Cursor::new(&[0; 16]), 16, &[device.path.clone(), missing], &mut |_, _| {}).is_err());
    device.check_ro(true);

    device.ro(false);
    device.flash(&image, image.len() as u64).unwrap();
    device.check_ro(false);

    // Exercise the automatic post-install preservation API itself, with
    // an OTA-style read-only next-slot target and a held Skyward image.
    let dir = tempfile::tempdir().unwrap();
    symlink(&device.path, dir.path().join("recovery_a")).unwrap();
    let mut skyward = samples::recovery(true);
    skyward.resize(4 << 20, 0);
    fs::write(dir.path().join("recovery_b"), &skyward).unwrap();
    let kept = Kept::hold(dir.path(), Slot::B).unwrap().unwrap();
    device.ro(true);
    let wanted = kept.wanted(Slot::A).unwrap();
    assert_eq!(wanted, [Slot::A]);
    kept.write(&wanted, &mut |_, _| {}).unwrap();
    device.check_ro(true);
    assert_eq!(fs::read(&device.path).unwrap(), skyward);
    assert!(kept.wanted(Slot::A).unwrap().is_empty());
}
