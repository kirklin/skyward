//! The init_boot image the other system starts from: the bootloader joins
//! its ramdisk to the installed system's vendor ramdisk, and the program
//! Skyward puts in it finds the system's root in the data partition and
//! starts the system's own first program from it.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use skyward_block::area::Area;
use skyward_bootimg::{lz4_legacy, BootImage, Ramdisk};

/// Where the program finds what to start, in the ramdisk.
pub const START: &str = "skyward/system.toml";
/// The program, as the system's first program where the system brings no
/// ramdisk of its own.
pub const INIT: &str = "init";
/// The program, where the system brings its own ramdisk: it lays the root
/// out as a device and says where, for that ramdisk's own first program.
pub const MAP: &str = "skyward/map";

/// What the program reads of the system it starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Start {
    /// The system's id, which the program names when the start stops.
    pub id: String,
    pub filesystem: String,
    pub init: String,
    pub root: Area,
}

impl Start {
    pub fn parse(text: &str) -> Result<Start> {
        toml::from_str(text).context("read what the system starts with")
    }
}

/// Skyward's archive in an init_boot image: `program` at `at`, and `start`
/// beside it, compressed as the vendor ramdisk before it is, for the kernel
/// to read on into it from there.
fn archive(at: &str, program: &[u8], start: &Start) -> Result<Vec<u8>> {
    let mut rd = Ramdisk { entries: Vec::new() };
    rd.add(at, 0o750, program.to_vec())?;
    rd.add(START, 0o644, toml::to_string(start).context("write what the system starts with")?.into_bytes())?;
    Ok(lz4_legacy(&rd.archive()?))
}

/// The init_boot image that starts the system `start` describes: the
/// header of `template`, the phone's own init_boot, and a ramdisk holding
/// `program` as the first program with `start` beside it.
pub fn init_boot(template: &[u8], program: &[u8], start: &Start) -> Result<Vec<u8>> {
    let mut image = BootImage::parse(template).context("read the phone's init_boot image")?;
    ensure!(image.kernel.is_empty(), "the phone's init_boot image holds a kernel");
    image.ramdisk = archive(INIT, program, start)?;
    image.to_bytes()
}

/// The system's own init_boot image `own` with the program as [`MAP`] and
/// `start` beside it, in an archive after its ramdisk's own; and the bytes
/// of that ramdisk of its own.
pub fn with_start(own: &[u8], program: &[u8], start: &Start) -> Result<(Vec<u8>, u64)> {
    let mut image = BootImage::parse(own).context("read the system's init_boot image")?;
    let own_ramdisk = image.ramdisk.len() as u64;
    image.ramdisk.extend_from_slice(&archive(MAP, program, start)?);
    Ok((image.to_bytes()?, own_ramdisk))
}

/// The init_boot image `image`, as [`init_boot`] made it or, for a system
/// whose own ramdisk of `own_ramdisk` bytes comes first in it, as
/// [`with_start`] did, made again with `program` and `start`.
pub fn with_program(image: &[u8], own_ramdisk: Option<u64>, program: &[u8], start: &Start) -> Result<Vec<u8>> {
    let mut image = BootImage::parse(image).context("read the other system's init_boot image")?;
    let (kept, at) = match own_ramdisk {
        Some(bytes) => (bytes, MAP),
        None => (0, INIT),
    };
    ensure!(kept <= image.ramdisk.len() as u64, "the other system's own ramdisk of {kept} bytes is longer than the {} its init_boot image holds", image.ramdisk.len());
    image.ramdisk.truncate(kept as usize);
    image.ramdisk.extend_from_slice(&archive(at, program, start)?);
    image.to_bytes()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::state::tests::area;
    use abootimg_oxide::{HeaderV3, OsVersionPatch};

    /// A version 4 init_boot image with `ramdisk` and no kernel, filling a
    /// partition of a MiB.
    pub fn template(ramdisk: &[u8]) -> Vec<u8> {
        let header = HeaderV3 {
            kernel_size: 0,
            ramdisk_size: ramdisk.len() as u32,
            osversionpatch: OsVersionPatch(0),
            cmdline: Box::new([0; 1536]),
            v4_signature_size: Some(0),
        };
        let mut image = BootImage { header, kernel: Vec::new(), ramdisk: ramdisk.to_vec() }.to_bytes().unwrap();
        image.resize(1 << 20, 0);
        image
    }

    /// The entries of Skyward's archive `bytes`, by name, with their data.
    pub fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(bytes[..4], [0x02, 0x21, 0x4c, 0x18], "LZ4 in the legacy framing, as the vendor ramdisk is");
        Ramdisk::decode(bytes).unwrap().entries.into_iter().map(|e| (e.name, e.data)).collect()
    }

    pub fn start() -> Start {
        Start { id: "test".into(), filesystem: "ext4".into(), init: "/sbin/init".into(), root: area("/data/skyward/systems/test", 1 << 20) }
    }

    #[test]
    fn the_new_ramdisk_starts_the_program_with_what_it_starts_beside_it() {
        let image = init_boot(&template(b"stock"), b"program", &start()).unwrap();
        let back = BootImage::parse(&image).unwrap();
        assert_eq!(back.header.header_version(), 4);
        assert!(back.kernel.is_empty());
        let found = entries(&back.ramdisk);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [INIT, "skyward", START]);
        assert_eq!(found[0].1, b"program");
        assert_eq!(Start::parse(std::str::from_utf8(&found[2].1).unwrap()).unwrap(), start());
    }

    #[test]
    fn a_systems_own_ramdisk_keeps_its_own_first_program() {
        let own = template(b"its own archive");
        let (image, own_ramdisk) = with_start(&own, b"program", &start()).unwrap();
        assert_eq!(own_ramdisk, b"its own archive".len() as u64);
        let back = BootImage::parse(&image).unwrap();
        let (theirs, ours) = back.ramdisk.split_at(b"its own archive".len());
        assert_eq!(theirs, b"its own archive", "its archive first");
        let names: Vec<String> = entries(ours).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["skyward", MAP, START], "and no first program of Skyward's");
    }

    #[test]
    fn an_image_made_again_carries_the_new_program_and_nothing_of_the_old() {
        let mut made = init_boot(&template(b"stock"), b"old program", &start()).unwrap();
        // As the store and the partition hold it: zeros to the partition's end.
        made.resize(made.len() + 8192, 0);
        let again = BootImage::parse(&with_program(&made, None, b"new program", &start()).unwrap()).unwrap();
        assert_eq!((again.header.header_version(), again.kernel.len()), (4, 0), "still an init_boot image");
        let found = entries(&again.ramdisk);
        assert_eq!(found.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), [INIT, "skyward", START]);
        assert_eq!(found[0].1, b"new program");
        assert_eq!(Start::parse(std::str::from_utf8(&found[2].1).unwrap()).unwrap(), start());
    }

    #[test]
    fn a_systems_own_ramdisk_is_kept_when_its_image_is_made_again() {
        let (made, own_ramdisk) = with_start(&template(b"its own archive"), b"old program", &start()).unwrap();
        let again = BootImage::parse(&with_program(&made, Some(own_ramdisk), b"new", &start()).unwrap()).unwrap();
        let (theirs, ours) = again.ramdisk.split_at(own_ramdisk as usize);
        assert_eq!(theirs, b"its own archive");
        let found = entries(ours);
        assert_eq!(found.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["skyward", MAP, START]);
        assert_eq!(found[1].1, b"new");
        assert!(with_program(&made, Some(1 << 20), b"new", &start()).is_err(), "an own ramdisk longer than the image's");
    }
}
