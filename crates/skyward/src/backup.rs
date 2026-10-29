//! Backing up and restoring on a thread of its own, telling the screen how
//! far it has come.

use anyhow::{ensure, Context, Result};
use skyward_backup::archive::{self, Skipped};
use skyward_backup::store::{self, Reader, Stored, Writer};
use skyward_backup::{Item, Manifest, Part, FORMAT};
use skyward_block::{mount, BY_NAME};
use skyward_files::{Cancelled, Ownership};
use skyward_vab::{Slot, SnapshotState};
use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
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
/// it starts, then the baseband's, then the data partition's files, which
/// take longest.
const RESTORE_ORDER: [Part; 4] = [Part::Boot, Part::System, Part::Efs, Part::Data];

/// What a backup of the data partition leaves out on every phone, and a
/// restore leaves as it is: internal storage, which is the user's own; the
/// keys the partition opens with, and what the lock screen and the key
/// service bind to the secure world, which an older copy would not open;
/// and what the system makes again, its caches, compiled code and the
/// updates it downloaded.
const DATA_SKIPPED: [&str; 20] = [
    "media",
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
];

/// What of the data partition backups leave out on this phone: what every
/// phone's do, and what the phone's own secure world keeps in step.
pub fn data_skipped(phone: &[String]) -> Vec<String> {
    DATA_SKIPPED.iter().map(|s| s.to_string()).chain(phone.iter().cloned()).collect()
}

pub struct BackupTask {
    /// Which of [`Part::ALL`] to take.
    pub parts: [bool; 4],
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
    /// codename, and the system it runs, by its version.
    pub made: i64,
    pub device: String,
    pub system: Option<String>,
    /// The phone's partitions of the baseband's settings and the sensors'
    /// calibration, and what of the data partition it leaves out.
    pub efs: Vec<String>,
    pub skipped: Vec<String>,
}

pub struct RestoreTask {
    pub folder: PathBuf,
    /// Which of [`Part::ALL`] to restore.
    pub parts: [bool; 4],
    pub device: String,
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
    /// The data partition has been made afresh since the backup.
    DataReset,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Room { needed, free } => write!(f, "the backup takes {needed} bytes, and {free} are free"),
            Refusal::Update(state) => write!(f, "an update is in flight ({state:?})"),
            Refusal::OtherPhone(device) => write!(f, "the backup was made on {device}"),
            Refusal::Resized(name) => write!(f, "{name} is not the size it was backed up at"),
            Refusal::DataReset => write!(f, "the data partition has been made afresh since the backup"),
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
    /// held in them.
    Restored { unmade: Vec<PathBuf> },
    /// Stopped as asked, with nothing of the backup left, or nothing of
    /// the restore written.
    Stopped,
    Refused(Refusal),
    Failed(String),
}

pub struct Worker {
    events: Receiver<Event>,
    /// Readable while events wait.
    wake: OwnedFd,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(task: Task) -> Result<Worker> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(io::Error::last_os_error()).context("make the backup worker's wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let thread = thread::Builder::new()
            .name("backup".into())
            .spawn(move || {
                let tell = |event: Event| {
                    // The screen only stops listening once it has let the
                    // worker go.
                    let _ = tx.send(event);
                    unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
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
            })
            .context("start the backup worker")?;
        Ok(Worker { events, wake, cancel, thread: Some(thread) })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.events.try_iter().collect()
    }

    /// Ask the work to stop; it does between two chunks, and a restore
    /// only while nothing is written yet.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Worker {
    /// The screen lets the worker go once its work has ended.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the backup worker does not panic");
        }
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
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The bytes free where `path` is, or would be made.
fn free_bytes(path: &Path) -> Result<u64> {
    let there = path.ancestors().find(|p| p.exists()).context("the root is always there")?;
    let c = std::ffi::CString::new(there.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", there.display()))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error()).with_context(|| format!("look at the room on {}", there.display()));
    }
    Ok(st.f_bavail as u64 * st.f_frsize as u64)
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
    let [boot, system, data, efs] = t.parts;
    if system {
        no_update()?;
    }
    let skipped = Skipped::new(&t.skipped);
    let mut plan: Vec<(Part, String, Source, u64)> = Vec::new();
    // What the parts' files take uncompressed: the bytes each holds, and
    // the data partition's archive headers.
    let mut needed = 0;
    if boot {
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
                (Part::Data, false) => format!("{name}.tar"),
                (Part::Data, true) => format!("{name}.tar.lz4"),
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
            }
            items.push(Item { part: *part, name: name.clone(), stored: writer.finish()? });
        }
        if t.verify {
            for (k, item) in items.iter().enumerate() {
                let mut telling = Telling::new(tell, k, item.stored.size, true);
                check(&folder, item, cancel, &mut telling).with_context(|| format!("check {}", item.name))?;
            }
        }
        let data_key = if data { Some(data_key()?) } else { None };
        let manifest =
            Manifest { format: FORMAT, device: t.device.clone(), made: t.made, slot: slot.suffix().into(), system: t.system.clone(), data_key, items };
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

/// Read a part back whole, checking its digest and data archive metadata.
fn check(folder: &Path, item: &Item, cancel: &AtomicBool, telling: &mut Telling) -> Result<()> {
    let stored = &item.stored;
    let mut reader = Reader::open(folder, stored);
    if item.part == Part::Data {
        let read = Cell::new(0);
        let from = Counted { inner: &mut reader, read: &read };
        archive::validate(from, cancel, &mut |_| telling.done(read.get())).context("check the data archive's metadata")?;
    } else {
        copy(&mut reader, &mut io::sink(), stored.size, cancel, telling)?;
    }
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
                if manifest.data_key.as_deref() != Some(data_key()?.as_str()) {
                    return Err(Refusal::DataReset.into());
                }
            }
        }
    }
    tell(Event::Plan(items.iter().map(|i| (i.name.clone(), i.stored.size)).collect()));
    // A damaged backup is not written anywhere.
    for (k, item) in items.iter().enumerate() {
        let mut telling = Telling::new(tell, k, item.stored.size, true);
        check(&t.folder, item, cancel, &mut telling).with_context(|| format!("check {}", item.name))?;
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
    // Once anything is written, the restore goes on to its end.
    let go_on = AtomicBool::new(false);
    let skipped = Skipped::new(&t.skipped);
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
            Part::Data => {
                let mut telling = Telling::new(tell, k, item.stored.size, false);
                let read = Cell::new(0);
                let mut reader = Reader::open(&t.folder, &item.stored);
                let from = Counted { inner: &mut reader, read: &read };
                let unpacked = archive::unpack(from, Path::new(DATA), &skipped, &go_on, &mut |_| telling.done(read.get()))
                    .context("restore the data partition's files")?;
                io::copy(&mut reader, &mut io::sink()).context("read the rest of the backup")?;
                reader.check(&item.stored)?;
                unmade = unpacked.unmade.iter().map(|u| Path::new(DATA).join(u)).collect();
            }
        }
        skyward_sys::kmsg(&format!("restored {}", item.name))?;
    }
    unsafe { libc::sync() };
    Ok(unmade)
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
