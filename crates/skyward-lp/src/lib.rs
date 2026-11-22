use anyhow::{anyhow, bail, ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::mem::size_of;
use std::os::unix::fs::FileExt;
use std::path::Path;
use zerocopy::byteorder::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout, Unaligned};

/// Sectors are 512 bytes whatever the storage's block size.
pub const SECTOR_SIZE: u64 = 512;

pub const ATTR_READONLY: u32 = 1 << 0;
pub const ATTR_SLOT_SUFFIXED: u32 = 1 << 1;
pub const ATTR_UPDATED: u32 = 1 << 2;
pub const ATTR_DISABLED: u32 = 1 << 3;

pub const HEADER_FLAG_VIRTUAL_AB_DEVICE: u32 = 1 << 0;
pub const HEADER_FLAG_OVERLAYS_ACTIVE: u32 = 1 << 1;

const GEOMETRY_MAGIC: u32 = 0x616c_4467;
const GEOMETRY_SIZE: u64 = 4096;
const HEADER_MAGIC: u32 = 0x414c_5030;
const MAJOR_VERSION: u16 = 10;
const MINOR_VERSION_MAX: u16 = 2;
const VERSION_FOR_UPDATED_ATTR: u16 = 1;
const VERSION_FOR_EXPANDED_HEADER: u16 = 2;
/// Kept free at the start of super so it never looks like a boot sector.
const RESERVED_BYTES: u64 = 4096;
const TARGET_LINEAR: u32 = 0;
const TARGET_ZERO: u32 = 1;
const GROUP_SLOT_SUFFIXED: u32 = 1 << 0;
const BLOCK_DEVICE_SLOT_SUFFIXED: u32 = 1 << 0;
const NAME_LEN: usize = 36;

// On-disk structures, little-endian and packed.

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone)]
#[repr(C)]
struct RawGeometry {
    magic: U32,
    struct_size: U32,
    checksum: [u8; 32],
    metadata_max_size: U32,
    metadata_slot_count: U32,
    logical_block_size: U32,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone)]
#[repr(C)]
struct RawTable {
    /// Relative to the end of the header.
    offset: U32,
    num_entries: U32,
    entry_size: U32,
}

/// The 1.2 header; 1.0 and 1.1 headers stop before `flags`.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone)]
#[repr(C)]
struct RawHeader {
    magic: U32,
    major_version: U16,
    minor_version: U16,
    header_size: U32,
    header_checksum: [u8; 32],
    tables_size: U32,
    tables_checksum: [u8; 32],
    partitions: RawTable,
    extents: RawTable,
    groups: RawTable,
    block_devices: RawTable,
    flags: U32,
    reserved: [u8; 124],
}

const HEADER_V1_0_SIZE: usize = 128;

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawPartition {
    name: [u8; NAME_LEN],
    attributes: U32,
    first_extent_index: U32,
    num_extents: U32,
    group_index: U32,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawExtent {
    num_sectors: U64,
    target_type: U32,
    target_data: U64,
    target_source: U32,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawGroup {
    name: [u8; NAME_LEN],
    flags: U32,
    maximum_size: U64,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawBlockDevice {
    first_logical_sector: U64,
    alignment: U32,
    alignment_offset: U32,
    size: U64,
    partition_name: [u8; NAME_LEN],
    flags: U32,
}

const _: () = {
    assert!(size_of::<RawGeometry>() == 52);
    assert!(size_of::<RawHeader>() == 256);
    assert!(std::mem::offset_of!(RawHeader, flags) == HEADER_V1_0_SIZE);
    assert!(size_of::<RawPartition>() == 52);
    assert!(size_of::<RawExtent>() == 24);
    assert!(size_of::<RawGroup>() == 48);
    assert!(size_of::<RawBlockDevice>() == 64);
};

/// How the metadata copies are laid out at the start of super.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub metadata_max_size: u32,
    pub metadata_slot_count: u32,
    pub logical_block_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub geometry: Geometry,
    pub major_version: u16,
    pub minor_version: u16,
    pub header_flags: u32,
    pub partitions: Vec<Partition>,
    pub groups: Vec<Group>,
    /// The first entry is the device holding the metadata itself.
    pub block_devices: Vec<BlockDevice>,
    /// Problems with the primary copies when a backup copy had to be read.
    pub damaged: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub name: String,
    pub attributes: u32,
    /// Index into [`Metadata::groups`].
    pub group: usize,
    pub extents: Vec<Extent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    /// `sectors` sectors starting at `sector` on block device `device`.
    Linear { sectors: u64, device: usize, sector: u64 },
    Zero { sectors: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    /// Bytes; 0 means unlimited.
    pub maximum_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    pub partition_name: String,
    pub first_logical_sector: u64,
    pub alignment: u32,
    pub alignment_offset: u32,
    pub size: u64,
}

impl Extent {
    pub fn sectors(&self) -> u64 {
        match *self {
            Extent::Linear { sectors, .. } | Extent::Zero { sectors } => sectors,
        }
    }
}

impl Partition {
    pub fn sectors(&self) -> u64 {
        self.extents.iter().map(Extent::sectors).sum()
    }

    pub fn bytes(&self) -> u64 {
        self.sectors() * SECTOR_SIZE
    }
}

impl Metadata {
    pub fn partition(&self, name: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.name == name)
    }

    /// Make partition `name` hold `bytes`, rounded up to whole logical
    /// blocks, as liblp resizes one. A partition that shrinks gives up its
    /// last sectors. One that grows takes what lies unaligned right behind
    /// its last extent, then the free regions of super's block devices in
    /// their order; where they hold too little, or its group would hold
    /// more than it may, nothing changes.
    pub fn resize(&mut self, name: &str, bytes: u64) -> Result<()> {
        let block = u64::from(self.geometry.logical_block_size);
        ensure!(block != 0 && block.is_multiple_of(SECTOR_SIZE), "logical blocks of {block} bytes are not whole sectors");
        let wanted = bytes.div_ceil(block).checked_mul(block).with_context(|| format!("{bytes} bytes are more than a partition holds"))?;
        let index = self.partitions.iter().position(|p| p.name == name).with_context(|| format!("super holds no {name}"))?;
        let held = self.partitions[index].bytes();
        if wanted < held {
            shrink(&mut self.partitions[index].extents, (held - wanted) / SECTOR_SIZE);
        } else if wanted > held {
            let group = &self.groups[self.partitions[index].group];
            if group.maximum_size != 0 {
                let others: u64 = self.partitions.iter().filter(|p| p.group == self.partitions[index].group).map(Partition::bytes).sum::<u64>() - held;
                ensure!(
                    others + wanted <= group.maximum_size,
                    "{name} at {wanted} bytes would bring its group {} to {}, and it holds {} at most",
                    group.name,
                    others + wanted,
                    group.maximum_size
                );
            }
            let taken = self.take((wanted - held) / SECTOR_SIZE, index, block / SECTOR_SIZE).with_context(|| format!("make {name} hold {wanted} bytes"))?;
            let extents = &mut self.partitions[index].extents;
            for new in taken {
                match (extents.last_mut(), new) {
                    (Some(Extent::Linear { sectors, device, sector }), Extent::Linear { sectors: more, device: on, sector: at })
                        if *device == on && *sector + *sectors == at =>
                    {
                        *sectors += more
                    }
                    _ => extents.push(new),
                }
            }
        }
        Ok(())
    }

    /// Extents of `needed` sectors in all for partition `index` to grow by,
    /// each of whole logical blocks of `per_block` sectors.
    fn take(&self, mut needed: u64, index: usize, per_block: u64) -> Result<Vec<Extent>> {
        let mut taken = Vec::new();
        let used = self.used();
        // The sectors between the end of its last extent and where the next
        // aligned region begins are in no free region.
        if let Some(&Extent::Linear { sectors, device, sector }) = self.partitions[index].extents.last() {
            let end = sector + sectors;
            let aligned = align(&self.block_devices[device], end)?;
            let gap = (aligned - end).min(needed);
            let free = !used[device].iter().any(|&(start, stop)| start < end + gap && end < stop);
            if gap != 0 && free && aligned <= self.block_devices[device].size / SECTOR_SIZE {
                ensure!(gap.is_multiple_of(per_block), "the {gap} sectors behind sector {end} are not whole logical blocks");
                taken.push(Extent::Linear { sectors: gap, device, sector: end });
                needed -= gap;
            }
        }
        for (device, start, stop) in self.free(&used)? {
            if needed == 0 {
                break;
            }
            ensure!(
                start.is_multiple_of(per_block) && (stop - start).is_multiple_of(per_block),
                "the free sectors {start} to {stop} are not whole logical blocks"
            );
            let sectors = needed.min(stop - start);
            taken.push(Extent::Linear { sectors, device, sector: start });
            needed -= sectors;
        }
        ensure!(needed == 0, "super has {} bytes too few free", needed * SECTOR_SIZE);
        Ok(taken)
    }

    /// The sectors every partition's extents take on each block device, as
    /// (first, one past the last), in their order on the device.
    fn used(&self) -> Vec<Vec<(u64, u64)>> {
        let mut used = vec![Vec::new(); self.block_devices.len()];
        for extent in self.partitions.iter().flat_map(|p| &p.extents) {
            if let Extent::Linear { sectors, device, sector } = *extent {
                used[device].push((sector, sector + sectors));
            }
        }
        for on in &mut used {
            on.sort_unstable();
        }
        used
    }

    /// The free regions of each block device, as (device, first sector, one
    /// past the last), in the order of the devices and then of the sectors:
    /// what lies between the extents `used` lists, from where the device's
    /// alignment lets a region begin.
    fn free(&self, used: &[Vec<(u64, u64)>]) -> Result<Vec<(usize, u64, u64)>> {
        let mut free = Vec::new();
        for (k, device) in self.block_devices.iter().enumerate() {
            let last = device.size / SECTOR_SIZE;
            let mut end = device.first_logical_sector;
            for &(start, stop) in used[k].iter().chain([&(last, last)]) {
                if end < start {
                    let aligned = align(device, end)?;
                    if aligned < start {
                        free.push((k, aligned, start));
                    }
                }
                end = end.max(stop);
            }
        }
        Ok(free)
    }
}

/// Take `sectors` sectors off the end of a partition's extents.
fn shrink(extents: &mut Vec<Extent>, mut sectors: u64) {
    while sectors != 0 {
        let last = extents.last_mut().expect("a partition gives up no more sectors than its extents hold");
        let held = last.sectors();
        if held > sectors {
            match last {
                Extent::Linear { sectors: of, .. } | Extent::Zero { sectors: of } => *of -= sectors,
            }
            return;
        }
        sectors -= held;
        extents.pop();
    }
}

/// The first sector at or after `sector` where the block device's alignment
/// lets a region begin.
fn align(device: &BlockDevice, sector: u64) -> Result<u64> {
    if device.alignment == 0 {
        return Ok(sector);
    }
    let (alignment, offset) = (u64::from(device.alignment), u64::from(device.alignment_offset));
    let byte = sector * SECTOR_SIZE;
    let aligned = byte.div_ceil(alignment) * alignment + offset;
    let aligned = if aligned >= alignment && aligned - alignment >= byte { aligned - alignment } else { aligned };
    ensure!(aligned.is_multiple_of(SECTOR_SIZE), "{} aligns to {alignment} bytes and {offset} more, which is no whole sector", device.partition_name);
    Ok(aligned / SECTOR_SIZE)
}

/// Read the metadata for `slot` (0 is _a, 1 is _b) from a super partition
/// or an image of its start. A damaged primary geometry or metadata copy is
/// replaced by its backup, and the damage is listed in `damaged`.
pub fn read(super_dev: &Path, slot: u32) -> Result<Metadata> {
    let file = File::open(super_dev).with_context(|| format!("open {}", super_dev.display()))?;
    let at = |offset: u64, len: usize| -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        file.read_exact_at(&mut buf, offset)
            .with_context(|| format!("read {len} bytes at {offset} in {}", super_dev.display()))?;
        Ok(buf)
    };

    let mut damaged = Vec::new();
    let geometry = match Geometry::parse(&at(RESERVED_BYTES, GEOMETRY_SIZE as usize)?) {
        Ok(g) => g,
        Err(e) => {
            damaged.push(format!("primary geometry: {e:#}"));
            Geometry::parse(&at(RESERVED_BYTES + GEOMETRY_SIZE, GEOMETRY_SIZE as usize)?)
                .with_context(|| format!("both geometry copies are damaged; primary: {e:#}"))?
        }
    };
    ensure!(
        slot < geometry.metadata_slot_count,
        "metadata slot {slot} does not exist; super keeps {}",
        geometry.metadata_slot_count
    );

    let size = geometry.metadata_max_size as usize;
    let mut metadata = match parse_metadata(&geometry, &at(geometry.primary_offset(slot), size)?, Some(slot)) {
        Ok(m) => m,
        Err(e) => {
            damaged.push(format!("primary metadata for slot {slot}: {e:#}"));
            parse_metadata(&geometry, &at(geometry.backup_offset(slot), size)?, Some(slot))
                .with_context(|| format!("both metadata copies for slot {slot} are damaged; primary: {e:#}"))?
        }
    };
    metadata.damaged = damaged;
    Ok(metadata)
}

/// Read metadata kept in a file of its own, as libfiemap keeps where its
/// images lie on the data partition: the geometry, then a single copy of
/// the metadata, its names taken as they are.
pub fn read_image(path: &Path) -> Result<Metadata> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let geometry = data.get(..GEOMETRY_SIZE as usize).with_context(|| format!("{} ends inside its geometry", path.display()))?;
    let geometry = Geometry::parse(geometry).with_context(|| format!("read the geometry of {}", path.display()))?;
    parse_metadata(&geometry, &data[GEOMETRY_SIZE as usize..], None).with_context(|| format!("read the metadata of {}", path.display()))
}

/// Read the metadata for `slot` out of an image of a whole super, which
/// `at` reads so many bytes of from an offset: its primary copies, as the
/// image is made with them.
pub fn read_with(mut at: impl FnMut(u64, usize) -> Result<Vec<u8>>, slot: u32) -> Result<Metadata> {
    let geometry = Geometry::parse(&at(RESERVED_BYTES, GEOMETRY_SIZE as usize)?).context("read the geometry")?;
    ensure!(slot < geometry.metadata_slot_count, "metadata slot {slot} does not exist; the image keeps {}", geometry.metadata_slot_count);
    let copy = at(geometry.primary_offset(slot), geometry.metadata_max_size as usize)?;
    parse_metadata(&geometry, &copy, Some(slot)).with_context(|| format!("read the metadata for slot {slot}"))
}

/// Whether `block`, the 4096 bytes that follow the first 4096 of a
/// partition or an image, is the geometry a super begins with.
pub fn is_geometry(block: &[u8]) -> bool {
    Geometry::parse(block).is_ok()
}

/// Write `md` over the metadata of each of `slots` in super, as liblp
/// updates a partition table: where only one copy of a slot's metadata
/// reads, or the two differ, they are first made the same, so that one of
/// them reads at every moment of the writing; then the primary copy is
/// written, and the backup after it.
pub fn write(super_dev: &Path, md: &Metadata, slots: &[u32]) -> Result<()> {
    let blob = serialize(md)?;
    let mut file = OpenOptions::new().read(true).write(true).open(super_dev).with_context(|| format!("open {}", super_dev.display()))?;
    let size = file.seek(SeekFrom::End(0)).with_context(|| format!("find the size of {}", super_dev.display()))?;
    let read = |file: &File, offset: u64, len: usize| -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        file.read_exact_at(&mut buf, offset).with_context(|| format!("read {len} bytes at {offset} in {}", super_dev.display()))?;
        Ok(buf)
    };
    let put = |file: &File, offset: u64, bytes: &[u8]| -> Result<()> {
        file.write_all_at(bytes, offset).with_context(|| format!("write {} bytes at {offset} in {}", bytes.len(), super_dev.display()))?;
        file.sync_data().with_context(|| format!("flush {}", super_dev.display()))
    };
    let geometry = Geometry::parse(&read(&file, RESERVED_BYTES, GEOMETRY_SIZE as usize)?)
        .or_else(|_| Geometry::parse(&read(&file, RESERVED_BYTES + GEOMETRY_SIZE, GEOMETRY_SIZE as usize)?))
        .with_context(|| format!("read the geometry of {}", super_dev.display()))?;
    ensure!(geometry == md.geometry, "{} is laid out as {geometry:?}, and the metadata as {:?}", super_dev.display(), md.geometry);
    let max = geometry.metadata_max_size as usize;
    ensure!(blob.len() <= max, "the metadata takes {} bytes, and a copy of it {max} at most", blob.len());
    check(md, size).with_context(|| format!("the metadata does not fit {}", super_dev.display()))?;
    for &slot in slots {
        ensure!(slot < geometry.metadata_slot_count, "metadata slot {slot} does not exist; super keeps {}", geometry.metadata_slot_count);
        let (primary, backup) = (geometry.primary_offset(slot), geometry.backup_offset(slot));
        let (first, second) = (copy(&geometry, &read(&file, primary, max)?), copy(&geometry, &read(&file, backup, max)?));
        match (&first, &second) {
            (Some(first), second) if second.as_ref() != Some(first) => put(&file, backup, first)?,
            (None, Some(second)) => put(&file, primary, second)?,
            _ => {}
        }
        put(&file, primary, &blob)?;
        put(&file, backup, &blob)?;
    }
    Ok(())
}

/// The bytes of a metadata copy in `buf` that reads, its header and tables.
fn copy(geometry: &Geometry, buf: &[u8]) -> Option<Vec<u8>> {
    parse_metadata(geometry, buf, None).ok()?;
    let (header, _) = RawHeader::read_from_prefix(buf).ok()?;
    Some(buf[..header.header_size.get() as usize + header.tables_size.get() as usize].to_vec())
}

/// Check that `md` describes a super of `size` bytes: its copies fit before
/// the first sector partitions may take, and every extent lies on its block
/// device.
fn check(md: &Metadata, size: u64) -> Result<()> {
    let first = md.block_devices.first().context("no block devices")?;
    ensure!(first.size == size, "{} holds {size} bytes, and its metadata says {}", first.partition_name, first.size);
    ensure!(md.geometry.metadata_region() <= first.first_logical_sector * SECTOR_SIZE, "metadata overlaps the partitions");
    for p in &md.partitions {
        ensure!(p.group < md.groups.len(), "{} is in group {}", p.name, p.group);
        for extent in &p.extents {
            if let Extent::Linear { sectors, device, sector } = *extent {
                let on = md.block_devices.get(device).with_context(|| format!("{} has an extent on block device {device}", p.name))?;
                ensure!(
                    sector >= on.first_logical_sector && sector + sectors <= on.size / SECTOR_SIZE,
                    "{} has sectors {sector} to {} on {}, which holds {} to {}",
                    p.name,
                    sector + sectors,
                    on.partition_name,
                    on.first_logical_sector,
                    on.size / SECTOR_SIZE
                );
            }
        }
    }
    Ok(())
}

/// A name as its field holds it, NUL-padded.
fn field(name: &str) -> Result<[u8; NAME_LEN]> {
    ensure!(name.len() <= NAME_LEN, "{name} is longer than {NAME_LEN} bytes");
    let mut field = [0u8; NAME_LEN];
    field[..name.len()].copy_from_slice(name.as_bytes());
    Ok(field)
}

/// A metadata copy as liblp writes it: the header, then the tables of
/// partitions, extents, groups and block devices, each partition's extents
/// together and in the partitions' order.
fn serialize(md: &Metadata) -> Result<Vec<u8>> {
    ensure!(md.major_version == MAJOR_VERSION && md.minor_version <= MINOR_VERSION_MAX, "unsupported version {}.{}", md.major_version, md.minor_version);
    let expanded = md.minor_version >= VERSION_FOR_EXPANDED_HEADER;
    ensure!(expanded || md.header_flags == 0, "a version {}.{} header keeps no flags", md.major_version, md.minor_version);
    let header_size = if expanded { size_of::<RawHeader>() } else { HEADER_V1_0_SIZE };
    let (mut partitions, mut extents, mut groups, mut block_devices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut extent_count = 0u32;
    for p in &md.partitions {
        let raw = RawPartition {
            name: field(&p.name)?,
            attributes: U32::new(p.attributes),
            first_extent_index: U32::new(extent_count),
            num_extents: U32::new(p.extents.len() as u32),
            group_index: U32::new(p.group as u32),
        };
        partitions.extend_from_slice(raw.as_bytes());
        for extent in &p.extents {
            let raw = match *extent {
                Extent::Linear { sectors, device, sector } => RawExtent {
                    num_sectors: U64::new(sectors),
                    target_type: U32::new(TARGET_LINEAR),
                    target_data: U64::new(sector),
                    target_source: U32::new(device as u32),
                },
                Extent::Zero { sectors } => {
                    RawExtent { num_sectors: U64::new(sectors), target_type: U32::new(TARGET_ZERO), target_data: U64::new(0), target_source: U32::new(0) }
                }
            };
            extents.extend_from_slice(raw.as_bytes());
            extent_count += 1;
        }
    }
    for g in &md.groups {
        groups.extend_from_slice(RawGroup { name: field(&g.name)?, flags: U32::new(0), maximum_size: U64::new(g.maximum_size) }.as_bytes());
    }
    for d in &md.block_devices {
        let raw = RawBlockDevice {
            first_logical_sector: U64::new(d.first_logical_sector),
            alignment: U32::new(d.alignment),
            alignment_offset: U32::new(d.alignment_offset),
            size: U64::new(d.size),
            partition_name: field(&d.partition_name)?,
            flags: U32::new(0),
        };
        block_devices.extend_from_slice(raw.as_bytes());
    }
    let table = |offset: usize, entries: usize, entry: usize| RawTable {
        offset: U32::new(offset as u32),
        num_entries: U32::new(entries as u32),
        entry_size: U32::new(entry as u32),
    };
    let tables = [partitions, extents, groups, block_devices].concat();
    let mut header = RawHeader::new_zeroed();
    header.magic = U32::new(HEADER_MAGIC);
    header.major_version = U16::new(md.major_version);
    header.minor_version = U16::new(md.minor_version);
    header.header_size = U32::new(header_size as u32);
    header.tables_size = U32::new(tables.len() as u32);
    header.tables_checksum = sha256(&tables);
    let extents_at = md.partitions.len() * size_of::<RawPartition>();
    let groups_at = extents_at + extent_count as usize * size_of::<RawExtent>();
    let block_devices_at = groups_at + md.groups.len() * size_of::<RawGroup>();
    header.partitions = table(0, md.partitions.len(), size_of::<RawPartition>());
    header.extents = table(extents_at, extent_count as usize, size_of::<RawExtent>());
    header.groups = table(groups_at, md.groups.len(), size_of::<RawGroup>());
    header.block_devices = table(block_devices_at, md.block_devices.len(), size_of::<RawBlockDevice>());
    header.flags = U32::new(md.header_flags);
    header.header_checksum = sha256(&header.as_bytes()[..header_size]);
    Ok([&header.as_bytes()[..header_size], &tables].concat())
}

impl Geometry {
    fn parse(block: &[u8]) -> Result<Geometry> {
        let (raw, _) = RawGeometry::read_from_prefix(block).map_err(|_| anyhow!("short geometry block"))?;
        ensure!(raw.magic.get() == GEOMETRY_MAGIC, "bad magic {:#x}", raw.magic.get());
        let size = raw.struct_size.get() as usize;
        ensure!(size == size_of::<RawGeometry>(), "struct size {size}");
        let mut unsummed = raw.clone();
        unsummed.checksum = [0; 32];
        ensure!(sha256(&unsummed.as_bytes()[..size]) == raw.checksum, "bad checksum");
        let g = Geometry {
            metadata_max_size: raw.metadata_max_size.get(),
            metadata_slot_count: raw.metadata_slot_count.get(),
            logical_block_size: raw.logical_block_size.get(),
        };
        ensure!(g.metadata_slot_count != 0, "no metadata slots");
        ensure!(
            u64::from(g.metadata_max_size) % SECTOR_SIZE == 0,
            "metadata size {} is not a whole number of sectors",
            g.metadata_max_size
        );
        Ok(g)
    }

    fn primary_offset(&self, slot: u32) -> u64 {
        RESERVED_BYTES + GEOMETRY_SIZE * 2 + u64::from(self.metadata_max_size) * u64::from(slot)
    }

    fn backup_offset(&self, slot: u32) -> u64 {
        self.primary_offset(slot) + u64::from(self.metadata_max_size) * u64::from(self.metadata_slot_count)
    }

    /// Bytes from the start of super to the end of the last metadata copy.
    fn metadata_region(&self) -> u64 {
        RESERVED_BYTES
            + (GEOMETRY_SIZE + u64::from(self.metadata_max_size) * u64::from(self.metadata_slot_count)) * 2
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

impl RawTable {
    fn fits(&self, tables_size: u32) -> bool {
        let offset = u64::from(self.offset.get());
        let size = u64::from(self.num_entries.get()) * u64::from(self.entry_size.get());
        offset <= u64::from(tables_size) && size <= u64::from(tables_size) - offset
    }

    /// Each entry's first `size_of::<T>()` bytes; `fits` has checked the bounds.
    fn entries<T: FromBytes>(&self, tables: &[u8]) -> Vec<T> {
        let (offset, entry) = (self.offset.get() as usize, self.entry_size.get() as usize);
        (0..self.num_entries.get() as usize)
            .map(|i| {
                let start = offset + i * entry;
                T::read_from_bytes(&tables[start..start + size_of::<T>()]).expect("slice has the entry's size")
            })
            .collect()
    }
}

/// A NUL-padded name; a name filling the whole field has no terminator.
fn name(field: &[u8; NAME_LEN]) -> Result<String> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    String::from_utf8(field[..end].to_vec()).map_err(|_| anyhow!("name {:?} is not text", &field[..end]))
}

/// Names flagged as slot-suffixed (retrofit devices) get the suffix of the
/// slot read, where one is.
fn suffixed(name: String, flagged: bool, slot: Option<u32>) -> Result<String> {
    let Some(slot) = slot.filter(|_| flagged) else { return Ok(name) };
    let suffix = match slot {
        0 => "_a",
        1 => "_b",
        _ => bail!("{name} needs a slot suffix, and slot {slot} has none"),
    };
    let full = name + suffix;
    ensure!(full.len() <= NAME_LEN, "{full} is longer than {NAME_LEN} bytes");
    Ok(full)
}

fn parse_metadata(geometry: &Geometry, buf: &[u8], slot: Option<u32>) -> Result<Metadata> {
    ensure!(buf.len() >= size_of::<RawHeader>(), "short metadata copy");
    let mut header = RawHeader::new_zeroed();
    header.as_mut_bytes()[..HEADER_V1_0_SIZE].copy_from_slice(&buf[..HEADER_V1_0_SIZE]);
    ensure!(header.magic.get() == HEADER_MAGIC, "bad magic {:#x}", header.magic.get());
    let (major, minor) = (header.major_version.get(), header.minor_version.get());
    ensure!(major == MAJOR_VERSION && minor <= MINOR_VERSION_MAX, "unsupported version {major}.{minor}");
    let expected = if minor < VERSION_FOR_EXPANDED_HEADER { HEADER_V1_0_SIZE } else { size_of::<RawHeader>() };
    let header_size = header.header_size.get() as usize;
    ensure!(header_size == expected, "header size {header_size} for version {major}.{minor}");
    header.as_mut_bytes()[HEADER_V1_0_SIZE..header_size].copy_from_slice(&buf[HEADER_V1_0_SIZE..header_size]);
    let mut unsummed = header.clone();
    unsummed.header_checksum = [0; 32];
    ensure!(sha256(&unsummed.as_bytes()[..header_size]) == header.header_checksum, "bad header checksum");

    let tables_size = header.tables_size.get();
    for (what, table) in [
        ("partition", &header.partitions),
        ("extent", &header.extents),
        ("group", &header.groups),
        ("block device", &header.block_devices),
    ] {
        ensure!(table.fits(tables_size), "{what} table runs past the tables");
    }
    for (what, table, size) in [
        ("partition", &header.partitions, size_of::<RawPartition>()),
        ("extent", &header.extents, size_of::<RawExtent>()),
        ("group", &header.groups, size_of::<RawGroup>()),
    ] {
        ensure!(table.entry_size.get() as usize == size, "{what} entries take {} bytes", table.entry_size.get());
    }
    ensure!(
        header.block_devices.entry_size.get() as usize >= size_of::<RawBlockDevice>(),
        "block device entries take {} bytes",
        header.block_devices.entry_size.get()
    );
    ensure!(tables_size <= geometry.metadata_max_size, "tables take {tables_size} bytes");
    let tables = buf
        .get(header_size..header_size + tables_size as usize)
        .context("tables run past the metadata copy")?;
    ensure!(sha256(tables) == header.tables_checksum, "bad tables checksum");

    let block_devices = header
        .block_devices
        .entries::<RawBlockDevice>(tables)
        .iter()
        .map(|d| {
            Ok(BlockDevice {
                partition_name: suffixed(name(&d.partition_name)?, d.flags.get() & BLOCK_DEVICE_SLOT_SUFFIXED != 0, slot)?,
                first_logical_sector: d.first_logical_sector.get(),
                alignment: d.alignment.get(),
                alignment_offset: d.alignment_offset.get(),
                size: d.size.get(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let super_device = block_devices.first().context("no block devices")?;
    ensure!(
        geometry.metadata_region() <= super_device.first_logical_sector * SECTOR_SIZE,
        "metadata overlaps the partitions"
    );

    let extents = header
        .extents
        .entries::<RawExtent>(tables)
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let sectors = e.num_sectors.get();
            Ok(match e.target_type.get() {
                TARGET_LINEAR => {
                    let device = e.target_source.get() as usize;
                    ensure!(device < block_devices.len(), "extent {i} is on block device {device}");
                    Extent::Linear { sectors, device, sector: e.target_data.get() }
                }
                TARGET_ZERO => Extent::Zero { sectors },
                t => bail!("extent {i} has target type {t}"),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let groups = header
        .groups
        .entries::<RawGroup>(tables)
        .iter()
        .map(|g| {
            Ok(Group {
                name: suffixed(name(&g.name)?, g.flags.get() & GROUP_SLOT_SUFFIXED != 0, slot)?,
                maximum_size: g.maximum_size.get(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let valid = if minor >= VERSION_FOR_UPDATED_ATTR {
        ATTR_READONLY | ATTR_SLOT_SUFFIXED | ATTR_UPDATED | ATTR_DISABLED
    } else {
        ATTR_READONLY | ATTR_SLOT_SUFFIXED
    };
    let partitions = header
        .partitions
        .entries::<RawPartition>(tables)
        .iter()
        .map(|p| {
            let name = name(&p.name)?;
            let attributes = p.attributes.get();
            ensure!(attributes & !valid == 0, "{name} has attributes {attributes:#x}");
            let (first, count) = (p.first_extent_index.get() as usize, p.num_extents.get() as usize);
            ensure!(first + count <= extents.len(), "{name} owns extents {first}..{}", first + count);
            let group = p.group_index.get() as usize;
            ensure!(group < groups.len(), "{name} is in group {group}");
            Ok(Partition {
                name: suffixed(name, attributes & ATTR_SLOT_SUFFIXED != 0, slot)?,
                attributes: attributes & !ATTR_SLOT_SUFFIXED,
                group,
                extents: extents[first..first + count].to_vec(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Metadata {
        geometry: *geometry,
        major_version: major,
        minor_version: minor,
        header_flags: header.flags.get(),
        partitions,
        groups,
        block_devices,
        damaged: Vec::new(),
    })
}
