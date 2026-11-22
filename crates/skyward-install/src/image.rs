//! Partition images that come over sideload or lie in a file: what an
//! image is from what it holds, as it is or in Android's sparse format,
//! which partitions it can go to, and writing it to them the way fastboot
//! does.

use abootimg_oxide::binrw::io::Cursor;
use abootimg_oxide::binrw::BinRead;
use abootimg_oxide::{Header, VendorHeader};
use android_sparse_image::{ChunkHeader, ChunkType, FileHeader, CHUNK_HEADER_BYTES_LEN, FILE_HEADER_BYTES_LEN, HEADER_MAGIC};
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

/// The most read of an image to tell its kind: the headers at its start,
/// a filesystem's superblock, and the geometry of a super.
pub const HEAD: usize = 8192;

/// Super keeps its first 4096 bytes clear, and its geometry in the 4096
/// after them.
const GEOMETRY: std::ops::Range<usize> = 4096..8192;

/// The filesystems Android puts on a logical partition each keep a
/// superblock 1024 bytes in. Those of erofs and f2fs start with their
/// magic number, and ext4 keeps its own 56 bytes further on; all three are
/// little-endian.
const EROFS_MAGIC: (usize, &[u8]) = (1024, &[0xe2, 0xe1, 0xf5, 0xe0]);
const F2FS_MAGIC: (usize, &[u8]) = (1024, &[0x10, 0x20, 0xf5, 0xf2]);
const EXT4_MAGIC: (usize, &[u8]) = (1080, &[0x53, 0xef]);

/// The partition that holds the logical ones.
const SUPER: &str = "super";

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
    /// All of super, with every logical partition in it.
    Super,
    /// A filesystem, as a logical partition holds one.
    Filesystem(Filesystem),
    /// Anything else, which tells nothing of where it goes: only its
    /// file's name does.
    Other,
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

/// The filesystem an image holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filesystem {
    Erofs,
    Ext4,
    F2fs,
}

impl Filesystem {
    /// Its name, as the kernel has it.
    pub fn name(self) -> &'static str {
        match self {
            Filesystem::Erofs => "erofs",
            Filesystem::Ext4 => "ext4",
            Filesystem::F2fs => "f2fs",
        }
    }
}

impl ImageKind {
    /// The partitions what an image of this kind holds shows it can go
    /// to, the likeliest first. A recovery that brings its own kernel can
    /// also be the boot image of a phone whose recovery lives in boot;
    /// without one it needs boot's.
    fn partitions(self) -> &'static [&'static str] {
        match self {
            ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery } => &["recovery", "boot"],
            ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery } => &["recovery"],
            ImageKind::Boot { kernel: true, .. } => &["boot"],
            ImageKind::Boot { kernel: false, .. } => &["init_boot"],
            ImageKind::VendorBoot => &["vendor_boot"],
            ImageKind::Dtbo => &["dtbo"],
            ImageKind::Vbmeta => &["vbmeta", "vbmeta_system", "vbmeta_vendor"],
            ImageKind::Super | ImageKind::Filesystem(_) | ImageKind::Other => &[],
        }
    }
}

/// What a stretch of an image's partition is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The file's bytes from this offset on.
    File(u64),
    /// These four bytes, over and over.
    Fill([u8; 4]),
}

/// A stretch of an image's partition that the image writes: `len` bytes
/// from `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    at: u64,
    len: u64,
    source: Source,
}

/// What an image file puts into its partition, and where. A file in
/// Android's sparse format says so chunk by chunk, and leaves what need
/// not be written out; any other file goes in as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The bytes the image takes in its partition, written or not.
    pub size: u64,
    pub sparse: bool,
    /// What is written, in the partition's order.
    runs: Vec<Run>,
}

impl Layout {
    /// A file of `len` bytes that goes in as it is.
    pub fn whole(len: u64) -> Layout {
        let runs = if len == 0 { Vec::new() } else { vec![Run { at: 0, len, source: Source::File(0) }] };
        Layout { size: len, sparse: false, runs }
    }

    /// How `file`, of `len` bytes, lays its image out: by its chunks where
    /// it starts as a sparse image does, each checked to lie in the file
    /// and all of them to make up the image its header tells of.
    pub fn read<R: Read + Seek>(file: &mut R, len: u64) -> Result<Layout> {
        let mut start = [0u8; FILE_HEADER_BYTES_LEN];
        if len < start.len() as u64 {
            return Ok(Layout::whole(len));
        }
        file.seek(SeekFrom::Start(0)).context("rewind the image")?;
        file.read_exact(&mut start).context("read the image's first bytes")?;
        if start[..4] != HEADER_MAGIC.to_le_bytes() {
            return Ok(Layout::whole(len));
        }
        let header = FileHeader::from_bytes(&start).context("read the sparse image's header")?;
        let size = header.total_size();
        let (mut at, mut from) = (0u64, start.len() as u64);
        let mut runs = Vec::new();
        for k in 0..header.chunks {
            let mut raw = [0u8; CHUNK_HEADER_BYTES_LEN];
            file.read_exact(&mut raw).with_context(|| format!("read the header of chunk {k} at {from}"))?;
            let chunk = ChunkHeader::from_bytes(&raw).with_context(|| format!("read the header of chunk {k} at {from}"))?;
            let data = u64::from(chunk.total_size)
                .checked_sub(raw.len() as u64)
                .with_context(|| format!("chunk {k} says it is shorter than its header"))?;
            from += raw.len() as u64;
            ensure!(from + data <= len, "chunk {k} runs to byte {}, past the {len} the file holds", from + data);
            let out = match chunk.chunk_type {
                ChunkType::Raw => {
                    let out = chunk.out_size(&header);
                    ensure!(data == out, "chunk {k} brings {data} bytes for {out} of the image");
                    runs.push(Run { at, len: out, source: Source::File(from) });
                    out
                }
                ChunkType::Fill => {
                    ensure!(data == 4, "chunk {k} fills with {data} bytes, and a fill takes 4");
                    let mut pattern = [0u8; 4];
                    file.read_exact(&mut pattern).with_context(|| format!("read what chunk {k} fills with"))?;
                    runs.push(Run { at, len: chunk.out_size(&header), source: Source::Fill(pattern) });
                    chunk.out_size(&header)
                }
                ChunkType::DontCare => {
                    ensure!(data == 0, "chunk {k} brings {data} bytes for what it leaves as it is");
                    chunk.out_size(&header)
                }
                // A checksum of what came before it, which takes no room in
                // the image.
                ChunkType::Crc32 => {
                    ensure!(data == 4, "chunk {k} is a checksum of {data} bytes, and one takes 4");
                    0
                }
            };
            at = at.checked_add(out).with_context(|| format!("chunk {k} takes the image past what a partition holds"))?;
            from += data;
            file.seek(SeekFrom::Start(from)).with_context(|| format!("seek past chunk {k}"))?;
        }
        ensure!(at == size, "the chunks make up {at} bytes of an image of {size}");
        runs.retain(|run| run.len != 0);
        Ok(Layout { size, sparse: true, runs })
    }
}

/// An image as its partition reads once the image is in it, where what
/// the image leaves unwritten reads as zeros.
pub struct Expanded<'a, R> {
    file: &'a mut R,
    layout: &'a Layout,
    at: u64,
}

impl<'a, R: Read + Seek> Expanded<'a, R> {
    pub fn new(file: &'a mut R, layout: &'a Layout) -> Self {
        Expanded { file, layout, at: 0 }
    }
}

impl<R: Read + Seek> Read for Expanded<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.layout.size || buf.is_empty() {
            return Ok(0);
        }
        let runs = &self.layout.runs;
        let next = runs.partition_point(|run| run.at + run.len <= self.at);
        let n = match runs.get(next) {
            Some(run) if run.at <= self.at => {
                let within = self.at - run.at;
                let n = (buf.len() as u64).min(run.len - within) as usize;
                match run.source {
                    Source::File(from) => {
                        self.file.seek(SeekFrom::Start(from + within))?;
                        self.file.read_exact(&mut buf[..n])?;
                    }
                    Source::Fill(pattern) => {
                        for (k, byte) in buf[..n].iter_mut().enumerate() {
                            *byte = pattern[((within + k as u64) % 4) as usize];
                        }
                    }
                }
                n
            }
            unwritten => {
                let until = unwritten.map_or(self.layout.size, |run| run.at);
                let n = (buf.len() as u64).min(until - self.at) as usize;
                buf[..n].fill(0);
                n
            }
        };
        self.at += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Expanded<'_, R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(by) => self.layout.size.checked_add_signed(by),
            SeekFrom::Current(by) => self.at.checked_add_signed(by),
        };
        self.at = at.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a place before the image's start"))?;
        Ok(self.at)
    }
}

/// An image file: what it holds, and how it lays that out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub kind: ImageKind,
    pub layout: Layout,
}

/// What the file `file` of `len` bytes is as an image.
pub fn open<R: Read + Seek>(file: &mut R, len: u64) -> Result<Image> {
    let layout = Layout::read(file, len)?;
    let mut image = Expanded::new(file, &layout);
    let mut head = Vec::with_capacity(HEAD);
    (&mut image).take(HEAD as u64).read_to_end(&mut head).context("read the image's first bytes")?;
    let kind = detect(&mut image, &head)?;
    Ok(Image { kind, layout })
}

/// What kind of image `file` holds, from `head`, its first [`HEAD`] bytes,
/// and for a boot image its ramdisk.
pub fn detect<R: Read + Seek>(file: &mut R, head: &[u8]) -> Result<ImageKind> {
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
        return Ok(ImageKind::Boot { kernel, ramdisk });
    }
    if head.starts_with(VENDOR_BOOT_MAGIC) {
        VendorHeader::read(&mut Cursor::new(head)).context("read the vendor boot image header")?;
        return Ok(ImageKind::VendorBoot);
    }
    if head.starts_with(DTBO_MAGIC) {
        return Ok(ImageKind::Dtbo);
    }
    if head.starts_with(VBMETA_MAGIC) {
        return Ok(ImageKind::Vbmeta);
    }
    if head.get(GEOMETRY).is_some_and(skyward_lp::is_geometry) {
        return Ok(ImageKind::Super);
    }
    let holds = |(at, magic): (usize, &[u8])| head.get(at..at + magic.len()) == Some(magic);
    for (magic, filesystem) in [(EROFS_MAGIC, Filesystem::Erofs), (F2FS_MAGIC, Filesystem::F2fs), (EXT4_MAGIC, Filesystem::Ext4)] {
        if holds(magic) {
            return Ok(ImageKind::Filesystem(filesystem));
        }
    }
    Ok(ImageKind::Other)
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

/// Where a partition an image can go to is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// One of a pair, a partition in each slot.
    Slotted,
    /// A partition the phone has once: super.
    Single,
    /// A logical partition in super, of the slot the phone runs from.
    Logical,
}

/// A partition an image can go to, by its name without a slot's suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub place: Place,
}

/// The partition the file `file_name` is named for: its name without
/// what follows its last dot and without a slot's suffix, so that both
/// `vendor_dlkm.img` and `vendor_dlkm_b.img` name vendor_dlkm.
pub fn named(file_name: &str) -> &str {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem);
    [Slot::A, Slot::B].iter().find_map(|slot| stem.strip_suffix(slot.suffix())).unwrap_or(stem)
}

/// The partitions of this phone an image of `kind` can go to, the
/// likeliest first: `named` is the partition its file is named for, where
/// the file has a name, `dir` holds the links to the phone's partitions,
/// and `logical` lists the logical partitions of the slot the phone runs
/// from. What the image holds decides where it can go, and its name which
/// of those comes first. One that holds a filesystem fits any logical
/// partition. Beyond that the name alone decides, and names only what the
/// phone has in both slots or in super: those are the partitions an update
/// writes, where one it has once is its own to keep.
pub fn targets(kind: ImageKind, named: Option<&str>, dir: &Path, logical: &[String]) -> Result<Vec<Target>> {
    let slotted = |name: &str| -> Result<bool> {
        for slot in [Slot::A, Slot::B] {
            let path = device(dir, name, slot);
            if !path.try_exists().with_context(|| format!("look for {}", path.display()))? {
                return Ok(false);
            }
        }
        Ok(true)
    };
    let mut found: Vec<Target> = Vec::new();
    match kind {
        ImageKind::Boot { .. } | ImageKind::VendorBoot | ImageKind::Dtbo | ImageKind::Vbmeta => {
            for name in kind.partitions() {
                if slotted(name)? {
                    found.push(Target { name: name.to_string(), place: Place::Slotted });
                }
            }
            if let Some(first) = named.and_then(|named| found.iter().position(|t| t.name == named)) {
                found[..=first].rotate_right(1);
            }
        }
        ImageKind::Super => {
            let path = dir.join(SUPER);
            if path.try_exists().with_context(|| format!("look for {}", path.display()))? {
                found.push(Target { name: SUPER.to_string(), place: Place::Single });
            }
        }
        ImageKind::Filesystem(_) | ImageKind::Other => {
            if let Some(named) = named {
                if slotted(named)? {
                    found.push(Target { name: named.to_string(), place: Place::Slotted });
                } else if logical.iter().any(|name| name == named) {
                    found.push(Target { name: named.to_string(), place: Place::Logical });
                }
            }
            if matches!(kind, ImageKind::Filesystem(_)) {
                let rest = logical.iter().filter(|name| Some(name.as_str()) != named);
                found.extend(rest.map(|name| Target { name: name.clone(), place: Place::Logical }));
            }
        }
    }
    Ok(found)
}

/// Check that the image of a whole super in `image` is one for a super
/// of `size` bytes: what its metadata gives its partitions lies within
/// that.
pub fn check_super<R: Read + Seek>(image: &mut R, size: u64) -> Result<()> {
    let at = |offset: u64, len: usize| -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        image.seek(SeekFrom::Start(offset)).with_context(|| format!("seek to {offset} in the image"))?;
        image.read_exact(&mut buf).with_context(|| format!("read {len} bytes at {offset} in the image"))?;
        Ok(buf)
    };
    let md = skyward_lp::read_with(at, 0).context("read the image's metadata")?;
    let made_for = md.block_devices.first().context("the image's metadata names no super")?.size;
    ensure!(made_for <= size, "the image is of a super of {made_for} bytes, and this phone's holds {size}");
    Ok(())
}

/// Write `len` bytes of `image` to all of `devices` in one pass, then read
/// each back and compare. A signed image smaller than its partition gets
/// its footer moved to the partition's end, with zeros between, as
/// fastboot does. `progress` hears the bytes written and read back so far,
/// of all.
pub fn flash<R: Read + Seek>(image: &mut R, len: u64, devices: &[PathBuf], progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    write(image, &Layout::whole(len), devices, true, progress)
}

/// Write what `layout` lays out of the file `image` to all of `devices`
/// in one pass, then read each back and compare; what a sparse image
/// leaves out stays as the partition has it. With `footer`, an image that
/// is signed, not sparse and smaller than its partition gets its footer
/// moved to the partition's end, with zeros between, as fastboot moves it
/// in every partition but a logical one, which is as large as its image.
/// `progress` hears the bytes written and read back so far, of all.
pub fn write<R: Read + Seek>(image: &mut R, layout: &Layout, devices: &[PathBuf], footer: bool, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    ensure!(!devices.is_empty(), "no partition to write");
    let mut outs = Vec::with_capacity(devices.len());
    for path in devices {
        let mut f = OpenOptions::new().read(true).write(true).open(path).with_context(|| format!("open {}", path.display()))?;
        let size = f.seek(SeekFrom::End(0)).with_context(|| format!("find the size of {}", path.display()))?;
        outs.push((path, f, size));
    }
    let size = outs[0].2;
    ensure!(outs.iter().all(|(_, _, s)| *s == size), "the partition differs in size between slots");
    let len = layout.size;
    ensure!(len <= size, "the image is {len} bytes, more than the {size} its partition holds");

    let mut runs = layout.runs.clone();
    if footer && !layout.sparse && len >= FOOTER_LEN && len < size {
        let mut end = [0u8; FOOTER_LEN as usize];
        image.seek(SeekFrom::Start(len - FOOTER_LEN)).context("seek to the image's end")?;
        image.read_exact(&mut end).context("read the image's end")?;
        if end.starts_with(FOOTER_MAGIC) {
            runs.push(Run { at: len, len: size - len - FOOTER_LEN, source: Source::Fill([0; 4]) });
            runs.push(Run { at: size - FOOTER_LEN, len: FOOTER_LEN, source: Source::File(len - FOOTER_LEN) });
            runs.retain(|run| run.len != 0);
        }
    }
    let total = runs.iter().map(|run| run.len).sum::<u64>() * outs.len() as u64 * 2;

    let mut buf = vec![0u8; CHUNK];
    let mut written = Sha256::new();
    let mut done = 0;
    for run in &runs {
        match run.source {
            Source::File(from) => {
                image.seek(SeekFrom::Start(from)).with_context(|| format!("seek to {from} in the image"))?;
            }
            // A run starts on a block and a chunk is whole patterns long,
            // so every chunk of it starts the pattern anew.
            Source::Fill(pattern) => buf.chunks_exact_mut(pattern.len()).for_each(|four| four.copy_from_slice(&pattern)),
        }
        for (path, f, _) in &mut outs {
            f.seek(SeekFrom::Start(run.at)).with_context(|| format!("seek to {} in {}", run.at, path.display()))?;
        }
        let mut left = run.len;
        while left > 0 {
            let n = left.min(CHUNK as u64) as usize;
            if let Source::File(_) = run.source {
                image.read_exact(&mut buf[..n]).context("read the image")?;
            }
            written.update(&buf[..n]);
            for (path, f, _) in &mut outs {
                f.write_all(&buf[..n]).with_context(|| format!("write {}", path.display()))?;
                done += n as u64;
                progress(done, total);
            }
            left -= n as u64;
        }
    }
    let written = written.finalize();
    for (path, f, _) in &mut outs {
        f.sync_all().with_context(|| format!("flush {}", path.display()))?;
        drop_cache(f).with_context(|| format!("drop the cached pages of {}", path.display()))?;
    }
    for (path, f, _) in &mut outs {
        let mut back = Sha256::new();
        for run in &runs {
            f.seek(SeekFrom::Start(run.at)).with_context(|| format!("seek to {} in {}", run.at, path.display()))?;
            let mut left = run.len;
            while left > 0 {
                let n = left.min(CHUNK as u64) as usize;
                f.read_exact(&mut buf[..n]).with_context(|| format!("read back {}", path.display()))?;
                back.update(&buf[..n]);
                left -= n as u64;
                done += n as u64;
                progress(done, total);
            }
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

    fn kind_of(image: &[u8]) -> Result<ImageKind> {
        Ok(open(&mut Cursor::new(image), image.len() as u64)?.kind)
    }

    #[test]
    fn boot_images_go_where_what_they_carry_fits() {
        let recovery = archive(&["init", "system", "system/bin", "system/bin/recovery", "sbin"]);
        let generic = archive(&["init", "first_stage_ramdisk", "system/bin/snapuserd"]);

        let kind = kind_of(&boot_image(0, &lz4_legacy(&recovery))).unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery });
        assert_eq!(kind.partitions(), ["recovery"]);
        let kind = kind_of(&boot_image(0, &gzip(&generic))).unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Generic });
        assert_eq!(kind.partitions(), ["init_boot"]);
        let kind = kind_of(&boot_image(5000, &recovery)).unwrap();
        assert_eq!(kind.partitions(), ["recovery", "boot"]);
        let kind = kind_of(&boot_image(5000, &lz4_legacy(&generic))).unwrap();
        assert_eq!(kind, ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Generic });
        assert_eq!(kind.partitions(), ["boot"]);
        assert_eq!(kind_of(&boot_image(5000, &[])).unwrap(), ImageKind::Boot { kernel: true, ramdisk: Ramdisk::None });

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
        assert_eq!(kind_of(bytes.get_ref()).unwrap(), ImageKind::VendorBoot);
        assert_eq!(kind_of(&[0xd7, 0xb7, 0xab, 0x1e, 0, 0, 0, 0x40]).unwrap(), ImageKind::Dtbo);
        assert_eq!(kind_of(b"AVB0\0\0\0\x01").unwrap(), ImageKind::Vbmeta);
        assert_eq!(kind_of(b"\x7fELF\x02\x01\x01").unwrap(), ImageKind::Other, "a bootloader's program, which only its name places");
        assert_eq!(kind_of(&[]).unwrap(), ImageKind::Other);
    }

    /// The first bytes of a filesystem image: zeros, but for `magic` at
    /// the offset its superblock keeps it at.
    fn filesystem((at, magic): (usize, &[u8])) -> Vec<u8> {
        let mut image = vec![0u8; 16384];
        image[at..at + magic.len()].copy_from_slice(magic);
        image
    }

    #[test]
    fn filesystems_and_a_whole_super_are_told_by_what_they_start_with() {
        assert_eq!(kind_of(&filesystem(EROFS_MAGIC)).unwrap(), ImageKind::Filesystem(Filesystem::Erofs));
        assert_eq!(kind_of(&filesystem(F2FS_MAGIC)).unwrap(), ImageKind::Filesystem(Filesystem::F2fs));
        assert_eq!(kind_of(&filesystem(EXT4_MAGIC)).unwrap(), ImageKind::Filesystem(Filesystem::Ext4));
        assert_eq!(kind_of(&filesystem(EROFS_MAGIC)[..1026]).unwrap(), ImageKind::Other, "a file that ends inside the magic number");
        assert_eq!(kind_of(&vec![0u8; 16384]).unwrap(), ImageKind::Other, "an erased partition");
        // The start of a Xiaomi 13 Ultra's own super.
        let ishtar = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../skyward-lp/tests/ishtar-super-metadata.bin")).unwrap();
        assert_eq!(kind_of(&ishtar).unwrap(), ImageKind::Super);
        let mut image = Cursor::new(&ishtar);
        check_super(&mut image, 9_663_676_416).unwrap();
        check_super(&mut image, 1 << 40).unwrap();
        let err = check_super(&mut image, 9_663_676_416 - 4096).unwrap_err();
        assert!(format!("{err:#}").contains("a super of 9663676416 bytes"), "{err:#}");
    }

    /// A sparse image with 4096-byte blocks, made of `chunks`: each a
    /// type, the blocks it stands for, and the bytes it brings.
    fn sparse(chunks: &[(ChunkType, u32, &[u8])]) -> Vec<u8> {
        let header = FileHeader {
            block_size: android_sparse_image::DEFAULT_BLOCKSIZE,
            blocks: chunks.iter().filter(|(kind, ..)| *kind != ChunkType::Crc32).map(|(_, blocks, _)| blocks).sum(),
            chunks: chunks.len() as u32,
            checksum: 0,
        };
        let mut out = header.to_bytes().to_vec();
        for &(chunk_type, chunk_size, data) in chunks {
            let total_size = (CHUNK_HEADER_BYTES_LEN + data.len()) as u32;
            out.extend_from_slice(&ChunkHeader { chunk_type, chunk_size, total_size }.to_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    /// `blocks` blocks of bytes that differ along the image, from `seed`.
    fn blocks(blocks: usize, seed: u8) -> Vec<u8> {
        (0..blocks * 4096).map(|i| (i % 253) as u8 ^ seed).collect()
    }

    #[test]
    fn a_sparse_image_reads_as_the_image_it_stands_for() {
        let (first, second) = (blocks(2, 0x11), blocks(1, 0x77));
        let file = sparse(&[
            (ChunkType::Raw, 2, &first),
            (ChunkType::DontCare, 3, &[]),
            (ChunkType::Fill, 2, &[1, 2, 3, 4]),
            (ChunkType::Crc32, 0, &[0; 4]),
            (ChunkType::Raw, 1, &second),
            (ChunkType::DontCare, 1, &[]),
        ]);
        let layout = Layout::read(&mut Cursor::new(&file), file.len() as u64).unwrap();
        assert!(layout.sparse);
        assert_eq!(layout.size, 9 * 4096);
        assert_eq!(layout.runs.iter().map(|run| (run.at, run.len)).collect::<Vec<_>>(), [(0, 8192), (5 * 4096, 8192), (7 * 4096, 4096)]);
        let mut whole = Vec::new();
        let mut source = Cursor::new(&file);
        Expanded::new(&mut source, &layout).read_to_end(&mut whole).unwrap();
        let mut expected = first.clone();
        expected.extend_from_slice(&[0; 3 * 4096]);
        expected.extend([1u8, 2, 3, 4].iter().cycle().take(2 * 4096));
        expected.extend_from_slice(&second);
        expected.extend_from_slice(&[0; 4096]);
        assert!(whole == expected, "what it leaves out reads as zeros");
        // Read from the middle of a fill, across into the next chunk.
        let mut view = Expanded::new(&mut source, &layout);
        view.seek(SeekFrom::Start(7 * 4096 - 3)).unwrap();
        let mut across = [0u8; 6];
        view.read_exact(&mut across).unwrap();
        assert_eq!(across, [2, 3, 4, second[0], second[1], second[2]]);
        assert_eq!(view.seek(SeekFrom::End(-1)).unwrap(), 9 * 4096 - 1);

        // A file that is no sparse image goes in as it is.
        let plain = Layout::read(&mut Cursor::new(&first), first.len() as u64).unwrap();
        assert_eq!(plain, Layout::whole(8192));
        assert!(!plain.sparse);
    }

    #[test]
    fn what_a_sparse_image_holds_is_told_as_of_any_other() {
        let recovery = archive(&["init", "system/bin/recovery"]);
        let mut boot = boot_image(5000, &lz4_legacy(&recovery));
        boot.resize(boot.len().div_ceil(4096) * 4096, 0);
        let (head, rest) = boot.split_at(4096);
        let file = sparse(&[(ChunkType::Raw, 1, head), (ChunkType::Raw, (rest.len() / 4096) as u32, rest), (ChunkType::DontCare, 100, &[])]);
        let image = open(&mut Cursor::new(&file), file.len() as u64).unwrap();
        assert_eq!(image.kind, ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery }, "its ramdisk is read through the chunks");
        assert_eq!(image.layout.size, boot.len() as u64 + 100 * 4096);

        let erofs = filesystem(EROFS_MAGIC);
        let file = sparse(&[(ChunkType::Raw, 4, &erofs), (ChunkType::Fill, 12, &[0; 4])]);
        assert_eq!(kind_of(&file).unwrap(), ImageKind::Filesystem(Filesystem::Erofs));
        let file = sparse(&[(ChunkType::DontCare, 16, &[])]);
        assert_eq!(kind_of(&file).unwrap(), ImageKind::Other, "an image of nothing");
    }

    #[test]
    fn a_sparse_image_that_does_not_add_up_is_refused() {
        let data = blocks(2, 0);
        let read = |file: &[u8]| Layout::read(&mut Cursor::new(file), file.len() as u64).map_err(|e| format!("{e:#}"));
        let good = sparse(&[(ChunkType::Raw, 2, &data), (ChunkType::DontCare, 1, &[])]);
        assert!(read(&good).is_ok());
        assert!(read(&good[..good.len() - 12 - 4096]).unwrap_err().contains("past the"), "a file cut short");
        assert!(read(&sparse(&[(ChunkType::Raw, 3, &data)])).unwrap_err().contains("brings 8192 bytes for 12288"));
        assert!(read(&sparse(&[(ChunkType::Fill, 1, &[0; 8])])).unwrap_err().contains("a fill takes 4"));
        assert!(read(&sparse(&[(ChunkType::DontCare, 1, &[0; 4])])).unwrap_err().contains("leaves as it is"));
        // A header that tells of more blocks than the chunks make up.
        let mut short = good.clone();
        short[16..20].copy_from_slice(&9u32.to_le_bytes());
        assert!(read(&short).unwrap_err().contains("12288 bytes of an image of 36864"), "{:?}", read(&short));
        let mut version = good.clone();
        version[4] = 2;
        assert!(read(&version).unwrap_err().contains("sparse image's header"));
        let mut unknown = good;
        unknown[28] = 0xff;
        assert!(read(&unknown).unwrap_err().contains("chunk 0"));
    }

    /// Links to partitions named `names`, each an empty file.
    fn phone(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            std::fs::File::create(dir.path().join(name)).unwrap();
        }
        dir
    }

    fn names(targets: &[Target]) -> Vec<(&str, Place)> {
        targets.iter().map(|t| (t.name.as_str(), t.place)).collect()
    }

    #[test]
    fn only_partitions_the_phone_has_in_both_slots_are_offered() {
        let dir = phone(&["boot_a", "boot_b", "recovery_a", "recovery_b", "init_boot_a", "init_boot_b", "vbmeta_a", "vbmeta_b", "vbmeta_system_a"]);
        let both = ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery };
        assert_eq!(names(&targets(both, None, dir.path(), &[]).unwrap()), [("recovery", Place::Slotted), ("boot", Place::Slotted)]);
        assert_eq!(names(&targets(ImageKind::Vbmeta, None, dir.path(), &[]).unwrap()), [("vbmeta", Place::Slotted)]);
        assert!(targets(ImageKind::Dtbo, None, dir.path(), &[]).unwrap().is_empty());
    }

    #[test]
    fn a_files_name_says_which_partition_comes_first_and_places_what_nothing_else_does() {
        assert_eq!(named("vendor_dlkm.img"), "vendor_dlkm");
        assert_eq!(named("vendor_dlkm_b.img"), "vendor_dlkm");
        assert_eq!(named("system_a.sparse.img"), "system_a.sparse");
        assert_eq!(named("abl.elf"), "abl");
        assert_eq!(named("modem"), "modem");

        let dir = phone(&[
            "boot_a", "boot_b", "recovery_a", "recovery_b", "vbmeta_a", "vbmeta_b", "vbmeta_system_a", "vbmeta_system_b", "modem_a", "modem_b",
            "super", "persist", "userdata", "sda",
        ]);
        let logical: Vec<String> = ["odm", "system", "vendor"].map(String::from).to_vec();
        let of = |kind, named| targets(kind, named, dir.path(), &logical).unwrap();
        let both = ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery };
        assert_eq!(names(&of(both, Some("boot"))), [("boot", Place::Slotted), ("recovery", Place::Slotted)]);
        assert_eq!(names(&of(both, Some("vendor"))), [("recovery", Place::Slotted), ("boot", Place::Slotted)], "a name it does not fit says nothing");
        assert_eq!(names(&of(ImageKind::Vbmeta, Some("vbmeta_system"))), [("vbmeta_system", Place::Slotted), ("vbmeta", Place::Slotted)]);

        let erofs = ImageKind::Filesystem(Filesystem::Erofs);
        assert_eq!(names(&of(erofs, None)), [("odm", Place::Logical), ("system", Place::Logical), ("vendor", Place::Logical)]);
        assert_eq!(names(&of(erofs, Some("vendor"))), [("vendor", Place::Logical), ("odm", Place::Logical), ("system", Place::Logical)]);
        assert_eq!(
            names(&of(ImageKind::Filesystem(Filesystem::Ext4), Some("modem"))),
            [("modem", Place::Slotted), ("odm", Place::Logical), ("system", Place::Logical), ("vendor", Place::Logical)],
            "a filesystem named for a partition the phone has in both slots"
        );

        assert_eq!(names(&of(ImageKind::Other, Some("modem"))), [("modem", Place::Slotted)]);
        assert_eq!(names(&of(ImageKind::Other, Some("system"))), [("system", Place::Logical)]);
        assert!(of(ImageKind::Other, None).is_empty(), "nothing says where it goes");
        for kept in ["persist", "userdata", "sda", "super", "no_such"] {
            assert!(of(ImageKind::Other, Some(kept)).is_empty(), "{kept} is the phone's own to keep");
        }
        assert_eq!(names(&of(erofs, Some("userdata"))).len(), 3, "a filesystem still fits the logical partitions");

        assert_eq!(names(&of(ImageKind::Super, Some("vendor"))), [("super", Place::Single)]);
        let none = phone(&["boot_a", "boot_b"]);
        assert!(targets(ImageKind::Super, None, none.path(), &[]).unwrap().is_empty());
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

    /// Write the image file `file` to `devices` as it lays itself out.
    fn write_all(file: &[u8], devices: &[PathBuf], footer: bool) -> Result<Vec<(u64, u64)>> {
        let layout = Layout::read(&mut Cursor::new(file), file.len() as u64)?;
        let mut seen = Vec::new();
        write(&mut Cursor::new(file), &layout, devices, footer, &mut |done, total| seen.push((done, total)))?;
        Ok(seen)
    }

    #[test]
    fn a_sparse_image_is_written_chunk_by_chunk_and_what_it_leaves_out_stays() {
        let dir = tempfile::tempdir().unwrap();
        let slots = [partition(dir.path(), "modem_a", 12 * 4096), partition(dir.path(), "modem_b", 12 * 4096)];
        let (first, second) = (blocks(2, 0x11), blocks(1, 0x77));
        let file = sparse(&[
            (ChunkType::Raw, 2, &first),
            (ChunkType::DontCare, 3, &[]),
            (ChunkType::Fill, 2, &[1, 2, 3, 4]),
            (ChunkType::Raw, 1, &second),
            (ChunkType::DontCare, 1, &[]),
        ]);
        let seen = write_all(&file, &slots, true).unwrap();
        for slot in &slots {
            let got = std::fs::read(slot).unwrap();
            assert_eq!(&got[..8192], &first[..]);
            assert!(got[8192..5 * 4096].iter().all(|&b| b == 0xaa), "left as the partition had it");
            assert!(got[5 * 4096..7 * 4096].chunks(4).all(|four| four == [1, 2, 3, 4]));
            assert_eq!(&got[7 * 4096..8 * 4096], &second[..]);
            assert!(got[8 * 4096..].iter().all(|&b| b == 0xaa));
        }
        let total = 5 * 4096 * 2 * 2;
        assert_eq!(seen.last(), Some(&(total, total)), "only what is written is counted, and read back");
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));

        let large = sparse(&[(ChunkType::DontCare, 13, &[])]);
        let err = write_all(&large, &slots, true).unwrap_err();
        assert!(err.to_string().contains("53248"), "{err}");
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_a_sparse_image_goes_into_a_block_device_as_into_a_partition() {
        let run = |args: &[&str]| {
            let out = std::process::Command::new("/system/bin/losetup").args(args).output().unwrap();
            assert!(out.status.success(), "losetup {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        let dir = tempfile::tempdir().unwrap();
        let backing = partition(dir.path(), "partition", 4096 * 4096);
        let device = PathBuf::from(run(&["-f", "--show", backing.to_str().unwrap()]));
        // More than one chunk of writing, so that a run is read and
        // written in pieces.
        let (first, second) = (blocks(700, 0x11), blocks(3, 0x77));
        let file = sparse(&[
            (ChunkType::Raw, 700, &first),
            (ChunkType::DontCare, 1000, &[]),
            (ChunkType::Fill, 600, &[9, 8, 7, 6]),
            (ChunkType::Raw, 3, &second),
            (ChunkType::DontCare, 1793, &[]),
        ]);
        let seen = write_all(&file, std::slice::from_ref(&device), false).unwrap();
        let total = 1303 * 4096 * 2;
        assert_eq!(seen.last(), Some(&(total, total)));
        let got = std::fs::read(&device).unwrap();
        assert!(got[..700 * 4096] == first[..]);
        assert!(got[700 * 4096..1700 * 4096].iter().all(|&b| b == 0xaa), "left as the device had it");
        assert!(got[1700 * 4096..2300 * 4096].chunks(4).all(|four| four == [9, 8, 7, 6]));
        assert!(got[2300 * 4096..2303 * 4096] == second[..]);
        assert!(got[2303 * 4096..].iter().all(|&b| b == 0xaa));
        run(&["-d", device.to_str().unwrap()]);
    }

    #[test]
    fn a_footer_moves_only_in_an_image_that_is_not_sparse_and_where_it_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let size = 16 * 4096;
        let mut image = vec![0x11u8; 2 * 4096 - 64];
        let mut footer = b"AVBf".to_vec();
        footer.resize(FOOTER_LEN as usize, 0x22);
        image.extend_from_slice(&footer);

        // In a logical partition, which is made as large as its image.
        let logical = [partition(dir.path(), "vendor", size)];
        write_all(&image, &logical, false).unwrap();
        let got = std::fs::read(&logical[0]).unwrap();
        assert_eq!(&got[..image.len()], &image[..]);
        assert!(got[image.len()..].iter().all(|&b| b == 0xaa));

        let sparse_image = sparse(&[(ChunkType::Raw, 2, &image), (ChunkType::DontCare, 2, &[])]);
        let slots = [partition(dir.path(), "boot_a", size)];
        write_all(&sparse_image, &slots, true).unwrap();
        let got = std::fs::read(&slots[0]).unwrap();
        assert_eq!(&got[..image.len()], &image[..]);
        assert!(got[image.len()..].iter().all(|&b| b == 0xaa), "a sparse image says itself where everything goes");
    }
}
