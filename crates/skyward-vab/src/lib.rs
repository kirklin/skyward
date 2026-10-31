use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

#[cfg(target_os = "android")]
pub mod boot;
mod partitions;
pub mod snapshots;

pub use partitions::{
    check_merge, map_slot, map_whole, mount_one, mount_partitions, mount_point, mounted_partitions, release_partitions, slot_bases,
    slot_sizes, snapshots_mapped, start_merge, unmap_slot, unmap_whole, unmount_one, unmount_partitions, Mounted, Mounts, SlotMaps,
    Whole,
};
pub use snapshots::Merge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    pub fn suffix(self) -> &'static str {
        match self {
            Slot::A => "_a",
            Slot::B => "_b",
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Slot::A => "A",
            Slot::B => "B",
        }
    }

    /// The slot's metadata copy in super.
    pub fn number(self) -> u32 {
        match self {
            Slot::A => 0,
            Slot::B => 1,
        }
    }

    pub fn other(self) -> Slot {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A,
        }
    }

    pub fn from_suffix(s: &str) -> Option<Slot> {
        match s {
            "_a" => Some(Slot::A),
            "_b" => Some(Slot::B),
            _ => None,
        }
    }
}

/// What the phone's boot control holds of the slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSlots {
    /// The slot the bootloader starts next.
    pub active: Slot,
    /// Of each slot, A first: whether the bootloader may start it, and
    /// whether its system has started fully since it was written.
    pub bootable: [bool; 2],
    pub successful: [bool; 2],
}

/// The booted slot, or `None` on devices that are not A/B.
pub fn current_slot() -> Result<Option<Slot>> {
    match skyward_block::slot_suffix()? {
        None => Ok(None),
        Some(s) => Slot::from_suffix(&s).map(Some).ok_or_else(|| anyhow!("slot suffix {s:?}")),
    }
}

/// Virtual A/B update phase as libsnapshot records it. Anything but `None`
/// means dynamic partitions have live snapshots, and writing them would
/// corrupt the pending update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotState {
    /// No update in flight.
    None,
    /// An update is being written, or is written and waits to be booted.
    Snapshotted,
    /// Snapshots are being merged back into the base partitions.
    Merging,
    /// The merge failed; Android has to recover it.
    MergeFailed,
    /// /metadata is not mounted, so the state cannot be read.
    Unknown,
}

const METADATA: &str = "/metadata";
const OTA_DIR: &str = "/metadata/ota";

/// What libsnapshot keeps in its directory on /metadata: the update's state,
/// and the slot it was installed from, written once the new slot's
/// partitions are.
const STATE: &str = "state";
const SOURCE: &str = "snapshot-boot";

/// libsnapshot's `SnapshotUpdateStatus` from snapshot.proto, every field of
/// it, so that the state written back keeps all it held.
#[derive(Clone, PartialEq, Message)]
pub struct UpdateStatus {
    #[prost(enumeration = "UpdateState", tag = "1")]
    pub state: i32,
    /// What the update's kernel snapshots had allocated when the merge
    /// began, the measure of how far it has come.
    #[prost(uint64, tag = "2")]
    pub sectors_allocated: u64,
    #[prost(uint64, tag = "3")]
    pub total_sectors: u64,
    #[prost(uint64, tag = "4")]
    pub metadata_sectors: u64,
    /// Its snapshots are compressed, and served by snapuserd.
    #[prost(bool, tag = "5")]
    pub using_snapuserd: bool,
    #[prost(enumeration = "MergePhase", tag = "6")]
    pub merge_phase: i32,
    #[prost(enumeration = "FailureCode", tag = "7")]
    pub merge_failure_code: i32,
    #[prost(string, tag = "8")]
    pub source_build_fingerprint: String,
    #[prost(bool, tag = "9")]
    pub userspace_snapshots: bool,
    #[prost(bool, tag = "10")]
    pub io_uring_enabled: bool,
    #[prost(bool, tag = "11")]
    pub legacy_snapuserd: bool,
    #[prost(bool, tag = "12")]
    pub o_direct: bool,
    #[prost(uint32, tag = "13")]
    pub cow_op_merge_size: u32,
    #[prost(uint32, tag = "14")]
    pub num_worker_threads: u32,
}

/// Which of an update's snapshots are merged first: those of partitions
/// the update shrank go before the others, which may write where they
/// were.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum MergePhase {
    NoMerge = 0,
    FirstPhase = 1,
    SecondPhase = 2,
}

/// Why a merge failed, as libsnapshot records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum FailureCode {
    Ok = 0,
    ReadStatus = 1,
    GetTableInfo = 2,
    UnknownTable = 3,
    GetTableParams = 4,
    ActivateNewTable = 5,
    AcquireLock = 6,
    ListSnapshots = 7,
    WriteStatus = 8,
    UnknownTargetType = 9,
    QuerySnapshotStatus = 10,
    ExpectedMergeTarget = 11,
    UnmergedSectorsAfterCompletion = 12,
    UnexpectedMergeState = 13,
    GetCowPathConsistencyCheck = 14,
    OpenCowConsistencyCheck = 15,
    ParseCowConsistencyCheck = 16,
    OpenCowDirectConsistencyCheck = 17,
    MemAlignConsistencyCheck = 18,
    DirectReadConsistencyCheck = 19,
    WrongMergeCountConsistencyCheck = 20,
}

/// Where an update stands, as the boot control keeps it for the bootloader,
/// which will not erase or write the data partition, /metadata or super
/// while an update's snapshots wait to be merged or are being merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStatus {
    None = 0,
    Unknown = 1,
    /// The new slot's partitions are snapshots, waiting to be merged.
    Snapshotted = 2,
    Merging = 3,
    Cancelled = 4,
}

/// Where an update stands, as libsnapshot names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum UpdateState {
    None = 0,
    /// The new slot's partitions are being written.
    Initiated = 1,
    /// They are written, and wait for the system to start from them and
    /// confirm them.
    Unverified = 2,
    Merging = 3,
    /// Merged, with a restart left to finish cleaning up.
    MergeNeedsReboot = 4,
    MergeCompleted = 5,
    MergeFailed = 6,
    Cancelled = 7,
}

impl UpdateState {
    fn phase(self) -> SnapshotState {
        match self {
            UpdateState::None | UpdateState::MergeCompleted | UpdateState::Cancelled => SnapshotState::None,
            UpdateState::Initiated | UpdateState::Unverified => SnapshotState::Snapshotted,
            UpdateState::Merging | UpdateState::MergeNeedsReboot => SnapshotState::Merging,
            UpdateState::MergeFailed => SnapshotState::MergeFailed,
        }
    }
}

/// An update as libsnapshot keeps track of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Update {
    pub state: UpdateState,
    /// The slot the update was installed from; `None` until the new slot's
    /// partitions are written.
    pub source: Option<Slot>,
}

/// Why erasing the data partition has to wait for an update. Erasing it
/// erases /metadata too, where the update's snapshots are tracked: an update
/// being merged into the slot the phone runs from leaves that slot half old
/// and half new, and an update that slot runs on unconfirmed is lost with
/// its snapshots. Android's own recovery merges the update first, or marks
/// its slot unbootable; erasing here waits for the system to finish it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The slot the phone runs from got an update the system has not yet
    /// confirmed.
    Unconfirmed,
    Merging,
    MergeFailed,
}

impl Update {
    /// What keeps the data partition from being erased while the phone runs
    /// from `current`, if anything. An update still being written, one the
    /// phone left for the slot it came from, and one merged with only a
    /// restart's cleaning up left go as Android's recovery lets them, with
    /// /metadata.
    pub fn holds_wipe(&self, current: Slot) -> Option<Hold> {
        match self.state {
            UpdateState::Unverified if self.source.is_some_and(|source| source != current) => Some(Hold::Unconfirmed),
            UpdateState::Merging => Some(Hold::Merging),
            UpdateState::MergeFailed => Some(Hold::MergeFailed),
            _ => None,
        }
    }
}

pub fn snapshot_state() -> Result<SnapshotState> {
    if !skyward_block::mount::is_mount_point(Path::new(METADATA))? {
        return Ok(SnapshotState::Unknown);
    }
    Ok(read_state(Path::new(OTA_DIR))?.phase())
}

/// The update libsnapshot keeps track of, where /metadata is mounted to
/// tell; `None` where it is not.
pub fn update() -> Result<Option<Update>> {
    if !skyward_block::mount::is_mount_point(Path::new(METADATA))? {
        return Ok(None);
    }
    read_update(Path::new(OTA_DIR)).map(Some)
}

fn read_update(dir: &Path) -> Result<Update> {
    let state = read_state(dir)?;
    let path = dir.join(SOURCE);
    let source = match fs::read_to_string(&path) {
        Ok(suffix) => Some(Slot::from_suffix(suffix.trim()).with_context(|| format!("{} names the slot {suffix:?}", path.display()))?),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    Ok(Update { state, source })
}

fn read_state(dir: &Path) -> Result<UpdateState> {
    state_of(&read_status(dir)?)
}

/// The update's whole status in `dir`.
fn read_status(dir: &Path) -> Result<UpdateStatus> {
    let path = dir.join(STATE);
    match fs::read(&path) {
        Ok(bytes) => decode_status(&bytes).with_context(|| format!("read {}", path.display())),
        // libsnapshot writes the file, and /metadata/ota, with the first update.
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(UpdateStatus::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// An update's status as libsnapshot writes it, or as it wrote it once, as
/// the state's own name, which it still reads.
fn decode_status(bytes: &[u8]) -> Result<UpdateStatus> {
    if let Ok(status) = UpdateStatus::decode(bytes) {
        return Ok(status);
    }
    let state = match std::str::from_utf8(bytes).map_err(|_| anyhow!("neither SnapshotUpdateStatus nor an update state's name"))? {
        "" | "none" => UpdateState::None,
        "initiated" => UpdateState::Initiated,
        "unverified" => UpdateState::Unverified,
        "merging" => UpdateState::Merging,
        "merge-completed" => UpdateState::MergeCompleted,
        "merge-needs-reboot" => UpdateState::MergeNeedsReboot,
        "merge-failed" => UpdateState::MergeFailed,
        "cancelled" => UpdateState::Cancelled,
        other => bail!("update state {other:?}"),
    };
    Ok(UpdateStatus { state: state as i32, ..UpdateStatus::default() })
}

fn state_of(status: &UpdateStatus) -> Result<UpdateState> {
    UpdateState::try_from(status.state).map_err(|e| anyhow!("update state: {e}"))
}

/// Where an update stands for the phone running from a slot, as a
/// recovery shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    /// None waits to be merged.
    None,
    /// /metadata, where the update is tracked, cannot be read.
    Unknown,
    /// The system was writing it to the other slot, and has not finished.
    Writing,
    /// Written to the other slot; it merges once the system there has
    /// started. A system's updater may leave having that slot start next
    /// until it restarts the phone itself.
    Installed(Slot),
    /// Written to the slot the phone runs from, whose system has not
    /// started all the way: it merges once it has, or here.
    Waiting,
    /// Its snapshots are compressed, which the system merges and a recovery
    /// that does not run snapuserd does not.
    Compressed,
    /// Being merged, this share of the way, 0 to 1.
    Merging(f64),
    /// The merge stopped for this reason; it goes on when the snapshots are
    /// next mapped.
    Failed(FailureCode),
    /// Merged, with what the update left cleaned up when the system next
    /// starts.
    Merged,
}

/// Where an update stands for the phone running from `current`.
pub fn standing(current: Slot) -> Result<Standing> {
    if !skyward_block::mount::is_mount_point(Path::new(METADATA))? {
        return Ok(Standing::Unknown);
    }
    let Some(snaps) = snapshots::Snapshots::lock(false)? else { return Ok(Standing::None) };
    let update = snaps.update()?;
    Ok(match state_of(&update)? {
        UpdateState::None | UpdateState::Cancelled => Standing::None,
        UpdateState::Initiated => Standing::Writing,
        UpdateState::Unverified => match snaps.side(current)? {
            snapshots::Side::Target if update.using_snapuserd => Standing::Compressed,
            snapshots::Side::Target => Standing::Waiting,
            snapshots::Side::Source => Standing::Installed(current.other()),
            snapshots::Side::Unknown => Standing::Writing,
        },
        UpdateState::Merging if update.using_snapuserd => Standing::Compressed,
        UpdateState::Merging => Standing::Merging(snaps.progress()?),
        UpdateState::MergeFailed => Standing::Failed(FailureCode::try_from(update.merge_failure_code).unwrap_or(FailureCode::Ok)),
        UpdateState::MergeNeedsReboot | UpdateState::MergeCompleted => Standing::Merged,
    })
}

/// Mount /metadata writable, to merge an update, or read-only again.
pub fn metadata_writable(writable: bool) -> Result<()> {
    skyward_block::mount::remount_writable(Path::new(METADATA), writable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_round_trip() {
        assert_eq!(Slot::from_suffix("_a"), Some(Slot::A));
        assert_eq!(Slot::A.other(), Slot::B);
        assert_eq!(Slot::B.suffix(), "_b");
        assert_eq!(Slot::B.letter(), "B");
        assert_eq!((Slot::A.number(), Slot::B.number()), (0, 1));
        assert_eq!(Slot::from_suffix("_c"), None);
    }

    fn encoded(state: UpdateState) -> Vec<u8> {
        UpdateStatus { state: state as i32, ..UpdateStatus::default() }.encode_to_vec()
    }

    fn decode_state(bytes: &[u8]) -> Result<UpdateState> {
        state_of(&decode_status(bytes)?)
    }

    fn phase(bytes: &[u8]) -> SnapshotState {
        decode_state(bytes).unwrap().phase()
    }

    #[test]
    fn empty_state_file_means_no_update() {
        // A Xiaomi 17 Ultra with no update pending keeps a zero-length file.
        assert_eq!(phase(&[]), SnapshotState::None);
    }

    #[test]
    fn update_states_map_to_phases() {
        assert_eq!(phase(&encoded(UpdateState::Unverified)), SnapshotState::Snapshotted);
        assert_eq!(phase(&encoded(UpdateState::Initiated)), SnapshotState::Snapshotted);
        assert_eq!(phase(&encoded(UpdateState::Merging)), SnapshotState::Merging);
        assert_eq!(phase(&encoded(UpdateState::MergeNeedsReboot)), SnapshotState::Merging);
        assert_eq!(phase(&encoded(UpdateState::MergeFailed)), SnapshotState::MergeFailed);
        assert_eq!(phase(&encoded(UpdateState::MergeCompleted)), SnapshotState::None);
    }

    #[test]
    fn unknown_state_value_is_an_error() {
        let bytes = UpdateStatus { state: 42, ..UpdateStatus::default() }.encode_to_vec();
        assert!(decode_state(&bytes).is_err());
    }

    #[test]
    fn a_state_written_by_name_as_libsnapshot_once_wrote_it_is_read() {
        assert_eq!(decode_state(b"merging").unwrap(), UpdateState::Merging);
        assert_eq!(decode_state(b"merge-needs-reboot").unwrap(), UpdateState::MergeNeedsReboot);
        assert_eq!(decode_state(b"initiated").unwrap(), UpdateState::Initiated);
        assert_eq!(decode_state(b"none").unwrap(), UpdateState::None);
        assert!(decode_state(b"half-merged").is_err());
    }

    #[test]
    fn a_status_keeps_every_field_it_is_written_with() {
        let status = UpdateStatus {
            state: UpdateState::Merging as i32,
            sectors_allocated: 4_210_688,
            total_sectors: 4_225_024,
            metadata_sectors: 33_024,
            merge_phase: MergePhase::SecondPhase as i32,
            source_build_fingerprint: "Xiaomi/ishtar/ishtar:13/TKQ1.221114.001/OS3.0.305.0.WMACNXM:user/release-keys".into(),
            ..UpdateStatus::default()
        };
        assert_eq!(decode_status(&status.encode_to_vec()).unwrap(), status);
    }

    fn scratch(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("skyward-vab-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_update_is_read_with_the_slot_it_came_from_once_that_is_written() {
        let dir = scratch("read");
        assert_eq!(read_update(&dir).unwrap(), Update { state: UpdateState::None, source: None }, "no update yet");
        fs::write(dir.join(STATE), encoded(UpdateState::Initiated)).unwrap();
        assert_eq!(read_update(&dir).unwrap(), Update { state: UpdateState::Initiated, source: None });
        fs::write(dir.join(STATE), encoded(UpdateState::Unverified)).unwrap();
        fs::write(dir.join(SOURCE), "_a").unwrap();
        assert_eq!(read_update(&dir).unwrap(), Update { state: UpdateState::Unverified, source: Some(Slot::A) });
        fs::write(dir.join(SOURCE), "_c").unwrap();
        assert!(read_update(&dir).is_err(), "a slot that is neither");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn erasing_waits_only_where_androids_recovery_has_to_merge_or_give_the_update_up() {
        let update = |state, source| Update { state, source };
        let free = [
            update(UpdateState::None, None),
            update(UpdateState::Initiated, None),
            update(UpdateState::Unverified, Some(Slot::B)),
            update(UpdateState::Unverified, None),
            update(UpdateState::MergeNeedsReboot, Some(Slot::A)),
            update(UpdateState::MergeCompleted, Some(Slot::A)),
            update(UpdateState::Cancelled, Some(Slot::A)),
        ];
        for u in free {
            assert_eq!(u.holds_wipe(Slot::B), None, "{u:?} from slot B");
        }
        assert_eq!(update(UpdateState::Unverified, Some(Slot::A)).holds_wipe(Slot::B), Some(Hold::Unconfirmed));
        assert_eq!(update(UpdateState::Merging, Some(Slot::A)).holds_wipe(Slot::B), Some(Hold::Merging));
        assert_eq!(update(UpdateState::MergeFailed, Some(Slot::A)).holds_wipe(Slot::B), Some(Hold::MergeFailed));
    }
}
