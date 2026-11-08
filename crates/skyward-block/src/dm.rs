// The table encoding serves the Android ioctls and the host tests.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use anyhow::{anyhow, ensure, Result};
use std::mem::size_of;
use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

/// One target of a device-mapper table, in 512-byte sectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `sectors` sectors of block device `device` (major, minor), starting
    /// at its sector `offset`.
    Linear { sectors: u64, device: (u32, u32), offset: u64 },
    Zero { sectors: u64 },
    /// `sectors` sectors of `device`, decrypted with `key` under `cipher`
    /// as the kernel names it. `wrapped` says the key is one only the
    /// storage hardware can unwrap, which is how a phone keeps the key its
    /// data partition is encrypted with out of the running system.
    DefaultKey { sectors: u64, device: String, cipher: String, key: String, wrapped: bool },
    /// `sectors` sectors of block device `base` as changed by what block
    /// device `cow` keeps, both (major, minor), in chunks of
    /// [`SNAPSHOT_CHUNK`] sectors that last across restarts. Merging, the
    /// kernel writes the changes into `base` as it goes. `overflow` has the
    /// kernel say when `cow` runs out, where its target can.
    Snapshot { sectors: u64, base: (u32, u32), cow: (u32, u32), merge: bool, overflow: bool },
}

/// The chunk an update's snapshots keep their changes in, in sectors.
pub const SNAPSHOT_CHUNK: u32 = 8;

impl Target {
    pub fn sectors(&self) -> u64 {
        match self {
            Target::Linear { sectors, .. }
            | Target::Zero { sectors }
            | Target::DefaultKey { sectors, .. }
            | Target::Snapshot { sectors, .. } => *sectors,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Target::Linear { .. } => "linear",
            Target::Zero { .. } => "zero",
            Target::DefaultKey { .. } => "default-key",
            Target::Snapshot { merge: false, .. } => "snapshot",
            Target::Snapshot { merge: true, .. } => "snapshot-merge",
        }
    }

    fn params(&self) -> String {
        match self {
            Target::Linear { device: (major, minor), offset, .. } => format!("{major}:{minor} {offset}"),
            Target::Zero { .. } => String::new(),
            Target::DefaultKey { device, cipher, key, wrapped, .. } => {
                let mut extra = vec!["allow_discards", "sector_size:4096", "iv_large_sectors"];
                if *wrapped {
                    extra.push("wrappedkey_v0");
                }
                format!("{cipher} {key} 0 {device} 0 {} {}", extra.len(), extra.join(" "))
            }
            Target::Snapshot { base: (bmaj, bmin), cow: (cmaj, cmin), overflow, .. } => {
                let mode = if *overflow { "PO" } else { "P" };
                format!("{bmaj}:{bmin} {cmaj}:{cmin} {mode} {SNAPSHOT_CHUNK}")
            }
        }
    }
}

/// A target of a device's live table as the kernel tells it: where it
/// starts and how long it is, in sectors, its kind, and its parameters or
/// what it says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start: u64,
    pub sectors: u64,
    pub kind: String,
    pub text: String,
}

// From <linux/dm-ioctl.h>.
const DM_NAME_LEN: usize = 128;
const DM_UUID_LEN: usize = 129;
const DM_MAX_TYPE_NAME: usize = 16;
const DM_READONLY_FLAG: u32 = 1 << 0;
/// Asks DM_TABLE_STATUS for each target's parameters rather than its
/// status.
const DM_STATUS_TABLE_FLAG: u32 = 1 << 4;
/// The kernel had more to say than the buffer held.
const DM_BUFFER_FULL_FLAG: u32 = 1 << 8;
/// The oldest interface version with every command used here.
const DM_VERSION: [u32; 3] = [4, 0, 0];

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: i32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    dev: u64,
    name: [u8; DM_NAME_LEN],
    uuid: [u8; DM_UUID_LEN],
    data: [u8; 7],
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
struct DmTargetSpec {
    sector_start: u64,
    length: u64,
    status: i32,
    /// Bytes from this spec to the next one.
    next: u32,
    target_type: [u8; DM_MAX_TYPE_NAME],
}

/// An entry of DM_LIST_VERSIONS, its name following it.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
struct DmTargetVersions {
    /// Bytes from this entry to the next one; 0 for the last.
    next: u32,
    version: [u32; 3],
}

const _: () = {
    assert!(size_of::<DmIoctl>() == 312);
    assert!(size_of::<DmTargetSpec>() == 40);
    assert!(size_of::<DmTargetVersions>() == 16);
};

/// `_IOWR(DM_IOCTL, nr, struct dm_ioctl)`.
const fn request(nr: u32) -> u32 {
    (3 << 30) | ((size_of::<DmIoctl>() as u32) << 16) | (0xfd << 8) | nr
}

const DM_DEV_CREATE: u32 = request(3);
const DM_DEV_REMOVE: u32 = request(4);
const DM_DEV_SUSPEND: u32 = request(6);
const DM_DEV_STATUS: u32 = request(7);
const DM_TABLE_LOAD: u32 = request(9);
const DM_TABLE_STATUS: u32 = request(12);
const DM_LIST_VERSIONS: u32 = request(13);

impl DmIoctl {
    fn new(name: &str) -> Result<DmIoctl> {
        ensure!(!name.is_empty() && name.len() < DM_NAME_LEN, "device-mapper name {name:?}");
        let mut io = DmIoctl::new_zeroed();
        io.version = DM_VERSION;
        io.data_size = size_of::<DmIoctl>() as u32;
        io.name[..name.len()].copy_from_slice(name.as_bytes());
        Ok(io)
    }
}

/// The DM_TABLE_LOAD argument: the ioctl header, then each target's spec and
/// NUL-terminated parameters, padded to 8 bytes.
fn table_load(name: &str, targets: &[Target], readonly: bool) -> Result<Vec<u8>> {
    ensure!(!targets.is_empty(), "{name} has no targets");
    let mut body = Vec::new();
    let mut start = 0;
    for t in targets {
        let params = t.params();
        let len = (size_of::<DmTargetSpec>() + params.len() + 1).next_multiple_of(8);
        let mut spec = DmTargetSpec::new_zeroed();
        spec.sector_start = start;
        spec.length = t.sectors();
        spec.next = len as u32;
        spec.target_type[..t.kind().len()].copy_from_slice(t.kind().as_bytes());
        let at = body.len();
        body.extend_from_slice(spec.as_bytes());
        body.extend_from_slice(params.as_bytes());
        body.resize(at + len, 0);
        start += t.sectors();
    }
    let mut io = DmIoctl::new(name)?;
    io.data_start = size_of::<DmIoctl>() as u32;
    io.data_size = (size_of::<DmIoctl>() + body.len()) as u32;
    io.target_count = targets.len() as u32;
    if readonly {
        io.flags |= DM_READONLY_FLAG;
    }
    let mut buf = io.as_bytes().to_vec();
    buf.extend_from_slice(&body);
    Ok(buf)
}

/// A NUL-padded name.
fn c_name(field: &[u8]) -> String {
    String::from_utf8_lossy(field.split(|&b| b == 0).next().unwrap_or_default()).into_owned()
}

/// The targets DM_TABLE_STATUS returned in `buf` under header `io`: each
/// target's spec, then its text, up to the next spec, which starts `next`
/// bytes after the first one.
fn lines(buf: &[u8], io: &DmIoctl) -> Result<Vec<Line>> {
    let (first, end) = (io.data_start as usize, (io.data_size as usize).min(buf.len()));
    let mut cursor = first;
    let mut out = Vec::with_capacity(io.target_count as usize);
    for _ in 0..io.target_count {
        ensure!(cursor + size_of::<DmTargetSpec>() <= end, "the table ends inside a target's spec");
        let (spec, _) = DmTargetSpec::read_from_prefix(&buf[cursor..end]).expect("the spec fits, as checked above");
        let text = cursor + size_of::<DmTargetSpec>();
        let next = (first + spec.next as usize).min(end);
        let text = if next > text { c_name(&buf[text..next]) } else { String::new() };
        out.push(Line { start: spec.sector_start, sectors: spec.length, kind: c_name(&spec.target_type), text });
        cursor = next;
    }
    Ok(out)
}

/// The kinds of target DM_LIST_VERSIONS returned in `data`, each with its
/// version, in turn `next` bytes apart.
fn versions(data: &[u8]) -> Result<Vec<(String, [u32; 3])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let (entry, name) = DmTargetVersions::read_from_prefix(&data[at..]).map_err(|_| anyhow!("the list ends inside a target's version"))?;
        out.push((c_name(name), entry.version));
        if entry.next == 0 {
            break;
        }
        at += entry.next as usize;
    }
    Ok(out)
}

#[cfg(target_os = "android")]
mod sys {
    use super::*;
    use anyhow::{bail, Context};
    use std::fs::{File, OpenOptions};
    use std::os::unix::io::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    const CONTROL: &str = "/dev/device-mapper";
    /// How long ueventd gets to make the node of a new device.
    const NODE_TIMEOUT: Duration = Duration::from_secs(5);

    fn control() -> Result<File> {
        OpenOptions::new().read(true).write(true).open(CONTROL).with_context(|| format!("open {CONTROL}"))
    }

    fn call(ctl: &File, request: u32, buf: &mut [u8]) -> std::io::Result<()> {
        // bionic's ioctl request is a c_int; the bits are reinterpreted.
        if unsafe { libc::ioctl(ctl.as_raw_fd(), request as libc::c_int, buf.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Create device `name` from `targets` and wait for ueventd to make its
    /// node; returns /dev/block/dm-N. A device that fails to come up is
    /// removed again.
    pub fn create(name: &str, targets: &[Target], readonly: bool) -> Result<PathBuf> {
        let (_, minor) = activate(name, targets, readonly)?;
        let node = PathBuf::from(format!("/dev/block/dm-{minor}"));
        match wait_for(&node) {
            Ok(()) => Ok(node),
            Err(e) => match remove(name) {
                Ok(()) => Err(e),
                Err(r) => Err(e.context(format!("removing {name} afterwards failed too: {r:#}"))),
            },
        }
    }

    /// Create device `name` from `targets` and bring it up, where no
    /// ueventd makes nodes: returns its (major, minor) for the caller to
    /// make its node by. A device that fails to come up is removed again.
    pub fn activate(name: &str, targets: &[Target], readonly: bool) -> Result<(u32, u32)> {
        let ctl = control()?;
        let mut io = DmIoctl::new(name)?;
        call(&ctl, DM_DEV_CREATE, io.as_mut_bytes()).with_context(|| format!("create device-mapper device {name}"))?;
        let dev = decode(io.dev);
        let up = (|| -> Result<()> {
            let mut table = table_load(name, targets, readonly)?;
            call(&ctl, DM_TABLE_LOAD, &mut table).with_context(|| format!("load the table of {name}"))?;
            let mut resume = DmIoctl::new(name)?;
            call(&ctl, DM_DEV_SUSPEND, resume.as_mut_bytes()).with_context(|| format!("activate {name}"))
        })();
        match up {
            Ok(()) => Ok(dev),
            Err(e) => match remove(name) {
                Ok(()) => Err(e),
                Err(r) => Err(e.context(format!("removing {name} afterwards failed too: {r:#}"))),
            },
        }
    }

    pub fn remove(name: &str) -> Result<()> {
        let mut io = DmIoctl::new(name)?;
        call(&control()?, DM_DEV_REMOVE, io.as_mut_bytes()).with_context(|| format!("remove device-mapper device {name}"))
    }

    /// Give device `name`, which may be in use, `targets` as its table,
    /// read-only or not: the device is suspended while the table is
    /// swapped in, and resumes with it.
    pub fn reload(name: &str, targets: &[Target], readonly: bool) -> Result<()> {
        let ctl = control()?;
        let mut table = table_load(name, targets, readonly)?;
        call(&ctl, DM_TABLE_LOAD, &mut table).with_context(|| format!("load a new table for {name}"))?;
        let mut resume = DmIoctl::new(name)?;
        call(&ctl, DM_DEV_SUSPEND, resume.as_mut_bytes()).with_context(|| format!("swap in the new table of {name}"))
    }

    /// The (major, minor) of device `name`, or `None` when there is none.
    pub fn status(name: &str) -> Result<Option<(u32, u32)>> {
        let mut io = DmIoctl::new(name)?;
        match call(&control()?, DM_DEV_STATUS, io.as_mut_bytes()) {
            Ok(()) => Ok(Some(decode(io.dev))),
            Err(e) if e.raw_os_error() == Some(libc::ENXIO) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("look up device-mapper device {name}")),
        }
    }

    /// Whether device `name` is read-only.
    pub fn readonly(name: &str) -> Result<bool> {
        let mut io = DmIoctl::new(name)?;
        call(&control()?, DM_DEV_STATUS, io.as_mut_bytes()).with_context(|| format!("look up device-mapper device {name}"))?;
        Ok(io.flags & DM_READONLY_FLAG != 0)
    }

    /// The targets of device `name`'s live table, with their parameters.
    pub fn table(name: &str) -> Result<Vec<Line>> {
        read_table(name, DM_STATUS_TABLE_FLAG).with_context(|| format!("read the table of {name}"))
    }

    /// The targets of device `name`'s live table, with what each says of
    /// itself.
    pub fn table_status(name: &str) -> Result<Vec<Line>> {
        read_table(name, 0).with_context(|| format!("read the status of {name}"))
    }

    fn read_table(name: &str, flags: u32) -> Result<Vec<Line>> {
        let ctl = control()?;
        let mut size = 4096;
        loop {
            let mut io = DmIoctl::new(name)?;
            io.data_size = size as u32;
            io.data_start = size_of::<DmIoctl>() as u32;
            io.flags = flags;
            let mut buf = io.as_bytes().to_vec();
            buf.resize(size, 0);
            call(&ctl, DM_TABLE_STATUS, &mut buf)?;
            let (io, _) = DmIoctl::read_from_prefix(&buf).expect("the buffer starts with the header");
            if io.flags & DM_BUFFER_FULL_FLAG == 0 {
                return lines(&buf, &io);
            }
            size *= 2;
        }
    }

    /// The version of the kernel's target `kind`, or `None` where the
    /// kernel has no such target.
    pub fn target_version(kind: &str) -> Result<Option<[u32; 3]>> {
        // Room for far more kinds of target than a kernel has.
        let size = size_of::<DmIoctl>() + 128 * (size_of::<DmTargetVersions>() + DM_MAX_TYPE_NAME);
        let mut io = DmIoctl::new_zeroed();
        io.version = DM_VERSION;
        io.data_size = size as u32;
        io.data_start = size_of::<DmIoctl>() as u32;
        let mut buf = io.as_bytes().to_vec();
        buf.resize(size, 0);
        call(&control()?, DM_LIST_VERSIONS, &mut buf).context("list the kernel's device-mapper targets")?;
        let (io, _) = DmIoctl::read_from_prefix(&buf).expect("the buffer starts with the header");
        ensure!(io.flags & DM_BUFFER_FULL_FLAG == 0, "the kernel has more device-mapper targets than {size} bytes hold");
        let data = &buf[size_of::<DmIoctl>()..(io.data_size as usize).min(size)];
        Ok(versions(data)?.into_iter().find(|(k, _)| k == kind).map(|(_, v)| v))
    }

    fn decode(dev: u64) -> (u32, u32) {
        let dev = dev as libc::dev_t;
        (libc::major(dev) as u32, libc::minor(dev) as u32)
    }

    fn wait_for(node: &Path) -> Result<()> {
        let start = Instant::now();
        while !node.exists() {
            if start.elapsed() > NODE_TIMEOUT {
                bail!("ueventd made no {} within {:?}", node.display(), NODE_TIMEOUT);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
mod sys {
    use super::{Line, Target};
    use anyhow::{bail, Result};
    use std::path::PathBuf;

    pub fn table(name: &str) -> Result<Vec<Line>> {
        bail!("this host has no device-mapper to read the table of {name}")
    }

    pub fn table_status(name: &str) -> Result<Vec<Line>> {
        bail!("this host has no device-mapper to read the status of {name}")
    }

    pub fn target_version(kind: &str) -> Result<Option<[u32; 3]>> {
        bail!("this host has no device-mapper to have a {kind} target")
    }

    pub fn create(name: &str, _targets: &[Target], _readonly: bool) -> Result<PathBuf> {
        bail!("this host has no device-mapper to create {name}")
    }

    pub fn activate(name: &str, _targets: &[Target], _readonly: bool) -> Result<(u32, u32)> {
        bail!("this host has no device-mapper to create {name}")
    }

    pub fn remove(name: &str) -> Result<()> {
        bail!("this host has no device-mapper to remove {name}")
    }

    pub fn reload(name: &str, _targets: &[Target], _readonly: bool) -> Result<()> {
        bail!("this host has no device-mapper to reload {name}")
    }

    pub fn status(name: &str) -> Result<Option<(u32, u32)>> {
        bail!("this host has no device-mapper to look up {name}")
    }

    pub fn readonly(name: &str) -> Result<bool> {
        bail!("this host has no device-mapper to look up {name}")
    }
}

pub use sys::{activate, create, readonly, reload, remove, status, table, table_status, target_version};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_codes_match_the_kernel_header() {
        assert_eq!(DM_DEV_CREATE, 0xc138_fd03);
        assert_eq!(DM_TABLE_LOAD, 0xc138_fd09);
    }

    #[test]
    fn table_lists_targets_back_to_back() {
        let targets = [
            Target::Linear { sectors: 4_078_120, device: (259, 29), offset: 2048 },
            Target::Zero { sectors: 8 },
        ];
        let buf = table_load("odm_b", &targets, true).unwrap();
        let (io, rest) = DmIoctl::read_from_prefix(&buf).unwrap();
        assert_eq!(&io.name[..6], b"odm_b\0");
        assert_eq!((io.data_start, io.data_size as usize, io.target_count), (312, buf.len(), 2));
        assert_eq!(io.flags, DM_READONLY_FLAG);

        let (first, params) = DmTargetSpec::read_from_prefix(rest).unwrap();
        assert_eq!((first.sector_start, first.length), (0, 4_078_120));
        assert_eq!(&first.target_type[..7], b"linear\0");
        assert_eq!(&params[..16], b"259:29 2048\0\0\0\0\0");
        assert_eq!(first.next, 56);

        let (second, tail) = DmTargetSpec::read_from_prefix(&rest[first.next as usize..]).unwrap();
        assert_eq!((second.sector_start, second.length), (4_078_120, 8));
        assert_eq!(&second.target_type[..5], b"zero\0");
        assert_eq!((second.next, tail.len()), (48, 8));
    }

    #[test]
    fn a_snapshot_names_its_devices_and_how_it_keeps_its_changes() {
        let plain = Target::Snapshot { sectors: 1_780_392, base: (253, 12), cow: (253, 14), merge: false, overflow: true };
        assert_eq!((plain.kind(), plain.params().as_str()), ("snapshot", "253:12 253:14 PO 8"));
        let merging = Target::Snapshot { sectors: 1_780_392, base: (253, 12), cow: (253, 14), merge: true, overflow: false };
        assert_eq!((merging.kind(), merging.params().as_str()), ("snapshot-merge", "253:12 253:14 P 8"));
        assert_eq!(merging.sectors(), 1_780_392);
    }

    /// A target as DM_TABLE_STATUS returns it, after those in `body`: its
    /// spec, whose `next` counts from the first spec, then its text.
    fn returned(body: &mut Vec<u8>, kind: &str, start: u64, sectors: u64, text: &str) {
        let at = body.len();
        let len = (size_of::<DmTargetSpec>() + text.len() + 1).next_multiple_of(8);
        let mut spec = DmTargetSpec::new_zeroed();
        spec.sector_start = start;
        spec.length = sectors;
        spec.next = (at + len) as u32;
        spec.target_type[..kind.len()].copy_from_slice(kind.as_bytes());
        body.extend_from_slice(spec.as_bytes());
        body.extend_from_slice(text.as_bytes());
        body.resize(at + len, 0);
    }

    #[test]
    fn a_tables_targets_are_read_with_their_text() {
        let mut body = Vec::new();
        returned(&mut body, "snapshot-merge", 0, 1_780_392, "2048/233936 64");
        returned(&mut body, "linear", 1_780_392, 8, "253:3 2048");
        let mut io = DmIoctl::new("system_b").unwrap();
        io.data_start = 312;
        io.data_size = (312 + body.len()) as u32;
        io.target_count = 2;
        let mut buf = io.as_bytes().to_vec();
        buf.extend_from_slice(&body);
        // What the kernel left of the buffer unfilled.
        buf.resize(buf.len() + 64, 0);
        assert_eq!(lines(&buf, &io).unwrap(), [
            Line { start: 0, sectors: 1_780_392, kind: "snapshot-merge".into(), text: "2048/233936 64".into() },
            Line { start: 1_780_392, sectors: 8, kind: "linear".into(), text: "253:3 2048".into() },
        ]);
        io.target_count = 3;
        assert!(lines(&buf[..312 + body.len()], &io).is_err(), "a target past the end");
    }

    #[test]
    fn the_kernels_targets_are_listed_with_their_versions() {
        let mut data = Vec::new();
        let kinds = [("snapshot", [1, 16, 0]), ("snapshot-merge", [1, 5, 0]), ("user", [1, 0, 0])];
        for (k, (name, version)) in kinds.iter().enumerate() {
            let at = data.len();
            let len = (size_of::<DmTargetVersions>() + name.len() + 1).next_multiple_of(8);
            let next = if k + 1 == kinds.len() { 0 } else { len as u32 };
            data.extend_from_slice(DmTargetVersions { next, version: *version }.as_bytes());
            data.extend_from_slice(name.as_bytes());
            data.resize(at + len, 0);
        }
        let listed = versions(&data).unwrap();
        assert_eq!(listed.iter().map(|(n, v)| (n.as_str(), *v)).collect::<Vec<_>>(), kinds);
        assert!(versions(&[]).unwrap().is_empty(), "a kernel with none");
    }
}
