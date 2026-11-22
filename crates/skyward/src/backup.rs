//! Backing up and restoring on a thread of its own, telling the screen how
//! far it has come.

use anyhow::{ensure, Context, Result};
use skyward_backup::archive::{self, Others, Skipped};
use skyward_backup::store::{self, Reader, Stored, Writer};
use skyward_backup::{Item, Manifest, Part, FOLDER, FORMAT, FORMAT_WITHOUT_STORAGE};
use skyward_block::{mount, BY_NAME};
use skyward_crypto::data::SHARED_STORAGE;
use skyward_files::{Cancelled, Ownership};
use skyward_vab::{Slot, SnapshotState};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most often how far the work has come is told.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// How much is read or written at a time between two checks of whether
/// the work is to stop.
const CHUNK: usize = 4 << 20;

/// The data partition, and the reference of the key its own folders are
/// encrypted with: one made afresh has another.
const DATA: &str = "/data";
const DATA_KEY: &str = "/data/unencrypted/ref";

/// What the data partition's archive takes for each entry beyond what a
/// file holds: its headers, and the end of its last block.
const ENTRY_BYTES: u64 = 2048;

/// The slot's partitions that start the phone and check what it starts, in
/// the order a backup takes them: those the phone has.
const BOOT: [&str; 8] = ["boot", "init_boot", "vendor_boot", "dtbo", "vbmeta", "vbmeta_system", "vbmeta_vendor", "recovery"];

/// The order a restore writes the parts in: what starts the phone and what
/// it starts, then the baseband's, then the files of the data partition
/// and of internal storage, which take longest. The data partition's go
/// first, the users' own folders in internal storage being among what
/// they are put back into.
const RESTORE_ORDER: [Part; 5] = [Part::Boot, Part::System, Part::Efs, Part::Data, Part::Media];

/// Internal storage, from the data partition.
const STORAGE: &str = "media";

/// What of internal storage is the apps', from the data partition, and
/// goes with the data partition's files: each user's Android folder, where
/// an app keeps files under its own name and as its own user, and the
/// folder the users share the apps' expansion files in.
const APPS_FOLDERS: [&str; 2] = ["media/*/Android", "media/obb"];

/// The same from internal storage, which a backup of the users' own files
/// there leaves out.
const APPS_FOLDERS_IN_STORAGE: [&str; 2] = ["*/Android", "obb"];

/// Each user's Android folder in internal storage, and the folders in it
/// the apps keep theirs in.
const APPS_FOLDER: &str = "Android";
const APPS_KINDS: [&str; 3] = ["data", "media", "obb"];

/// Where each user the phone has keeps a folder named by the user's
/// number, from the data partition: what the user's apps read before the
/// screen lock is entered.
const USERS: &str = "user_de";

/// What a backup of the data partition leaves out on every phone, and a
/// restore leaves as it is: the keys the partition opens with, and what
/// the lock screen and the key service bind to the secure world, which an
/// older copy would not open; what the system makes again, its caches,
/// compiled code and the updates it downloaded; and the second system,
/// which lies where /metadata keeps it. Internal storage is left out
/// besides, but for the apps' folders in it.
const DATA_SKIPPED: [&str; 20] = [
    "lost+found",
    "per_boot",
    "unencrypted",
    "misc/vold",
    "misc/keystore",
    "misc/gatekeeper",
    "system/locksettings.db",
    "system/locksettings.db-wal",
    "system/locksettings.db-shm",
    "system/locksettings.db-journal",
    "system_de/*/spblob",
    "vendor_de/*/fpdata",
    "dalvik-cache",
    "resource-cache",
    "system/package_cache",
    "misc/apexdata/com.android.art/dalvik-cache",
    "cache",
    "ota",
    "ota_package",
    "skyward/systems",
];

/// What of the data partition a backup in the layout `format` leaves out
/// on this phone: what every phone's does, what the phone's own secure
/// world keeps in step, which `phone` lists, and internal storage; all of
/// it in the layout before, all but the apps' folders since.
fn data_skipped(phone: &[String], format: u32) -> Skipped {
    let paths: Vec<String> = DATA_SKIPPED.iter().map(|s| s.to_string()).chain([STORAGE.to_string()]).chain(phone.iter().cloned()).collect();
    let skipped = Skipped::new(&paths);
    match format {
        FORMAT_WITHOUT_STORAGE => skipped,
        _ => skipped.but(&APPS_FOLDERS.map(String::from)),
    }
}

/// What of internal storage a backup of the users' own files leaves out:
/// the apps' folders, which go with the data partition's files, and the
/// backups kept there, this one among them.
fn storage_skipped() -> Skipped {
    let paths: Vec<String> = APPS_FOLDERS_IN_STORAGE.iter().map(|s| s.to_string()).chain([format!("*/{FOLDER}")]).collect();
    Skipped::new(&paths)
}

pub struct BackupTask {
    /// Which of [`Part::ALL`] to take.
    pub parts: [bool; 5],
    /// The folder of backups at the place chosen; this one goes into a
    /// folder of its own there, called `stamp`.
    pub into: PathBuf,
    pub stamp: String,
    pub ownership: Ownership,
    pub compress: bool,
    pub verify: bool,
    /// The mount the place is, when it is shown read-only and so is made
    /// writable for the backup, and read-only again after: a USB drive.
    pub remount: Option<PathBuf>,
    /// When the backup is made, in Unix seconds, the phone by its
    /// codename, and the system it runs, by its version and by the
    /// Android it is.
    pub made: i64,
    pub device: String,
    pub system: Option<String>,
    pub android: Option<String>,
    /// The phone's partitions of the baseband's settings and the sensors'
    /// calibration, and what of the data partition its own secure world
    /// keeps in step, which a backup leaves out.
    pub efs: Vec<String>,
    pub skipped: Vec<String>,
}

pub struct RestoreTask {
    pub folder: PathBuf,
    /// Which of [`Part::ALL`] to restore.
    pub parts: [bool; 5],
    pub device: String,
    /// The Android the phone's system is, as its release reads.
    pub android: Option<String>,
    /// What of the data partition the phone's own secure world keeps in
    /// step, which a restore leaves as it is.
    pub skipped: Vec<String>,
    /// What has to happen before the slot's partitions in super are written
    /// over, as before an update.
    pub detach: skyward_install::Detach,
}

pub enum Task {
    Backup(BackupTask),
    Restore(RestoreTask),
}

/// Why backing up or restoring does not start.
#[derive(Debug)]
pub enum Refusal {
    /// Too little room where the backup goes: the bytes it takes, and the
    /// bytes free.
    Room { needed: u64, free: u64 },
    /// An update is in flight, and the slot's partitions in super are its
    /// to change.
    Update(SnapshotState),
    /// The backup was made on this other phone.
    OtherPhone(String),
    /// This partition in super is not the size it was backed up at.
    Resized(String),
    /// The data partition's files are those of a newer Android than the
    /// phone's system is, which does not read them: the release the backup
    /// was made on, and the phone's.
    OlderAndroid { was: String, now: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Room { needed, free } => write!(f, "the backup takes {needed} bytes, and {free} are free"),
            Refusal::Update(state) => write!(f, "an update is in flight ({state:?})"),
            Refusal::OtherPhone(device) => write!(f, "the backup was made on {device}"),
            Refusal::Resized(name) => write!(f, "{name} is not the size it was backed up at"),
            Refusal::OlderAndroid { was, now } => write!(f, "the data partition was backed up on Android {was}, and the phone runs Android {now}"),
        }
    }
}

impl std::error::Error for Refusal {}

pub enum Event {
    /// The parts the work goes through, each by name with its bytes; told
    /// once they are known, as measuring the data partition's files takes
    /// a while.
    Plan(Vec<(String, u64)>),
    /// The part under way, the bytes of it done of how many in this pass
    /// over it, and whether this is the pass that checks a backup against
    /// what it recorded.
    At { item: usize, done: u64, of: u64, checking: bool },
    Ended(End),
}

pub enum End {
    /// A backup made, in this folder.
    Saved(PathBuf),
    /// A backup restored, but for the folders a key of their own began in
    /// that are gone, which only the system makes, with what the backup
    /// held in them: those of users the phone has not got.
    Restored { unmade: Vec<PathBuf> },
    /// Stopped as asked, with nothing of the backup left, or nothing of
    /// the restore written.
    Stopped,
    Refused(Refusal),
    Failed(String),
}

pub struct Worker {
    cancel: Arc<AtomicBool>,
    /// The screen lets it go once the work has ended.
    worker: skyward_sys::worker::Worker<Event>,
}

impl Worker {
    pub fn start(task: Task) -> Result<Worker> {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = skyward_sys::worker::Worker::start("backup", move |teller| {
            let tell = |event: Event| {
                teller.tell(event);
            };
            let ended = match task {
                Task::Backup(t) => backup(t, &tell, &stop).map(End::Saved),
                Task::Restore(t) => restore(t, &tell, &stop).map(|unmade| End::Restored { unmade }),
            };
            let end = match ended {
                Ok(end) => end,
                Err(e) if e.is::<Cancelled>() => End::Stopped,
                Err(e) => match e.downcast::<Refusal>() {
                    Ok(refusal) => End::Refused(refusal),
                    Err(e) => End::Failed(format!("{e:#}")),
                },
            };
            let said = match &end {
                End::Saved(folder) => format!("backup saved in {}", folder.display()),
                End::Restored { unmade } => format!("backup restored, all but {unmade:?}"),
                End::Stopped => "backup stopped".into(),
                End::Refused(why) => format!("backup refused: {why}"),
                End::Failed(why) => format!("backup failed: {why}"),
            };
            skyward_sys::kmsg(&said).expect("the kernel log takes a line");
            tell(Event::Ended(end));
        })?;
        Ok(Worker { cancel, worker })
    }

    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.worker.fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        self.worker.events()
    }

    /// Ask the work to stop; it does between two chunks, and a restore
    /// only while nothing is written yet.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Where a part comes from for a backup.
enum Source {
    /// A partition of its own, at this device.
    Block(PathBuf),
    /// One of the slot's partitions in super, by its base name.
    Logical(String),
    /// The data partition's files.
    Data,
    /// The users' own files in internal storage.
    Media,
}

/// A partition by its name among the links the phone gives them.
fn partition(name: &str) -> PathBuf {
    Path::new(BY_NAME).join(name)
}

/// Tells how far a part has come in a pass over it, at most every
/// [`PROGRESS_EVERY`].
struct Telling<'a> {
    tell: &'a dyn Fn(Event),
    item: usize,
    of: u64,
    checking: bool,
    told: Instant,
}

impl Telling<'_> {
    fn new(tell: &dyn Fn(Event), item: usize, of: u64, checking: bool) -> Telling<'_> {
        tell(Event::At { item, done: 0, of, checking });
        Telling { tell, item, of, checking, told: Instant::now() }
    }

    fn done(&mut self, done: u64) {
        if self.told.elapsed() >= PROGRESS_EVERY {
            self.told = Instant::now();
            (self.tell)(Event::At { item: self.item, done, of: self.of, checking: self.checking });
        }
    }
}

/// Copy `len` bytes from `from` to `to`, telling how far, and stopping when
/// asked.
fn copy(from: &mut impl Read, to: &mut impl Write, len: u64, cancel: &AtomicBool, telling: &mut Telling) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < len {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let want = (len - done).min(CHUNK as u64) as usize;
        from.read_exact(&mut buf[..want]).context("read")?;
        to.write_all(&buf[..want]).context("write")?;
        done += want as u64;
        telling.done(done);
    }
    Ok(())
}

/// The reference of the key the data partition's own folders are encrypted
/// with, in hex.
fn data_key() -> Result<String> {
    let bytes = fs::read(DATA_KEY).with_context(|| format!("read {DATA_KEY}"))?;
    Ok(hex::encode(bytes))
}

/// The bytes free where `path` is, or would be made.
fn free_bytes(path: &Path) -> Result<u64> {
    let there = path.ancestors().find(|p| p.exists()).context("the root is always there")?;
    Ok(mount::space(there).with_context(|| format!("look at the room on {}", there.display()))?.available)
}

/// The slot's partitions in super are read or written whole only while no
/// update is changing them.
fn no_update() -> Result<()> {
    let state = skyward_vab::snapshot_state()?;
    if state != SnapshotState::None {
        return Err(Refusal::Update(state).into());
    }
    Ok(())
}

fn backup(t: BackupTask, tell: &dyn Fn(Event), cancel: &AtomicBool) -> Result<PathBuf> {
    let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let [boot, system, data, media, efs] = t.parts;
    if system {
        no_update()?;
    }
    let skipped = data_skipped(&t.skipped, FORMAT);
    let storage_skipped = storage_skipped();
    let mut plan: Vec<(Part, String, Source, u64)> = Vec::new();
    // What the parts' files take uncompressed: the bytes each holds, and
    // the data partition's archive headers.
    let mut needed = 0;
    if boot {
        // The boot partitions are backed up as the installed system's,
        // whose boot set goes back into them first.
        skyward_systems::install::ensure_installed()?;
        for name in BOOT {
            let device = partition(&format!("{name}{}", slot.suffix()));
            if device.exists() {
                let size = skyward_block::size_bytes(&device)?;
                plan.push((Part::Boot, name.to_string(), Source::Block(device), size));
            }
        }
    }
    if system {
        // A partition of the slot that holds nothing has nothing to keep.
        for (base, size) in skyward_vab::slot_sizes(slot)?.into_iter().filter(|(_, size)| *size > 0) {
            plan.push((Part::System, base.clone(), Source::Logical(base), size));
        }
    }
    if data {
        let (bytes, entries) = archive::measure(Path::new(DATA), &skipped, cancel).context("measure the data partition's files")?;
        plan.push((Part::Data, "data".into(), Source::Data, bytes));
        needed += entries * ENTRY_BYTES;
    }
    if media {
        let (bytes, entries) = archive::measure(Path::new(SHARED_STORAGE), &storage_skipped, cancel).context("measure internal storage's files")?;
        plan.push((Part::Media, "media".into(), Source::Media, bytes));
        needed += entries * ENTRY_BYTES;
    }
    if efs {
        for name in &t.efs {
            let device = partition(name);
            ensure!(device.exists(), "the phone has no partition {name}");
            let size = skyward_block::size_bytes(&device)?;
            plan.push((Part::Efs, name.clone(), Source::Block(device), size));
        }
    }
    // Compressed, what the files take is not known until they are written.
    needed += plan.iter().map(|p| p.3).sum::<u64>();
    let free = free_bytes(&t.into)?;
    if !t.compress && needed > free {
        return Err(Refusal::Room { needed, free }.into());
    }
    tell(Event::Plan(plan.iter().map(|(_, name, _, size)| (name.clone(), *size)).collect()));
    if let Some(at) = &t.remount {
        mount::remount_writable(at, true)?;
    }
    let folder = t.into.join(&t.stamp);
    let made = (|| -> Result<()> {
        skyward_files::make_folders(&folder, t.ownership)?;
        let mut items = Vec::new();
        for (k, (part, name, source, size)) in plan.iter().enumerate() {
            let file = match (part, t.compress) {
                (Part::Data | Part::Media, false) => format!("{name}.tar"),
                (Part::Data | Part::Media, true) => format!("{name}.tar.lz4"),
                (_, false) => format!("{name}.img"),
                (_, true) => format!("{name}.img.lz4"),
            };
            let mut writer = Writer::create(&folder, &file, t.compress, t.ownership);
            let mut telling = Telling::new(tell, k, *size, false);
            match source {
                Source::Block(device) => {
                    let mut from = File::open(device).with_context(|| format!("open {}", device.display()))?;
                    copy(&mut from, &mut writer, *size, cancel, &mut telling).with_context(|| format!("back up {name}"))?;
                }
                Source::Logical(base) => {
                    let whole = skyward_vab::map_whole(slot, base, &format!("skyward-backup-{base}{}", slot.suffix()), true)?;
                    let copied = File::open(&whole.device)
                        .with_context(|| format!("open {}", whole.device.display()))
                        .and_then(|mut from| copy(&mut from, &mut writer, whole.size, cancel, &mut telling));
                    skyward_vab::unmap_whole(&whole)?;
                    copied.with_context(|| format!("back up {name}"))?;
                }
                Source::Data => {
                    archive::pack(Path::new(DATA), &skipped, &mut writer, cancel, &mut |done| telling.done(done))
                        .context("back up the data partition's files")?;
                }
                Source::Media => {
                    archive::pack(Path::new(SHARED_STORAGE), &storage_skipped, &mut writer, cancel, &mut |done| telling.done(done))
                        .context("back up internal storage's files")?;
                }
            }
            items.push(Item { part: *part, name: name.clone(), stored: writer.finish()? });
        }
        if t.verify {
            for (k, item) in items.iter().enumerate() {
                let mut telling = Telling::new(tell, k, item.stored.size, true);
                check(&folder, &item.stored, cancel, &mut telling).with_context(|| format!("check {}", item.name))?;
            }
        }
        let data_key = if data { Some(data_key()?) } else { None };
        let manifest = Manifest {
            format: FORMAT,
            device: t.device.clone(),
            made: t.made,
            slot: slot.suffix().into(),
            system: t.system.clone(),
            android: t.android.clone(),
            data_key,
            items,
        };
        manifest.write(&folder, t.ownership)?;
        unsafe { libc::sync() };
        Ok(())
    })();
    // A backup that did not finish is not left half made.
    let made = match made {
        Err(e) if folder.exists() => match fs::remove_dir_all(&folder) {
            Ok(()) => Err(e),
            Err(r) => Err(e.context(format!("taking away {} afterwards failed too: {r}", folder.display()))),
        },
        other => other,
    };
    if let Some(at) = &t.remount {
        let read_only = mount::remount_writable(at, false);
        made?;
        read_only?;
    } else {
        made?;
    }
    Ok(folder)
}

/// Read a part back from its files whole and see that it is what was
/// stored.
fn check(folder: &Path, stored: &Stored, cancel: &AtomicBool, telling: &mut Telling) -> Result<()> {
    let mut reader = Reader::open(folder, stored);
    copy(&mut reader, &mut io::sink(), stored.size, cancel, telling)?;
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).context("read")?;
    ensure!(rest.is_empty(), "the backup holds more than was stored");
    reader.check(stored)
}

/// The index of a part in [`Part::ALL`].
fn index(part: Part) -> usize {
    Part::ALL.iter().position(|&p| p == part).expect("every part is listed")
}

/// A reader that counts the bytes it gives, where the count can be read
/// while the reader is lent out.
struct Counted<'a, R> {
    inner: R,
    read: &'a Cell<u64>,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.set(self.read.get() + n as u64);
        Ok(n)
    }
}

fn restore(t: RestoreTask, tell: &dyn Fn(Event), cancel: &AtomicBool) -> Result<Vec<PathBuf>> {
    let manifest = Manifest::read(&t.folder)?;
    if manifest.device != t.device {
        return Err(Refusal::OtherPhone(manifest.device).into());
    }
    let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let mut items: Vec<&Item> = manifest.items.iter().filter(|i| t.parts[index(i.part)]).collect();
    ensure!(!items.is_empty(), "nothing chosen to restore is in the backup");
    items.sort_by_key(|i| RESTORE_ORDER.iter().position(|&p| p == i.part));
    // All that can be seen before anything is written.
    let system = items.iter().any(|i| i.part == Part::System);
    if system || items.iter().any(|i| i.part == Part::Boot) {
        no_update()?;
    }
    let sizes = if system { skyward_vab::slot_sizes(slot)? } else { Vec::new() };
    for item in &items {
        match item.part {
            Part::Boot | Part::Efs => {
                let device = target(item, slot);
                ensure!(device.exists(), "the phone has no partition {}", device.display());
                let size = skyward_block::size_bytes(&device)?;
                ensure!(size == item.stored.size, "{} holds {size} bytes, and its backup {}", device.display(), item.stored.size);
            }
            Part::System => {
                let size = sizes.iter().find(|(base, _)| *base == item.name).map(|(_, size)| *size);
                if size != Some(item.stored.size) {
                    return Err(Refusal::Resized(item.name.clone()).into());
                }
            }
            Part::Data => {
                if let (Some(was), Some(now)) = (&manifest.android, &t.android)
                    && let (Some(backed_up), Some(running)) = (release(was), release(now))
                    && running < backed_up
                {
                    return Err(Refusal::OlderAndroid { was: was.clone(), now: now.clone() }.into());
                }
            }
            Part::Media => {}
        }
    }
    // A data partition made afresh since the backup, as a factory reset
    // makes it, has another key: the backup's files go into it under the
    // keys it has now.
    let afresh = match items.iter().any(|i| i.part == Part::Data) {
        true => manifest.data_key.as_deref() != Some(data_key()?.as_str()),
        false => false,
    };
    tell(Event::Plan(items.iter().map(|i| (i.name.clone(), i.stored.size)).collect()));
    // A damaged backup is not written anywhere.
    for (k, item) in items.iter().enumerate() {
        let mut telling = Telling::new(tell, k, item.stored.size, true);
        check(&t.folder, &item.stored, cancel, &mut telling).with_context(|| format!("check {}", item.name))?;
    }
    for item in items.iter().filter(|i| matches!(i.part, Part::Boot | Part::Efs)) {
        let device = target(item, slot);
        for point in mount::points_of(skyward_block::devnum(&device)?)? {
            mount::umount(&point).with_context(|| format!("{} is in use at {}", device.display(), point.display()))?;
        }
    }
    if system {
        (t.detach)().context("take the slot's partitions off before writing over them")?;
    }
    // The boot partitions are restored as the installed system's, whose
    // boot set goes back into them first, so that the second system's is
    // not left kept as being there.
    if items.iter().any(|i| i.part == Part::Boot) {
        skyward_systems::install::ensure_installed()?;
    }
    // Once anything is written, the restore goes on to its end.
    let go_on = AtomicBool::new(false);
    let mut unmade = Vec::new();
    for (k, item) in items.iter().enumerate() {
        match item.part {
            Part::Boot | Part::Efs => {
                let mut telling = Telling::new(tell, k, item.stored.size * 2, false);
                write_image(&target(item, slot), &t.folder, &item.stored, &mut telling)?;
            }
            Part::System => {
                let mut telling = Telling::new(tell, k, item.stored.size * 2, false);
                let whole = skyward_vab::map_whole(slot, &item.name, &format!("skyward-restore-{}{}", item.name, slot.suffix()), false)?;
                let written = write_image(&whole.device, &t.folder, &item.stored, &mut telling);
                skyward_vab::unmap_whole(&whole)?;
                written?;
            }
            Part::Data | Part::Media => {
                let mut telling = Telling::new(tell, k, item.stored.size, false);
                let read = Cell::new(0);
                let mut reader = Reader::open(&t.folder, &item.stored);
                let from = Counted { inner: &mut reader, read: &read };
                let mut progress = |_: u64| telling.done(read.get());
                let mut gone = match item.part {
                    Part::Data => restore_data(from, Path::new(DATA), &t.skipped, manifest.format, afresh, &go_on, &mut progress)
                        .context("restore the data partition's files")?,
                    _ => restore_storage(from, Path::new(SHARED_STORAGE), &go_on, &mut progress).context("restore internal storage's files")?,
                };
                io::copy(&mut reader, &mut io::sink()).context("read the rest of the backup")?;
                reader.check(&item.stored)?;
                unmade.append(&mut gone);
            }
        }
        skyward_sys::kmsg(&format!("restored {}", item.name))?;
    }
    unsafe { libc::sync() };
    Ok(unmade)
}

/// The Android a release names, by its number: 17 for "17", 8 for "8.1.0".
fn release(text: &str) -> Option<u32> {
    text.split('.').next()?.trim().parse().ok()
}

/// The users the phone whose data partition is at `data` has: the system
/// makes each a folder named by the user's number, under a key of the
/// user's own.
fn users(data: &Path) -> Result<BTreeSet<u32>> {
    let folder = data.join(USERS);
    let mut users = BTreeSet::new();
    for entry in fs::read_dir(&folder).with_context(|| format!("read {}", folder.display()))? {
        let entry = entry.with_context(|| format!("read {}", folder.display()))?;
        let kind = entry.file_type().with_context(|| format!("look at {}", entry.path().display()))?;
        if let Some(user) = entry.file_name().to_str().and_then(|name| name.parse().ok())
            && kind.is_dir()
        {
            users.insert(user);
        }
    }
    Ok(users)
}

/// The user that `rel`, from the data partition, is kept for alone outside
/// the folders a key of that user's own begins in: the system's record of
/// the user, which is what has it count the user among the phone's, with
/// the copies it keeps of the record as it writes; the settings it keeps
/// for them; the certificates they trust; and their apps' profiles.
fn user_of(rel: &Path) -> Option<u32> {
    let names: Vec<&str> = rel.components().map(|c| c.as_os_str().to_str().unwrap_or_default()).collect();
    let name = match names.as_slice() {
        ["system", "users", name, ..] | ["misc", "user", name, ..] | ["misc", "profiles", "cur", name, ..] => name,
        _ => return None,
    };
    name.split('.').next()?.parse().ok()
}

/// Make the data partition at `data` hold the files the backup read from
/// `from` holds of it, in the layout `format`: all but what backups leave
/// out, which stays as the phone has it, as do the phone's own keys, under
/// which the backup's files are written. Nothing kept for one user alone is
/// put back for a user the phone has not got, whose own folders only the
/// system makes: the system counts as its users those it has a record of.
/// On a data partition made `afresh` since a backup in the layout before,
/// which holds nothing of the apps' folders in internal storage, what is in
/// those folders now goes: it is owned by the apps as the system made
/// afresh numbered them, and the backup's apps, numbered as they were,
/// could not open it. Gives the folders of users gone, which are not made.
fn restore_data(
    from: impl Read,
    data: &Path,
    phone_skipped: &[String],
    format: u32,
    afresh: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<Vec<PathBuf>> {
    let users = users(data)?;
    let skipped = data_skipped(phone_skipped, format).and(move |rel| user_of(rel).is_some_and(|user| !users.contains(&user)));
    let unpacked = archive::unpack(from, data, &skipped, Others::Removed, cancel, progress)?;
    if afresh && format == FORMAT_WITHOUT_STORAGE {
        empty_apps_folders(&data.join(STORAGE))?;
    }
    Ok(unpacked.unmade.iter().map(|u| data.join(u)).collect())
}

/// Put the users' own files the backup read from `from` holds back into
/// internal storage at `storage`: a file of the same name is the backup's
/// again, and what the backup does not hold stays. Gives the folders of
/// users gone, which are not made.
fn restore_storage(from: impl Read, storage: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<Vec<PathBuf>> {
    let unpacked = archive::unpack(from, storage, &storage_skipped(), Others::Kept, cancel, progress)?;
    Ok(unpacked.unmade.iter().map(|u| storage.join(u)).collect())
}

/// Take away what the apps keep in each user's internal storage under
/// `storage`, the folders they keep it in staying.
fn empty_apps_folders(storage: &Path) -> Result<()> {
    for user in fs::read_dir(storage).with_context(|| format!("read {}", storage.display()))? {
        let user = user.with_context(|| format!("read {}", storage.display()))?.path();
        for kind in APPS_KINDS {
            let folder = user.join(APPS_FOLDER).join(kind);
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => continue,
                Err(e) => return Err(e).with_context(|| format!("read {}", folder.display())),
            };
            for entry in entries {
                let entry = entry.with_context(|| format!("read {}", folder.display()))?;
                let kind = entry.file_type().with_context(|| format!("look at {}", entry.path().display()))?;
                match kind.is_dir() {
                    true => fs::remove_dir_all(entry.path()),
                    false => fs::remove_file(entry.path()),
                }
                .with_context(|| format!("take away {}", entry.path().display()))?;
            }
        }
    }
    Ok(())
}

/// The partition a part of a backup goes back to.
fn target(item: &Item, slot: Slot) -> PathBuf {
    match item.part {
        Part::Efs => partition(&item.name),
        _ => partition(&format!("{}{}", item.name, slot.suffix())),
    }
}

/// Write a part's image to the partition at `device`, then read it back
/// from the storage and see that it holds what was stored: twice the
/// part's bytes, told as they go.
fn write_image(device: &Path, folder: &Path, stored: &Stored, telling: &mut Telling) -> Result<()> {
    let mut reader = Reader::open(folder, stored);
    let mut out = OpenOptions::new().write(true).open(device).with_context(|| format!("open {}", device.display()))?;
    copy(&mut reader, &mut out, stored.size, &AtomicBool::new(false), telling).with_context(|| format!("write {}", device.display()))?;
    reader.check(stored)?;
    out.sync_all().with_context(|| format!("write {}", device.display()))?;
    // What is read back comes from the storage, not the pages just written.
    let rc = unsafe { libc::posix_fadvise(out.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc)).with_context(|| format!("drop the cached pages of {}", device.display()));
    }
    let back = File::open(device).with_context(|| format!("open {}", device.display()))?;
    let read = store::digest(back, stored.size, &mut |n| telling.done(stored.size + n)).with_context(|| format!("read {} back", device.display()))?;
    ensure!(read == stored.sha256, "{} reads back different from the backup", device.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skyward_crypto::fscrypt::{self, NAME_BYTES};

    /// A folder for a test, in the folder the users' shared storage is in:
    /// on the data partition, and encrypted under no key of its own, as
    /// the data partition's own top is, so a folder in it takes a key the
    /// way the system's and each user's folders do.
    fn scratch(test: &str) -> PathBuf {
        let at = Path::new(SHARED_STORAGE).join(format!("skyward-backup-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&at);
        fs::create_dir_all(&at).unwrap();
        at
    }

    /// The `n`th key of a test, let into the filesystem `root` is on, as
    /// the kernel names it.
    fn key(root: &Path, n: u8) -> [u8; NAME_BYTES] {
        let raw: Vec<u8> = (0..64u8).map(|b| b.wrapping_mul(7) ^ n.wrapping_mul(31) ^ std::process::id() as u8).collect();
        fscrypt::add_key(root, &raw, false).unwrap()
    }

    /// Make `folder` and encrypt what it comes to hold under the key the
    /// kernel names `key`, as a phone encrypts its files: contents with
    /// AES-256-XTS, names with AES-256-CTS padded to 16 bytes.
    fn encrypted(folder: &Path, key: &[u8; NAME_BYTES]) {
        const SET_POLICY: u32 = 0x800c_6613;
        fs::create_dir_all(folder).unwrap();
        let mut policy = [0u8; 24];
        policy[..4].copy_from_slice(&[2, 1, 4, 2]);
        policy[8..].copy_from_slice(key);
        let dir = File::open(folder).unwrap();
        let set = unsafe { libc::ioctl(dir.as_raw_fd(), SET_POLICY as libc::Ioctl, policy.as_ptr()) };
        assert_eq!(set, 0, "encrypt {}: {}", folder.display(), io::Error::last_os_error());
    }

    /// Take the key the kernel names `key` out of the filesystem `folder`
    /// is on; nothing it was used on is open any more.
    fn remove_key(folder: &Path, key: &[u8; NAME_BYTES]) {
        const REMOVE_KEY: u32 = 0xc040_6618;
        const BY_IDENTIFIER: u32 = 2;
        let mut arg = [0u8; 64];
        arg[..4].copy_from_slice(&BY_IDENTIFIER.to_ne_bytes());
        arg[8..8 + NAME_BYTES].copy_from_slice(key);
        let dir = File::open(folder).unwrap();
        let removed = unsafe { libc::ioctl(dir.as_raw_fd(), REMOVE_KEY as libc::Ioctl, arg.as_mut_ptr()) };
        assert_eq!(removed, 0, "remove the key: {}", io::Error::last_os_error());
        assert_eq!(&arg[40..44], &[0; 4], "no file kept the key in use");
    }

    fn put(path: &Path, holds: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, holds).unwrap();
    }

    /// The folders of a data partition at `data` as the system makes them:
    /// its own under the key `system`, and for each of `users` those a key
    /// of the user's own begins in, in the folders that hold every user's.
    fn partition(data: &Path, system: &[u8; NAME_BYTES], users: &[(u32, [u8; NAME_BYTES])]) {
        for own in ["system", "misc"] {
            encrypted(&data.join(own), system);
        }
        fs::create_dir_all(data.join("unencrypted")).unwrap();
        for (user, key) in users {
            for of_every_user in [USERS, STORAGE] {
                encrypted(&data.join(of_every_user).join(user.to_string()), key);
            }
        }
    }

    fn quiet(_: u64) {}

    #[test]
    fn on_the_phone_a_backup_goes_back_under_the_keys_of_a_data_partition_made_afresh() {
        let root = scratch("afresh");
        let (was, now) = (root.join("was"), root.join("now"));
        let old_keys = [key(&root, 1), key(&root, 2), key(&root, 3)];
        let new_keys = [key(&root, 4), key(&root, 5)];
        // The phone as it was backed up: the owner, and a second user.
        partition(&was, &old_keys[0], &[(0, old_keys[1]), (10, old_keys[2])]);
        put(&was.join("unencrypted/ref"), b"the key it had");
        put(&was.join("system/users/userlist.xml"), b"the users then");
        put(&was.join("system/locksettings.db"), b"the screen lock then");
        put(&was.join("system/packages.list"), b"com.example 10123");
        put(&was.join("misc/vold/user_keys/de/0/encrypted_key"), b"wrapped then");
        for user in ["0", "10"] {
            put(&was.join("system/users").join(format!("{user}.xml")), b"a user then");
            put(&was.join("system/users").join(format!("{user}.xml.reservecopy")), b"a copy of it");
            put(&was.join("system/users").join(user).join("settings_secure.xml"), b"settings then");
            put(&was.join("misc/user").join(user).join("cacerts-added/1.0"), b"a certificate");
            put(&was.join("misc/profiles/cur").join(user).join("com.example/primary.prof"), b"a profile");
            put(&was.join(USERS).join(user).join("com.example/files/alarm"), b"7:00");
            put(&was.join(STORAGE).join(user).join("DCIM/Camera/IMG_0001.jpg"), b"a photo");
            put(&was.join(STORAGE).join(user).join("Android/data/com.example/files/export.txt"), b"exported");
            put(&was.join(STORAGE).join(user).join("Skyward/backups/20261021-200110/data.tar.000"), b"an older backup");
        }
        put(&was.join(STORAGE).join("obb/com.example/main.obb"), b"expansion");
        let never = AtomicBool::new(false);
        let (mut data, mut storage) = (Vec::new(), Vec::new());
        archive::pack(&was, &data_skipped(&[], FORMAT), &mut data, &never, &mut quiet).unwrap();
        archive::pack(&was.join(STORAGE), &storage_skipped(), &mut storage, &never, &mut quiet).unwrap();
        // The backup's keys are gone with the data partition it was made
        // on.
        fs::remove_dir_all(&was).unwrap();
        for old in &old_keys {
            remove_key(&root, old);
        }

        // The phone since a factory reset, started once: the owner alone.
        partition(&now, &new_keys[0], &[(0, new_keys[1])]);
        put(&now.join("unencrypted/ref"), b"the key it has now");
        put(&now.join("system/users/userlist.xml"), b"the users now");
        put(&now.join("system/users/0.xml"), b"a user now");
        put(&now.join("system/locksettings.db"), b"the screen lock now");
        put(&now.join("system/packages.list"), b"com.fresh 10087");
        put(&now.join("misc/vold/user_keys/de/0/encrypted_key"), b"wrapped now");
        put(&now.join(USERS).join("0/com.fresh/files/first"), b"made as it started");
        put(&now.join(STORAGE).join("0/DCIM/Camera/IMG_0002.jpg"), b"a photo since");
        put(&now.join(STORAGE).join("0/Android/data/com.fresh/files/first"), b"made as it started");
        assert_eq!(users(&now).unwrap(), BTreeSet::from([0]));

        let unmade = restore_data(data.as_slice(), &now, &[], FORMAT, true, &never, &mut quiet).unwrap();
        assert_eq!(unmade, [now.join(STORAGE).join("10"), now.join(USERS).join("10")], "the second user's own folders are not made");
        let read = |rel: &str| fs::read(now.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
        assert_eq!(read("system/users/userlist.xml"), b"the users then");
        assert_eq!(read("system/users/0.xml"), b"a user then");
        assert_eq!(read("system/users/0/settings_secure.xml"), b"settings then");
        assert_eq!(read("system/packages.list"), b"com.example 10123");
        assert_eq!(read("misc/profiles/cur/0/com.example/primary.prof"), b"a profile");
        assert_eq!(read("user_de/0/com.example/files/alarm"), b"7:00");
        assert_eq!(read("media/0/Android/data/com.example/files/export.txt"), b"exported");
        assert_eq!(read("media/obb/com.example/main.obb"), b"expansion");
        for kept in [("unencrypted/ref", &b"the key it has now"[..]), ("system/locksettings.db", b"the screen lock now"), ("misc/vold/user_keys/de/0/encrypted_key", b"wrapped now")] {
            assert_eq!(read(kept.0), kept.1, "the phone's own keys and screen lock stay");
        }
        for gone in ["system/users/10.xml", "system/users/10.xml.reservecopy", "system/users/10", "misc/user/10", "misc/profiles/cur/10"] {
            assert!(!now.join(gone).exists(), "{gone}: the system finds no record of a user whose folders it cannot open");
        }
        for swept in ["user_de/0/com.fresh", "media/0/Android/data/com.fresh"] {
            assert!(!now.join(swept).exists(), "{swept}: an app the backup does not hold goes, with its folder in internal storage");
        }
        assert_eq!(read("media/0/DCIM/Camera/IMG_0002.jpg"), b"a photo since", "the user's own files stay");
        assert!(!now.join("media/0/DCIM/Camera/IMG_0001.jpg").exists(), "and are a part of their own");

        let unmade = restore_storage(storage.as_slice(), &now.join(STORAGE), &never, &mut quiet).unwrap();
        assert_eq!(unmade, [now.join(STORAGE).join("10")]);
        assert_eq!(read("media/0/DCIM/Camera/IMG_0001.jpg"), b"a photo");
        assert_eq!(read("media/0/DCIM/Camera/IMG_0002.jpg"), b"a photo since", "what the backup does not hold stays");
        assert!(!now.join("media/0/Skyward/backups").exists(), "a backup holds none of the backups");
        assert_eq!(read("media/0/Android/data/com.example/files/export.txt"), b"exported");

        fs::remove_dir_all(&now).unwrap();
        for new in &new_keys {
            remove_key(&root, new);
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn on_the_phone_a_backup_in_the_layout_before_holds_nothing_of_internal_storage() {
        let root = scratch("before");
        let (was, same, afresh) = (root.join("was"), root.join("same"), root.join("afresh"));
        for data in [&was, &same, &afresh] {
            put(&data.join("system/packages.list"), b"com.example 10123");
            put(&data.join(USERS).join("0/com.example/files/alarm"), b"7:00");
            put(&data.join(STORAGE).join("0/DCIM/Camera/IMG_0001.jpg"), b"a photo");
            for kind in APPS_KINDS {
                put(&data.join(STORAGE).join("0").join(APPS_FOLDER).join(kind).join("com.example/kept"), b"the app's own");
            }
            put(&data.join(STORAGE).join("0/Android/data/.nomedia"), b"");
        }
        let never = AtomicBool::new(false);
        let mut data = Vec::new();
        archive::pack(&was, &data_skipped(&[], FORMAT_WITHOUT_STORAGE), &mut data, &never, &mut quiet).unwrap();
        fs::write(same.join(USERS).join("0/com.example/files/alarm"), b"8:30").unwrap();

        // The data partition the backup was made on leaves internal
        // storage as it is.
        assert!(restore_data(data.as_slice(), &same, &[], FORMAT_WITHOUT_STORAGE, false, &never, &mut quiet).unwrap().is_empty());
        assert_eq!(fs::read(same.join(USERS).join("0/com.example/files/alarm")).unwrap(), b"7:00");
        for kind in APPS_KINDS {
            assert_eq!(fs::read(same.join(STORAGE).join("0/Android").join(kind).join("com.example/kept")).unwrap(), b"the app's own");
        }

        // One made afresh has the apps' folders emptied, the folders that
        // hold them and the user's own files staying.
        assert!(restore_data(data.as_slice(), &afresh, &[], FORMAT_WITHOUT_STORAGE, true, &never, &mut quiet).unwrap().is_empty());
        for kind in APPS_KINDS {
            let folder = afresh.join(STORAGE).join("0/Android").join(kind);
            assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "{}", folder.display());
        }
        assert_eq!(fs::read(afresh.join(STORAGE).join("0/DCIM/Camera/IMG_0001.jpg")).unwrap(), b"a photo");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn on_the_phone_what_is_kept_for_one_user_alone_is_told_by_where_it_is() {
        for (rel, user) in [
            ("system/users/10.xml", Some(10)),
            ("system/users/10.xml.reservecopy", Some(10)),
            ("system/users/999/settings_secure.xml", Some(999)),
            ("system/users/0", Some(0)),
            ("misc/user/10/cacerts-added/1.0", Some(10)),
            ("misc/profiles/cur/10/com.example/primary.prof", Some(10)),
            ("system/users/userlist.xml", None),
            ("system/users", None),
            ("misc/profiles/ref/com.example/primary.prof", None),
            ("user_de/10/com.example", None),
        ] {
            assert_eq!(user_of(Path::new(rel)), user, "{rel}");
        }
        assert_eq!((release("17"), release("8.1.0"), release("Baklava")), (Some(17), Some(8), None));
    }
}
