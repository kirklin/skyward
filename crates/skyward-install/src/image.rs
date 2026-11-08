//! Partition images that come over sideload: what an image is from its
//! header and ramdisk, which partitions it can go to, and writing it to
//! them the way fastboot does.

use abootimg_oxide::binrw::io::Cursor;
use abootimg_oxide::binrw::BinRead;
use abootimg_oxide::{Header, VendorHeader};
use anyhow::{bail, ensure, Context, Result};
use sha2::{Digest, Sha256};
use skyward_vab::Slot;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const BOOT_MAGIC: &[u8] = b"ANDROID!";
const VENDOR_BOOT_MAGIC: &[u8] = b"VNDRBOOT";
/// A dtbo image starts with this, big-endian.
const DTBO_MAGIC: &[u8] = &[0xd7, 0xb7, 0xab, 0x1e];
const VBMETA_MAGIC: &[u8] = b"AVB0";

/// How a ramdisk is packed, by its first bytes: LZ4 in the legacy framing
/// Android uses or the standard one, gzip, or a bare newc archive.
const LZ4_LEGACY: &[u8] = &[0x02, 0x21, 0x4c, 0x18];
const LZ4_FRAME: &[u8] = &[0x04, 0x22, 0x4d, 0x18];
const GZIP: &[u8] = &[0x1f, 0x8b];
const NEWC: &[u8] = b"070701";

/// Where a recovery's ramdisk keeps its program, now and in older layouts.
const RECOVERY_PROGRAMS: [&str; 2] = ["system/bin/recovery", "sbin/recovery"];

/// A signed image ends with a footer that locates its signature, and the
/// bootloader looks for it in the last bytes of the partition.
const FOOTER_MAGIC: &[u8] = b"AVBf";
const FOOTER_LEN: u64 = 64;

const CHUNK: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// A boot image; what it carries decides where it can go.
    Boot { kernel: bool, ramdisk: Ramdisk },
    VendorBoot,
    Dtbo,
    Vbmeta,
}

/// The ramdisk a boot image carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ramdisk {
    None,
    /// The one that starts Android.
    Generic,
    /// A recovery's, which holds the recovery program.
    Recovery,
}

impl ImageKind {
    /// The partitions an image of this kind can go to, the likeliest first.
    /// A recovery that brings its own kernel can also be the boot image of
    /// a phone whose recovery lives in boot; without one it needs boot's.
    pub fn partitions(self) -> &'static [&'static str] {
        match self {
            ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery } => &["recovery", "boot"],
            ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery } => &["recovery"],
            ImageKind::Boot { kernel: true, .. } => &["boot"],
            ImageKind::Boot { kernel: false, .. } => &["init_boot"],
            ImageKind::VendorBoot => &["vendor_boot"],
            ImageKind::Dtbo => &["dtbo"],
            ImageKind::Vbmeta => &["vbmeta", "vbmeta_system", "vbmeta_vendor"],
        }
    }
}

/// The image `file` holds, from `head`, its first bytes, and for a boot
/// image its ramdisk; `None` when it is no image this recovery writes.
pub fn detect<R: Read + Seek>(file: &mut R, head: &[u8]) -> Result<Option<ImageKind>> {
    if head.starts_with(BOOT_MAGIC) {
        let header = Header::parse(&mut Cursor::new(head)).context("read the boot image header")?;
        let kernel = header.kernel_size() > 0;
        let ramdisk = match header.ramdisk_size() {
            0 => Ramdisk::None,
            size => {
                file.seek(SeekFrom::Start(header.ramdisk_position() as u64)).context("seek to the ramdisk")?;
                ramdisk(file.take(u64::from(size))).context("read the ramdisk")?
            }
        };
        ensure!(kernel || ramdisk != Ramdisk::None, "the boot image holds neither a kernel nor a ramdisk");
        return Ok(Some(ImageKind::Boot { kernel, ramdisk }));
    }
    if head.starts_with(VENDOR_BOOT_MAGIC) {
        VendorHeader::read(&mut Cursor::new(head)).context("read the vendor boot image header")?;
        return Ok(Some(ImageKind::VendorBoot));
    }
    if head.starts_with(DTBO_MAGIC) {
        return Ok(Some(ImageKind::Dtbo));
    }
    if head.starts_with(VBMETA_MAGIC) {
        return Ok(Some(ImageKind::Vbmeta));
    }
    Ok(None)
}

/// Whether a packed ramdisk is a recovery's, by the names in its archive.
fn ramdisk(packed: impl Read) -> Result<Ramdisk> {
    Ok(match holds(packed, |name| RECOVERY_PROGRAMS.contains(&name))? {
        true => Ramdisk::Recovery,
        false => Ramdisk::Generic,
    })
}

/// Whether a packed ramdisk has an entry whose name `wanted` picks.
fn holds(packed: impl Read, wanted: impl Fn(&str) -> bool) -> Result<bool> {
    Ok(find(packed, wanted, |_| Ok(()))?.is_some())
}

/// What `take` makes of the first entry of a packed ramdisk whose name
/// `wanted` picks, read from its start; none when no entry's name is
/// picked.
fn find<T>(packed: impl Read, wanted: impl Fn(&str) -> bool, take: impl FnOnce(&mut dyn Read) -> Result<T>) -> Result<Option<T>> {
    let mut packed = BufReader::new(packed);
    let start = packed.fill_buf().context("read the ramdisk's first bytes")?;
    let mut rest: Box<dyn Read + '_> = if start.starts_with(LZ4_LEGACY) || start.starts_with(LZ4_FRAME) {
        Box::new(lz4_flex::frame::FrameDecoder::new(packed))
    } else if start.starts_with(GZIP) {
        Box::new(flate2::bufread::MultiGzDecoder::new(packed))
    } else if start.starts_with(NEWC) {
        Box::new(packed)
    } else {
        bail!("a ramdisk packed in an unknown way, starting {:02x?}", &start[..start.len().min(4)]);
    };
    loop {
        let mut entry = cpio::NewcReader::new(rest).context("read a ramdisk entry")?;
        if entry.entry().is_trailer() {
            return Ok(None);
        }
        if wanted(entry.entry().name().trim_start_matches("./")) {
            return take(&mut entry).map(Some);
        }
        rest = entry.finish().context("read past a ramdisk entry")?;
    }
}

/// Whether `file` is a Skyward recovery: a boot image whose ramdisk carries
/// the device configuration Skyward is packed with, packed in one of the
/// ways read here, as Skyward packs it.
pub fn is_skyward<R: Read + Seek>(file: &mut R) -> Result<bool> {
    Ok(device_config(file)?.is_some())
}

/// The device configuration a Skyward recovery carries in its ramdisk;
/// none when `file` is no Skyward recovery.
pub fn device_config<R: Read + Seek>(file: &mut R) -> Result<Option<Vec<u8>>> {
    let mut head = Vec::with_capacity(4096);
    file.take(4096).read_to_end(&mut head).context("read the image's first bytes")?;
    if !head.starts_with(BOOT_MAGIC) {
        return Ok(None);
    }
    let header = Header::parse(&mut Cursor::new(&head)).context("read the boot image header")?;
    if header.ramdisk_size() == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(header.ramdisk_position() as u64)).context("seek to the ramdisk")?;
    let mut ramdisk = BufReader::new(file.take(u64::from(header.ramdisk_size())));
    let start = ramdisk.fill_buf().context("read the ramdisk's first bytes")?;
    if ![LZ4_LEGACY, LZ4_FRAME, GZIP, NEWC].iter().any(|magic| start.starts_with(magic)) {
        return Ok(None);
    }
    let read = |entry: &mut dyn Read| {
        let mut config = Vec::new();
        entry.read_to_end(&mut config).context("read the device configuration")?;
        Ok(config)
    };
    find(ramdisk, |name| name == skyward_config::ramdisk::CONFIG, read).context("read the ramdisk")
}

/// The partition `name` of `slot` among the links in `dir`.
pub fn device(dir: &Path, name: &str, slot: Slot) -> PathBuf {
    dir.join(format!("{name}{}", slot.suffix()))
}

/// The partitions `kind` fits that this phone has in both slots, among the
/// links in `dir`, the likeliest first.
pub fn partitions(kind: ImageKind, dir: &Path) -> Result<Vec<&'static str>> {
    let mut found = Vec::new();
    for name in kind.partitions() {
        let mut has = true;
        for slot in [Slot::A, Slot::B] {
            let path = device(dir, name, slot);
            has &= path.try_exists().with_context(|| format!("look for {}", path.display()))?;
        }
        if has {
            found.push(*name);
        }
    }
    Ok(found)
}

/// Write `len` bytes of `image` to all of `devices` in one pass, then read
/// each back and compare. A signed image smaller than its partition gets
/// its footer moved to the partition's end, with zeros between, as
/// fastboot does. `progress` hears the bytes written and read back so far,
/// of all.
pub fn flash<R: Read + Seek>(image: &mut R, len: u64, devices: &[PathBuf], progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    ensure!(!devices.is_empty(), "no partition to write");
    let mut outs = Vec::with_capacity(devices.len());
    for path in devices {
        let mut f = OpenOptions::new().read(true).write(true).open(path).with_context(|| format!("open {}", path.display()))?;
        let size = f.seek(SeekFrom::End(0)).with_context(|| format!("find the size of {}", path.display()))?;
        f.seek(SeekFrom::Start(0)).with_context(|| format!("rewind {}", path.display()))?;
        outs.push((path, f, size));
    }
    let size = outs[0].2;
    ensure!(outs.iter().all(|(_, _, s)| *s == size), "the partition differs in size between slots");
    ensure!(len <= size, "the image is {len} bytes, more than the {size} its partition holds");

    let mut footer = [0u8; FOOTER_LEN as usize];
    let moved = len >= FOOTER_LEN && len < size && {
        image.seek(SeekFrom::Start(len - FOOTER_LEN)).context("seek to the image's end")?;
        image.read_exact(&mut footer).context("read the image's end")?;
        footer.starts_with(FOOTER_MAGIC)
    };
    image.seek(SeekFrom::Start(0)).context("rewind the image")?;
    let (gap, tail): (u64, &[u8]) = if moved { (size - len - FOOTER_LEN, &footer) } else { (0, &[]) };
    let end = len + gap + tail.len() as u64;
    let total = end * outs.len() as u64 * 2;
    let mut source = image.take(len).chain(io::repeat(0).take(gap)).chain(tail);

    let mut buf = vec![0u8; CHUNK];
    let mut written = Sha256::new();
    let mut done = 0;
    let mut at = 0;
    while at < end {
        let n = (end - at).min(CHUNK as u64) as usize;
        source.read_exact(&mut buf[..n]).context("read the image")?;
        written.update(&buf[..n]);
        for (path, f, _) in &mut outs {
            f.write_all(&buf[..n]).with_context(|| format!("write {}", path.display()))?;
            done += n as u64;
            progress(done, total);
        }
        at += n as u64;
    }
    let written = written.finalize();
    for (path, f, _) in &mut outs {
        f.sync_all().with_context(|| format!("flush {}", path.display()))?;
        drop_cache(f).with_context(|| format!("drop the cached pages of {}", path.display()))?;
    }
    for (path, f, _) in &mut outs {
        f.seek(SeekFrom::Start(0)).with_context(|| format!("rewind {}", path.display()))?;
        let mut back = Sha256::new();
        let mut left = end;
        while left > 0 {
            let n = left.min(CHUNK as u64) as usize;
            f.read_exact(&mut buf[..n]).with_context(|| format!("read back {}", path.display()))?;
            back.update(&buf[..n]);
            left -= n as u64;
            done += n as u64;
            progress(done, total);
        }
        ensure!(back.finalize() == written, "{} reads back different from what was written", path.display());
    }
    Ok(())
}

pub(crate) use skyward_block::drop_cache;

/// Images made up for the tests here and in the modules that look into
/// partitions.
#[cfg(test)]
pub(crate) mod samples {
    use super::*;
    use abootimg_oxide::{HeaderV3, OsVersionPatch};

    /// A version 4 boot image with a kernel of `kernel` bytes and `ramdisk`
    /// as its ramdisk, each padded to the page.
    pub fn boot_image(kernel: u32, ramdisk: &[u8]) -> Vec<u8> {
        let header = Header::V3(HeaderV3 {
            kernel_size: kernel,
            ramdisk_size: ramdisk.len() as u32,
            osversionpatch: OsVersionPatch(0),
            cmdline: Box::new([0; 1536]),
            v4_signature_size: Some(0),
        });
        let mut out = Vec::new();
        header.write(&mut out).unwrap();
        out.resize(4096, 0);
        out.resize(4096 + (kernel as usize).div_ceil(4096) * 4096, 0x4b);
        out.extend_from_slice(ramdisk);
        out.resize(out.len().div_ceil(4096) * 4096, 0);
        out
    }

    /// A newc archive of empty files named `names`.
    pub fn archive(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for name in names {
            out = cpio::NewcBuilder::new(name).mode(0o100644).write(out, 0).finish().unwrap();
        }
        cpio::newc::trailer(out).unwrap()
    }

    pub use skyward_bootimg::lz4_legacy;

    /// A recovery image, Skyward's or the stock one it is packed on.
    pub fn recovery(skyward: bool) -> Vec<u8> {
        let mut names = vec!["init", "system/bin/recovery", "prop.default"];
        if skyward {
            names.extend(["skyward", "skyward/device.toml", "skyward/MiSansVF.ttf"]);
        }
        boot_image(0, &lz4_legacy(&archive(&names)))
    }

    /// Skyward's recovery for the phone `codename` names, its device
    /// configuration naming it.
    pub fn skyward_for(codename: &str) -> Vec<u8> {
        let config = format!("[device]\ncodename = \"{codename}\"\n");
        let mut out = Vec::new();
        for (name, data) in [("init", &b""[..]), ("system/bin/recovery", b""), ("skyward/device.toml", config.as_bytes())] {
            let mut entry = cpio::NewcBuilder::new(name).mode(0o100644).write(out, data.len() as u32);
            entry.write_all(data).unwrap();
            out = entry.finish().unwrap();
        }
        boot_image(0, &lz4_legacy(&cpio::newc::trailer(out).unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::samples::{archive, boot_image, lz4_legacy, recovery, skyward_for};
    use super::*;
    use abootimg_oxide::binrw::BinWrite;
    use abootimg_oxide::VendorHeaderV4;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        w.write_all(data).unwrap();
        w.finish().unwrap()
    }

    fn kind_of(image: &[u8]) -> Result<Option<ImageKind>> {
        detect(&mut Cursor::new(image), &image[..image.len().min(4096)])
    }

    #[test]
    fn boot_images_go_where_what_they_carry_fits() {
        let recovery = archive(&["init", "system", "system/bin", "system/bin/recovery", "sbin"]);
        let generic = archive(&["init", "first_stage_ramdisk", "system/bin/snapuserd"]);

        let kind = kind_of(&boot_image(0, &lz4_legacy(&recovery))).unwrap().unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery });
        assert_eq!(kind.partitions(), ["recovery"]);
        let kind = kind_of(&boot_image(0, &gzip(&generic))).unwrap().unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Generic });
        assert_eq!(kind.partitions(), ["init_boot"]);
        let kind = kind_of(&boot_image(5000, &recovery)).unwrap().unwrap();
        assert_eq!(kind.partitions(), ["recovery", "boot"]);
        let kind = kind_of(&boot_image(5000, &lz4_legacy(&generic))).unwrap().unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Generic });
        assert_eq!(kind.partitions(), ["boot"]);
        assert_eq!(kind_of(&boot_image(5000, &[])).unwrap(), Some(ImageKind::Boot { kernel: true, ramdisk: Ramdisk::None }));

        assert!(kind_of(&boot_image(0, &[])).is_err());
        assert!(kind_of(&boot_image(0, b"BZh91AY&SY")).is_err(), "a ramdisk packed some other way");
        assert!(kind_of(b"ANDROID!").is_err());
    }

    #[test]
    fn skyward_is_told_from_the_stock_recovery_it_is_packed_on() {
        assert!(is_skyward(&mut Cursor::new(recovery(true))).unwrap());
        assert!(!is_skyward(&mut Cursor::new(recovery(false))).unwrap());
        assert!(!is_skyward(&mut Cursor::new(boot_image(5000, &[]))).unwrap(), "a boot image with no ramdisk");
        assert!(!is_skyward(&mut Cursor::new(boot_image(0, b"\xfd7zXZ\0"))).unwrap(), "a ramdisk packed as Skyward never packs it");
        assert!(!is_skyward(&mut Cursor::new(vec![0u8; 8192])).unwrap(), "an erased partition");
    }

    #[test]
    fn skyward_carries_its_device_configuration() {
        let config = device_config(&mut Cursor::new(skyward_for("ishtar"))).unwrap().expect("Skyward carries one");
        assert_eq!(config, b"[device]\ncodename = \"ishtar\"\n");
        assert!(is_skyward(&mut Cursor::new(skyward_for("ishtar"))).unwrap());
        assert_eq!(device_config(&mut Cursor::new(recovery(false))).unwrap(), None);
    }

    #[test]
    fn other_images_are_told_by_their_magic() {
        let vendor = VendorHeader {
            page_size: 4096,
            kernel_addr: 0,
            ramdisk_addr: 0,
            vendor_ramdisk_size: 0,
            cmdline: Box::new([0; 2048]),
            tags_addr: 0,
            board_name: [0; 16],
            dtb_size: 0,
            dtb_addr: 0,
            v4: Some(VendorHeaderV4 {
                vendor_ramdisk_table_size: 0,
                vendor_ramdisk_table_entry_num: 0,
                vendor_ramdisk_table_entry_size: 0,
                bootconfig_size: 0,
            }),
        };
        let mut bytes = Cursor::new(Vec::new());
        vendor.write(&mut bytes).unwrap();
        assert_eq!(kind_of(bytes.get_ref()).unwrap(), Some(ImageKind::VendorBoot));
        assert_eq!(kind_of(&[0xd7, 0xb7, 0xab, 0x1e, 0, 0, 0, 0x40]).unwrap(), Some(ImageKind::Dtbo));
        assert_eq!(kind_of(b"AVB0\0\0\0\x01").unwrap(), Some(ImageKind::Vbmeta));
        assert_eq!(kind_of(b"\x3a\xff\x26\xed sparse").unwrap(), None);
    }

    #[test]
    fn only_partitions_the_phone_has_in_both_slots_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["boot_a", "boot_b", "recovery_a", "recovery_b", "init_boot_a", "init_boot_b", "vbmeta_a", "vbmeta_b", "vbmeta_system_a"] {
            std::fs::File::create(dir.path().join(name)).unwrap();
        }
        let both = ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery };
        assert_eq!(partitions(both, dir.path()).unwrap(), ["recovery", "boot"]);
        assert_eq!(partitions(ImageKind::Vbmeta, dir.path()).unwrap(), ["vbmeta"]);
        assert!(partitions(ImageKind::Dtbo, dir.path()).unwrap().is_empty());
    }

    /// A partition of `size` bytes filled with 0xAA, so untouched bytes
    /// show.
    fn partition(dir: &Path, name: &str, size: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![0xaa; size]).unwrap();
        path
    }

    fn flash_all(image: &[u8], devices: &[PathBuf]) -> Result<Vec<(u64, u64)>> {
        let mut seen = Vec::new();
        flash(&mut Cursor::new(image), image.len() as u64, devices, &mut |done, total| seen.push((done, total)))?;
        Ok(seen)
    }

    #[test]
    fn an_unsigned_image_leaves_the_rest_of_the_partition_alone() {
        let dir = tempfile::tempdir().unwrap();
        let slots = [partition(dir.path(), "dtbo_a", 3 << 20), partition(dir.path(), "dtbo_b", 3 << 20)];
        let image: Vec<u8> = (0..(1 << 20) + 300).map(|i| (i % 251) as u8).collect();
        let seen = flash_all(&image, &slots).unwrap();
        for slot in &slots {
            let got = std::fs::read(slot).unwrap();
            assert_eq!(&got[..image.len()], &image[..]);
            assert!(got[image.len()..].iter().all(|&b| b == 0xaa));
        }
        let total = image.len() as u64 * 4;
        assert_eq!(seen.last(), Some(&(total, total)));
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn a_signed_image_has_its_footer_moved_to_the_partition_end() {
        let dir = tempfile::tempdir().unwrap();
        let size = 2 << 20;
        let slots = [partition(dir.path(), "boot_a", size)];
        let mut image = vec![0x11u8; 100_000];
        let mut footer = b"AVBf".to_vec();
        footer.resize(FOOTER_LEN as usize, 0x22);
        image.extend_from_slice(&footer);
        flash_all(&image, &slots).unwrap();
        let got = std::fs::read(&slots[0]).unwrap();
        assert_eq!(&got[..image.len()], &image[..]);
        assert!(got[image.len()..size - 64].iter().all(|&b| b == 0));
        assert_eq!(&got[size - 64..], &footer[..]);
    }

    #[test]
    fn an_image_the_size_of_its_partition_is_written_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let slots = [partition(dir.path(), "vbmeta_a", 8192)];
        let mut image = vec![0x33u8; 8192 - 64];
        image.extend_from_slice(b"AVBf");
        image.resize(8192, 0x44);
        flash_all(&image, &slots).unwrap();
        assert_eq!(std::fs::read(&slots[0]).unwrap(), image);
    }

    #[test]
    fn an_image_larger_than_its_partition_is_refused_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let slots = [partition(dir.path(), "dtbo_a", 4096)];
        let err = flash_all(&[1u8; 4097], &slots).unwrap_err();
        assert!(err.to_string().contains("4097"), "{err}");
        assert!(std::fs::read(&slots[0]).unwrap().iter().all(|&b| b == 0xaa));
    }
}
