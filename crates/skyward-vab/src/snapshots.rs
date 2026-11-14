//! An update's snapshots, as libsnapshot keeps them on /metadata: the slot's
//! partitions mapped through them, and the snapshots merged into the
//! partitions they snapshot, as Android's own recovery does it with
//! libsnapshot. The kernel keeps the snapshots and merges them; snapshots
//! served by snapuserd, as compressed updates make them, are not read here.

use crate::partitions::targets;
use crate::{read_status, state_of, FailureCode, MergePhase, MergeStatus, Slot, UpdateState, UpdateStatus, OTA_DIR, SOURCE, STATE};
use anyhow::{anyhow, bail, ensure, Context, Result};
use prost::Message;
use skyward_block::{devnum, dm, BY_NAME};
use skyward_lp::{Metadata, Partition, ATTR_UPDATED, SECTOR_SIZE};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Each snapshot's status, by the name of the partition it snapshots.
const SNAPSHOTS: &str = "snapshots";
/// There for an update applied without switching slots, which the slot it
/// was applied from then starts through.
const WITHOUT_SLOT_SWITCH: &str = "snapshot-boot-without-slot-switch";
/// libfiemap's record of where the copy-on-write images lie on the data
/// partition.
const IMAGES: &str = "/metadata/gsi/ota/lp_metadata";
/// The group super keeps an update's copy-on-write partitions in.
const COW_GROUP: &str = "cow";

/// Where one partition's snapshot stands, as libsnapshot names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum PartState {
    None = 0,
    /// Made, and written by the update.
    Created = 1,
    Merging = 2,
    MergeCompleted = 3,
}

/// libsnapshot's `SnapshotStatus` from snapshot.proto, every field of it, so
/// that the status written back keeps all it held.
#[derive(Clone, PartialEq, Message)]
pub struct SnapshotStatus {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(enumeration = "PartState", tag = "2")]
    pub state: i32,
    /// The partition's size, which its base device has.
    #[prost(uint64, tag = "3")]
    pub device_size: u64,
    #[prost(uint64, tag = "4")]
    pub snapshot_size: u64,
    /// Copy-on-write space in super, and in an image on the data partition.
    #[prost(uint64, tag = "5")]
    pub cow_partition_size: u64,
    #[prost(uint64, tag = "6")]
    pub cow_file_size: u64,
    #[prost(uint64, tag = "7")]
    pub sectors_allocated: u64,
    #[prost(uint64, tag = "8")]
    pub metadata_sectors: u64,
    #[prost(bool, tag = "9")]
    pub using_snapuserd: bool,
    #[prost(uint64, tag = "10")]
    pub old_partition_size: u64,
    #[prost(string, tag = "11")]
    pub compression_algorithm: String,
    #[prost(uint64, tag = "12")]
    pub estimated_cow_size: u64,
    #[prost(bool, tag = "13")]
    pub enable_threading: bool,
    #[prost(bool, tag = "14")]
    pub batched_writes: bool,
    #[prost(uint64, tag = "15")]
    pub estimated_ops_buffer_size: u64,
    #[prost(uint64, tag = "16")]
    pub compression_factor: u64,
    #[prost(uint32, tag = "17")]
    pub read_ahead_size: u32,
    #[prost(uint64, tag = "19")]
    pub verify_block_size: u64,
    #[prost(uint32, tag = "20")]
    pub num_verify_threads: u32,
}

impl SnapshotStatus {
    pub fn part_state(&self) -> Result<PartState> {
        PartState::try_from(self.state).map_err(|e| anyhow!("{} snapshot state: {e}", self.name))
    }

    /// The phase this snapshot merges in: a partition the update shrank
    /// before the others, which may write where it was. Only snapuserd's
    /// snapshots are merged so.
    pub fn phase(&self) -> MergePhase {
        if self.using_snapuserd && self.device_size < self.old_partition_size {
            MergePhase::FirstPhase
        } else {
            MergePhase::SecondPhase
        }
    }
}

/// Which slot the phone runs from, as the update sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// No update has been written all the way yet.
    Unknown,
    /// The slot the update was applied from.
    Source,
    /// The slot it was applied to.
    Target,
}

/// How a partition stands in the slot's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placed {
    /// Written by the update.
    Updated,
    /// Written over since, by flashing.
    Flashed,
    /// Not in the slot.
    None,
}

fn placed(md: &Metadata, name: &str) -> Placed {
    match md.partition(name) {
        None => Placed::None,
        Some(p) if p.attributes & ATTR_UPDATED != 0 => Placed::Updated,
        Some(_) => Placed::Flashed,
    }
}

/// What libsnapshot keeps of an update, read and written under its lock on
/// its directory, which is held while this lives.
pub struct Snapshots {
    dir: PathBuf,
    exclusive: bool,
    _lock: File,
}

impl Snapshots {
    /// Take libsnapshot's lock, shared to read what it keeps or exclusive
    /// to change it; `None` where no update was ever applied.
    pub fn lock(exclusive: bool) -> Result<Option<Snapshots>> {
        match Snapshots::lock_in(Path::new(OTA_DIR), exclusive) {
            Err(e) if e.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == ErrorKind::NotFound) => Ok(None),
            other => other.map(Some),
        }
    }

    fn lock_in(dir: &Path, exclusive: bool) -> Result<Snapshots> {
        let lock = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(dir)?;
        let how = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH };
        loop {
            if unsafe { libc::flock(lock.as_raw_fd(), how) } == 0 {
                break;
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(e).with_context(|| format!("lock {}", dir.display()));
            }
        }
        Ok(Snapshots { dir: dir.into(), exclusive, _lock: lock })
    }

    /// The update's whole status.
    pub fn update(&self) -> Result<UpdateStatus> {
        read_status(&self.dir)
    }

    pub fn state(&self) -> Result<UpdateState> {
        state_of(&self.update()?)
    }

    /// Write the update's status. The bootloader is told where the update
    /// stands as libsnapshot tells it: before a state that keeps it from
    /// erasing is written, and after one that lets it.
    fn write_update(&self, status: &UpdateStatus, tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<()> {
        ensure!(self.exclusive, "the update's state is written only under an exclusive lock");
        let merge = merge_status(state_of(status)?);
        let first = told_first(merge);
        if first {
            tell(merge)?;
        }
        write_atomic(&self.dir.join(STATE), &status.encode_to_vec())?;
        if !first {
            tell(merge)?;
        }
        Ok(())
    }

    /// Move the update to `state`, keeping what describes the update itself
    /// between the states of one update, as libsnapshot does.
    fn set_state(&self, state: UpdateState, failure: FailureCode, tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<()> {
        let mut status = UpdateStatus { state: state as i32, ..UpdateStatus::default() };
        if state == UpdateState::MergeFailed {
            status.merge_failure_code = failure as i32;
        }
        if !matches!(state, UpdateState::Initiated | UpdateState::None) {
            let old = self.update()?;
            status.using_snapuserd = old.using_snapuserd;
            status.source_build_fingerprint = old.source_build_fingerprint;
            status.merge_phase = old.merge_phase;
            status.userspace_snapshots = old.userspace_snapshots;
            status.io_uring_enabled = old.io_uring_enabled;
            status.legacy_snapuserd = old.legacy_snapuserd;
            status.o_direct = old.o_direct;
            status.cow_op_merge_size = old.cow_op_merge_size;
            status.num_worker_threads = old.num_worker_threads;
        }
        self.write_update(&status, tell)
    }

    /// The partitions the update has a snapshot of, those of system and
    /// product first, which libsnapshot merges first.
    pub fn list(&self) -> Result<Vec<String>> {
        let dir = self.dir.join(SNAPSHOTS);
        let mut names = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|n| anyhow!("snapshot {n:?} is not named in text"))?;
            if matches!(name.as_str(), "system_a" | "system_b" | "product_a" | "product_b") {
                names.insert(0, name);
            } else {
                names.push(name);
            }
        }
        Ok(names)
    }

    pub fn status(&self, name: &str) -> Result<SnapshotStatus> {
        let path = self.dir.join(SNAPSHOTS).join(name);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let mut status = SnapshotStatus::decode(&bytes[..]).with_context(|| format!("read {}", path.display()))?;
        status.name = name.into();
        Ok(status)
    }

    fn write_status(&self, status: &SnapshotStatus) -> Result<()> {
        ensure!(self.exclusive, "a snapshot's status is written only under an exclusive lock");
        write_atomic(&self.dir.join(SNAPSHOTS).join(&status.name), &status.encode_to_vec())
    }

    /// The slot the update was applied from, once its partitions are
    /// written.
    pub fn source(&self) -> Result<Option<Slot>> {
        let path = self.dir.join(SOURCE);
        match fs::read_to_string(&path) {
            Ok(suffix) => Ok(Some(Slot::from_suffix(suffix.trim()).with_context(|| format!("{} names the slot {suffix:?}", path.display()))?)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn side(&self, current: Slot) -> Result<Side> {
        Ok(match self.source()? {
            None => Side::Unknown,
            Some(source) if source == current => Side::Source,
            Some(_) => Side::Target,
        })
    }

    fn without_slot_switch(&self) -> bool {
        self.dir.join(WITHOUT_SLOT_SWITCH).exists()
    }

    /// Whether the slot the phone runs from reads through the update's
    /// snapshots, as first-stage init decides it: when the update went to
    /// that slot and waits there to be merged, or is being merged.
    pub fn needed(&self, current: Slot) -> Result<bool> {
        if self.without_slot_switch() {
            return Ok(self.side(current)? == Side::Source);
        }
        if self.side(current)? != Side::Target {
            return Ok(false);
        }
        Ok(matches!(self.state()?, UpdateState::Unverified | UpdateState::Merging | UpdateState::MergeFailed))
    }

    /// The snapshot partition `p` reads through, where it has one: made of
    /// it by the update, not flashed over since, and not merged yet.
    pub fn live(&self, p: &Partition) -> Result<Option<SnapshotStatus>> {
        if !self.without_slot_switch() && p.attributes & ATTR_UPDATED == 0 {
            return Ok(None);
        }
        if !self.dir.join(SNAPSHOTS).join(&p.name).exists() {
            return Ok(None);
        }
        let status = self.status(&p.name)?;
        let state = status.part_state()?;
        let cow = status.cow_partition_size + status.cow_file_size;
        if matches!(state, PartState::None | PartState::MergeCompleted) || cow == 0 {
            return Ok(None);
        }
        Ok(Some(status))
    }

    /// The share of the merge done, 0 to 1, from the snapshots the kernel
    /// is merging: their changes left to write back, of those there were as
    /// the merge began.
    pub fn progress(&self) -> Result<f64> {
        let update = self.update()?;
        match state_of(&update)? {
            UpdateState::MergeCompleted => return Ok(1.0),
            UpdateState::Merging => {}
            _ => return Ok(0.0),
        }
        let mut sum = Usage::default();
        for name in self.list()? {
            if let Some((_, Ok(u))) = snapshot_target(&name)? {
                sum.allocated += u.allocated;
                sum.total += u.total;
                sum.metadata += u.metadata;
            }
        }
        Ok(merge_percent(sum, update.sectors_allocated) / 100.0)
    }
}

/// Where the bootloader is told an update in `state` stands: merging while
/// it merges or failed to, snapshotted while it waits, and otherwise
/// nowhere it need keep from erasing.
fn merge_status(state: UpdateState) -> MergeStatus {
    match state {
        UpdateState::None | UpdateState::MergeNeedsReboot | UpdateState::MergeCompleted | UpdateState::Initiated => MergeStatus::None,
        UpdateState::Unverified => MergeStatus::Snapshotted,
        UpdateState::Merging | UpdateState::MergeFailed => MergeStatus::Merging,
        UpdateState::Cancelled => MergeStatus::Unknown,
    }
}

/// Whether the bootloader hears of `status` before the state is written:
/// one that keeps it from erasing is in force before the state that needs
/// it.
fn told_first(status: MergeStatus) -> bool {
    matches!(status, MergeStatus::Snapshotted | MergeStatus::Merging)
}

/// Write `data` to `path` whole or not at all: to a file beside it first,
/// synced, then put in its place, and the folder synced.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
    file.write_all(data).with_context(|| format!("write {}", tmp.display()))?;
    file.sync_all().with_context(|| format!("sync {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("put {} in place of {}", tmp.display(), path.display()))?;
    let dir = path.parent().context("a file kept for an update is in a folder")?;
    File::open(dir).and_then(|d| d.sync_all()).with_context(|| format!("sync {}", dir.display()))
}

/// What a kernel snapshot says of itself: the sectors its copy-on-write
/// space has given out, of how many, and how many of those keep its own
/// bookkeeping. Merged all the way, it has given out only those.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub allocated: u64,
    pub total: u64,
    pub metadata: u64,
}

/// A kernel snapshot's status line, or the trouble it reports in its place:
/// `Invalid`, `Overflow` or `Merge failed`.
fn parse_usage(text: &str) -> Result<Usage, String> {
    let parsed = (|| {
        let (counts, metadata) = text.split_once(' ')?;
        let (allocated, total) = counts.split_once('/')?;
        Some(Usage { allocated: allocated.parse().ok()?, total: total.parse().ok()?, metadata: metadata.trim().parse().ok()? })
    })();
    match parsed {
        Some(u) => Ok(u),
        None if matches!(text, "Invalid" | "Overflow" | "Merge failed") => Err(text.to_string()),
        None => Err(format!("unreadable status {text:?}")),
    }
}

/// How far a merge has come, in percent, from what its snapshots say of
/// themselves together and what they had allocated as it began, as
/// libdm reckons it.
fn merge_percent(u: Usage, initial: u64) -> f64 {
    let (s, t, m) = (u.allocated, u.total, u.metadata);
    let i = if initial == 0 { t } else { initial };
    if t <= s || i <= s {
        return 0.0;
    }
    if s == 0 || t == 0 || s <= m {
        return 100.0;
    }
    100.0 / (i - m) as f64 * (i - s) as f64
}

/// Device `name`'s kernel snapshot, where it is one: whether it is merging,
/// and what it says of itself.
fn snapshot_target(name: &str) -> Result<Option<(bool, Result<Usage, String>)>> {
    if dm::status(name)?.is_none() {
        return Ok(None);
    }
    let lines = dm::table_status(name)?;
    let [line] = lines.as_slice() else { return Ok(None) };
    let merging = match line.kind.as_str() {
        "snapshot" => false,
        "snapshot-merge" => true,
        _ => return Ok(None),
    };
    Ok(Some((merging, parse_usage(&line.text))))
}

/// A device as a device-mapper table names it, `major:minor`.
fn parse_device(text: &str) -> Result<(u32, u32)> {
    let (major, minor) = text.split_once(':').with_context(|| format!("device {text:?}"))?;
    Ok((major.parse().with_context(|| format!("device {text:?}"))?, minor.parse().with_context(|| format!("device {text:?}"))?))
}

/// Whether the kernel's target `kind` says when its copy-on-write space
/// runs out: snapshot from 1.15, snapshot-merge from 1.4.
fn reports_overflow(kind: &str) -> Result<bool> {
    let least = if kind == "snapshot-merge" { [1, 4, 0] } else { [1, 15, 0] };
    Ok(dm::target_version(kind)?.is_some_and(|v| v >= least))
}

/// Remove device-mapper device `name`, where there is one.
fn remove_if_mapped(name: &str) -> Result<()> {
    if dm::status(name)?.is_some() {
        dm::remove(name)?;
    }
    Ok(())
}

/// Map the copy-on-write image `name` where libfiemap records it lies on
/// the data partition.
fn map_image(name: &str) -> Result<PathBuf> {
    let md = skyward_lp::read_image(Path::new(IMAGES))?;
    let devices = md.block_devices.iter().map(|d| devnum(Path::new(BY_NAME).join(&d.partition_name))).collect::<Result<Vec<_>>>()?;
    let image = md.partition(name).with_context(|| format!("{IMAGES} records no {name}"))?;
    dm::create(name, &targets(image, &devices), false)
}

/// Whether device-mapper device `name` is a kernel snapshot.
pub fn is_snapshot(name: &str) -> Result<bool> {
    Ok(snapshot_target(name)?.is_some())
}

/// Undo [`map_snapshot`] where it was done: take away partition `name`'s
/// snapshot, its copy-on-write devices and its base, those that are there.
pub fn unmap_snapshot(name: &str) -> Result<()> {
    for dev in [name.to_string(), format!("{name}-cow"), format!("{name}-cow-img"), format!("{name}-base"), format!("{name}-src")] {
        remove_if_mapped(&dev)?;
    }
    Ok(())
}

/// Take away the copy-on-write devices of partition `name`'s snapshot, as
/// libsnapshot deletes a snapshot in recovery: its status and its image
/// stay for the system to delete.
fn delete_snapshot(name: &str) -> Result<()> {
    remove_if_mapped(&format!("{name}-cow"))?;
    remove_if_mapped(&format!("{name}-cow-img"))
}

/// Map partition `p` of the slot through its snapshot `status`, as the
/// kernel keeps it: the partition's own extents, writable, as its base;
/// its copy-on-write space in super and on the data partition, joined; and
/// the snapshot over both, merging where the update's merge has come to it.
/// `md` is the slot's metadata, and `devices` super's block devices in the
/// order it lists them. Returns the snapshot's device; what was mapped on
/// the way is taken away again if it does not come up.
pub fn map_snapshot(snaps: &Snapshots, md: &Metadata, devices: &[(u32, u32)], p: &Partition, status: &SnapshotStatus) -> Result<PathBuf> {
    let update = snaps.update()?;
    ensure!(
        !status.using_snapuserd && !update.using_snapuserd,
        "{}'s snapshot is compressed and served by snapuserd, which this recovery does not run",
        p.name
    );
    let sector = SECTOR_SIZE;
    ensure!(status.device_size.is_multiple_of(sector), "{} is {} bytes, not whole sectors", p.name, status.device_size);
    ensure!(
        status.snapshot_size.is_multiple_of(sector) && status.snapshot_size == status.device_size,
        "{}'s snapshot is {} bytes of the partition's {}",
        p.name,
        status.snapshot_size,
        status.device_size
    );
    ensure!(p.bytes() == status.device_size, "{} is {} bytes in super, and {} to its snapshot", p.name, p.bytes(), status.device_size);
    let state = status.part_state()?;
    ensure!(!matches!(state, PartState::None | PartState::MergeCompleted), "{}'s snapshot is {state:?}", p.name);
    let merge = match state_of(&update)? {
        UpdateState::MergeCompleted | UpdateState::MergeNeedsReboot => bail!("{}'s snapshot is merged already", p.name),
        UpdateState::Merging | UpdateState::MergeFailed => status.phase() as i32 == update.merge_phase || state == PartState::Merging,
        _ => state == PartState::Merging,
    };

    let mut made: Vec<String> = Vec::new();
    let mapped = (|| -> Result<PathBuf> {
        let base = format!("{}-base", p.name);
        let base_dev = dm::create(&base, &targets(p, devices), false)?;
        made.push(base);
        let image = format!("{}-cow-img", p.name);
        let image_dev = if status.cow_file_size > 0 {
            let dev = map_image(&image)?;
            made.push(image.clone());
            Some(dev)
        } else {
            None
        };
        let cow_dev = match (status.cow_partition_size, image_dev) {
            (0, Some(dev)) => dev,
            (_, image_dev) => {
                let name = format!("{}-cow", p.name);
                let part = md.partition(&name).with_context(|| format!("super holds no {name}"))?;
                ensure!(md.groups[part.group].name == COW_GROUP, "{name} is not in super's {COW_GROUP} group");
                let mut table = targets(part, devices);
                if let Some(dev) = image_dev {
                    table.push(dm::Target::Linear { sectors: status.cow_file_size / sector, device: devnum(&dev)?, offset: 0 });
                }
                let dev = dm::create(&name, &table, false)?;
                made.push(name);
                dev
            }
        };
        let kind = if merge { "snapshot-merge" } else { "snapshot" };
        let snapshot = dm::Target::Snapshot {
            sectors: status.snapshot_size / sector,
            base: devnum(&base_dev)?,
            cow: devnum(&cow_dev)?,
            merge,
            overflow: reports_overflow(kind)?,
        };
        dm::create(&p.name, &[snapshot], false)
    })();
    if mapped.is_err() {
        for name in made.iter().rev() {
            remove_if_mapped(name)?;
        }
    }
    mapped
}

/// How a look at a merge left it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Merge {
    /// Still merging, this share of the way, 0 to 1.
    Going(f64),
    /// Merged: each partition is mapped straight again, and the update is
    /// marked merged for the system to clean up when it next starts.
    Done,
    /// Merged, with cleaning up left that waits for a restart.
    NeedsReboot,
    /// The merge stopped, and goes on when the snapshots are next mapped.
    Failed(FailureCode),
    /// Partitions were flashed over since the update, which is given up.
    Cancelled,
    /// No merge is under way: the update stands here.
    Idle(UpdateState),
}

/// Start merging the update into the slot the phone runs from, as
/// libsnapshot does once the system the update put there has started: each
/// snapshot becomes a merging one, and the kernel writes it into the
/// partition it snapshots. From here there is no going back to the slot the
/// update came from. Snapshots of partitions flashed over since are left
/// out, and their copy-on-write devices taken away.
pub fn initiate(snaps: &Snapshots, current: Slot, md: &Metadata, tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<()> {
    let mut update = snaps.update()?;
    ensure!(state_of(&update)? == UpdateState::Unverified, "the update has not been written all the way, or is merging already");
    ensure!(snaps.side(current)? == Side::Target, "the phone runs from the slot the update came from, not the one it went to");
    ensure!(!update.using_snapuserd, "the update's snapshots are compressed and served by snapuserd, which this recovery does not run");
    let mut names = snaps.list()?;
    for name in names.iter().filter(|n| n.ends_with(current.suffix())) {
        ensure!(dm::status(name)?.is_some(), "{name} is not mapped, and everything has to merge at once");
    }
    let mut kept = Vec::new();
    for name in names.drain(..) {
        match placed(md, &name) {
            Placed::Updated => kept.push(name),
            Placed::Flashed | Placed::None => delete_snapshot(&name)?,
        }
    }
    let mut initial = Usage::default();
    let mut first = Vec::new();
    for name in &kept {
        let (_, usage) = snapshot_target(name)?.with_context(|| format!("{name} is not a snapshot"))?;
        let usage = usage.map_err(|why| anyhow!("{name}: {why}"))?;
        initial.allocated += usage.allocated;
        initial.total += usage.total;
        initial.metadata += usage.metadata;
        let status = snaps.status(name)?;
        ensure!(!status.using_snapuserd, "{name}'s snapshot is compressed and served by snapuserd, which this recovery does not run");
        if status.phase() == MergePhase::FirstPhase {
            first.push(name.clone());
        }
    }
    update.state = UpdateState::Merging as i32;
    update.sectors_allocated = initial.allocated;
    update.total_sectors = initial.total;
    update.metadata_sectors = initial.metadata;
    let group = if first.is_empty() {
        update.merge_phase = MergePhase::SecondPhase as i32;
        kept
    } else {
        update.merge_phase = MergePhase::FirstPhase as i32;
        first
    };
    // From here every snapshot taken in has to merge.
    snaps.write_update(&update, tell)?;
    let mut failure = FailureCode::Ok;
    for name in &group {
        if let Err(code) = switch_to_merge(snaps, name)
            && failure == FailureCode::Ok
        {
            failure = code;
        }
    }
    if failure != FailureCode::Ok {
        snaps.set_state(UpdateState::MergeFailed, failure, tell)?;
    }
    Ok(())
}

/// Have the kernel merge snapshot `name`: its table becomes a merging one
/// over the same base and copy-on-write devices, and its status says so.
fn switch_to_merge(snaps: &Snapshots, name: &str) -> Result<(), FailureCode> {
    let mut status = snaps.status(name).map_err(|_| FailureCode::ReadStatus)?;
    let lines = dm::table(name).map_err(|_| FailureCode::GetTableInfo)?;
    let [line] = lines.as_slice() else { return Err(FailureCode::UnknownTable) };
    if line.kind != "snapshot" {
        return Err(FailureCode::UnknownTable);
    }
    let mut params = line.text.split(' ');
    let (Some(base), Some(cow)) = (params.next(), params.next()) else { return Err(FailureCode::GetTableParams) };
    let (base, cow) = (parse_device(base).map_err(|_| FailureCode::GetTableParams)?, parse_device(cow).map_err(|_| FailureCode::GetTableParams)?);
    let overflow = reports_overflow("snapshot-merge").map_err(|_| FailureCode::ActivateNewTable)?;
    let merging = dm::Target::Snapshot { sectors: line.sectors, base, cow, merge: true, overflow };
    dm::reload(name, &[merging], false).map_err(|_| FailureCode::ActivateNewTable)?;
    status.state = PartState::Merging as i32;
    if let Ok(Some((_, Ok(usage)))) = snapshot_target(name) {
        status.sectors_allocated = usage.allocated;
        status.metadata_sectors = usage.metadata;
    }
    // The table switched is what counts; the status is only kept in step.
    let _ = snaps.write_status(&status);
    Ok(())
}

/// Look at the merge, as libsnapshot looks each time it polls one. A
/// snapshot merged all the way is marked so, its partition mapped straight
/// over super again, and its copy-on-write devices taken away. With every
/// snapshot merged, the update is marked merged, as Android's recovery
/// marks it, for the system to clean up when it next starts; a merge that
/// fails is marked failed, and goes on when the snapshots are next mapped.
/// `devices` are super's block devices in the order `md` lists them.
pub fn check(snaps: &Snapshots, current: Slot, md: &Metadata, devices: &[(u32, u32)], tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<Merge> {
    let update = snaps.update()?;
    let state = state_of(&update)?;
    match state {
        UpdateState::Merging | UpdateState::MergeNeedsReboot | UpdateState::MergeFailed => {}
        UpdateState::MergeCompleted => {
            snaps.set_state(UpdateState::MergeCompleted, FailureCode::Ok, tell)?;
            return Ok(Merge::Done);
        }
        other => return Ok(Merge::Idle(other)),
    }
    ensure!(!update.using_snapuserd, "the update's snapshots are compressed and served by snapuserd, which this recovery does not run");
    let (mut merging, mut needs_reboot, mut wrong_phase, mut cancelled) = (false, false, false, false);
    let mut failure = FailureCode::Ok;
    for name in snaps.list()?.iter().filter(|n| n.ends_with(current.suffix())) {
        match check_target(snaps, md, devices, name, &update)? {
            Target::Merging => merging = true,
            Target::Failed(code) => {
                if failure == FailureCode::Ok {
                    failure = code;
                }
            }
            Target::NeedsReboot => needs_reboot = true,
            Target::Merged => {}
            Target::Cancelled => cancelled = true,
            Target::WrongPhase => wrong_phase = true,
        }
    }
    if merging {
        return Ok(Merge::Going(snaps.progress()?));
    }
    if failure != FailureCode::Ok {
        if matches!(state, UpdateState::Merging | UpdateState::MergeNeedsReboot) {
            snaps.set_state(UpdateState::MergeFailed, failure, tell)?;
        }
        return Ok(Merge::Failed(failure));
    }
    if wrong_phase {
        return match merge_second_phase(snaps, current) {
            FailureCode::Ok => Ok(Merge::Going(snaps.progress()?)),
            code => {
                snaps.set_state(UpdateState::MergeFailed, code, tell)?;
                Ok(Merge::Failed(code))
            }
        };
    }
    if needs_reboot {
        snaps.set_state(UpdateState::MergeNeedsReboot, FailureCode::Ok, tell)?;
        return Ok(Merge::NeedsReboot);
    }
    if cancelled {
        return Ok(Merge::Cancelled);
    }
    snaps.set_state(UpdateState::MergeCompleted, FailureCode::Ok, tell)?;
    Ok(Merge::Done)
}

/// Where one snapshot's merge stands.
enum Target {
    Merging,
    Merged,
    NeedsReboot,
    Failed(FailureCode),
    /// Its partition was flashed over since the update.
    Cancelled,
    /// It merges in the phase after the one under way.
    WrongPhase,
}

/// Look at snapshot `name`'s merge, as libsnapshot looks at each one.
fn check_target(snaps: &Snapshots, md: &Metadata, devices: &[(u32, u32)], name: &str, update: &UpdateStatus) -> Result<Target> {
    let Ok(mut status) = snaps.status(name) else { return Ok(Target::Failed(FailureCode::ReadStatus)) };
    let state = status.part_state()?;
    let Some((merging, usage)) = snapshot_target(name)? else {
        if placed(md, name) != Placed::Updated {
            delete_snapshot(name)?;
            return Ok(Target::Cancelled);
        }
        // Found merged before its device could be mapped straight again:
        // cleaning up is tried once more.
        if state == PartState::MergeCompleted {
            let _ = merged(md, devices, name, &status);
            return Ok(Target::Merged);
        }
        return Ok(Target::Failed(FailureCode::UnknownTargetType));
    };
    let Ok(usage) = usage else { return Ok(Target::Failed(FailureCode::QuerySnapshotStatus)) };
    if !merging && status.phase() == MergePhase::SecondPhase && update.merge_phase == MergePhase::FirstPhase as i32 {
        return Ok(Target::WrongPhase);
    }
    if !merging {
        return Ok(Target::Failed(FailureCode::ExpectedMergeTarget));
    }
    if usage.allocated != usage.metadata {
        if state == PartState::MergeCompleted {
            return Ok(Target::Failed(FailureCode::UnmergedSectorsAfterCompletion));
        }
        return Ok(Target::Merging);
    }
    // Marked merged before cleaning up, so that whatever of that fails,
    // the next start does not map a snapshot of it again.
    status.state = PartState::MergeCompleted as i32;
    if snaps.write_status(&status).is_err() {
        return Ok(Target::Failed(FailureCode::WriteStatus));
    }
    Ok(match merged(md, devices, name, &status) {
        Ok(()) => Target::Merged,
        Err(_) => Target::NeedsReboot,
    })
}

/// Clean up after snapshot `name` merged all the way: its device is mapped
/// straight over super again, and its base and copy-on-write devices taken
/// away.
fn merged(md: &Metadata, devices: &[(u32, u32)], name: &str, status: &SnapshotStatus) -> Result<()> {
    if let Some((merging, usage)) = snapshot_target(name)? {
        ensure!(merging, "{name} is a snapshot that is not merging");
        let usage = usage.map_err(|why| anyhow!("{name}: {why}"))?;
        ensure!(usage.allocated == usage.metadata, "{name} has more to merge");
        collapse(md, devices, name, status)?;
    }
    delete_snapshot(name)
}

/// Map the merged snapshot `name` straight over its partition's extents in
/// super, in place, and take its base device away.
fn collapse(md: &Metadata, devices: &[(u32, u32)], name: &str, status: &SnapshotStatus) -> Result<()> {
    ensure!(status.snapshot_size.is_multiple_of(SECTOR_SIZE), "{name}'s snapshot is {} bytes, not whole sectors", status.snapshot_size);
    let p = md.partition(name).with_context(|| format!("super holds no {name}"))?;
    dm::reload(name, &targets(p, devices), true)?;
    remove_if_mapped(&format!("{name}-base"))?;
    remove_if_mapped(&format!("{name}-src"))
}

/// Go on to merge the snapshots of the second phase, those of the first all
/// merged.
fn merge_second_phase(snaps: &Snapshots, current: Slot) -> FailureCode {
    let Ok(mut update) = snaps.update() else { return FailureCode::ReadStatus };
    update.state = UpdateState::Merging as i32;
    update.merge_phase = MergePhase::SecondPhase as i32;
    // The phase is the update's own; the bootloader hears nothing new.
    if write_atomic(&snaps.dir.join(STATE), &update.encode_to_vec()).is_err() {
        return FailureCode::WriteStatus;
    }
    let Ok(names) = snaps.list() else { return FailureCode::ListSnapshots };
    let mut result = FailureCode::Ok;
    for name in names.iter().filter(|n| n.ends_with(current.suffix())) {
        let Ok(status) = snaps.status(name) else { return FailureCode::ReadStatus };
        if status.phase() != MergePhase::SecondPhase {
            continue;
        }
        if let Err(code) = switch_to_merge(snaps, name)
            && result == FailureCode::Ok
        {
            result = code;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshots_status_line_is_read_as_the_kernel_writes_it() {
        assert_eq!(parse_usage("2048/233936 64"), Ok(Usage { allocated: 2048, total: 233_936, metadata: 64 }));
        assert_eq!(parse_usage("Invalid"), Err("Invalid".into()));
        assert_eq!(parse_usage("Overflow"), Err("Overflow".into()));
        assert_eq!(parse_usage("Merge failed"), Err("Merge failed".into()));
        assert!(parse_usage("2048 64").is_err());
    }

    #[test]
    fn a_merge_is_measured_from_what_was_allocated_as_it_began() {
        let started = 10_000;
        let at = |allocated| merge_percent(Usage { allocated, total: 200_000, metadata: 2_000 }, started);
        assert_eq!(at(started), 0.0, "nothing written back yet");
        assert!((at(6_000) - 50.0).abs() < 1e-9, "half of what there was to write back");
        assert_eq!(at(2_000), 100.0, "only its own bookkeeping left");
        assert_eq!(merge_percent(Usage { allocated: 0, total: 0, metadata: 0 }, 0), 0.0, "no snapshots");
        assert_eq!(merge_percent(Usage { allocated: 4_000, total: 200_000, metadata: 2_000 }, 0), 100.0 / 198_000.0 * 196_000.0, "with no start kept, from the whole space");
    }

    #[test]
    fn the_bootloader_hears_of_a_state_that_keeps_it_from_erasing_before_it_is_written() {
        assert_eq!(merge_status(UpdateState::Merging), MergeStatus::Merging);
        assert_eq!(merge_status(UpdateState::MergeFailed), MergeStatus::Merging);
        assert_eq!(merge_status(UpdateState::Unverified), MergeStatus::Snapshotted);
        assert_eq!(merge_status(UpdateState::MergeCompleted), MergeStatus::None);
        assert_eq!(merge_status(UpdateState::MergeNeedsReboot), MergeStatus::None);
        assert!(told_first(MergeStatus::Merging) && told_first(MergeStatus::Snapshotted));
        assert!(!told_first(MergeStatus::None), "the bootloader is let erase once the state says so");
    }

    #[test]
    fn a_partition_the_update_shrank_merges_first_where_snapuserd_merges_it() {
        let status = |using_snapuserd, device_size, old_partition_size| SnapshotStatus {
            using_snapuserd,
            device_size,
            old_partition_size,
            ..SnapshotStatus::default()
        };
        assert_eq!(status(true, 100, 200).phase(), MergePhase::FirstPhase);
        assert_eq!(status(true, 200, 100).phase(), MergePhase::SecondPhase);
        assert_eq!(status(false, 100, 200).phase(), MergePhase::SecondPhase, "the kernel merges all at once");
    }

    #[test]
    fn a_device_is_read_as_a_table_names_it() {
        assert_eq!(parse_device("253:12").unwrap(), (253, 12));
        assert!(parse_device("dm-12").is_err());
    }

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skyward-snapshots-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(SNAPSHOTS)).unwrap();
        dir
    }

    fn partition(name: &str, attributes: u32) -> Partition {
        Partition { name: name.into(), attributes, group: 0, extents: vec![skyward_lp::Extent::Zero { sectors: 8 }] }
    }

    #[test]
    fn a_partition_reads_through_a_snapshot_the_update_made_and_has_not_merged() {
        let dir = scratch("live");
        let snaps = Snapshots::lock_in(&dir, true).unwrap();
        let written = SnapshotStatus {
            name: "vendor_a".into(),
            state: PartState::Created as i32,
            device_size: 4096,
            snapshot_size: 4096,
            cow_partition_size: 65_536,
            ..SnapshotStatus::default()
        };
        snaps.write_status(&written).unwrap();
        let updated = partition("vendor_a", skyward_lp::ATTR_READONLY | ATTR_UPDATED);
        assert_eq!(snaps.live(&updated).unwrap(), Some(written.clone()));
        assert_eq!(snaps.live(&partition("vendor_a", skyward_lp::ATTR_READONLY)).unwrap(), None, "flashed over since");
        assert_eq!(snaps.live(&partition("odm_a", ATTR_UPDATED)).unwrap(), None, "the update made no snapshot of it");
        snaps.write_status(&SnapshotStatus { state: PartState::MergeCompleted as i32, ..written.clone() }).unwrap();
        assert_eq!(snaps.live(&updated).unwrap(), None, "merged");
        snaps.write_status(&SnapshotStatus { cow_partition_size: 0, ..written }).unwrap();
        assert_eq!(snaps.live(&updated).unwrap(), None, "a snapshot with nowhere to keep its changes is not one");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_slot_reads_through_snapshots_where_the_update_went_to_it_and_waits_or_merges() {
        let dir = scratch("needed");
        let snaps = Snapshots::lock_in(&dir, true).unwrap();
        assert_eq!(snaps.side(Slot::A).unwrap(), Side::Unknown);
        assert!(!snaps.needed(Slot::A).unwrap(), "no update written");
        fs::write(dir.join(SOURCE), "_b").unwrap();
        let state = |s: UpdateState| fs::write(dir.join(STATE), UpdateStatus { state: s as i32, ..UpdateStatus::default() }.encode_to_vec()).unwrap();
        for (s, needed) in [
            (UpdateState::Unverified, true),
            (UpdateState::Merging, true),
            (UpdateState::MergeFailed, true),
            (UpdateState::MergeNeedsReboot, false),
            (UpdateState::MergeCompleted, false),
            (UpdateState::Initiated, false),
        ] {
            state(s);
            assert_eq!(snaps.needed(Slot::A).unwrap(), needed, "{s:?} on the slot the update went to");
            assert!(!snaps.needed(Slot::B).unwrap(), "{s:?} on the slot it came from");
        }
        assert_eq!((snaps.side(Slot::A).unwrap(), snaps.side(Slot::B).unwrap()), (Side::Target, Side::Source));
        fs::write(dir.join(WITHOUT_SLOT_SWITCH), "").unwrap();
        assert!(snaps.needed(Slot::B).unwrap(), "applied without switching slots, the slot it came from starts through it");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_state_moved_on_keeps_what_describes_the_update_and_tells_the_bootloader() {
        let dir = scratch("state");
        let snaps = Snapshots::lock_in(&dir, true).unwrap();
        let begun = UpdateStatus {
            state: UpdateState::Merging as i32,
            sectors_allocated: 4_210_688,
            merge_phase: MergePhase::SecondPhase as i32,
            source_build_fingerprint: "Xiaomi/ishtar/ishtar:13/TKQ1.221114.001/OS3.0.305.0.WMACNXM:user/release-keys".into(),
            ..UpdateStatus::default()
        };
        let told = std::cell::RefCell::new(Vec::new());
        let tell = |s: MergeStatus| {
            told.borrow_mut().push((s, fs::read(dir.join(STATE)).unwrap_or_default()));
            Ok(())
        };
        snaps.write_update(&begun, &tell).unwrap();
        assert_eq!(told.borrow()[0], (MergeStatus::Merging, Vec::new()), "told before the state was written");
        snaps.set_state(UpdateState::MergeCompleted, FailureCode::Ok, &tell).unwrap();
        let done = snaps.update().unwrap();
        assert_eq!(state_of(&done).unwrap(), UpdateState::MergeCompleted);
        assert_eq!((done.merge_phase, done.source_build_fingerprint.as_str()), (begun.merge_phase, begun.source_build_fingerprint.as_str()));
        assert_eq!(done.sectors_allocated, 0, "how far the merge came is not kept past it");
        assert_eq!(told.borrow()[1], (MergeStatus::None, done.encode_to_vec()), "told after the state was written");
        assert!(!dir.join(format!("{STATE}.tmp")).exists(), "nothing left beside it");
        drop(snaps);
        let shared = Snapshots::lock_in(&dir, false).expect("the lock is let go with the handle");
        assert!(shared.write_update(&begun, &tell).is_err(), "a shared lock writes nothing");
        fs::remove_dir_all(&dir).unwrap();
    }
}
