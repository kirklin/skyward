use skyward_lp::{read, read_image, write, Extent, Geometry, ATTR_READONLY, ATTR_UPDATED, HEADER_FLAG_VIRTUAL_AB_DEVICE};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The first 405504 bytes of the super partition of a Xiaomi 13 Ultra on
/// OS3.0.305.0, booted from slot b: geometry, its backup and three metadata
/// slots with their backups.
fn fixture() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ishtar-super-metadata.bin"))
}

/// The fixture with the byte at each offset inverted, in a temporary file.
fn damaged(name: &str, offsets: &[usize]) -> PathBuf {
    let mut data = std::fs::read(fixture()).unwrap();
    for &at in offsets {
        data[at] ^= 0xff;
    }
    let path = std::env::temp_dir().join(format!("skyward-lp-{name}-{}", std::process::id()));
    std::fs::write(&path, data).unwrap();
    path
}

const PRIMARY_GEOMETRY: usize = 4096;
const BACKUP_GEOMETRY: usize = 8192;
/// Primary metadata copy of slot 1: after both geometry blocks and slot 0.
const PRIMARY_SLOT_1: usize = 12288 + 65536;

#[test]
fn reads_slot_b() {
    let md = read(fixture(), 1).unwrap();
    assert_eq!(md.geometry, Geometry { metadata_max_size: 65536, metadata_slot_count: 3, logical_block_size: 4096 });
    assert_eq!((md.major_version, md.minor_version), (10, 2));
    assert_eq!(md.header_flags, HEADER_FLAG_VIRTUAL_AB_DEVICE);
    assert!(md.damaged.is_empty());

    let names: Vec<&str> = md.partitions.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["odm_b", "product_b", "system_b", "system_dlkm_b", "system_ext_b", "vendor_b", "vendor_dlkm_b", "mi_ext_b", "odm_b-cow"]
    );
    let groups: Vec<&str> = md.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(groups, ["default", "qti_dynamic_partitions_b", "cow"]);
    assert_eq!(md.groups[1].maximum_size, 9_653_190_656);

    let super_dev = &md.block_devices[0];
    assert_eq!(super_dev.partition_name, "super");
    assert_eq!((super_dev.first_logical_sector, super_dev.size), (2048, 9_663_676_416));

    let vendor_dlkm = md.partition("vendor_dlkm_b").unwrap();
    assert_eq!(vendor_dlkm.attributes, ATTR_READONLY | ATTR_UPDATED);
    assert_eq!(
        vendor_dlkm.extents,
        [
            Extent::Linear { sectors: 81_088, device: 0, sector: 18_248_976 },
            Extent::Linear { sectors: 32, device: 0, sector: 13_378_032 },
            Extent::Linear { sectors: 8, device: 0, sector: 13_320_184 },
        ]
    );
    assert_eq!(md.partition("odm_b").unwrap().sectors(), 4_078_440);
    assert_eq!(md.groups[md.partition("odm_b-cow").unwrap().group].name, "cow");
}

#[test]
fn slot_a_lists_its_own_partitions() {
    let md = read(fixture(), 0).unwrap();
    // Xiaomi keeps an odm copy-on-write partition for each slot.
    assert!(md.partitions.iter().all(|p| p.name.ends_with("_a") || p.name.ends_with("_a-cow")));
    assert_eq!(md.partition("system_a").unwrap().bytes(), 911_556_608);
    assert_eq!(md.partition("odm_a-cow").unwrap().bytes(), 234_942_464);
}

#[test]
fn refuses_a_slot_super_does_not_keep() {
    let err = read(fixture(), 3).unwrap_err();
    assert!(format!("{err:#}").contains("slot 3"));
}

#[test]
fn backup_geometry_stands_in_for_a_damaged_primary() {
    let path = damaged("geometry", &[PRIMARY_GEOMETRY + 10]);
    let md = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    let md = md.unwrap();
    assert_eq!(md.damaged.len(), 1);
    assert!(md.damaged[0].starts_with("primary geometry"));
    assert_eq!(md.partitions.len(), 9);
}

#[test]
fn both_geometry_copies_damaged_is_an_error() {
    let path = damaged("geometries", &[PRIMARY_GEOMETRY + 10, BACKUP_GEOMETRY + 10]);
    let md = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    assert!(format!("{:#}", md.unwrap_err()).contains("both geometry copies"));
}

#[test]
fn an_image_file_keeps_the_geometry_then_one_copy_of_the_metadata() {
    // Laid out as liblp writes an image file: the geometry block, then the
    // header and its tables with nothing after them.
    let data = std::fs::read(fixture()).unwrap();
    let header_size = u32::from_le_bytes(data[PRIMARY_SLOT_1 + 8..PRIMARY_SLOT_1 + 12].try_into().unwrap()) as usize;
    let tables_size = u32::from_le_bytes(data[PRIMARY_SLOT_1 + 44..PRIMARY_SLOT_1 + 48].try_into().unwrap()) as usize;
    let mut image = data[PRIMARY_GEOMETRY..PRIMARY_GEOMETRY + 4096].to_vec();
    image.extend_from_slice(&data[PRIMARY_SLOT_1..PRIMARY_SLOT_1 + header_size + tables_size]);
    let path = std::env::temp_dir().join(format!("skyward-lp-image-{}", std::process::id()));
    std::fs::write(&path, &image).unwrap();
    let read_back = read_image(&path);
    std::fs::write(&path, &image[..image.len() - 1]).unwrap();
    let cut = read_image(&path);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(read_back.unwrap(), read(fixture(), 1).unwrap());
    assert!(format!("{:#}", cut.unwrap_err()).contains("tables run past"), "an image cut short");
}

/// The fixture as the whole super it is the start of, the rest of it a
/// hole, in a temporary file.
fn whole(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("skyward-lp-{name}-{}", std::process::id()));
    std::fs::copy(fixture(), &path).unwrap();
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(SUPER_SIZE).unwrap();
    path
}

const SUPER_SIZE: u64 = 9_663_676_416;

/// The sectors of super the metadata gives to no partition, in the one
/// hole its partitions leave.
const FREE: std::ops::Range<u64> = 13_320_192..13_325_368;

#[test]
fn metadata_written_back_is_what_liblp_wrote() {
    let path = whole("written-back");
    for slot in 0..3 {
        write(&path, &read(&path, slot).unwrap(), &[slot]).unwrap();
    }
    // Only the start is read back: the rest of super is a hole of nine
    // gigabytes.
    let was = std::fs::read(fixture()).unwrap();
    let mut head = vec![0u8; was.len()];
    let read_back = std::fs::File::open(&path).and_then(|mut file| file.read_exact(&mut head));
    std::fs::remove_file(&path).unwrap();
    read_back.unwrap();
    assert!(head == was, "every copy reads as it did, byte for byte");
}

#[test]
fn a_partition_grows_into_the_free_room_behind_it() {
    let mut md = read(fixture(), 1).unwrap();
    let before = md.partition("vendor_dlkm_b").unwrap().bytes();
    md.resize("vendor_dlkm_b", before + 1_048_576 - 100).unwrap();
    let grown = md.partition("vendor_dlkm_b").unwrap();
    assert_eq!(grown.bytes(), before + 1_048_576, "whole logical blocks");
    // Its last extent ended where the free room begins, and goes on into it.
    assert_eq!(
        grown.extents,
        [
            Extent::Linear { sectors: 81_088, device: 0, sector: 18_248_976 },
            Extent::Linear { sectors: 32, device: 0, sector: 13_378_032 },
            Extent::Linear { sectors: 8 + 2048, device: 0, sector: 13_320_184 },
        ]
    );
    // Another partition takes what is left from where the first stopped.
    md.resize("system_dlkm_b", 680 * 512 + 8192).unwrap();
    assert_eq!(
        md.partition("system_dlkm_b").unwrap().extents,
        [Extent::Linear { sectors: 680, device: 0, sector: 15_157_560 }, Extent::Linear { sectors: 16, device: 0, sector: FREE.start + 2048 }]
    );
}

#[test]
fn a_partition_takes_no_more_than_super_or_its_group_holds() {
    let mut md = read(fixture(), 1).unwrap();
    let before = md.clone();
    let free = (FREE.end - FREE.start) * 512;
    let held = md.partition("vendor_b").unwrap().bytes();
    let err = md.resize("vendor_b", held + free + 4096).unwrap_err();
    assert!(format!("{err:#}").contains("super has 4096 bytes too few free"), "{err:#}");
    assert_eq!(md, before, "nothing changes where there is too little room");
    md.resize("vendor_b", held + free).unwrap();
    assert_eq!(md.partition("vendor_b").unwrap().bytes(), held + free, "all the room there is");

    let mut md = before.clone();
    let group = md.partition("vendor_b").unwrap().group;
    md.groups[group].maximum_size = md.partitions.iter().filter(|p| p.group == group).map(|p| p.bytes()).sum();
    let err = md.resize("vendor_b", held + 4096).unwrap_err();
    assert!(format!("{err:#}").contains("qti_dynamic_partitions_b"), "{err:#}");
    assert!(md.resize("no_such_b", 4096).is_err());
}

#[test]
fn a_partition_that_shrinks_gives_up_its_last_sectors() {
    let mut md = read(fixture(), 1).unwrap();
    md.resize("system_b", 104_857_600).unwrap();
    assert_eq!(md.partition("system_b").unwrap().extents, [Extent::Linear { sectors: 204_800, device: 0, sector: 13_392_312 }]);
    // What it gave up is free for another to take, in the order it lies
    // in super: the eight sectors of its last extent, then the hole.
    let held = md.partition("mi_ext_b").unwrap().bytes();
    md.resize("mi_ext_b", held + (FREE.end - FREE.start) * 512 + 4096).unwrap();
    let extents = &md.partition("mi_ext_b").unwrap().extents;
    assert_eq!(
        extents[extents.len() - 2..],
        [Extent::Linear { sectors: 8, device: 0, sector: 13_320_168 }, Extent::Linear { sectors: FREE.end - FREE.start, device: 0, sector: FREE.start }]
    );
    md.resize("system_b", 0).unwrap();
    assert!(md.partition("system_b").unwrap().extents.is_empty());
}

#[test]
fn resized_metadata_reads_back_from_every_slot_it_is_written_to() {
    let path = whole("resized");
    let mut md = read(&path, 1).unwrap();
    md.resize("odm_b", 1_073_741_824).unwrap();
    md.resize("vendor_b", 1_610_612_736).unwrap();
    write(&path, &md, &[0, 1]).unwrap();
    let (a, b, untouched) = (read(&path, 0), read(&path, 1), read(&path, 2));
    // With the primary copy of slot 1 damaged, its backup reads the same.
    let data = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    std::os::unix::fs::FileExt::write_all_at(&data, &[0xff; 16], PRIMARY_SLOT_1 as u64 + 300).unwrap();
    let from_backup = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(a.unwrap(), md);
    assert_eq!(b.unwrap(), md);
    assert_eq!(untouched.unwrap(), read(fixture(), 2).unwrap());
    let from_backup = from_backup.unwrap();
    assert_eq!(from_backup.damaged.len(), 1);
    assert_eq!(from_backup.partitions, md.partitions);
}

#[test]
fn a_copy_that_no_longer_reads_is_made_good_before_the_new_one_is_written() {
    let path = whole("made-good");
    let old = read(&path, 1).unwrap();
    // The backup copy of slot 1: after the three primary copies and the
    // backup of slot 0.
    let backup = 12288 + 65536 * 4;
    let damage = |at: usize| {
        let data = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        std::os::unix::fs::FileExt::write_all_at(&data, &[0xff; 16], at as u64 + 300).unwrap();
    };
    damage(backup);
    // Metadata too large to write stops before either copy is touched.
    let mut large = old.clone();
    large.partitions = (0..2000).map(|k| skyward_lp::Partition { name: format!("p{k}"), attributes: 0, group: 0, extents: Vec::new() }).collect();
    let refused = write(&path, &large, &[1]).unwrap_err();
    assert!(format!("{refused:#}").contains("65536 at most"), "{refused:#}");
    assert_eq!(read(&path, 1).unwrap(), old);
    let mut md = old.clone();
    md.resize("system_dlkm_b", 0).unwrap();
    write(&path, &md, &[1]).unwrap();
    damage(PRIMARY_SLOT_1);
    let from_backup = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(from_backup.unwrap().partitions, md.partitions, "the backup copy was written as well");
}

#[test]
fn metadata_for_another_super_is_not_written() {
    let path = whole("another");
    let mut md = read(&path, 1).unwrap();
    md.block_devices[0].size += 4096;
    let err = write(&path, &md, &[1]).unwrap_err();
    let unchanged = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    assert!(format!("{err:#}").contains("holds 9663676416 bytes"), "{err:#}");
    assert_eq!(unchanged.unwrap(), read(fixture(), 1).unwrap());
}

#[test]
fn backup_metadata_stands_in_for_damaged_tables() {
    // A byte in the partition table, past the 256-byte header.
    let path = damaged("tables", &[PRIMARY_SLOT_1 + 300]);
    let md = read(&path, 1);
    std::fs::remove_file(&path).unwrap();
    let md = md.unwrap();
    assert_eq!(md.damaged.len(), 1);
    assert!(md.damaged[0].contains("bad tables checksum"));
    assert_eq!(md.partition("vendor_dlkm_b").unwrap().extents.len(), 3);
}
