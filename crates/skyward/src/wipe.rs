//! Erasing what the system, the wipe page or an update asks to erase, the
//! way a phone's own recovery erases it, on a thread of its own.

use anyhow::{ensure, Context, Result};
use skyward_block::fstab::{self, Entry};
use skyward_block::{erase, mount};
use skyward_crypto::data::{self, Opening};
use skyward_sys::bcb;
use skyward_sys::worker::Worker;
use skyward_vab::Hold;
use std::ffi::CString;
use std::fs;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

const DATA: &str = "/data";
const CACHE: &str = "/cache";
const METADATA: &str = "/metadata";

/// Set where the system's shared storage looks names up without regard to
/// case, which it asks of the data partition's filesystem.
const CASEFOLD: &str = "external_storage.casefold.enabled";

/// Where the system keeps each user's apps' own data: the owner's apps
/// in their own folder, every other user's in one named by number; then
/// the part that reads before the user signs in.
const OWNER_APPS: &str = "/data/data";
const USER_APPS: &str = "/data/user";
const USER_APPS_EARLY: &str = "/data/user_de";

/// In each app's own data, the folders it keeps caches in: of what it
/// fetched or worked out, and of code it compiled itself.
const APP_CACHES: [&str; 2] = ["cache", "code_cache"];

/// Where the system notes the inode of each, on the folder of the app's
/// own data, so the folders are found while the user's names are locked.
/// Clearing an app's data takes the folders with it, and the notes too.
const CACHE_INODES: [&str; 2] = ["user.inode_cache", "user.inode_code_cache"];

/// In each user's shared storage, the folders apps keep their own files
/// in, one for each app; the first also holds each app's cache folder.
const SHARED_APP_FOLDERS: [&str; 2] = ["Android/data", "Android/media"];

/// The caches the system keeps for itself: the apps it has parsed, the
/// resources of the overlays it applies, and /cache, which on phones with
/// no cache partition is this folder.
const SYSTEM_CACHES: [&str; 3] = ["/data/system/package_cache", "/data/resource-cache", "/data/cache"];

/// Where ART keeps on the data partition what it compiles ahead of time:
/// the system's own apps, what the ART module compiles for the boot
/// classpath and the system server, and what is compiled for the slot an
/// update was installed to. The system compiles all of it again as it
/// needs it.
const ART_CACHES: [&str; 3] = ["/data/dalvik-cache", "/data/misc/apexdata/com.android.art/dalvik-cache", "/data/ota"];

/// Where every Android phone keeps logs and dumps: crashes, programs that
/// stopped answering, the system's dropbox of reports, logcat kept across
/// starts, traces, the recovery's logs copied into the system, the update
/// engine's logs, and the system server's heap dumps.
const LOGS: [&str; 8] = [
    "/data/tombstones",
    "/data/anr",
    "/data/system/dropbox",
    "/data/misc/logd",
    "/data/misc/perfetto-traces",
    "/data/misc/recovery",
    "/data/misc/update_engine_log",
    "/data/system/heapdump",
];

/// Where each user's bug reports are: in the data of the shell, which takes
/// them, under the part read before the user signs in.
const BUG_REPORTS: &str = "com.android.shell/files/bugreports";

/// Where the system downloads an update before it installs it.
const UPDATES: &str = "/data/ota_package";

/// The folder on the cache partition the recovery's logs are kept in, and
/// the most of each one kept while the partition is made afresh, as
/// Android's own recovery keeps them.
const CACHE_LOGS: &str = "recovery";
const LOG_KEPT: usize = 512 << 10;

/// How long the programs holding the data partition get to end.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The most often how far a part has come is told.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// A part erasing goes through, as it begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Data,
    Cache,
    /// /metadata, which holds the key the data partition is opened with.
    Metadata,
    /// Each app's own data, for every user.
    AppData,
    /// The files users share between apps.
    Media,
    /// Each app's caches, for every user.
    AppCaches,
    SystemCaches,
    /// What ART compiled ahead of time.
    Art,
    Logs,
    /// The updates the system downloaded.
    Updates,
}

impl Step {
    /// Whether the part is found only in folders users' own keys lock.
    fn needs_users(self) -> bool {
        matches!(self, Step::AppData | Step::AppCaches)
    }
}

/// Why erasing does not start.
#[derive(Debug)]
pub enum Refusal {
    /// An update the system has still to finish.
    Update(Hold),
    /// The system does not make an emptied data partition afresh: the
    /// phone's table does not have it encrypted whole, with its key in
    /// /metadata, and formattable.
    NotRebuilt,
    /// The phone has no cache partition.
    NoCache,
}

/// How erasing stopped short.
#[derive(Debug)]
pub enum Stop {
    Refused(Refusal),
    /// The data partition did not open, for the parts in it.
    Closed(String),
    /// These users' own files are still locked, and the parts chosen are
    /// partly in them: nothing was erased.
    Locked(Vec<u32>),
    /// Erasing this part went wrong.
    Failed(Step, anyhow::Error),
}

fn failed(step: Step) -> impl FnOnce(anyhow::Error) -> Stop {
    move |e| Stop::Failed(step, e)
}

/// What a factory reset erases on this phone, once it is known that it
/// can.
pub struct Plan {
    data: Entry,
    cache: Option<Entry>,
    metadata: Entry,
    /// The filesystem each partition is made afresh as, where the system
    /// asked for one.
    reformat: Option<String>,
}

/// Find what a factory reset erases: the data partition, the cache
/// partition where the phone has one, and /metadata. An update the system
/// has still to finish holds it back, as Android's own recovery is held back
/// by one it cannot finish for the system. The data partition is erased
/// only as one the system makes afresh as it starts, unless `reformat`
/// names the filesystem the system asked for it to be made as here.
pub fn factory_reset(reformat: Option<String>) -> Result<Plan, Stop> {
    let plan = (|| -> Result<Result<Plan, Refusal>> {
        if let Some(hold) = update_hold()? {
            return Ok(Err(Refusal::Update(hold)));
        }
        let data = first(DATA)?;
        let rebuilt = data.extra("keydirectory").is_some() && data.extras.iter().any(|e| e == "formattable");
        if !rebuilt && reformat.is_none() {
            return Ok(Err(Refusal::NotRebuilt));
        }
        Ok(Ok(Plan { data, cache: cache()?, metadata: first(METADATA)?, reformat }))
    })();
    match plan {
        Ok(Ok(plan)) => Ok(plan),
        Ok(Err(refusal)) => Err(Stop::Refused(refusal)),
        Err(e) => Err(Stop::Failed(Step::Data, e)),
    }
}

/// The update that keeps the data partition from being erased, if one does,
/// as /metadata tells it: mounted already, or mounted read-only to tell. A
/// /metadata that does not mount keeps track of no update to lose, and
/// erasing makes it afresh.
fn update_hold() -> Result<Option<Hold>> {
    let Some(current) = skyward_vab::current_slot()? else { return Ok(None) };
    let at = Path::new(METADATA);
    if !mount::is_mount_point(at)? {
        let device = fstab::device(&first(METADATA)?)?;
        if let Err(e) = mount::mount_ro(Path::new(&device), at) {
            skyward_sys::kmsg(&format!("{METADATA} does not mount, so it keeps no update: {e:#}"))?;
            return Ok(None);
        }
    }
    Ok(skyward_vab::update()?.and_then(|u| u.holds_wipe(current)))
}

impl Plan {
    /// The parts, in the order they are erased.
    pub fn steps(&self) -> Vec<Step> {
        let cache = self.cache.as_ref().map(|_| Step::Cache);
        std::iter::once(Step::Data).chain(cache).chain(std::iter::once(Step::Metadata)).collect()
    }

    /// Erase each part, telling `tell` of each as it begins. The programs
    /// holding the data partition end and the partition is closed; then it
    /// is discarded whole, and the system makes it afresh under a new key
    /// the next time it starts. Asked for another filesystem, every part is
    /// made afresh as it here instead, as Android's own recovery makes them:
    /// the data partition on the partition itself, which a system that
    /// encrypts it whole encrypts again as it next starts. Unless
    /// `keep_memtag_mode`, the memory tagging asked of the bootloader is
    /// forgotten too.
    pub fn carry_out(self, keep_memtag_mode: bool, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
        tell(Event::At(Step::Data));
        // The second system's boot set, and what keeps it, go with the data
        // partition and /metadata: the installed system's goes back into
        // the boot partitions first, while /metadata still tells where it
        // is kept. One that does not mount keeps track of none.
        if mount::is_mount_point(Path::new(METADATA)).map_err(failed(Step::Data))? {
            skyward_systems::install::ensure_installed().map_err(failed(Step::Data))?;
        }
        release_data().map_err(failed(Step::Data))?;
        let device = fstab::device(&self.data).map_err(failed(Step::Data))?;
        match &self.reformat {
            Some(filesystem) => erase::make(Path::new(&device), filesystem, &self.data.extras, casefolds(&self.data)),
            None => erase::discard(Path::new(&device)),
        }
        .map_err(failed(Step::Data))?;
        let made_as = |entry: &Entry| self.reformat.clone().unwrap_or_else(|| entry.filesystem.clone());
        if let Some(cache) = &self.cache {
            tell(Event::At(Step::Cache));
            remake_cache(cache, &made_as(cache)).map_err(failed(Step::Cache))?;
        }
        tell(Event::At(Step::Metadata));
        remake(&self.metadata, &made_as(&self.metadata)).map_err(failed(Step::Metadata))?;
        if !keep_memtag_mode {
            bcb::forget_memtag_mode(&bcb::misc()).map_err(failed(Step::Metadata))?;
        }
        Ok(())
    }
}

/// Parts erased on their own: what the data partition holds, each part as
/// the wipe page names it, and the cache partition.
pub struct Parts {
    steps: Vec<Step>,
    cache: Option<Entry>,
    /// The phone's own log folders in the data partition.
    logs: Vec<PathBuf>,
    /// The filesystem the cache partition is made afresh as, where the
    /// system asked for one.
    reformat: Option<String>,
}

/// Find the parts to erase on their own, in the order given: the cache
/// partition only where the phone has one, made afresh as `reformat` where
/// the system asked for that filesystem. `logs` are the phone's own log
/// folders in the data partition.
pub fn parts(steps: Vec<Step>, logs: &[String], reformat: Option<String>) -> Result<Parts, Stop> {
    assert!(
        steps.iter().all(|s| !matches!(s, Step::Data | Step::Metadata)),
        "the data partition and /metadata are erased whole only by a factory reset, not {steps:?}"
    );
    let cache = match steps.contains(&Step::Cache) {
        false => None,
        true => Some(cache().map_err(failed(Step::Cache))?.ok_or(Stop::Refused(Refusal::NoCache))?),
    };
    Ok(Parts { steps, cache, logs: logs.iter().map(PathBuf::from).collect(), reformat })
}

impl Parts {
    /// The parts, in the order they are erased.
    pub fn steps(&self) -> Vec<Step> {
        self.steps.clone()
    }

    /// Erase each part, telling `tell` of each as it begins and how far it
    /// has come. The data partition opens first where it is not open, as
    /// far as it opens without any user's credential. What users keep to
    /// themselves is deleted under the names it shows without their keys;
    /// the parts found only by name in users' own files wait for every
    /// user's files to be open, and until they are, nothing is erased.
    pub fn carry_out(self, tell: &mut dyn FnMut(Event)) -> Result<(), Stop> {
        let in_data = self.steps.iter().any(|&s| s != Step::Cache);
        if in_data && !mount::is_mount_point(Path::new(DATA)).map_err(|e| Stop::Closed(format!("{e:#}")))? {
            let opening = crate::decrypt::with_key_service(data::open_without_credential).map_err(Stop::Closed)?;
            tell(Event::Opened(opening));
        }
        let users = Users::of_the_phone();
        if self.steps.iter().any(|s| s.needs_users()) {
            let locked = users.locked().map_err(|e| Stop::Closed(format!("{e:#}")))?;
            if !locked.is_empty() {
                return Err(Stop::Locked(locked));
            }
        }
        for &step in &self.steps {
            tell(Event::At(step));
            let erased = match step {
                Step::AppData => users.app_data().and_then(|(paths, folders)| {
                    clear(&paths, tell)?;
                    forget_cache_inodes(&folders)
                }),
                Step::Media => shared_storage(&users.storage).and_then(|paths| clear(&paths, tell)),
                Step::AppCaches => users.app_caches().and_then(|paths| clear(&paths, tell)),
                Step::SystemCaches => clear_files(&SYSTEM_CACHES.map(PathBuf::from), tell),
                Step::Art => clear_files(&ART_CACHES.map(PathBuf::from), tell),
                Step::Logs => users.logs(&self.logs).and_then(|folders| clear_files(&folders, tell)),
                Step::Updates => clear_files(&[PathBuf::from(UPDATES)], tell),
                Step::Cache => {
                    let cache = self.cache.as_ref().expect("the cache partition was found as the parts were");
                    remake_cache(cache, self.reformat.as_deref().unwrap_or(&cache.filesystem))
                }
                Step::Data | Step::Metadata => unreachable!("only a factory reset erases {step:?}"),
            };
            erased.map_err(failed(step))?;
        }
        Ok(())
    }
}

/// Where the data partition keeps each user's apps' data and shared
/// storage.
struct Users {
    /// The owner's apps' own data, where the system has kept it since
    /// before phones had more users.
    owner_apps: PathBuf,
    /// Every user's apps' own data, in folders named by the user's number;
    /// the owner's is there too where the folder above is not.
    apps: PathBuf,
    /// The part of each user's apps' data that reads before they sign in.
    apps_early: PathBuf,
    /// Each user's shared storage.
    storage: PathBuf,
}

impl Users {
    fn of_the_phone() -> Users {
        Users {
            owner_apps: PathBuf::from(OWNER_APPS),
            apps: PathBuf::from(USER_APPS),
            apps_early: PathBuf::from(USER_APPS_EARLY),
            storage: PathBuf::from(data::SHARED_STORAGE),
        }
    }

    /// The users there is apps' data for, in order: each has a folder of
    /// it that reads before they sign in.
    fn list(&self) -> Result<Vec<u32>> {
        let mut users: Vec<u32> = contents(&self.apps_early)?
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse().ok()))
            .collect();
        users.sort_unstable();
        Ok(users)
    }

    /// Where `user`'s apps keep their own data, locked by the user's own
    /// key.
    fn apps(&self, user: u32) -> PathBuf {
        if user == 0 && fs::symlink_metadata(&self.owner_apps).is_ok_and(|m| m.is_dir()) {
            return self.owner_apps.clone();
        }
        self.apps.join(user.to_string())
    }

    /// Where `user`'s apps keep the part of their data that reads before
    /// the user signs in.
    fn apps_early(&self, user: u32) -> PathBuf {
        self.apps_early.join(user.to_string())
    }

    fn storage(&self, user: u32) -> PathBuf {
        self.storage.join(user.to_string())
    }

    /// The users whose own files are locked: their apps' data reads only
    /// as scrambled names until their key is let in.
    fn locked(&self) -> Result<Vec<u32>> {
        let mut locked = Vec::new();
        for user in self.list()? {
            let own = self.apps(user);
            if own.exists() && !skyward_crypto::fscrypt::unlocked(&own)? {
                locked.push(user);
            }
        }
        Ok(locked)
    }

    /// What clearing every app's data deletes, as the system clears one
    /// app's: all that the folders of each app's own data hold, for every
    /// user, both parts of it, and what each app keeps in its folders in
    /// shared storage. The folders stay as the system made them. The
    /// folders the system notes the caches of come with it, as the notes
    /// go too.
    fn app_data(&self) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let (mut paths, mut own) = (Vec::new(), Vec::new());
        for user in self.list()? {
            for app in folders(&self.apps(user))? {
                paths.extend(contents(&app)?);
                own.push(app);
            }
            for app in folders(&self.apps_early(user))? {
                paths.extend(contents(&app)?);
            }
            for shared in SHARED_APP_FOLDERS {
                for app in folders(&self.storage(user).join(shared))? {
                    paths.extend(contents(&app)?);
                }
            }
        }
        Ok((paths, own))
    }

    /// What clearing every app's caches deletes, as the system clears one
    /// app's: all its cache folders hold, in both parts of its own data and
    /// in its folder in shared storage, for every user. The folders stay.
    fn app_caches(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for user in self.list()? {
            for app in folders(&self.apps(user))?.into_iter().chain(folders(&self.apps_early(user))?) {
                for cache in APP_CACHES {
                    paths.extend(folder_contents(&app.join(cache))?);
                }
            }
            for app in folders(&self.storage(user).join(SHARED_APP_FOLDERS[0]))? {
                paths.extend(folder_contents(&app.join(APP_CACHES[0]))?);
            }
        }
        Ok(paths)
    }

    /// The folders the logs and crash records are in: every phone's, each
    /// user's bug reports, and the phone's own `extra`.
    fn logs(&self, extra: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut out: Vec<PathBuf> = LOGS.iter().map(PathBuf::from).collect();
        for user in self.list()? {
            out.push(self.apps_early(user).join(BUG_REPORTS));
        }
        out.extend(extra.iter().cloned());
        Ok(out)
    }
}

/// The folders a folder holds, each by its full path; links and other
/// entries left out, and nothing for a folder that is not there.
fn folders(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in contents(folder)? {
        if fs::symlink_metadata(&entry).with_context(|| format!("look at {}", entry.display()))?.is_dir() {
            out.push(entry);
        }
    }
    Ok(out)
}

/// What `path` holds where it is a folder; nothing where it is not there,
/// or is something an app made in the folder's place.
fn folder_contents(path: &Path) -> Result<Vec<PathBuf>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => contents(path),
        Ok(_) => Ok(Vec::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("look at {}", path.display())),
    }
}

/// Forget the inodes noted on each of `folders` for the cache folders that
/// clearing an app's data took away; the app notes the new ones as it makes
/// them again.
fn forget_cache_inodes(folders: &[PathBuf]) -> Result<()> {
    for folder in folders {
        let path = path_cstr(folder)?;
        for name in CACHE_INODES {
            let name = cstr(name);
            if unsafe { libc::lremovexattr(path.as_ptr(), name.as_ptr()) } != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ENODATA) {
                    return Err(e).with_context(|| format!("forget the caches noted on {}", folder.display()));
                }
            }
        }
    }
    Ok(())
}

/// The cache partition, where the phone has one.
pub fn cache() -> Result<Option<Entry>> {
    Ok(fstab::read(CACHE)?.into_iter().next())
}

/// The first entry the phone's table has for `mount_point`: the one the
/// system tries first.
fn first(mount_point: &str) -> Result<Entry> {
    fstab::read(mount_point)?.into_iter().next().with_context(|| format!("the phone's table has no {mount_point}"))
}

/// Make the partition an entry names afresh as `filesystem`, with nothing
/// mounted from it.
pub fn remake(entry: &Entry, filesystem: &str) -> Result<()> {
    let at = Path::new(&entry.mount_point);
    if mount::is_mount_point(at)? {
        mount::umount(at)?;
    }
    erase::make(Path::new(&fstab::device(entry)?), filesystem, &entry.extras, casefolds(entry))
}

/// Whether the filesystem made for an entry looks names up without regard
/// to case: the data partition's, where the system's shared storage asks
/// for it, as Android's own recovery makes it.
fn casefolds(entry: &Entry) -> bool {
    entry.mount_point == DATA && skyward_sys::bool_prop(CASEFOLD, false)
}

/// Let go of the data partition: every program but this one holding what is
/// mounted from it ends, the terminal's shell among them, and the partition
/// is closed.
pub fn release_data() -> Result<()> {
    let at = Path::new(DATA);
    if mount::is_mount_point(at)? {
        let device = mount::visible(at)?.with_context(|| format!("nothing shows at {DATA}"))?.device;
        end_holders(device)?;
    }
    data::close()
}

/// End every program but this one that holds the filesystem on `device`,
/// and wait for them to be gone.
fn end_holders(device: (u32, u32)) -> Result<()> {
    let me = std::process::id() as i32;
    let others = || -> Result<Vec<(i32, String)>> { Ok(mount::holding(device)?.into_iter().filter(|(pid, _)| *pid != me).collect()) };
    for (pid, name) in others()? {
        skyward_sys::kmsg(&format!("ending {name} ({pid}), which holds the data partition"))?;
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(e).with_context(|| format!("end {name} ({pid})"));
            }
        }
    }
    let until = Instant::now() + RELEASE_TIMEOUT;
    loop {
        let left = others()?;
        if left.is_empty() {
            return Ok(());
        }
        ensure!(
            Instant::now() < until,
            "{} still hold the data partition",
            left.iter().map(|(pid, name)| format!("{name} ({pid})")).collect::<Vec<_>>().join(", ")
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// What a folder holds, each entry by its full path; nothing for a folder
/// that is not there.
fn contents(folder: &Path) -> Result<Vec<PathBuf>> {
    match fs::read_dir(folder) {
        Ok(entries) => entries.map(|e| e.map(|e| e.path())).collect::<std::io::Result<_>>().with_context(|| format!("read {}", folder.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", folder.display())),
    }
}

/// What erasing the shared storage at `root` deletes: all that each user's
/// folder in it holds, and the folder apps share their large files in, the
/// folders themselves staying as the system made them, with their owners,
/// labels and encryption.
fn shared_storage(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in contents(root)? {
        let meta = fs::symlink_metadata(&entry).with_context(|| format!("look at {}", entry.display()))?;
        match meta.is_dir() {
            true => out.extend(contents(&entry)?),
            false => out.push(entry),
        }
    }
    Ok(out)
}

/// Delete `paths`, folders with all they hold, telling `tell` how many
/// entries are gone of how many.
fn clear(paths: &[PathBuf], tell: &mut dyn FnMut(Event)) -> Result<()> {
    told(tell, |cancel, progress| skyward_files::delete(paths, cancel, progress))
}

/// Delete every file under `folders`, leaving each folder in them as the
/// system made it, telling `tell` how many files are gone of how many.
fn clear_files(folders: &[PathBuf], tell: &mut dyn FnMut(Event)) -> Result<()> {
    told(tell, |cancel, progress| skyward_files::delete_files(folders, cancel, progress))
}

/// Do `work`, telling `tell` how far it has come at most every
/// [`PROGRESS_EVERY`], and once more as it ends.
fn told(
    tell: &mut dyn FnMut(Event),
    work: impl FnOnce(&AtomicBool, &mut dyn FnMut(skyward_files::Progress)) -> Result<()>,
) -> Result<()> {
    let mut since = Instant::now() - PROGRESS_EVERY;
    let mut last = None;
    let done = work(&AtomicBool::new(false), &mut |p| {
        last = Some((p.entries, p.total_entries));
        if since.elapsed() >= PROGRESS_EVERY {
            since = Instant::now();
            tell(Event::Progress(p.entries, p.total_entries));
        }
    });
    if let Some((done, total)) = last {
        tell(Event::Progress(done, total));
    }
    done
}

/// A log kept from the cache partition while it is made afresh: its name
/// and what it holds, up to [`LOG_KEPT`], with the owner, permissions and
/// label it had.
struct Log {
    name: PathBuf,
    data: Vec<u8>,
    look: Look,
}

/// An entry's owner, permissions and SELinux label.
struct Look {
    uid: u32,
    gid: u32,
    mode: u32,
    label: Option<Vec<u8>>,
}

const LABEL: &str = "security.selinux";

fn look(path: &Path) -> Result<Look> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("look at {}", path.display()))?;
    Ok(Look { uid: meta.uid(), gid: meta.gid(), mode: meta.mode() & 0o7777, label: label(path)? })
}

fn cstr(text: &str) -> CString {
    CString::new(text).expect("the names here hold no NUL")
}

fn path_cstr(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes()).with_context(|| format!("{} holds a NUL", path.display()))
}

/// The SELinux label `path` carries, as stored; none on a filesystem that
/// keeps none.
fn label(path: &Path) -> Result<Option<Vec<u8>>> {
    let (p, name) = (path_cstr(path)?, cstr(LABEL));
    let mut buf = vec![0u8; 256];
    let n = unsafe { libc::lgetxattr(p.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        let e = std::io::Error::last_os_error();
        if matches!(e.raw_os_error(), Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)) {
            return Ok(None);
        }
        return Err(e).with_context(|| format!("read the label of {}", path.display()));
    }
    buf.truncate(n as usize);
    Ok(Some(buf))
}

/// Give `path` the owner, permissions and label of `look`.
fn give(path: &Path, look: &Look) -> Result<()> {
    let p = path_cstr(path)?;
    if let Some(label) = &look.label {
        let name = cstr(LABEL);
        if unsafe { libc::lsetxattr(p.as_ptr(), name.as_ptr(), label.as_ptr().cast(), label.len(), 0) } != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("label {}", path.display()));
        }
    }
    if unsafe { libc::lchown(p.as_ptr(), look.uid, look.gid) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("give {} its owner", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(look.mode)).with_context(|| format!("give {} its permissions", path.display()))
}

/// Make the cache partition afresh as `filesystem`, keeping the recovery's
/// logs on it as Android's own recovery keeps them: the earlier ones and the
/// last, each up to [`LOG_KEPT`], back in their folder with the owners,
/// permissions and labels they had. A partition that no longer mounts has
/// none to keep.
pub fn remake_cache(entry: &Entry, filesystem: &str) -> Result<()> {
    let at = Path::new(&entry.mount_point);
    let device = PathBuf::from(fstab::device(entry)?);
    let kept = read_logs(&device, at)?;
    remake(entry, filesystem)?;
    let Some((folder, logs)) = kept else { return Ok(()) };
    fs::create_dir_all(at).with_context(|| format!("make {}", at.display()))?;
    // The table's options are for the filesystem it names.
    let options: &[String] = if filesystem == entry.filesystem { &entry.options } else { &[] };
    mount::mount(&device, at, filesystem, options)?;
    let written = (|| -> Result<()> {
        let dir = at.join(CACHE_LOGS);
        fs::create_dir(&dir).with_context(|| format!("make {}", dir.display()))?;
        give(&dir, &folder)?;
        for log in &logs {
            let path = dir.join(&log.name);
            fs::write(&path, &log.data).with_context(|| format!("write {}", path.display()))?;
            give(&path, &log.look)?;
        }
        Ok(())
    })();
    let unmounted = mount::umount(at);
    written?;
    unmounted
}

/// The recovery's logs on the cache partition on `device`, read with it
/// mounted read-only at `at`, and the folder that holds them; nothing where
/// the partition does not mount or has no such folder.
fn read_logs(device: &Path, at: &Path) -> Result<Option<(Look, Vec<Log>)>> {
    if mount::is_mount_point(at)? {
        mount::umount(at)?;
    }
    fs::create_dir_all(at).with_context(|| format!("make {}", at.display()))?;
    if let Err(e) = mount::mount_ro(device, at) {
        skyward_sys::kmsg(&format!("{} does not mount, so it keeps no logs: {e:#}", device.display()))?;
        return Ok(None);
    }
    let read = (|| -> Result<Option<(Look, Vec<Log>)>> {
        let dir = at.join(CACHE_LOGS);
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut logs = Vec::new();
        for path in contents(&dir)? {
            let name = path.file_name().with_context(|| format!("{} has no name", path.display()))?.to_owned();
            let text = name.to_string_lossy();
            if !(text.starts_with("last_") || text == "log") || !path.is_file() {
                continue;
            }
            let mut data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            data.truncate(LOG_KEPT);
            logs.push(Log { name: PathBuf::from(name), data, look: look(&path)? });
        }
        Ok(Some((look(&dir)?, logs)))
    })();
    let unmounted = mount::umount(at);
    let read = read?;
    unmounted?;
    Ok(read)
}

/// What erasing tells the screen.
pub enum Event {
    At(Step),
    /// How far the part under way has come: entries done, of how many.
    Progress(u64, u64),
    /// Erasing opened the data partition, as far as it opens without any
    /// user's credential.
    Opened(Opening),
    /// Done, or how it stopped.
    Ended(Result<(), Stop>),
}

/// Erasing on a thread of its own, so the screen keeps up while it runs:
/// discarding a whole data partition can take minutes, and so can deleting
/// all a user keeps.
pub struct Eraser {
    /// The screen lets it go once it has ended.
    worker: Worker<Event>,
}

impl Eraser {
    /// Run `work` on its thread; what it tells, and how it ends, come back
    /// as events.
    pub fn start(work: impl FnOnce(&mut dyn FnMut(Event)) -> Result<(), Stop> + Send + 'static) -> Result<Eraser> {
        let worker = Worker::start("eraser", move |teller| {
            let mut tell = |event: Event| {
                teller.tell(event);
            };
            let ended = work(&mut tell);
            tell(Event::Ended(ended));
        })?;
        Ok(Eraser { worker })
    }

    /// An eraser that ends at once, as `stop` says.
    pub fn stopped(stop: Stop) -> Result<Eraser> {
        Eraser::start(move |_| Err(stop))
    }

    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.worker.fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        self.worker.events()
    }
}

/// How erasing stopped, said for the kernel log.
pub fn said(stop: &Stop) -> String {
    match stop {
        Stop::Refused(r) => format!("refused: {r:?}"),
        Stop::Closed(why) => format!("the data partition did not open: {why}"),
        Stop::Locked(users) => format!("the own files of users {users:?} are locked"),
        Stop::Failed(step, e) => format!("{step:?} failed: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skyward_crypto::fscrypt::{self, NAME_BYTES};
    use std::os::fd::AsRawFd;

    /// A folder for a test, in the folder the users' shared storage is in:
    /// on the data partition, and encrypted under no key of its own, so a
    /// folder in it takes a key the way each user's own folder there does.
    fn scratch(test: &str) -> PathBuf {
        let at = Path::new(data::SHARED_STORAGE).join(format!("skyward-wipe-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&at);
        fs::create_dir_all(&at).unwrap();
        at
    }

    /// Encrypt what `folder`, empty, comes to hold under the key the kernel
    /// names `key`, as a phone encrypts a user's own files: contents with
    /// AES-256-XTS, names with AES-256-CTS padded to 16 bytes.
    fn encrypt(folder: &Path, key: &[u8; NAME_BYTES]) {
        const SET_POLICY: u32 = 0x800c_6613;
        let mut policy = [0u8; 24];
        policy[..4].copy_from_slice(&[2, 1, 4, 2]);
        policy[8..].copy_from_slice(key);
        let dir = fs::File::open(folder).unwrap();
        let set = unsafe { libc::ioctl(dir.as_raw_fd(), SET_POLICY as libc::Ioctl, policy.as_ptr()) };
        assert_eq!(set, 0, "encrypt {}: {}", folder.display(), std::io::Error::last_os_error());
    }

    /// Take the key the kernel names `key` out of the filesystem `folder` is
    /// on, as a phone does when a user's own files lock again.
    fn remove_key(folder: &Path, key: &[u8; NAME_BYTES]) {
        const REMOVE_KEY: u32 = 0xc040_6618;
        const BY_IDENTIFIER: u32 = 2;
        let mut arg = [0u8; 64];
        arg[..4].copy_from_slice(&BY_IDENTIFIER.to_ne_bytes());
        arg[8..8 + NAME_BYTES].copy_from_slice(key);
        let dir = fs::File::open(folder).unwrap();
        let removed = unsafe { libc::ioctl(dir.as_raw_fd(), REMOVE_KEY as libc::Ioctl, arg.as_mut_ptr()) };
        assert_eq!(removed, 0, "remove the key: {}", std::io::Error::last_os_error());
        assert_eq!(&arg[40..44], &[0; 4], "no file kept the key in use");
    }

    #[test]
    fn on_the_phone_what_a_user_keeps_under_a_key_that_is_gone_is_deleted_by_the_names_it_shows() {
        let root = scratch("locked");
        assert!(contents(&root.join("absent")).unwrap().is_empty(), "a folder that is not there holds nothing");
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        let raw: Vec<u8> = (0..64u8).map(|b| b ^ std::process::id() as u8).collect();
        let key = fscrypt::add_key(&root, &raw, false).unwrap();
        encrypt(&locked, &key);
        fs::create_dir_all(locked.join("DCIM/Camera")).unwrap();
        fs::write(locked.join("DCIM/Camera/IMG_0001.jpg"), vec![7u8; 100_000]).unwrap();
        fs::write(locked.join("note.txt"), b"mine").unwrap();
        remove_key(&root, &key);
        let entries = contents(&locked).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(!entries.iter().any(|e| e.ends_with("note.txt") || e.ends_with("DCIM")), "without the key the names do not read: {entries:?}");
        let mut told = Vec::new();
        clear(&entries, &mut |e| {
            if let Event::Progress(done, total) = e {
                told.push((done, total));
            }
        })
        .unwrap();
        assert!(contents(&locked).unwrap().is_empty(), "all of it is gone, the folder staying");
        assert_eq!(told.last(), Some(&(4, 4)), "the folder, the one in it, and two files");
        fs::remove_dir_all(&root).unwrap();
    }

    /// The folders of users' apps' data and shared storage, made the way
    /// the system makes them, under `root`: the owner and user 10, each
    /// with an app that keeps files, caches and a folder in shared storage.
    fn users_at(root: &Path) -> Users {
        let users = Users {
            owner_apps: root.join("data"),
            apps: root.join("user"),
            apps_early: root.join("user_de"),
            storage: root.join("media"),
        };
        fs::create_dir_all(&users.owner_apps).unwrap();
        for user in [0, 10] {
            let app = users.apps(user).join("com.example.notes");
            let early = users.apps_early(user).join("com.example.notes");
            for folder in [app.join("cache/thumbnails"), app.join("code_cache"), app.join("databases"), early.join("cache"), early.join("files")] {
                fs::create_dir_all(folder).unwrap();
            }
            fs::write(app.join("databases/notes.db"), b"notes").unwrap();
            fs::write(app.join("cache/thumbnails/1.png"), b"png").unwrap();
            fs::write(app.join("code_cache/jit.bin"), b"jit").unwrap();
            fs::write(early.join("cache/early.tmp"), b"tmp").unwrap();
            fs::write(early.join("files/alarm"), b"7:00").unwrap();
            let shared = users.storage(user).join("Android/data/com.example.notes");
            fs::create_dir_all(shared.join("cache")).unwrap();
            fs::write(shared.join("cache/download.part"), b"part").unwrap();
            fs::write(shared.join("export.txt"), b"exported").unwrap();
            fs::create_dir_all(users.storage(user).join("Android/media/com.example.notes")).unwrap();
            fs::write(users.storage(user).join("Android/media/com.example.notes/voice.m4a"), b"voice").unwrap();
            fs::create_dir_all(users.storage(user).join("DCIM")).unwrap();
            fs::write(users.storage(user).join("DCIM/photo.jpg"), b"photo").unwrap();
            for (name, inode) in CACHE_INODES.iter().zip([app.join("cache"), app.join("code_cache")]) {
                let ino = fs::metadata(inode).unwrap().ino().to_ne_bytes();
                let (path, name) = (path_cstr(&app).unwrap(), cstr(name));
                assert_eq!(unsafe { libc::lsetxattr(path.as_ptr(), name.as_ptr(), ino.as_ptr().cast(), ino.len(), 0) }, 0);
            }
        }
        users
    }

    fn quiet(_: Event) {}

    #[test]
    fn on_the_phone_every_apps_caches_go_and_the_rest_of_its_data_stays() {
        let root = scratch("caches");
        let users = users_at(&root);
        assert_eq!(users.list().unwrap(), [0, 10]);
        assert_eq!(users.apps(0), root.join("data"), "the owner's apps are where the system has always kept them");
        clear(&users.app_caches().unwrap(), &mut quiet).unwrap();
        for user in [0, 10] {
            let app = users.apps(user).join("com.example.notes");
            let early = users.apps_early(user).join("com.example.notes");
            let shared = users.storage(user).join("Android/data/com.example.notes");
            for emptied in [app.join("cache"), app.join("code_cache"), early.join("cache"), shared.join("cache")] {
                assert!(contents(&emptied).unwrap().is_empty(), "{} is emptied", emptied.display());
            }
            for kept in [app.join("databases/notes.db"), early.join("files/alarm"), shared.join("export.txt")] {
                assert!(kept.exists(), "{} stays", kept.display());
            }
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn on_the_phone_every_apps_data_goes_and_the_users_own_files_stay() {
        let root = scratch("app-data");
        let users = users_at(&root);
        let (paths, own) = users.app_data().unwrap();
        clear(&paths, &mut quiet).unwrap();
        forget_cache_inodes(&own).unwrap();
        for user in [0, 10] {
            let app = users.apps(user).join("com.example.notes");
            let storage = users.storage(user);
            for emptied in [
                app.clone(),
                users.apps_early(user).join("com.example.notes"),
                storage.join("Android/data/com.example.notes"),
                storage.join("Android/media/com.example.notes"),
            ] {
                assert!(emptied.is_dir() && contents(&emptied).unwrap().is_empty(), "{} is emptied and stays", emptied.display());
            }
            assert!(storage.join("DCIM/photo.jpg").exists(), "the user's own files stay");
            for name in CACHE_INODES {
                let (path, name) = (path_cstr(&app).unwrap(), cstr(name));
                let mut buf = [0u8; 8];
                let read = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
                assert_eq!(read, -1, "the note of a cache folder that went goes with it");
            }
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn on_the_phone_a_user_whose_key_is_out_is_locked() {
        let root = scratch("lock");
        let users = users_at(&root);
        let other = users.apps(10);
        fs::remove_dir_all(&other).unwrap();
        fs::create_dir(&other).unwrap();
        let raw: Vec<u8> = (0..64u8).map(|b| b.wrapping_mul(3) ^ std::process::id() as u8).collect();
        let key = fscrypt::add_key(&root, &raw, false).unwrap();
        encrypt(&other, &key);
        fs::create_dir(other.join("com.example.notes")).unwrap();
        assert!(users.locked().unwrap().is_empty(), "with the key in, every user reads");
        remove_key(&root, &key);
        assert_eq!(users.locked().unwrap(), [10], "the owner's folder is not encrypted here; user 10's key is out");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn on_the_phone_the_logs_take_in_each_users_bug_reports_and_the_phones_own() {
        let root = scratch("logs");
        let users = users_at(&root);
        let extra = [PathBuf::from("/data/vendor/diag")];
        let folders = users.logs(&extra).unwrap();
        assert_eq!(&folders[..LOGS.len()], LOGS.map(PathBuf::from));
        assert!(folders.contains(&users.apps_early(10).join(BUG_REPORTS)));
        assert_eq!(folders.last(), Some(&extra[0]));
        fs::remove_dir_all(&root).unwrap();
    }

    /// A cache partition on a loop device, made as ext4 and holding the
    /// recovery's logs as Android's own recovery leaves them among other
    /// files, unmounted, with its entry in the phone's table.
    struct Cache {
        root: PathBuf,
        device: String,
        entry: Entry,
        /// The device, held open so nothing lets it go before the test does.
        held: fs::File,
        /// The label the earlier log carries.
        labelled: Option<Vec<u8>>,
    }

    fn cache_with_logs(test: &str) -> Cache {
        let root = scratch(test);
        let image = root.join("cache.img");
        fs::File::create(&image).unwrap().set_len(128 << 20).unwrap();
        let out = std::process::Command::new("/system/bin/losetup").args(["-f", "--show"]).arg(&image).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let device = String::from_utf8(out.stdout).unwrap().trim().to_string();
        let held = fs::File::open(&device).unwrap();
        erase::make(Path::new(&device), "ext4", &[], false).unwrap();
        let at = root.join("mnt");
        fs::create_dir(&at).unwrap();
        let entry = Entry {
            device: device.clone(),
            mount_point: at.to_string_lossy().into_owned(),
            filesystem: "ext4".into(),
            options: Vec::new(),
            extras: Vec::new(),
        };
        mount::mount(Path::new(&device), &at, "ext4", &[]).unwrap();
        let logs = at.join(CACHE_LOGS);
        fs::create_dir(&logs).unwrap();
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o770)).unwrap();
        std::os::unix::fs::chown(&logs, Some(1000), Some(2001)).unwrap();
        fs::write(logs.join("last_kmsg"), vec![b'k'; LOG_KEPT + 1000]).unwrap();
        fs::write(logs.join("last_log.1"), b"an earlier log").unwrap();
        fs::set_permissions(logs.join("last_log.1"), fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::chown(logs.join("last_log.1"), Some(1000), Some(2001)).unwrap();
        fs::write(logs.join("log"), b"this one").unwrap();
        fs::write(logs.join("command"), b"--wipe_data").unwrap();
        fs::write(at.join("stray"), b"cached").unwrap();
        let labelled = label(&logs.join("last_log.1")).unwrap();
        mount::umount(&at).unwrap();
        Cache { root, device, entry, held, labelled }
    }

    impl Cache {
        /// Check that the cache partition, made afresh as `filesystem`,
        /// holds the logs as they were and nothing else, and let it go.
        fn holds_the_logs_alone(self, filesystem: &str) {
            let at = PathBuf::from(&self.entry.mount_point);
            let logs = at.join(CACHE_LOGS);
            assert_eq!(skyward_block::probe::filesystem(Path::new(&self.device)).unwrap().as_deref(), Some(filesystem));
            mount::mount(Path::new(&self.device), &at, filesystem, &[]).unwrap();
            assert!(!at.join("stray").exists(), "the rest is gone");
            assert!(!logs.join("command").exists(), "only the logs are kept");
            assert_eq!(fs::read(logs.join("last_kmsg")).unwrap().len(), LOG_KEPT, "each up to the most kept");
            assert_eq!(fs::read(logs.join("last_log.1")).unwrap(), b"an earlier log");
            assert_eq!(fs::read(logs.join("log")).unwrap(), b"this one");
            let earlier = fs::metadata(logs.join("last_log.1")).unwrap();
            assert_eq!((earlier.uid(), earlier.gid(), earlier.mode() & 0o7777), (1000, 2001, 0o640));
            let folder = fs::metadata(&logs).unwrap();
            assert_eq!((folder.uid(), folder.gid(), folder.mode() & 0o7777), (1000, 2001, 0o770));
            assert_eq!(label(&logs.join("last_log.1")).unwrap(), self.labelled, "with the label it had");
            mount::umount(&at).unwrap();
            drop(self.held);
            let detached = std::process::Command::new("/system/bin/losetup").args(["-d", &self.device]).status().unwrap();
            assert!(detached.success(), "let {} go", self.device);
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn on_the_phone_the_cache_partition_is_made_afresh_with_the_recoverys_logs_kept() {
        let cache = cache_with_logs("cache");
        remake_cache(&cache.entry, "ext4").unwrap();
        cache.holds_the_logs_alone("ext4");
    }

    #[test]
    fn on_the_phone_the_cache_partition_made_afresh_as_another_filesystem_keeps_the_logs() {
        let cache = cache_with_logs("cache-f2fs");
        remake_cache(&cache.entry, "f2fs").unwrap();
        cache.holds_the_logs_alone("f2fs");
    }
}
