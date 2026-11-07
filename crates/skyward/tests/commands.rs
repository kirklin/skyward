//! The commands install scripts run, run as the scripts run them: by name,
//! from a folder of links to this binary on their PATH, in a pipe.

use std::fs;
use std::io::{Cursor, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Bytes that do not repeat, so they keep their size compressed.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x6a09_e667_f3bc_c908u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// A ROM's zip, with its install script and its super image as zstd
/// compressed it, stored as such zips store it.
fn rom(dir: &Path, super_zst: &[u8]) -> PathBuf {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let script = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    w.start_file("META-INF/com/google/android/update-binary", script).unwrap();
    w.write_all(b"#!/sbin/sh\n").unwrap();
    w.start_file("images/super.img.zst", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
    w.write_all(super_zst).unwrap();
    let path = dir.join("rom.zip");
    fs::write(&path, w.finish().unwrap().into_inner()).unwrap();
    path
}

/// Run `script` with sh, the commands linked to this binary on its PATH
/// as the ramdisk links them, and `ZIPFILE` and `OUT` set.
fn script(dir: &Path, script: &str, zip: &Path, out: &Path) -> ExitStatus {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for command in skyward_config::ramdisk::COMMANDS {
        let link = bin.join(Path::new(command).file_name().unwrap());
        if !link.exists() {
            symlink(env!("CARGO_BIN_EXE_skyward"), link).unwrap();
        }
    }
    Command::new("sh")
        .args(["-c", script])
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("ZIPFILE", zip)
        .env("OUT", out)
        .status()
        .unwrap()
}

/// The two ways ROMs' scripts pour their super image out of their zip.
const FLASH_SUPER: [&str; 2] = [
    r#"7za e -so "$ZIPFILE" "images/super.img.zst" | zstd -c -d > "$OUT""#,
    r#"unzip -p "$ZIPFILE" images/super.img.zst | zstd -c -d > "$OUT""#,
];

#[test]
fn a_rom_s_script_pours_its_super_image_out_of_the_zip_through_zstd() {
    let dir = tempfile::tempdir().unwrap();
    let image = noise(8 << 20);
    let zip = rom(dir.path(), &zstd::encode_all(image.as_slice(), 3).unwrap());
    let out = dir.path().join("super");
    for flash in FLASH_SUPER {
        assert!(script(dir.path(), flash, &zip, &out).success(), "{flash}");
        assert!(fs::read(&out).unwrap() == image, "{flash}");
        fs::remove_file(&out).unwrap();
    }
}

#[test]
fn a_super_image_cut_short_in_the_zip_fails_the_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let whole = zstd::encode_all(noise(1 << 20).as_slice(), 3).unwrap();
    let zip = rom(dir.path(), &whole[..whole.len() / 2]);
    for flash in FLASH_SUPER {
        assert!(!script(dir.path(), flash, &zip, &dir.path().join("super")).success(), "{flash}");
    }
}
