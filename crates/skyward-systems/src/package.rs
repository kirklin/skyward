//! A system packed to be installed beside the installed one: a zip holding
//! what the system is in `system.toml`, the boot image the bootloader
//! starts, the image of its root filesystem, and, where the system brings
//! its own first ramdisk, its init_boot image.

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::io::{Read, Seek};
use zip::ZipArchive;

pub const MANIFEST: &str = "system.toml";
pub const BOOT: &str = "boot.img";
pub const INIT_BOOT: &str = "init_boot.img";
pub const ROOT: &str = "rootfs.img";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// What a request to switch to the system names it by: lower-case
    /// letters, digits and `.-_`, at most 32.
    pub id: String,
    pub name: String,
    pub version: String,
    /// The filesystem its root image is, as the kernel names it.
    pub filesystem: String,
    /// The program its root starts with.
    #[serde(default = "default_init")]
    pub init: String,
    /// The least room its root takes, in bytes, where the image alone is
    /// too little to run in.
    #[serde(default)]
    pub minimum: u64,
}

fn default_init() -> String {
    "/sbin/init".into()
}

/// What a package holds, as far as installing it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contents {
    pub manifest: Manifest,
    /// The boot image's bytes.
    pub boot: u64,
    /// The root image's bytes.
    pub root: u64,
    /// The system brings its own init_boot image.
    pub init_boot: bool,
}

impl Contents {
    /// The least the system's root takes: the image, or what the system
    /// asks for where that is more.
    pub fn least(&self) -> u64 {
        self.root.max(self.manifest.minimum)
    }
}

/// Whether the zip is a system to install beside the installed one.
pub fn is_system<R: Read + Seek>(zip: &ZipArchive<R>) -> bool {
    zip.index_for_name(MANIFEST).is_some()
}

/// Read what the package holds, and check it is whole: a manifest the
/// recovery can go by, a boot image and a root image.
pub fn read<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<Contents> {
    let mut text = String::new();
    zip.by_name(MANIFEST).context("open system.toml")?.read_to_string(&mut text).context("read system.toml")?;
    let manifest: Manifest = toml::from_str(&text).context("read system.toml")?;
    check(&manifest)?;
    let size = |zip: &mut ZipArchive<R>, name: &str| -> Result<u64> { Ok(zip.by_name(name).with_context(|| format!("the package holds no {name}"))?.size()) };
    let boot = size(zip, BOOT)?;
    let root = size(zip, ROOT)?;
    ensure!(boot > 0 && root > 0, "the package's boot or root image is empty");
    Ok(Contents { manifest, boot, root, init_boot: zip.index_for_name(INIT_BOOT).is_some() })
}

fn check(m: &Manifest) -> Result<()> {
    let id_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-_".contains(c);
    ensure!(!m.id.is_empty() && m.id.len() <= 32 && m.id.chars().all(id_char), "the system's id {:?} is not 1 to 32 of a-z, 0-9 and .-_", m.id);
    ensure!(m.id != "installed", "the system's id names the installed system");
    ensure!(!m.name.trim().is_empty() && m.name.chars().count() <= 32, "the system's name {:?} is not 1 to 32 characters", m.name);
    ensure!(m.version.chars().count() <= 32, "the system's version {:?} is longer than 32 characters", m.version);
    ensure!(!m.filesystem.is_empty() && m.filesystem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "the filesystem {:?} is no kernel's name of one", m.filesystem);
    ensure!(m.init.starts_with('/'), "the program the system starts with, {:?}, is not a full path", m.init);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    /// A package of `files`, each a name and its bytes.
    pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            out.start_file(*name, SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    pub const MANIFEST_TEXT: &[u8] = b"id = \"test\"\nname = \"Test\"\nversion = \"1.0\"\nfilesystem = \"ext4\"\n";

    #[test]
    fn a_whole_package_is_read_for_what_it_holds() {
        let bytes = zip(&[(MANIFEST, MANIFEST_TEXT), (BOOT, &[1u8; 300]), (ROOT, &[2u8; 1000])]);
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(is_system(&archive));
        let c = read(&mut archive).unwrap();
        assert_eq!((c.manifest.id.as_str(), c.manifest.init.as_str(), c.boot, c.root, c.init_boot), ("test", "/sbin/init", 300, 1000, false));
        assert_eq!(c.least(), 1000);
    }

    #[test]
    fn a_package_missing_a_part_or_misnaming_itself_is_refused() {
        let read_of = |files: &[(&str, &[u8])]| read(&mut ZipArchive::new(Cursor::new(zip(files))).unwrap());
        assert!(read_of(&[(MANIFEST, MANIFEST_TEXT), (BOOT, &[1u8; 3])]).is_err(), "no root image");
        assert!(read_of(&[(MANIFEST, MANIFEST_TEXT), (ROOT, &[1u8; 3])]).is_err(), "no boot image");
        let bad = b"id = \"Bad Id\"\nname = \"x\"\nversion = \"1\"\nfilesystem = \"ext4\"\n";
        assert!(read_of(&[(MANIFEST, bad), (BOOT, &[1u8; 3]), (ROOT, &[1u8; 3])]).is_err(), "an id with spaces and capitals");
        let installed = b"id = \"installed\"\nname = \"x\"\nversion = \"1\"\nfilesystem = \"ext4\"\n";
        assert!(read_of(&[(MANIFEST, installed), (BOOT, &[1u8; 3]), (ROOT, &[1u8; 3])]).is_err());
        let relative = b"id = \"x\"\nname = \"x\"\nversion = \"1\"\nfilesystem = \"ext4\"\ninit = \"sbin/init\"\n";
        assert!(read_of(&[(MANIFEST, relative), (BOOT, &[1u8; 3]), (ROOT, &[1u8; 3])]).is_err());
        let unknown = b"id = \"x\"\nname = \"x\"\nversion = \"1\"\nfilesystem = \"ext4\"\nkernel = \"y\"\n";
        assert!(read_of(&[(MANIFEST, unknown), (BOOT, &[1u8; 3]), (ROOT, &[1u8; 3])]).is_err(), "a field it does not know");
        let minimum = b"id = \"x\"\nname = \"x\"\nversion = \"1\"\nfilesystem = \"ext4\"\nminimum = 4096\n";
        let c = read_of(&[(MANIFEST, minimum), (BOOT, &[1u8; 3]), (ROOT, &[1u8; 3]), (INIT_BOOT, &[1u8; 3])]).unwrap();
        assert_eq!((c.least(), c.init_boot), (4096, true));
    }
}
