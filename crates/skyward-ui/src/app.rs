use crate::canvas::{Canvas, Run};
use crate::color::Rgba;
use crate::editor::{self, Editor};
use crate::files::{self, FileType};
use crate::fling::{Fling, Tracker};
use crate::fmt;
use crate::focus::{self, Group, Spot, Stop};
use crate::font::Fonts;
use crate::geom::Rect;
use crate::glow::{self, GlowClock};
use crate::lang::{Lang, Text};
use crate::motion::{self, Spring};
use crate::pages::{self, Band, Ctx};
use crate::sprite::{Blurs, Sprite};
use crate::search;
use crate::term::{self, Term, TermKey};
use crate::theme::Theme;
use crate::tr;
use crate::widgets::{self, Bar, Bolt, Cap, Layer, WlanMark};
use skyward_info::DeviceInfo;
use skyward_install::{Event, ImageKind, Outcome, Reflash, Refusal, Stage};
use skyward_net::download::{Failure, Progress};
use skyward_net::release::{Release, Unanswered};
use skyward_net::supplicant::Security;
use skyward_net::wlan::{Joined, Network, Power, Stage as Joining, Traffic, Wlan};
use skyward_vab::{BootSlots, Hold, Slot, SnapshotState, Standing};

/// The pages of the recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Decrypt,
    Home,
    Settings,
    Reboot,
    Wipe,
    /// Mounting and unmounting the slot's partitions.
    Mount,
    Install,
    /// A place's folders and the files in them that can be installed, to
    /// choose one.
    Choose(Place),
    /// Installing a package, which the computer sends with `adb sideload`
    /// or which is a file chosen in a place, and how it went.
    Installing,
    About,
    Specs,
    /// How long the screen stays on with nothing touched.
    ScreenOff,
    /// Finding a feature or partition by name, typed on the screen's own
    /// keyboard.
    Search,
    Backup,
    /// The backups found, to restore one.
    Restore,
    /// One backup, and what to restore from it.
    RestoreBackup,
    /// Backing up or restoring under way, and how it ended.
    BackupJob,
    /// Tools for the experienced: files, a terminal, the logs.
    Advanced,
    Files,
    /// One file, and what can be done with it.
    FileInfo,
    /// The permission bits of the entry the file page shows, a switch each.
    Permissions,
    /// A text file, changed on the screen's keyboard and then saved.
    Editor,
    Terminal,
    /// Giving the data partition's files the SELinux labels the installed
    /// system's rules give them: what it does and a swipe to start, then
    /// how far it has come and how it ended.
    Relabel,
    /// The partitions whose filesystems can be repaired or made afresh as
    /// another.
    Filesystems,
    /// One of them: what it holds, and what to do to it, with a swipe to
    /// do it.
    Filesystem,
    /// Work on whole partitions under way, and how it ended: the
    /// filesystem page's, or what the system asked for before the recovery
    /// started.
    PartWork,
    Language,
    TimeZone,
    /// Saving the logs, as a bug report asks for them.
    Logs,
    /// Where an update the system installed stands, and merging it into
    /// the slot it went to.
    Update,
    /// Skyward's own licence.
    License,
    /// The interface typeface and the terms it is used under.
    FontLicense,
    /// Which slot the phone starts from next.
    Slot,
    /// The phone's own fastbootd serving the computer's fastboot, and where
    /// to go from it. Its only page, worked with the keys: the partitions
    /// the touchscreen runs from are left out while fastbootd rewrites
    /// super.
    Fastboot,
    /// Erasing what the system asked to erase before the recovery started,
    /// all user data or the cache alone: how far it has come, and where to
    /// go if it stops.
    Erase,
    /// The system could not start Android and asks whether to erase all
    /// user data, which may be what keeps it from starting.
    CantStart,
    /// Something the system asked of the recovery that it does not do.
    Unhandled,
    /// A new name for the entry the file page shows, typed on the screen's
    /// keyboard.
    Rename,
    /// The folder an entry is copied or moved to already holds that name;
    /// what to do is the user's to choose.
    Clash,
    /// Copying, moving or deleting under way, and how it ended.
    FileJob,
    /// The WLAN switch, the network chosen and how joining it goes, and
    /// the networks in range.
    Wlan,
    /// The password of the network chosen, typed on the screen's keyboard.
    WlanPassword,
    /// The network joined, and the address and the rest it gave.
    WlanNetwork,
    /// A network added by hand: its name, how it is protected and its
    /// password, typed on the screen's keyboard.
    WlanAdd,
}

/// How far asking the update server for the newest system has come.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Check {
    #[default]
    Idle,
    Asking,
    Failed(Unanswered),
    /// The newest system the server knows; none when it knows of none for
    /// the installed one.
    Answered(Option<Release>),
}

/// What internal storage holds of the newest system's full package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Have {
    #[default]
    Nothing,
    /// Part of it, this many bytes, from a download stopped before its end.
    Part(u64),
    /// All of it, checked, this many bytes.
    Whole(u64),
}

/// Downloading the newest system's full package, as the update page shows
/// it: how far it has come, how fast, and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadJob {
    /// None until the download has begun.
    pub progress: Option<Progress>,
    /// Bytes a second.
    pub rate: u64,
    pub outcome: Option<DownloadEnd>,
    /// Stopping was asked for and is under way.
    pub stopping: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadEnd {
    Downloaded,
    Stopped,
    Failed(Failure),
}

/// Where the full packages downloaded go, from the top of internal
/// storage: the folder the stock updater keeps them in.
pub const DOWNLOADS: &str = "downloaded_rom";

/// Where the USB drive plugged in is shown.
pub const USB_DRIVE: &str = "/mnt/usb";

/// What a USB drive plugged in that cannot be read is said to be, and what
/// is asked for while none is plugged in.
pub const USB_DRIVE_UNREADABLE: Text = ["无法读取该 U 盘", "無法讀取這個 USB 隨身碟", "This USB drive can't be read"];
pub const USB_DRIVE_WANTED: Text = ["请通过 OTG 连接 U 盘", "請透過 OTG 連接 USB 隨身碟", "Connect a USB drive through OTG"];

/// The data partition's name, as the filesystem page lists it.
pub const DATA_PARTITION: &str = "data";

/// What work that needs the data partition is told while it is being
/// opened.
pub const DATA_OPENING: Text = ["正在打开 data 分区，请稍后再试", "正在開啟 data 分割區，請稍後再試", "Opening the data partition. Try again shortly"];

/// What an install is told while the last one has not ended.
pub const INSTALL_UNFINISHED: Text = ["上一次安装尚未结束，请稍后再试", "上一次安裝尚未結束，請稍後再試", "The last install hasn't ended. Try again shortly"];

/// What work on the data partition is told once it has been erased whole.
pub const DATA_ERASED: Text = ["data 分区已清除", "data 分割區已清除", "The data partition was wiped"];

/// What work on internal storage is told before the data partition is
/// decrypted.
pub const DECRYPT_FIRST: Text = ["请先完成「解密」", "請先完成「解密」", "Decrypt first"];

/// What going back is told while work that can be stopped goes on.
const WAIT_OR_STOP: Text = ["请等待完成或停止后再返回", "請等待完成或停止後再返回", "Wait for it to finish, or stop it, before going back"];

/// A password typed, which what is printed of it leaves out: the recovery
/// prints each effect the pages ask for.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({} characters)", self.0.chars().count())
    }
}

/// A network being added by hand, as its page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adding {
    pub name: String,
    pub security: Security,
    /// What the keyboard types into.
    pub field: Field,
    /// The protections to choose from are open over the page.
    pub choosing: bool,
}

/// A text field of the page that adds a network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Password,
}

/// How a network added by hand can be protected, in the order the stock
/// list gives them: those the recovery joins.
pub const ADDABLE: [Security; 3] = [Security::Open, Security::Psk, Security::Sae];

/// The longest name a network can have, in bytes.
const SSID_MAX: usize = 32;

/// What turns down a password the protection `security` cannot take.
fn refused(l: Lang, security: Security, password: &str) -> Option<&'static str> {
    match security {
        Security::Psk | Security::PskSae if !(8..=63).contains(&password.len()) => {
            Some(l.pick(["请输入 8 到 63 位密码", "請輸入 8 到 63 個字元的密碼", "Enter a password of 8 to 63 characters"]))
        }
        _ if password.is_empty() => Some(l.pick(["请输入密码", "請輸入密碼", "Enter a password"])),
        _ => None,
    }
}

/// Where a file to install is chosen from, and where backups go and are
/// found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The owner's internal storage, which opens with the data partition.
    Storage,
    /// A USB drive plugged into the port, which the runtime shows read
    /// only.
    UsbDrive,
}

/// The USB drive plugged in, as the runtime found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbDrive {
    /// Shown, for a file on it to be chosen.
    Shown,
    /// Nothing on it could be read: why.
    Unreadable(String),
}

/// Where the package being installed comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The computer, with `adb sideload`.
    #[default]
    Computer,
    /// A file chosen in a place.
    File(Place),
    /// A file opened in the file manager; `drive` for one on the USB drive.
    Opened { drive: bool },
    /// A package on the phone the system asked to be installed before the
    /// recovery started; `security` for a security update.
    Asked { security: bool },
    /// The computer, with `adb rescue`.
    Rescue,
}

/// What can be done with an entry picked in the file manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOp {
    Copy,
    Move,
}

/// An entry picked to copy or move, while the file manager looks for the
/// folder it goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub op: FileOp,
    /// Where the entry is.
    pub path: String,
}

/// What to do with an entry of the same name already where one goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClashChoice {
    /// The new entry takes the old one's place.
    Replace,
    /// The old entry stays, and the new one is left out.
    Skip,
    /// The new entry gets a name of its own beside the old one.
    KeepBoth,
}

/// Work on files as its page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileTask {
    Copy { path: String, into: String },
    Move { path: String, into: String },
    Delete { path: String },
}

/// Why an entry was not renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameRefused {
    /// Another entry in the folder has the name, as the folder compares
    /// names: shared storage takes names in any case.
    Taken,
    /// What went wrong, as the system says it.
    Failed(String),
}

/// How the terminal page's shell ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEnd {
    /// It exited with this code.
    Exited(i32),
    /// This signal stopped it.
    Signalled(i32),
    /// Neither, as the system says it.
    Other(String),
}

/// How work on files ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEnd {
    Done,
    /// Stopped as asked, with what was done by then left as it is.
    Stopped,
    Failed(String),
}

/// Work on files under way: what it is, how far it has come, and how it
/// ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileJob {
    pub task: FileTask,
    /// Bytes and entries, each as done and out of how many; none while the
    /// work still measures what it covers.
    pub progress: Option<((u64, u64), (u64, u64))>,
    pub outcome: Option<JobEnd>,
    /// Stopping was asked for and is under way.
    pub stopping: bool,
}

/// Giving the data partition's files their SELinux labels, as its page
/// shows it: how far it has come and how it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelabelJob {
    /// Entries looked at and how many there are, 0 while they are still
    /// counted, and how many took a new label.
    pub checked: u64,
    pub total: u64,
    pub changed: u64,
    pub outcome: Option<JobEnd>,
    /// Stopping was asked for and is under way.
    pub stopping: bool,
}

/// A partition the filesystem page lists, as the runtime read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsRow {
    /// Its name, such as data or metadata.
    pub name: String,
    pub mount_point: String,
    /// The filesystem it holds, where that can be read: the data
    /// partition's only once it is open.
    pub filesystem: Option<String>,
    /// Its size in bytes.
    pub size: u64,
    /// The filesystems the phone's own table lists for it, besides the one
    /// it holds, that the recovery can make.
    pub others: Vec<String>,
}

/// What the filesystem page does to the partition it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsAction {
    /// Check the whole of the filesystem and fix what is wrong.
    Repair,
    /// Make the partition afresh as this filesystem.
    Remake(String),
}

/// A choice on the page of one partition, as a control carries it: the
/// filesystem to make it afresh as goes by its place among the others it
/// may hold, a control's action being copied freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsChoice {
    Repair,
    Remake(usize),
}

/// Work on whole partitions one after another, as its page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartWork {
    pub kind: PartKind,
    /// The system asked for it before the recovery started: done, the phone
    /// leaves on its own.
    pub asked: bool,
    /// The partitions it goes through, once known, and where each stands.
    pub items: Vec<(String, ItemState)>,
    /// What the program that worked on the last of them said.
    pub output: Vec<String>,
    /// How it ended: done, or why it stopped before going through the
    /// partitions.
    pub outcome: Option<Result<(), PartStop>>,
}

/// What partition work does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartKind {
    /// Erase the partitions a wipe package lists.
    Disable,
    /// Undo the sharing of blocks in ext4 partitions.
    Unshare,
    Repair,
    /// Make the partition afresh as this filesystem.
    Remake(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemState {
    Waiting,
    Working,
    Ended(ItemEnd),
}

/// How a partition's work ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemEnd {
    Erased,
    Unshared,
    /// It holds this filesystem, none where blkid knows of none, and shares
    /// no blocks.
    NotShared(String),
    /// Nothing was wrong with the filesystem.
    Clean,
    Fixed,
    Made,
    /// The work went wrong: why, where what the program working on it said
    /// does not tell it.
    Failed(Option<String>),
}

/// Why partition work stopped before going through the partitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartStop {
    /// The wipe package is not one this phone may be erased by.
    Refused(Refusal),
    /// The program it takes is only on this partition of the installed
    /// system, which is not mounted.
    Unmounted(&'static str),
    Failed(String),
}

/// The settings' sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Brightness,
    /// How strongly the motor taps.
    Vibration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Vibrate,
    /// Offer internal storage to a computer over USB.
    FileTransfer,
    /// Compress a backup.
    Compress,
    /// Read a backup back once it is written.
    Verify,
    /// Run a zip's install script only if the zip was signed with one of
    /// the phone's update keys.
    Signature,
    /// Write Skyward back into the recovery partitions an install writes
    /// over.
    KeepRecovery,
    Wlan,
    /// Show the password typed rather than a dot for each character.
    ShowPassword,
}

/// What a backup can take, in the order the backup page lists it.
pub const BACKUP_PARTS: [Text; 4] = [
    ["启动分区", "啟動分割區", "Boot partitions"],
    ["系统分区", "系統分割區", "System partitions"],
    ["data 分区", "data 分割區", "data partition"],
    ["基带与 EFS", "基頻與 EFS", "Modem and EFS"],
];

/// A backup found in a place's Skyward/backups folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupEntry {
    pub place: Place,
    /// Its folder's name: when it was begun.
    pub name: String,
    /// What it holds, as its manifest records it, or why that cannot be
    /// read: a backup that did not finish has none.
    pub held: Result<Held, String>,
}

/// What a backup holds, and where it was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    /// When it was made, in Unix seconds.
    pub made: i64,
    /// The bytes its files take.
    pub size: u64,
    /// Which of [`BACKUP_PARTS`] it holds.
    pub parts: [bool; 4],
    /// The phone it was made on, by its codename, the system it ran, as
    /// its version reads, and the slot it was made from.
    pub device: String,
    pub system: Option<String>,
    pub slot: Option<Slot>,
}

/// Backing up or restoring, as its page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupJob {
    /// Restoring a backup, rather than making one.
    pub restore: bool,
    /// Where the backup goes, or comes from.
    pub place: Place,
    /// Each part is read back to check it: all once they are written, for a
    /// backup; all before any is written, for a restore.
    pub checks: bool,
    /// The parts it goes through, by name with their bytes; none while it
    /// still measures them.
    pub plan: Vec<(String, u64)>,
    pub at: Option<BackupAt>,
    pub outcome: Option<BackupEnd>,
    /// Stopping was asked for and is under way.
    pub stopping: bool,
}

/// The part under way, by its place in the plan, and how far it has come:
/// bytes done of how many in this pass over it, and whether this is the
/// pass that checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupAt {
    pub item: usize,
    pub done: u64,
    pub of: u64,
    pub checking: bool,
}

/// How backing up or restoring ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupEnd {
    /// A backup made, in the folder of this name.
    Saved(String),
    /// A backup restored, but for what it held in the folders of users gone
    /// since, which only the system makes: where those were.
    Restored { unmade: Vec<String> },
    /// Stopped as asked: a backup stopped leaves nothing of itself, and a
    /// restore stops only while nothing is written.
    Stopped,
    Refused(BackupRefusal),
    /// What went wrong, as the system says it.
    Failed(String),
}

/// Why backing up or restoring did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupRefusal {
    /// Too little room where the backup goes: the bytes it takes, and the
    /// bytes free.
    Room { needed: u64, free: u64 },
    /// An update is in flight, and the slot's partitions in super are its
    /// to change.
    Update(SnapshotState),
    /// The backup was made on another phone, by its codename.
    OtherPhone(String),
    /// This partition is not the size it was backed up at, as after an
    /// update.
    Resized(String),
    /// The data partition has been made afresh since the backup, as a
    /// factory reset makes it: the files backed up would not open in it.
    DataReset,
}

/// An entry of a folder, as the file manager lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub name: String,
    pub kind: FileKind,
    /// Bytes; for a link, the link's own.
    pub size: u64,
    /// Last changed, in Unix seconds.
    pub modified: i64,
    /// Type and permission bits, as `st_mode` holds them.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A folder, and how many entries it holds when it can be read.
    Folder { items: Option<usize> },
    File,
    /// A symbolic link: where it points, and whether that is a folder.
    Link { target: String, folder: bool },
    /// A device, socket or pipe.
    Special,
}

/// A folder as a list of its entries shows it: where it is, what is in it
/// as the runtime read it or why it could not, and when it was read, in
/// Unix seconds: dates in the list are told against that day.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub path: String,
    pub entries: Result<Vec<FileRow>, String>,
    pub listed: i64,
    /// The folders left for one inside them, outermost first, with how far
    /// each was scrolled.
    left: Vec<(String, f32)>,
}

impl Folder {
    /// The folder at `path`, not read yet.
    pub fn new(path: &str) -> Folder {
        Folder { path: path.into(), entries: Ok(Vec::new()), listed: 0, left: Vec::new() }
    }

    /// The entry at this place in the list.
    fn entry(&self, i: usize) -> &FileRow {
        &self.entries.as_ref().expect("an entry touched is one the list shows")[i]
    }

    /// Where the entry of this name in the folder is.
    fn inside(&self, name: &str) -> String {
        std::path::Path::new(&self.path).join(name).to_string_lossy().into_owned()
    }

    /// The folder this one is in; none for the root.
    fn up(&self) -> Option<String> {
        std::path::Path::new(&self.path).parent().map(|p| p.to_string_lossy().into_owned())
    }

    /// The folder this many levels below the root, by its place in the
    /// path shown over the list; none for this folder itself.
    fn ancestor(&self, depth: usize) -> Option<String> {
        let path = std::path::Path::new(&self.path);
        let names: Vec<_> = path.components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect();
        if depth >= names.len() {
            return None;
        }
        let mut up = if path.has_root() { std::path::PathBuf::from("/") } else { std::path::PathBuf::new() };
        up.extend(&names[..depth]);
        Some(up.to_string_lossy().into_owned())
    }

    /// Going into a folder inside this one: this one keeps how far it was
    /// scrolled, for coming back.
    fn leave(&mut self, scroll: f32) {
        self.left.push((self.path.clone(), scroll));
    }

    /// How far `path` is scrolled as it takes this folder's place: where it
    /// was left, for a folder gone back up to; its top for any other.
    fn scroll_for(&mut self, path: &str) -> f32 {
        let kept = self.left.iter().position(|(left, _)| left == path);
        let scroll = kept.map_or(0.0, |k| self.left[k].1);
        self.left.truncate(kept.unwrap_or(self.left.len()));
        scroll
    }
}

/// Whether a list in a place shows an entry: a folder, or a file the
/// installer takes. Hidden entries are left out, as the stock file manager
/// leaves them out.
fn installable(e: &FileRow) -> bool {
    if e.name.starts_with('.') {
        return false;
    }
    match e.kind {
        FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => true,
        FileKind::File | FileKind::Link { folder: false, .. } => installs(e),
        FileKind::Special => false,
    }
}

/// Whether the installer takes a file by its name: an update package, a
/// zip that installs itself, or a partition image.
pub(crate) fn installs(e: &FileRow) -> bool {
    let named = std::path::Path::new(&e.name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("img"));
    named && matches!(e.kind, FileKind::File | FileKind::Link { folder: false, .. })
}

/// The last part of `path`, as the installing page names the file there.
fn file_name(path: &str) -> Option<String> {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned())
}

/// A partition the mount page lists: its name, where it sits, and how it
/// shows now. The runtime fills these in; the page draws them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountRow {
    /// The partition's base name in super, such as `system`: the row's
    /// title, and what the runtime acts on.
    pub base: String,
    pub point: String,
    pub mounted: bool,
    /// The programs using it, which keep it from being unmounted.
    pub holders: Vec<String>,
}

/// A mount or unmount the runtime could not carry out. The mount page says
/// so until the next try, or until it is left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountFailure {
    /// Mounting, rather than unmounting.
    pub mounting: bool,
    pub base: String,
    /// The error and its causes.
    pub detail: String,
}

/// Which control answers a user's screen lock, by the kind of lock it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    Pattern,
    Digits,
    Letters,
}

/// Where the data partition stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Data {
    /// Not opened; the decrypt page opens it as it shows.
    #[default]
    Closed,
    /// Being opened as far as it opens with nothing a user knows, which is
    /// also when each user's screen lock is read.
    Opening,
    /// Open, with this user's own files waiting on their screen lock,
    /// answered with `keys`; `checking` while what they entered is checked.
    Locked { user: u32, keys: Keys, checking: bool },
    /// Open, every user's own files with it.
    Open,
    /// Not opened, and why.
    Failed(String),
    /// Erased whole: nothing opens until the system makes it afresh, as it
    /// next starts.
    Erased,
}

impl Data {
    /// Whether the owner's internal storage can be read: the partition is
    /// open, and so are the owner's own files, which open first.
    pub fn storage_open(&self) -> bool {
        match self {
            Data::Open => true,
            // The owner is user 0.
            Data::Locked { user, .. } => *user != 0,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Reboot,
    Wipe,
    Flash,
    Backup,
    Restore,
    /// Start next from the slot chosen on the slot page.
    Slot,
    /// Erase all user data, as the system offered when it could not start.
    FactoryReset,
    /// Delete the entry the file page shows.
    DeleteEntry,
    /// Delete the backup the restore page shows.
    DeleteBackup,
    /// Install the update that waits to be agreed to.
    Install,
    /// Install an update that erases all user data after it.
    InstallWipe,
    /// Run the install script of the zip that waits to be agreed to.
    RunScript,
    /// Merge the update that waits in the slot the phone runs from.
    Merge,
    /// Give the data partition's files the SELinux labels the installed
    /// system's rules give them.
    Relabel,
    /// Do to the partition the filesystem page shows what it chose.
    Filesystem,
}

/// What the wipe page erases: everything a factory reset erases, or any of
/// the parts after it on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wipe {
    All,
    /// Each app's own data, for every user.
    AppData,
    /// The files users share between apps.
    Media,
    /// Each app's caches, for every user.
    AppCaches,
    SystemCaches,
    /// What ART compiled ahead of time.
    Art,
    /// The cache partition, on phones that have one.
    Cache,
    Logs,
    /// The updates the system downloaded.
    Updates,
}

/// What the wipe page offers, in the order it lists them.
pub const WIPES: [Wipe; 9] =
    [Wipe::All, Wipe::AppData, Wipe::Media, Wipe::AppCaches, Wipe::SystemCaches, Wipe::Art, Wipe::Logs, Wipe::Updates, Wipe::Cache];

/// A part of the phone that erasing goes through, in the order it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The data partition, whole.
    Data,
    Cache,
    /// /metadata, which holds the key the data partition is opened with.
    Metadata,
    /// The parts of the data partition the wipe page names.
    AppData,
    Media,
    AppCaches,
    SystemCaches,
    Art,
    Logs,
    Updates,
}

/// Erasing as its page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Erase {
    /// Everything users keep on the phone, as a factory reset erases it.
    pub reset: bool,
    /// The system asked for it before the recovery started; once it is
    /// done, the phone leaves on its own.
    pub asked: bool,
    pub parts: Vec<Part>,
    /// How many of `parts` are erased; the one after them is under way.
    pub done: usize,
    /// How far the part under way has come, entries of how many, where it
    /// tells.
    pub progress: Option<(u64, u64)>,
    /// How erasing ended: done, or why it stopped.
    pub outcome: Option<Result<(), EraseStop>>,
}

/// Why erasing did not start, or stopped short.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EraseStop {
    /// An update has to be finished by the system first.
    Update(Hold),
    /// The system does not make an emptied data partition afresh as it
    /// starts, so it is not erased this way.
    NotRebuilt,
    /// The phone has no cache partition.
    NoCache,
    /// The data partition is erased already, and holds nothing until the
    /// system makes it afresh.
    Erased,
    /// The data partition did not open, so nothing in it was erased: why.
    Closed(String),
    /// These users' own files are locked, and what was chosen is partly in
    /// them: nothing was erased.
    Locked(Vec<u32>),
    /// Erasing this part went wrong: what the system said.
    Failed(Part, String),
}

/// What the system asked of the recovery before it started, as the pages
/// that answer it show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asked {
    /// Why, as the system put it.
    pub reason: Option<String>,
    /// Power off once the work is done, instead of starting the system.
    pub shutdown_after: bool,
    /// Work the recovery does not do, as it was asked.
    pub unhandled: Option<String>,
}

/// Which slots an image goes to, by the slot the phone runs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Current,
    Other,
    Both,
}

impl Pick {
    pub const ALL: [Pick; 3] = [Pick::Current, Pick::Other, Pick::Both];

    /// The slots this means on a phone running from `current`.
    pub fn slots(self, current: Slot) -> Vec<Slot> {
        match self {
            Pick::Current => vec![current],
            Pick::Other => vec![current.other()],
            Pick::Both => vec![Slot::A, Slot::B],
        }
    }
}

/// Where installing stands, as the installer last told.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Install {
    /// No installer runs.
    #[default]
    Idle,
    /// USB is being set up for sideload or rescue, or the package the
    /// system asked for is looked for.
    Starting,
    /// The package the system asked for is in the data partition, which is
    /// being opened.
    Opening,
    /// It is in the own files of this user, which wait on their screen
    /// lock.
    Locked(u32),
    Waiting,
    /// The package of `size` bytes is looked at; a zip's signature is
    /// checked, once the installer tells how far.
    Checking { size: u64, stage: Option<Stage> },
    /// An image waits to be told where it goes.
    Choosing(Choice),
    /// An update waits for the user to agree: the version it brings, and
    /// whether all user data is erased after it.
    Confirming { version: Option<String>, wipe: bool },
    /// `overwriting` says whether the update writes over the running
    /// system.
    Updating { version: Option<String>, stage: Option<Stage>, overwriting: bool },
    Flashing { partition: &'static str, stage: Option<Stage> },
    /// A zip with an install script waits for the user to agree to run it.
    ConfirmingScript,
    /// Its install script is made ready and then, once `running`, runs; how
    /// far it says it is, once it says.
    Scripting { stage: Option<Stage>, running: bool },
    /// `damage` is what a failure may have left broken.
    Finished { outcome: Outcome, damage: Option<Damage> },
    /// In rescue, all user data is being erased, as the computer asked.
    Wiping,
    /// In rescue, erasing all user data ended: done, or why it stopped.
    Wiped(Result<(), EraseStop>),
    /// The page was left and the installer is stopping, handing USB back
    /// if it had it.
    Ending,
}

/// What a failed install may have left broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    /// The update had begun writing over the running system.
    System,
    /// The image had begun going into this partition.
    Partition(&'static str),
    /// The install script had begun to run, and may have written to any
    /// partition.
    Script,
}

/// An image, and where it goes so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub kind: ImageKind,
    pub size: u64,
    /// The partitions it fits, the likeliest first.
    pub partitions: Vec<&'static str>,
    /// Which of them is chosen.
    pub partition: usize,
    pub pick: Pick,
}

/// Where the reboot page sends the device, in the order it lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    System,
    Recovery,
    Fastboot,
    Bootloader,
    Edl,
    PowerOff,
}

pub const TARGETS: [Target; 6] = [Target::System, Target::Recovery, Target::Fastboot, Target::Bootloader, Target::Edl, Target::PowerOff];

/// Where the fastboot page goes, in the order it lists them: the volume
/// keys move between them and the power key takes one.
pub const FASTBOOT_CHOICES: [(Text, Action); 4] = [
    (["重启到系统", "重新啟動到系統", "Reboot to system"], Action::RebootTo(Target::System)),
    (["进入恢复模式", "進入復原模式", "Enter recovery"], Action::EnterRecovery),
    (["重启到 Bootloader", "重新啟動到 Bootloader", "Reboot to bootloader"], Action::RebootTo(Target::Bootloader)),
    (["关机", "關機", "Power off"], Action::RebootTo(Target::PowerOff)),
];

/// How far a computer has the USB port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbLink {
    /// No cable, or nothing answering at its other end.
    Unplugged,
    /// Plugged in, with the computer still setting the port up.
    Connecting,
    /// Set up by the computer, which can send commands.
    Connected,
}

/// Fastboot as its page shows it: whether the phone's fastbootd runs and
/// how far a computer has the port, as the runtime last read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fastboot {
    pub running: bool,
    pub link: UsbLink,
}

/// How long the screen stays on with nothing touched, in milliseconds, in
/// the order the page lists them; `None` keeps it on.
pub const SCREEN_OFF: [Option<u64>; 7] = [Some(15_000), Some(30_000), Some(60_000), Some(120_000), Some(300_000), Some(600_000), None];

/// The time zones the clock and the times on the pages can keep, in the
/// order their page lists them: the name, the cities it covers, and the
/// zone's name in the tz database.
pub const TIME_ZONES: [(Text, Text, &str); 9] = [
    (["中国标准时间", "中國標準時間", "China Standard Time"], ["北京、上海", "北京、上海", "Beijing, Shanghai"], "Asia/Shanghai"),
    (["香港时间", "香港時間", "Hong Kong Time"], ["香港", "香港", "Hong Kong"], "Asia/Hong_Kong"),
    (["台北时间", "台北時間", "Taipei Time"], ["台北", "台北", "Taipei"], "Asia/Taipei"),
    (["日本标准时间", "日本標準時間", "Japan Standard Time"], ["东京", "東京", "Tokyo"], "Asia/Tokyo"),
    (["协调世界时", "世界標準時間", "Coordinated Universal Time"], ["UTC", "UTC", "UTC"], "UTC"),
    (["英国时间", "英國時間", "British Time"], ["伦敦", "倫敦", "London"], "Europe/London"),
    (["中欧时间", "中歐時間", "Central European Time"], ["柏林、巴黎", "柏林、巴黎", "Berlin, Paris"], "Europe/Berlin"),
    (["美国东部时间", "美國東部時間", "Eastern Time"], ["纽约", "紐約", "New York"], "America/New_York"),
    (["美国太平洋时间", "美國太平洋時間", "Pacific Time"], ["洛杉矶", "洛杉磯", "Los Angeles"], "America/Los_Angeles"),
];

/// What a control does when touched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Open(Screen),
    /// Leave the decrypt page for the page it was opened from; the page
    /// the recovery started on, for the home page without keeping it
    /// behind.
    Skip,
    /// Try opening the data partition again, after it did not open.
    OpenData,
    Back,
    Theme { dark: bool },
    Toggle(Switch),
    RebootTarget(usize),
    /// Choose or leave out what the wipe page erases.
    WipeItem(Wipe),
    /// A slider's track: touching or dragging it sets the slider's level.
    Slider(Slider),
    /// Mount or unmount the mount page's partition at this index.
    MountToggle(usize),
    /// A swipe-to-confirm track; dragging the knob to the far end confirms.
    Confirm(Confirm),
    /// The pattern grid on the unlock page.
    Pattern,
    /// Start receiving a package over USB.
    Sideload,
    /// Send the image to one of the partitions it fits.
    FlashPartition(usize),
    /// Send the image to these slots.
    FlashSlots(Pick),
    /// Leave the recovery for this target at once.
    RebootTo(Target),
    /// Stop fastboot and bring the recovery's own pages up, without
    /// restarting.
    EnterRecovery,
    /// Leave what the system asked of the recovery for the recovery's own
    /// pages.
    Dismiss,
    /// Keep the screen on for this choice in [`SCREEN_OFF`].
    ScreenOff(usize),
    /// Keep the time zone at this place in [`TIME_ZONES`].
    TimeZone(usize),
    /// Speak this language.
    Language(Lang),
    /// Tick or untick a part of the backup, by its place in
    /// [`BACKUP_PARTS`].
    BackupPart(usize),
    /// Send the backup to this place.
    BackupTo(Place),
    /// Open a backup found, by its place in the list, to restore it.
    OpenBackup(usize),
    /// Tick or untick a part of the backup being restored.
    RestorePart(usize),
    /// Stop backing up, or checking a backup before it is restored.
    StopBackup,
    /// Tick or untick a log to export: the recovery's, the kernel's.
    LogPart(usize),
    /// Save the logs to /tmp, or to internal storage.
    LogsTo(usize),
    /// Save the logs ticked where they are to go.
    ExportLogs,
    /// Pick this slot to start next; a swipe then switches to it.
    PickSlot(Slot),
    /// A key of the on-screen keyboard.
    Type(Cap),
    /// A key of the row above the keyboard, the terminal's or the editor's.
    TermKey(TermKey),
    /// Empty the search field.
    ClearSearch,
    /// Leave the search, by the round button beside its field.
    CloseSearch,
    /// Open the file manager's entry at this place in the folder: a folder
    /// to look in, a text file to edit, or what can be done with any other.
    OpenEntry(usize),
    /// Open the terminal in the folder the file manager shows.
    TerminalHere,
    /// Open the entry at this place in a place's folder: a folder to look
    /// in, or the file to install.
    ChooseEntry(usize),
    /// Go up to the folder this many levels below the top of the list
    /// shown, by its place in the path shown over the list.
    OpenAncestor(usize),
    /// Pick the entry the file page shows to copy or move, and look for
    /// where it goes.
    PickEntry(FileOp),
    /// Copy or move the entry picked into the folder shown.
    Paste,
    /// Put the entry picked down again.
    Unpick,
    /// Give the entry the file page shows a new name.
    RenameEntry,
    /// Open the file the file page shows to edit it.
    EditEntry,
    /// Install the file the file page shows.
    InstallEntry,
    /// Write the file the editor page shows as it now stands.
    SaveText,
    /// Open the page of the permissions of the entry the file page shows.
    OpenPermissions,
    /// Turn this permission bit of the entry the file page shows on or off.
    ModeBit(u32),
    /// Ask to delete the entry the file page shows, or the backup the
    /// restore page shows; a swipe then confirms.
    AskDelete,
    /// How to go on when a name is taken where an entry goes.
    Resolve(ClashChoice),
    /// Stop giving the data partition's files their labels.
    StopRelabel,
    /// Open the page of the filesystem page's partition at this place.
    OpenFilesystem(usize),
    /// Choose what to do to the partition that page shows.
    ChooseFs(FsChoice),
    /// Stop copying, moving or deleting.
    StopJob,
    /// Join the network at this place in the list of those in range, or
    /// open the page of the one joined.
    JoinNetwork(usize),
    /// Open the details of the network at this place in the list of those
    /// in range.
    NetworkDetails(usize),
    /// The network chosen: its password again once refused, its details
    /// otherwise.
    OpenJoined,
    /// Leave the network joined, and forget it.
    LeaveNetwork,
    /// Look for the networks in range again.
    Rescan,
    /// Open the page that adds a network by hand.
    AddNetwork,
    /// Type into this field of the page that adds a network.
    TypeInto(Field),
    /// Open the protections a network added by hand can have.
    ChooseSecurity,
    /// Take this protection, and close the list.
    PickSecurity(Security),
    /// Close the list open over the page, choosing nothing.
    CloseChoices,
    /// Ask the update server for the newest system.
    CheckUpdate,
    /// Download the newest system's full package, or carry on downloading
    /// it.
    Download,
    StopDownload,
    /// Install the full package downloaded.
    InstallDownloaded,
    /// A control whose work is not wired up yet; touching it says so.
    Unavailable(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}

/// A rendered page in page coordinates. The status-bar band at its top is
/// left empty, and the `<`, the titles and the bar are laid over the page
/// as it scrolls.
pub struct Frame {
    pub canvas: Canvas,
    pub hits: Vec<Hit>,
    pub title: &'static str,
    pub back: bool,
    /// The page opens with its title in large type, scrolling away with it.
    pub header: bool,
    /// Where the page's content starts; once it slides under the top of
    /// the screen, the bar comes up over it.
    pub content_top: f32,
    /// The page sits on the About page's moving field.
    pub hero: Option<HeroArt>,
    /// Centre and span of the pattern grid, on the unlock page.
    pub pattern: Option<(f32, f32, f32)>,
    /// The colour of the page itself, past the canvas and under the bar.
    pub bg: Rgba,
    /// A list too long to draw whole: its rows are drawn as they scroll
    /// into view, below the canvas's content and past its end.
    pub rows: Option<Rows>,
    /// A strip that scrolls with the page until it reaches the bar, then
    /// stays under it.
    pub pinned: Option<Pinned>,
    /// A search bar under the large title that folds away as the page
    /// scrolls up.
    pub fold: Option<Fold>,
    /// A terminal's screen, or the editor's card, and the height of its
    /// rows: a drag on it scrolls through what it holds, rather than moving
    /// the page.
    pub scrollback: Option<(Rect, f32)>,
    /// A strip held at the bottom of the screen over the page; the page
    /// scrolls far enough for nothing to stay under it.
    pub dock: Option<Dock>,
    /// A text field's blinking cursor, in page coordinates.
    pub caret: Option<widgets::Caret>,
    /// The round button at the right end of the bar: what it shows, and
    /// what touching it does.
    pub button: Option<(widgets::Glyph, Action)>,
}

/// A strip held at the bottom of the screen over the page, such as the
/// file manager's choice of where a picked entry goes.
pub struct Dock {
    pub canvas: Canvas,
    /// In the strip's own coordinates.
    pub hits: Vec<Hit>,
}

/// The rows of a list drawn as they come into view, the way a list view
/// binds its rows: where each lies on the page and what touching one does,
/// for rows that answer touch.
pub struct Rows {
    /// Page y of each row's top, then of the last row's bottom; whole
    /// pixels, so the rows meet without seams.
    pub edges: Vec<f32>,
    pub action: Option<fn(usize) -> Action>,
}

impl Rows {
    pub fn count(&self) -> usize {
        self.edges.len() - 1
    }

    pub fn bottom(&self) -> f32 {
        *self.edges.last().expect("a list has its top edge at least")
    }

    /// The row under page height `y`.
    pub fn at(&self, y: f32) -> Option<usize> {
        if y < self.edges[0] || y >= self.bottom() {
            return None;
        }
        Some(self.edges.partition_point(|&e| e <= y) - 1)
    }

    /// The rows reaching into the band from `y0` to `y1`.
    pub fn within(&self, y0: f32, y1: f32) -> std::ops::Range<usize> {
        let first = self.edges.partition_point(|&e| e <= y0).saturating_sub(1);
        let end = self.edges.partition_point(|&e| e < y1).min(self.count());
        first.min(end)..end
    }

    /// Where row `i` lies on the page.
    pub fn rect(&self, i: usize, w: f32) -> Rect {
        Rect::new(0.0, self.edges[i], w, self.edges[i + 1] - self.edges[i])
    }
}

/// A strip drawn once and laid over the page at `top`, or just under the
/// bar once the page has scrolled it that far; its controls answer where
/// it shows.
pub struct Pinned {
    pub canvas: Canvas,
    pub top: f32,
    /// In the strip's own coordinates.
    pub hits: Vec<Hit>,
}

/// A search bar under a page's large title folding away as the page scrolls
/// up, as the stock settings home's does: its top stays put and its foot
/// rises with the page, its icon and words fading out first and then the
/// bar, until nothing of it is left. The content under it then closes the
/// gap it leaves, and only after that does the large title go up with the
/// page. A page let go with its bar part folded settles folded or unfolded,
/// whichever is nearer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fold {
    /// Where the bar stands unfolded, in page coordinates.
    pub bar: Rect,
    /// What the bar asks for, in grey.
    pub hint: &'static str,
    /// How far the page scrolls with its large title held still: the bar
    /// and the gap under it.
    pub span: f32,
}

impl Fold {
    /// What shows of the bar, on screen, with the page `offset` pixels
    /// down: all of it, pulled down with the page past its top, or as much
    /// as is left of it folding; none once it has folded away.
    pub fn shown(&self, offset: f32) -> Option<Rect> {
        let h = self.bar.h - offset.max(0.0);
        (h > 0.0).then(|| Rect { y: self.bar.y - offset.min(0.0), h, ..self.bar })
    }

    /// How strongly the bar, and its icon and words, show with the page
    /// `offset` pixels down: the words fade out over the first stretch of
    /// the fold, and the bar over the next.
    pub fn levels(&self, t: &Theme, offset: f32) -> (f32, f32) {
        let along = offset.max(0.0) / t.m.dp(FOLD_FADE_DP);
        ((2.0 - along).clamp(0.0, 1.0), (1.0 - along).clamp(0.0, 1.0))
    }

    /// Draw what shows of the bar with the page `offset` pixels down, its
    /// icon and words laid from `words`, which [`widgets::search_words`]
    /// makes.
    pub fn draw(&self, out: &mut Canvas, t: &Theme, words: &Canvas, offset: f32) {
        let Some(r) = self.shown(offset) else { return };
        let (bar, level) = self.levels(t, offset);
        widgets::search_bar(out, t, r, bar);
        // The icon and words stay centred in what is left of the bar.
        let sink = r.y + r.h / 2.0 - (self.bar.y + self.bar.h / 2.0);
        out.lay(words, self.bar.x.floor() as i32, (self.bar.y.floor() + sink).round() as i32, level);
    }

    /// How far the large title has gone up with the page `offset` pixels
    /// down: nowhere while the bar folds and its gap closes, then as far
    /// as the page has gone since; down with a page pulled past its top.
    pub fn lift(&self, offset: f32) -> f32 {
        if offset < 0.0 {
            offset
        } else {
            (offset - self.span).max(0.0)
        }
    }

    /// Where a page let go at `scroll` comes to rest: with the bar part
    /// folded, folded or unfolded, whichever is nearer; anywhere else,
    /// where it is.
    pub fn settle(&self, scroll: f32) -> f32 {
        match scroll {
            s if s <= 0.0 || s >= self.span => s,
            s if s < self.span * FOLD_SETTLE => 0.0,
            _ => self.span,
        }
    }
}

/// What the About page lays over its field: the wordmark, the version and
/// who made the recovery, which stay put while the cards scroll, and the
/// cards' coverage row by row.
pub struct HeroArt {
    pub wordmark: Sprite,
    pub version: Sprite,
    pub author: Sprite,
    pub runs: Vec<Vec<Run>>,
}

impl Frame {
    /// A page of cards on the grey page colour that opens with its title in
    /// large type and goes back with `<`.
    pub fn new(t: &Theme, canvas: Canvas, hits: Vec<Hit>, title: &'static str, content_top: f32) -> Frame {
        Frame {
            canvas,
            hits,
            title,
            back: true,
            header: true,
            content_top,
            hero: None,
            pattern: None,
            bg: t.pal.bg,
            rows: None,
            pinned: None,
            fold: None,
            scrollback: None,
            dock: None,
            caret: None,
            button: None,
        }
    }

    /// Where the control at `r` on the page shows, on screen, with the page
    /// `offset` pixels down: the folding search bar as much as is left of
    /// it, and none once folded away; any other with the page.
    pub fn shown(&self, r: Rect, offset: f32) -> Option<Rect> {
        match &self.fold {
            Some(fold) if fold.bar == r => fold.shown(offset),
            _ => Some(Rect { y: r.y - offset, ..r }),
        }
    }

    /// How far the large title has gone up with the page `offset` pixels
    /// down: as far as the page, but for a search bar folding under it.
    pub fn title_lift(&self, offset: f32) -> f32 {
        self.fold.as_ref().map_or(offset, |f| f.lift(offset))
    }

    /// Where a page let go at `scroll` comes to rest: see [`Fold::settle`].
    pub fn settle(&self, scroll: f32) -> f32 {
        self.fold.as_ref().map_or(scroll, |f| f.settle(scroll))
    }

    /// The topmost control under a point in page coordinates.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some(h) = self.hits.iter().rev().find(|h| h.rect.contains(x, y)) {
            return Some(*h);
        }
        let rows = self.rows.as_ref()?;
        let action = rows.action?;
        let i = rows.at(y)?;
        Some(Hit { rect: rows.rect(i, self.canvas.w as f32), action: action(i) })
    }

    /// How tall the page is: its canvas, or its rows and the margin after
    /// them if they reach further.
    pub fn height(&self, t: &Theme) -> f32 {
        let h = self.canvas.h as f32;
        let h = self.rows.as_ref().map_or(h, |r| h.max((r.bottom() + t.m.margin * 2.0).ceil()));
        h + self.dock.as_ref().map_or(0.0, |d| d.canvas.h as f32)
    }
}

/// Choices the pages show and the user changes, and where installing
/// stands.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    /// The language the pages speak.
    pub lang: Lang,
    pub dark: bool,
    pub brightness: f32,
    /// How long the screen stays on with nothing touched, by its place in
    /// [`SCREEN_OFF`].
    pub screen_off: usize,
    /// The time zone the clock and the times on the pages keep, by its
    /// place in [`TIME_ZONES`].
    pub time_zone: usize,
    pub vibrate: bool,
    /// How strongly the motor taps, from 0 at the slider's left end to 1
    /// at its right.
    pub vibration: f32,
    /// A computer connected over USB is offered internal storage.
    pub file_transfer: bool,
    pub reboot: usize,
    /// What the wipe page has chosen, by place in [`WIPES`].
    pub wipe: [bool; WIPES.len()],
    /// The phone has a cache partition, which the wipe page then offers.
    pub cache_partition: bool,
    /// How many folders of its own the phone keeps logs in on the data
    /// partition, which erasing the logs takes in.
    pub phone_logs: usize,
    pub pattern: Vec<usize>,
    /// How far the swipe-to-confirm knob is dragged, 0 to 1.
    pub confirm: f32,
    pub install: Install,
    /// Where the package being installed comes from.
    pub source: Source,
    /// The name of the file being installed, when it is one on the phone.
    pub package_name: Option<String>,
    /// What the install script last run showed, its latest lines.
    pub script_output: Vec<String>,
    /// A zip's install script runs only if the zip was signed with one of
    /// the phone's update keys.
    pub check_signature: bool,
    /// The phone keeps its recovery in a partition of each slot, which the
    /// install page then offers to keep Skyward in.
    pub recovery_partitions: bool,
    /// Skyward goes back into the recovery partitions an install writes
    /// over.
    pub keep_recovery: bool,
    /// How Skyward went back into the recovery partitions the install
    /// wrote over, where it did.
    pub reflash: Option<Reflash>,
    /// The mount page's partitions, as the runtime last read them, or why
    /// it could not.
    pub mounts: Result<Vec<MountRow>, String>,
    pub mount_failure: Option<MountFailure>,
    /// Which of [`BACKUP_PARTS`] a backup takes.
    pub backup: [bool; 4],
    /// Where a backup goes.
    pub backup_to: Place,
    pub compress: bool,
    pub verify: bool,
    /// The backups found, newest first, those that cannot be read last.
    pub backups: Vec<BackupEntry>,
    /// The backup the restore page shows, as it was found, and which of its
    /// parts to restore.
    pub restore: Option<(BackupEntry, [bool; 4])>,
    /// Backing up or restoring under way, or how it last ended.
    pub backup_job: Option<BackupJob>,
    /// Which logs to export: the recovery's, the kernel's.
    pub logs: [bool; 2],
    /// Where the logs go: 0 for /tmp, 1 for internal storage.
    pub logs_to: usize,
    /// What the phone's boot control holds of the slots, as the runtime
    /// last read it for the slot or reboot page, or why it could not.
    pub boot_slots: Option<Result<BootSlots, String>>,
    /// The slot picked on the slot page to start next; with none picked,
    /// the page shows the one the boot control starts next.
    pub next_slot: Option<Slot>,
    /// Where the data partition stands, and what is typed at the screen
    /// lock its users' own files wait on.
    pub data: Data,
    pub code: String,
    /// What is typed in the search field.
    pub search: String,
    /// The on-screen keyboard's layer.
    pub layer: Layer,
    /// The key of the on-screen keyboard, or the choice of a list open over
    /// the page, a finger holds down.
    pub held: Option<Action>,
    /// The folder the file manager shows.
    pub files: Folder,
    /// The folder of internal storage a file to install is chosen in, its
    /// path from the top of the storage.
    pub storage: Folder,
    /// The USB drive plugged in, as the runtime last found it, and the
    /// folder of it a file to install is chosen in, its path from the top
    /// of the drive.
    pub usb_drive: Option<UsbDrive>,
    pub drive: Folder,
    /// The entry the file page describes.
    pub file: Option<FileRow>,
    pub fastboot: Fastboot,
    pub erase: Erase,
    pub asked: Asked,
    /// The entry picked to copy or move, while a folder for it is looked
    /// for.
    pub clip: Option<Clip>,
    /// What is typed as an entry's new name.
    pub new_name: String,
    /// The text file the editor page shows, as typed so far.
    pub editor: Option<Editor>,
    /// Deleting the entry the file page shows, or the backup the restore
    /// page shows, was asked for; a swipe confirms it.
    pub asking_delete: bool,
    /// A copy or move waiting on what to do with the names taken where it
    /// goes, and those names.
    pub clash: Option<(Clip, String, Vec<String>)>,
    /// Work on files under way, or how it last ended.
    pub job: Option<FileJob>,
    /// Giving the data partition's files their SELinux labels under way, or
    /// how it last ended.
    pub relabel: Option<RelabelJob>,
    /// The filesystem page's partitions, as the runtime last read them, or
    /// why it could not; none until it has.
    pub filesystems: Option<Result<Vec<FsRow>, String>>,
    /// Which of them the page for one shows, and what it is to do to it:
    /// a repair, which leaves what it holds as it is, until another is
    /// chosen.
    pub fs_chosen: usize,
    pub fs_action: FsAction,
    /// Work on whole partitions under way, or how it last ended.
    pub part_work: Option<PartWork>,
    /// Where an update the system installed stands, as the runtime last
    /// read it.
    pub update: Standing,
    /// The WLAN, as the runtime last told it; none on a phone whose WLAN the
    /// recovery does not bring up.
    pub wlan: Option<Wlan>,
    /// The network the password page is for, and what is typed there.
    pub joining: Option<Network>,
    /// The network the details page is of, as it was found.
    pub details: Option<Network>,
    /// The network the page that adds one is adding.
    pub adding: Option<Adding>,
    /// The password typed on the password page or the page that adds a
    /// network, and whether it shows.
    pub password: String,
    pub show_password: bool,
    /// The phone maker's update server is known, and the update page asks
    /// it.
    pub update_server: bool,
    pub check: Check,
    pub have: Have,
    /// Downloading the newest system's full package, or how that last
    /// ended.
    pub download: Option<DownloadJob>,
    /// How fast data went over the network joined in the last second.
    pub traffic: Traffic,
}

impl Default for State {
    fn default() -> Self {
        State {
            lang: Lang::Hans,
            dark: false,
            brightness: SLIDER_START,
            // Two minutes.
            screen_off: 3,
            time_zone: 0,
            vibrate: true,
            vibration: SLIDER_START,
            file_transfer: true,
            reboot: 0,
            wipe: [false; WIPES.len()],
            cache_partition: false,
            phone_logs: 0,
            pattern: Vec::new(),
            confirm: 0.0,
            install: Install::Idle,
            source: Source::Computer,
            package_name: None,
            script_output: Vec::new(),
            check_signature: false,
            recovery_partitions: false,
            keep_recovery: true,
            reflash: None,
            mounts: Ok(Vec::new()),
            mount_failure: None,
            backup: [true, true, false, true],
            backup_to: Place::Storage,
            compress: false,
            verify: true,
            backups: Vec::new(),
            restore: None,
            backup_job: None,
            logs: [true, true],
            logs_to: 0,
            boot_slots: None,
            next_slot: None,
            data: Data::Closed,
            code: String::new(),
            search: String::new(),
            layer: Layer::Letters,
            held: None,
            files: Folder::new("/"),
            storage: Folder::new(""),
            usb_drive: None,
            drive: Folder::new(""),
            file: None,
            fastboot: Fastboot { running: false, link: UsbLink::Unplugged },
            erase: Erase::default(),
            asked: Asked::default(),
            clip: None,
            new_name: String::new(),
            editor: None,
            asking_delete: false,
            clash: None,
            job: None,
            relabel: None,
            filesystems: None,
            fs_chosen: 0,
            fs_action: FsAction::Repair,
            part_work: None,
            update: Standing::None,
            wlan: None,
            joining: None,
            details: None,
            adding: None,
            password: String::new(),
            show_password: false,
            update_server: false,
            check: Check::Idle,
            have: Have::Nothing,
            download: None,
            traffic: Traffic::default(),
        }
    }
}

impl State {
    /// The key of the on-screen keyboard a finger holds down.
    pub fn key_held(&self) -> Option<Cap> {
        match self.held {
            Some(Action::Type(cap)) => Some(cap),
            _ => None,
        }
    }

    /// The folder the list on page `screen` shows: the file manager's, or
    /// a place's to choose a file to install in.
    pub fn folder(&self, screen: Screen) -> &Folder {
        match screen {
            Screen::Files => &self.files,
            Screen::Choose(Place::Storage) => &self.storage,
            Screen::Choose(Place::UsbDrive) => &self.drive,
            other => unreachable!("{other:?} lists no folder"),
        }
    }

    fn folder_mut(&mut self, screen: Screen) -> &mut Folder {
        match screen {
            Screen::Files => &mut self.files,
            Screen::Choose(Place::Storage) => &mut self.storage,
            Screen::Choose(Place::UsbDrive) => &mut self.drive,
            other => unreachable!("{other:?} lists no folder"),
        }
    }

    /// Whether the files of `place` can be read: internal storage once it
    /// is open, a USB drive once the runtime shows it.
    pub fn place_open(&self, place: Place) -> bool {
        match place {
            Place::Storage => self.data.storage_open(),
            Place::UsbDrive => self.usb_drive == Some(UsbDrive::Shown),
        }
    }

    /// Whether a network is joined and has given an address.
    pub fn online(&self) -> bool {
        matches!(&self.wlan, Some(Wlan { joined: Some(j), .. }) if matches!(j.stage, Joining::Connected(_)))
    }

    /// The newest system the update server told of.
    pub fn newest(&self) -> Option<&Release> {
        match &self.check {
            Check::Answered(Some(r)) => Some(r),
            _ => None,
        }
    }

    /// Whether the full package is being downloaded.
    pub fn downloading(&self) -> bool {
        self.download.as_ref().is_some_and(|d| d.outcome.is_none())
    }
}

/// One finger, in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Touch {
    Down(f32, f32),
    Move(f32, f32),
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Power,
    VolumeUp,
    VolumeDown,
    Back,
}

/// Where a screenshot went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    /// Into the gallery's screenshots, in the owner's shared storage.
    Album,
    /// Into the recovery's own memory, to go into the gallery's once the
    /// owner's storage opens.
    Held,
}

/// Work the runtime carries out on the UI's behalf.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Reboot(Target),
    /// A short tap of the vibration motor.
    Haptic,
    /// Save what the panel shows as a picture: into the gallery's
    /// screenshots in the owner's shared storage where `storage` says it is
    /// open, and otherwise held until it is, telling how it went with
    /// [`App::screenshot_saved`].
    Screenshot { storage: bool },
    /// Offer internal storage to a computer over USB, or stop.
    FileTransfer(bool),
    /// Start receiving a package over USB.
    Sideload,
    /// Start installing the file at this path from the top of a place,
    /// telling how it goes as [`Effect::Sideload`] does.
    InstallFile(Place, String),
    /// Start installing the file at this full path, opened in the file
    /// manager, telling how it goes as [`Effect::Sideload`] does.
    InstallPath(String),
    /// Write the image the installer holds to `partition` in the slots
    /// `pick` means.
    Flash { partition: &'static str, pick: Pick },
    /// Stop waiting for the computer, for where an image goes, or for an
    /// update to be agreed to.
    Cancel,
    /// Read the slot's partitions for the mount page, which has just opened.
    ReadMounts,
    /// Read what the boot control holds of the slots, telling it with
    /// [`App::set_boot_slots`].
    ReadSlots,
    /// Have the bootloader start this slot next, telling how it went with
    /// [`App::slot_switched`].
    SetSlot(Slot),
    /// Mount the slot's partition with this base name, such as `system`.
    Mount(String),
    /// Unmount the slot's partition with this base name.
    Unmount(String),
    /// List the folder at this path for the file manager.
    ReadDir(String),
    /// List the folder at this path from the top of a place, to choose a
    /// file to install in, telling what it holds with [`App::set_place`].
    ReadPlace(Place, String),
    /// Open the data partition as far as it opens with nothing a user
    /// knows, telling what waits with [`App::data_waiting`] or why it did
    /// not open with [`App::data_failed`].
    OpenData,
    /// Open the own files of `user` with what they entered at their screen
    /// lock, telling what still waits with [`App::data_waiting`] or why it
    /// did not open with [`App::unlock_failed`].
    Unlock { user: u32, code: String },
    /// Hand the USB port back from fastboot to adb, bring up what the
    /// recovery runs from the slot's partitions, then show its pages with
    /// [`App::enter_recovery`].
    EnterRecovery,
    /// Erase all user data, as a factory reset does, telling how it goes
    /// with [`App::erase_at`] and [`App::erase_ended`].
    FactoryReset,
    /// Erase what the wipe page has chosen, [`Wipe::All`] as a factory
    /// reset does, telling how it goes with [`App::erase_at`],
    /// [`App::erase_progress`] and [`App::erase_ended`].
    Wipe(Vec<Wipe>),
    /// Install the update held for the user to agree to.
    Install,
    /// Forget what the system asked of the recovery, so that the next start
    /// goes where its reboot reason says.
    Dismiss,
    /// Start a shell for the terminal page in this folder, on a screen of
    /// the size [`App::term_size`] gives, telling of it with
    /// [`App::term_started`].
    StartShell(String),
    /// Have the terminal page's shell go to this folder if it waits for a
    /// command; one running a command is left to it, and [`App::report`]
    /// says so.
    ShellTo(String),
    /// Send these bytes to the shell, as typed.
    TermInput(Vec<u8>),
    /// Copy or move an entry into a folder; with no choice made yet, a name
    /// taken there is told with [`App::file_clash`] instead of going on.
    Paste { op: FileOp, path: String, into: String, clash: Option<ClashChoice> },
    /// Give an entry a new name; how it went comes back with
    /// [`App::renamed`].
    Rename { path: String, name: String },
    /// Give an entry these permission bits; how it went comes back with
    /// [`App::mode_set`].
    SetMode { path: String, mode: u32 },
    /// Read the file at this path to edit it, as much as
    /// [`editor::MAX_BYTES`] and one byte more, telling what it holds with
    /// [`App::text_opened`].
    OpenText(String),
    /// Write `text` as the contents of the file at `path`, which keeps what
    /// it has beside them; how it went comes back with
    /// [`App::text_saved`].
    SaveText { path: String, text: String },
    /// Delete an entry, folders whole.
    Delete(String),
    /// Stop the work on files under way.
    StopJob,
    /// Give the data partition's files the SELinux labels the installed
    /// system's rules give them, telling how it goes with
    /// [`App::relabel_started`], [`App::relabel_progress`] and
    /// [`App::relabel_ended`].
    Relabel,
    /// Stop giving the data partition's files their labels.
    StopRelabel,
    /// Read the partitions for the filesystem page, which has just opened,
    /// telling them with [`App::set_filesystems`].
    ReadFilesystems,
    /// Do `action` to the filesystem page's partition by its name, telling
    /// how it goes with [`App::part_work_started`], [`App::part_listed`],
    /// [`App::part_at`], [`App::part_ended`] and [`App::part_work_ended`];
    /// making the data partition afresh as another filesystem is a factory
    /// reset, which tells as erasing tells.
    Filesystem { name: String, action: FsAction },
    /// Save the logs chosen, the recovery's own and the kernel's, into a
    /// folder of their own in internal storage or the temporary folder,
    /// saying where with [`App::report`].
    ExportLogs { recovery: bool, kernel: bool, storage: bool },
    /// Back up the parts ticked, by their place in [`BACKUP_PARTS`], into
    /// a folder of their own in `place`, telling how it goes with
    /// [`App::backup_started`], [`App::backup_plan`], [`App::backup_at`]
    /// and [`App::backup_ended`].
    Backup { parts: [bool; 4], place: Place, compress: bool, verify: bool },
    /// Restore the parts ticked of the backup in the folder `name` of
    /// `place`, telling how it goes as a backup does.
    Restore { place: Place, name: String, parts: [bool; 4] },
    /// Look for backups in the places that can be read, telling what is
    /// found with [`App::set_backups`].
    FindBackups { storage: bool, usb: bool },
    /// Delete the backup in the folder `name` of `place`, telling how it
    /// went with [`App::backup_deleted`].
    DeleteBackup { place: Place, name: String },
    /// Stop the backup under way, or the check before a restore.
    StopBackup,
    /// Read where the update stands, telling it with [`App::set_update`].
    ReadUpdate,
    /// Turn the WLAN on or off; how it stands comes back with
    /// [`App::set_wlan`].
    Wlan(bool),
    ScanWlan,
    JoinWlan { ssid: Vec<u8>, name: String, security: Security, password: Secret },
    LeaveWlan,
    /// Ask the update server for the newest system, telling what it
    /// answered with [`App::update_checked`].
    CheckUpdate,
    /// Download `release`'s full package into [`DOWNLOADS`], telling how it
    /// goes with [`App::download_progress`] and [`App::download_ended`].
    Download(Release),
    StopDownload,
    /// Merge the update that waits in the slot the phone runs from,
    /// telling how it goes with [`App::set_update`].
    Merge,
}

enum Drag {
    Idle,
    /// Finger down and not yet past the touch slop. `edge` is set when the
    /// press began at a screen edge (true for the left one).
    /// `at` is when it came down: held long enough over an entry of the file
    /// manager, it opens what can be done with that entry.
    Press { x: f32, y: f32, action: Option<Action>, edge: Option<bool>, at: u64 },
    /// A press held long enough to have done its work; nothing it does
    /// counts until it lifts.
    Held,
    Scroll { last: f32 },
    /// Swiping in from a screen edge, the finger now `x` across; the cue
    /// keeps to height `cy`, set by where the swipe began. Past the
    /// threshold, lifting goes back.
    Edge { start: f32, from_left: bool, armed: bool, x: f32, cy: f32 },
    Slider(Slider, Rect),
    Confirm { which: Confirm, track: Rect, grab: f32 },
    /// Scrolling a terminal's screen back through what the shell wrote, or
    /// the editor's text, `carry` being the part of a row not yet scrolled.
    History { last: f32, carry: f32, pitch: f32 },
    Pattern,
    /// The finger that lit the dark panel; nothing it does counts until it
    /// lifts.
    Waking,
}

/// Where the keys are on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    /// Nowhere yet: the first control in view lights once the keys work
    /// the page.
    Unset,
    /// On no control, none showing where the keys scrolled the page.
    Off,
    /// On this control, as it was when the page was last drawn.
    On(Spot),
}

/// A button held down, and what holding it does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    /// A volume key, moving the light down the page or up it, and again at
    /// `next`.
    Volume { down: bool, next: u64 },
    /// The power key on the control lit, from `at`, which it works as it
    /// comes up; held long enough over an entry of the file manager, it
    /// opens what can be done with the entry, as a long press does.
    Power { spot: Spot, at: u64 },
    /// The power key on a swipe-to-confirm control, whose knob slides along
    /// its track at `track` from `at` while the key is held; let go once
    /// it has slid far enough, the control confirms.
    Slide { which: Confirm, track: Rect, at: u64 },
    /// Done with: nothing more comes of it until it is let go.
    Spent,
}

const TOAST_MS: u64 = 2000;
/// How long a text field's cursor shows and then hides, as the stock
/// fields blink it.
const BLINK_MS: u64 = 500;
/// The spring the home page's search bar moves to the search page's field
/// on, and back: critically damped, fitted to frames of the stock settings
/// on a Xiaomi 17 Ultra.
const MORPH_PERIOD: f32 = 0.292;
/// Along the move, from 0 at the home page to 1 at the search page: where
/// the large title has faded away, and the stretch over which the button
/// leaving the search comes in.
const MORPH_TITLE_GONE: f32 = 0.6;
const MORPH_CANCEL: (f32, f32) = (0.45, 0.95);
/// How blurred that button is as it starts to show, in dp.
const MORPH_CANCEL_BLUR_DP: f32 = 8.0;
/// When what the search page shows under its field comes in after the move
/// starts, and how long it takes, in ms.
const MORPH_BODY_MS: (u64, u64) = (380, 120);
/// A row a finger lands on starts darkening this long after, as the stock
/// settings' rows do; a finger moving on before then scrolls the page, and
/// the row never shows pressed.
const ROW_PRESS_DELAY_MS: u64 = 90;
/// The spring a row held down darkens on, and fades back on once let go:
/// critically damped, fitted to frames of the stock settings on a Xiaomi
/// 17 Ultra.
const ROW_PRESS_PERIOD: f32 = 0.18;
/// A row let go before it showed pressed shows pressed from the lift this
/// long, as Android shows a quick tap.
const QUICK_TAP_MS: u64 = 64;
/// A page opening slides in on a critically damped spring of this period,
/// set off already moving this fast, in screen widths a second; a page
/// going back slides off on a spring of this period, from rest. Both fitted
/// to frames of the stock settings on a Xiaomi 17 Ultra.
const OPEN_PERIOD: f32 = 0.37;
const OPEN_SPEED: f32 = 2.94;
const BACK_PERIOD: f32 = 0.393;
/// While a page in front covers it wholly, the page behind stands this far
/// left, in screen widths, darkened this much.
const BEHIND_SHIFT: f32 = 0.25;
const BEHIND_DIM: f32 = 0.47;
/// The most lines of what an install script shows that its page keeps,
/// the latest; the log keeps them all.
const SCRIPT_LINES: usize = 500;
/// How long a finger rests on an entry before it counts as held, as the
/// system's long press does.
const LONG_PRESS_MS: u64 = 400;
/// A finger lifted slower than this, in dp per second, leaves the page
/// where it is.
const MIN_FLING_DP: f32 = 50.0;
/// The fastest a page starts gliding, in dp per second.
const MAX_FLING_DP: f32 = 8000.0;
/// A glide running into a page's end faster than this, in dp per second,
/// swings out on the slower spring.
const HARD_BOUNCE_DP: f32 = 1667.0;
/// Scroll, in dp, over which a large title fades out.
const TITLE_FADE_DP: f32 = 20.0;
/// Blur, in dp, of a title faded out completely.
const TITLE_BLUR_DP: f32 = 25.0 / 3.0;
/// A search bar folding away: its icon and words fade out over the first
/// this many dp of the fold, then the bar over as many more, gone however
/// faint once its foot reaches its top. A page let go with the bar folded
/// this share of the way or further settles folded, and unfolded short of
/// it, on a critically damped spring of this period set off from rest.
/// Fitted to frames of the stock settings on a Xiaomi 17 Ultra.
const FOLD_FADE_DP: f32 = 26.0;
const FOLD_SETTLE: f32 = 0.5;
const FOLD_PERIOD: f32 = 0.354;
/// How far, in dp, a swipe from an edge has to come in to go back.
const BACK_SWIPE_DP: f32 = 56.0;
/// The back cue sits this far, in dp, above where the finger came down,
/// and stays at that height however the finger then wanders.
const CUE_LIFT_DP: f32 = 9.0;
/// On the About page, where the wordmark's area ends, in dp from the top:
/// the version has faded out by the time it passes under the bar.
const ABOUT_LOGO_DP: f32 = 160.0;
/// Scroll, in dp, over which the wordmark then shrinks and fades.
const ABOUT_LOGO_FADE_DP: f32 = 40.0;
/// Scroll, in dp, over which the About page's field fades out; the field's
/// discs sit in the band this far below the bar.
const ABOUT_FIELD_DP: f32 = 389.0;
/// The field is worked out every this many pixels and filled in between.
const FIELD_CELL: usize = 24;
/// Before the panel goes off it stays dim this long, in milliseconds, or
/// a fifth of the screen-off time when that is shorter.
const DIM_MS: u64 = 7000;
/// The dim level is this much below the brightness set, no brighter than
/// the ceiling, and never under the slider's floor unless the brightness
/// set is.
const DIM_DROP: f32 = 0.04;
const DIM_CEILING: f32 = 0.05;
/// The lowest brightness the slider sets.
const MIN_BRIGHTNESS: f32 = 0.02;
/// How much of a slider's span a volume key moves it.
const SLIDER_STEP: f32 = 0.1;
/// Where both sliders start: one step of the keys from their left end.
pub const SLIDER_START: f32 = SLIDER_STEP;
/// While the backlight eases, it steps once a frame of a 120 Hz panel.
const BACKLIGHT_STEP_MS: u64 = 8;
/// Just after the panel lights, a touch or key only keeps it awake: the
/// second tap of a wake habit, or the hand reaching for a phone whose
/// screen an install just lit, should not act. Android's double-tap window.
const WAKE_GUARD_MS: u64 = 400;
/// A volume key held this long moves the light again, then once every
/// [`REPEAT_MS`], as the system repeats a key held.
const REPEAT_DELAY_MS: u64 = 400;
const REPEAT_MS: u64 = 50;
/// The power key and volume down pressed within this long of each other
/// take a screenshot, as the system's key combination does; the first of
/// the two waits this long, or until it is let go, before it does what it
/// does alone.
const CHORD_MS: u64 = 150;
/// How far along its track the knob of a swipe-to-confirm control has to
/// be let go to confirm.
const SLID: f32 = 0.9;
/// How long the power key held takes the knob the whole way along.
const SLIDE_MS: u64 = 700;
/// The response, in seconds, of the spring the page scrolls on to show
/// the control the keys moved to.
const SEEK_PERIOD: f32 = 0.35;
/// How strongly the light tints the control the keys are on, and the one
/// the power key is held down on.
const LIGHT_ALPHA: u8 = 36;
const LIGHT_HELD_ALPHA: u8 = 72;
/// How far past the screen, in screens, a long list keeps its rows drawn.
const NEAR_SCREENS: f32 = 0.5;

/// How a part of the screen eases between shown and hidden.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ease {
    /// A critically damped spring of this period, in seconds.
    Spring(f32),
    /// A straight run over this many milliseconds.
    Linear(u64),
}

impl Ease {
    fn progress(self, ms: u64) -> f32 {
        match self {
            Ease::Spring(period) => motion::settle(ms, period),
            Ease::Linear(span) => (ms as f32 / span as f32).min(1.0),
        }
    }
}

/// A part of the screen fading in or out: where it is headed, since when,
/// how far in it was then, and how it eases each way.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fade {
    on: bool,
    since: u64,
    from: f32,
    show: Ease,
    hide: Ease,
}

impl Fade {
    fn hidden(show: Ease, hide: Ease) -> Fade {
        Fade { on: false, since: 0, from: 0.0, show, hide }
    }

    fn shown(show: Ease, hide: Ease) -> Fade {
        Fade { on: true, since: 0, from: 1.0, show, hide }
    }

    /// How far in, 0 to 1, at `now`.
    fn level(&self, now: u64) -> f32 {
        let (target, ease) = if self.on { (1.0, self.show) } else { (0.0, self.hide) };
        self.from + (target - self.from) * ease.progress(now.saturating_sub(self.since))
    }

    fn set(&mut self, on: bool, now: u64) {
        if on != self.on {
            self.from = self.level(now);
            self.since = now;
            self.on = on;
        }
    }

    /// Stand fully in or out at once.
    fn jump(&mut self, on: bool) {
        self.on = on;
        self.from = if on { 1.0 } else { 0.0 };
    }

    fn moving(&self, now: u64) -> bool {
        self.level(now) != if self.on { 1.0 } else { 0.0 }
    }
}

/// How the bar's title comes and goes: sharpening in on a spring and
/// blurring away on a quicker one, or on the About page, fading in a
/// straight line together with the bar.
fn title_fade(hero: bool) -> Fade {
    if hero {
        Fade::hidden(Ease::Linear(150), Ease::Linear(150))
    } else {
        Fade::hidden(Ease::Spring(0.3), Ease::Spring(0.15))
    }
}

fn bar_fade(hero: bool) -> Fade {
    if hero {
        Fade::hidden(Ease::Linear(150), Ease::Linear(150))
    } else {
        Fade::hidden(Ease::Linear(100), Ease::Linear(100))
    }
}

/// The panel at full brightness: it dims over half a second and comes
/// back quickly when touched.
fn full_brightness() -> Fade {
    Fade::shown(Ease::Linear(150), Ease::Linear(500))
}

/// The level the panel dims to shortly before it goes off, for the
/// brightness set.
fn dim_level(full: f32) -> f32 {
    (full - DIM_DROP).clamp(MIN_BRIGHTNESS, DIM_CEILING).min(full)
}

/// How long the panel stays dim before going off, for a screen-off time.
fn dim_span(after: u64) -> u64 {
    DIM_MS.min(after / 5)
}

/// Where a pinned strip shows on a page scrolled to `offset`: with the page,
/// or just under the bar once the page has carried it that far.
fn pinned_top(t: &Theme, p: &Pinned, offset: f32) -> f32 {
    (p.top - offset).max(widgets::bar_bottom(t))
}

/// Whether `r` reaches into the band of the screen from `top` to `bottom`.
fn within(r: Rect, (top, bottom): (f32, f32)) -> bool {
    r.y < bottom && r.y + r.h > top
}

/// Lay the sprite `blurs` keeps, moved by (`dx`, `dy`), in `color` at
/// `level` of its strength, blurred by `blur` pixels.
fn draw_faded(out: &mut Canvas, blurs: &mut Blurs, dx: f32, dy: f32, color: Rgba, level: f32, blur: f32) {
    if level > 0.0 {
        blurs.get(blur).draw(out, dx, dy, color, level);
    }
}

/// The home page's search bar moving up to become the search page's
/// field, or back down, as the stock settings move it: the home page's
/// content goes at once on the way up and comes along under the bar on
/// the way down; the large title fades and blurs as it goes up with the
/// bar; the button leaving the search fades and sharpens in; the keyboard
/// rises with the bar; and what the search page shows under its field
/// comes in last.
struct Morph {
    /// Up to the search page.
    into: bool,
    /// How far along it is, from 0 at the home page to 1 at the search
    /// page, as a spring in thousandths settling on the end it goes to.
    spring: Spring,
    start: u64,
    /// How far the home page was scrolled, where its bar stood.
    scroll: f32,
    /// The search page's keyboard, and where it rests.
    keyboard: Option<(Canvas, i32)>,
    /// What the search page shows between its field and the keyboard, and
    /// where.
    body: Option<(Canvas, i32)>,
    /// The home page's large title, and the button's disc and cross.
    title: Option<Blurs>,
    cancel: Option<(Blurs, Blurs)>,
}

impl Morph {
    /// How far along it is at `now`, and whether it has settled.
    fn at(&self, now: u64) -> (f32, bool) {
        let (x, done) = self.spring.at(now);
        (if self.into { 1.0 } else { 0.0 } + x / 1000.0, done)
    }
}

/// A row of the page under a finger, darkening behind its words and icon:
/// where it lies on the page, when it starts darkening, and when it starts
/// fading back.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RowPress {
    rect: Rect,
    rise: u64,
    fall: Option<u64>,
}

impl RowPress {
    /// How dark the row is at `now`, from 0 to 1.
    fn level(&self, now: u64) -> f32 {
        if now < self.rise {
            return 0.0;
        }
        let up = |t: u64| motion::settle(t - self.rise, ROW_PRESS_PERIOD);
        match self.fall {
            Some(fall) if now >= fall => up(fall) * (1.0 - motion::settle(now - fall, ROW_PRESS_PERIOD)),
            _ => up(now),
        }
    }

    /// Whether it has faded back after being let go.
    fn gone(&self, now: u64) -> bool {
        self.fall.is_some_and(|fall| now >= fall && motion::settle(now - fall, ROW_PRESS_PERIOD) == 1.0)
    }

    /// Whether it is darkening or fading back at `now`.
    fn changing(&self, now: u64) -> bool {
        let dark = self.fall.is_none() && now >= self.rise && motion::settle(now - self.rise, ROW_PRESS_PERIOD) == 1.0;
        now >= self.rise && !dark && !self.gone(now)
    }

    /// The finger lifted at `now`: a row that had not shown pressed yet
    /// shows it a moment.
    fn lift(&mut self, now: u64) {
        if now < self.rise {
            self.rise = now;
            self.fall = Some(now + QUICK_TAP_MS);
        } else {
            self.fall = Some(now);
        }
    }
}

/// How a page showed on a frame: how far scrolled, and how far past an end
/// with that; how far its bar and the bar's title had come in; where the
/// keys' light was; whether the cursor showed; how dark the row a finger
/// held was; and the button a finger held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    scroll: f32,
    offset: f32,
    bar: f32,
    title: f32,
    light: Option<(Group, Rect, f32, Rgba)>,
    caret: bool,
    row: f32,
    pressing: Option<Action>,
}

/// A page sliding over another as it opens or goes back, as the stock
/// settings slide them. A page opened comes in from the right edge over
/// the page it opens from, which shifts a quarter of the way left and
/// darkens; going back, the page left slides off to the right over the
/// page gone back to, which comes back from the left and brightens. The
/// page in front has its left corners rounded as the screen's are.
struct Turn {
    /// Going back: the page in front is the page left; else the page
    /// opened.
    back: bool,
    /// The page left, as it last showed.
    left: Canvas,
    /// How far from the left edge the page in front stands, in thousandths
    /// of the screen's width, as a spring settling where it goes; set off
    /// on the first frame of the slide.
    spring: Option<Spring>,
}

/// Whether a control is a row: as wide as the cards it lies on.
fn row_wide(t: &Theme, r: Rect) -> bool {
    let m = &t.m;
    (r.x - m.margin).abs() < 0.5 && (r.w - (m.w as f32 - m.margin * 2.0)).abs() < 0.5
}

/// The whole pixels of the page's `canvas` a held row's `rect` covers:
/// left, top, width and height.
fn row_part(r: Rect, canvas: &Canvas) -> (u32, u32, u32, u32) {
    let (x0, y0) = (r.x.floor().max(0.0) as u32, r.y.floor().max(0.0) as u32);
    let (x1, y1) = (((r.x + r.w).ceil() as u32).min(canvas.w), ((r.y + r.h).ceil() as u32).min(canvas.h));
    (x0, y0, x1 - x0, y1 - y0)
}

/// The keyboard at the foot of a typing page's `frame`, and where it rests.
fn keyboard_strip(t: &Theme, frame: &Frame) -> (Canvas, i32) {
    let top = (t.m.h as f32 - widgets::keyboard_height(t)).floor() as i32;
    let mut strip = Canvas::new(frame.canvas.w, (frame.canvas.h as i32 - top).max(0) as u32);
    strip.put_rows(&frame.canvas, top, frame.canvas.h as i32, 0);
    (strip, top)
}

pub struct App {
    pub state: State,
    /// The pages open, the one shown last: each with how far it is
    /// scrolled and where the keys are on it.
    stack: Vec<(Screen, f32, Focus)>,
    frame: Option<Frame>,
    /// The controls of the page drawn, in the order the keys go through
    /// them.
    spots: Option<Vec<Spot>>,
    /// The keys are working the pages: the control they are on is lit. A
    /// touch puts the light out, and a volume key brings it back.
    keys: bool,
    /// The button held down, and what holding it does.
    button: Option<(Key, Press)>,
    /// The first of the power key and volume down gone down, waiting for
    /// the other to take a screenshot, and when it went down.
    chord: Option<(Key, u64)>,
    /// The page scrolling of its own accord, to show the control the keys
    /// moved to or to fold its search bar whole or not at all: where it
    /// comes to rest, and the spring taking it there.
    seek: Option<(f32, Spring)>,
    /// A swipe-to-confirm control whose knob the power key moved, to draw
    /// again with the page.
    knob: Option<(Confirm, Rect)>,
    /// The slider lit takes the volume keys, until the power key lets it
    /// go.
    adjusting: bool,
    /// How tall the page was when last drawn: one being drawn again, as
    /// work under way tells how far it has come, scrolls as far meanwhile.
    height: f32,
    /// The rows of the page's long list drawn so far, by their place in it;
    /// those far off screen are let go, their canvases kept spare for the
    /// next rows to be drawn on.
    drawn: Vec<(usize, Canvas)>,
    spare: Vec<Canvas>,
    /// The current page's large and bar titles, for their words and the
    /// theme, with their blurred versions as they are needed. A page can
    /// change its title in place, as work on files does once it ends.
    titles: Option<((&'static str, bool), Blurs, Blurs)>,
    /// The icon and words of the current page's folding search bar, for
    /// its words and the theme.
    search_words: Option<((&'static str, bool), Canvas)>,
    /// The `<` chip for the theme (dark or not), and where it goes.
    chip: Option<(bool, Canvas, i32, i32)>,
    /// The chip behind the round button at the right end of the bar, as
    /// for `<`.
    button_chip: Option<(bool, Canvas, i32, i32)>,
    /// The folder the terminal page's shell last started in, where Enter
    /// starts it again once it has ended.
    shell_folder: String,
    /// When leaving the editor with its changes unsaved was last warned
    /// of: going back again while the warning shows leaves without them.
    unsaved_warned: Option<u64>,
    /// The About page field's dither, made for the screen once.
    dither: Option<glow::Dither>,
    toast: Option<(String, u64)>,
    drag: Drag,
    tracker: Tracker,
    /// The page gliding on after a flick.
    fling: Option<Fling>,
    /// A terminal's screen or the editor's text gliding on after a flick:
    /// the glide, a row's height, and how many rows it has scrolled so far.
    glide: Option<(Fling, f32, isize)>,
    /// How far a dragging finger has pulled the page past an end:
    /// negative past the top, positive past the bottom.
    pull: f32,
    /// The page springing back after being pulled or thrown past an end.
    bounce: Option<Spring>,
    /// The back cue shrinking away after a swipe that did not go back:
    /// its edge, where it sits, and its depth.
    cue: Option<(bool, f32, Spring)>,
    title: Fade,
    bar: Fade,
    /// A page was put up, or the file manager's folder changed: once it is
    /// rendered, the bar and its title stand as its scroll has them,
    /// without easing in or out.
    snap: bool,
    /// The About page's moving field, while that page shows.
    glow: Option<GlowClock>,
    /// The terminal page's shell, from when it is first opened; it goes on
    /// while other pages show.
    term: Option<Term>,
    display_on: bool,
    /// When the screen was last touched or a key pressed; set on the first
    /// tick, so the time before the UI started does not count.
    active: Option<u64>,
    /// When the panel last lit from dark, for the brief guard that keeps a
    /// stray touch or key from acting on a page just put up.
    woke_at: Option<u64>,
    /// The panel at full brightness, or easing to the dim level it takes
    /// shortly before going off.
    bright: Fade,
    /// While a text field's cursor is up: the page, how many edits had
    /// been typed, and when it came up whole on them. It shows for the
    /// first half of each blink from then on, so it is whole again at each
    /// edit and on each page it comes up on.
    blink: Option<(Screen, u64, u64)>,
    /// Edits typed into any text field so far.
    edits: u64,
    /// Whether the frame last composed showed the cursor.
    caret_shown: bool,
    /// The home page's search bar on its way to the search page's field,
    /// or back.
    morph: Option<Morph>,
    /// The row a finger holds down, or let go of while it fades back, and
    /// the part of the page it lies on as drawn and as drawn held down.
    row_press: Option<RowPress>,
    row_art: Option<(Canvas, Canvas)>,
    /// How the page showed on the last frame, while it can be drawn so
    /// again: until it is rendered anew.
    shown: Option<View>,
    /// A page sliding in over the page left, or off over the page gone
    /// back to.
    turn: Option<Turn>,
    /// While a page slides: the page shown drawn on its own, and how it
    /// showed, to slide it again as it was while it shows the same.
    layer: Option<(View, Canvas)>,
    /// Bands of rows of the page drawn that show what has changed since,
    /// to draw again.
    stale: Vec<(u32, u32)>,
    /// How many columns from the left edge of the page's rows drawn are
    /// drawn; a page opened is drawn across as it slides in.
    cols: u32,
    /// The canvas the last page left was drawn on, to draw the next on.
    backdrop: Option<Canvas>,
    /// How round the screen's corners are, in pixels.
    corners: f32,
    dirty: bool,
    w: u32,
    h: u32,
    px_per_dp: f32,
}

impl App {
    pub fn new(start: Screen, w: u32, h: u32, px_per_dp: f32) -> Self {
        App {
            state: State::default(),
            stack: vec![(start, 0.0, Focus::Unset)],
            frame: None,
            spots: None,
            // The touchscreen may run from the partitions fastboot leaves
            // out, so its page starts worked with the keys.
            keys: start == Screen::Fastboot,
            button: None,
            chord: None,
            seek: None,
            knob: None,
            adjusting: false,
            height: 0.0,
            drawn: Vec::new(),
            spare: Vec::new(),
            titles: None,
            search_words: None,
            chip: None,
            button_chip: None,
            shell_folder: "/".into(),
            unsaved_warned: None,
            dither: None,
            toast: None,
            drag: Drag::Idle,
            tracker: Tracker::default(),
            fling: None,
            glide: None,
            pull: 0.0,
            bounce: None,
            cue: None,
            title: title_fade(start == Screen::About),
            bar: bar_fade(start == Screen::About),
            snap: false,
            glow: None,
            term: None,
            display_on: true,
            active: None,
            woke_at: None,
            bright: full_brightness(),
            blink: None,
            edits: 0,
            caret_shown: false,
            morph: None,
            row_press: None,
            row_art: None,
            shown: None,
            turn: None,
            layer: None,
            stale: Vec::new(),
            cols: 0,
            backdrop: None,
            corners: 0.0,
            dirty: true,
            w,
            h,
            px_per_dp,
        }
    }

    /// The screen's corners are rounded to `radius` pixels: so are the
    /// leading corners of a page sliding over another.
    pub fn set_screen_corners(&mut self, radius: f32) {
        self.corners = radius;
    }

    pub fn theme(&self) -> Theme {
        if self.state.dark {
            Theme::dark(self.w, self.h, self.px_per_dp)
        } else {
            Theme::light(self.w, self.h, self.px_per_dp)
        }
    }

    pub fn screen(&self) -> Screen {
        self.top().0
    }

    fn top(&self) -> &(Screen, f32, Focus) {
        self.stack.last().expect("the page stack always holds a page")
    }

    fn top_mut(&mut self) -> &mut (Screen, f32, Focus) {
        self.stack.last_mut().expect("the page stack always holds a page")
    }

    /// Whether the panel needs a new frame since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// When time next has to pass through [`App::tick`]: at once while
    /// something on the lit screen moves on its own, a frame on while the
    /// backlight eases, else when a shown message should disappear or the
    /// panel should dim or go off.
    pub fn deadline(&self, now: u64) -> Option<u64> {
        let sliding = matches!(self.button, Some((_, Press::Slide { .. }))) && self.state.confirm < 1.0;
        let moving = self.display_on
            && (self.fling.is_some()
                || self.glide.is_some()
                || self.bounce.is_some()
                || self.seek.is_some()
                || sliding
                || self.cue.is_some()
                || self.title.moving(now)
                || self.bar.moving(now)
                || self.morph.is_some()
                || self.turn.is_some()
                || self.row_press.is_some_and(|p| p.changing(now) || p.gone(now))
                || self.glowing());
        if moving {
            return Some(0);
        }
        let darkens = self.row_press.map(|p| p.rise).filter(|&rise| rise > now);
        let easing = self.bright.moving(now).then_some(now + BACKLIGHT_STEP_MS);
        let held = match self.drag {
            Drag::Press { action: Some(Action::OpenEntry(_)), at, .. } => Some(at + LONG_PRESS_MS),
            _ => None,
        };
        let repeat = match self.button {
            Some((_, Press::Volume { next, .. })) => Some(next),
            Some((_, Press::Power { spot: Spot { stop: Stop::Hit(Action::OpenEntry(_)), .. }, at })) => Some(at + LONG_PRESS_MS),
            _ => None,
        };
        let blink = self.blink_at(now).filter(|_| self.display_on).map(|(_, next)| next);
        let chord = self.chord.map(|(_, at)| at + CHORD_MS);
        [self.toast.as_ref().map(|t| t.1), easing, self.idle_due(), held, repeat, blink, darkens, chord].into_iter().flatten().min()
    }

    /// Whether a text field's cursor shows at `now`, and when it next turns;
    /// `None` while no cursor is up.
    fn blink_at(&self, now: u64) -> Option<(bool, u64)> {
        let (_, _, since) = self.blink?;
        let n = now.saturating_sub(since) / BLINK_MS;
        Some((n.is_multiple_of(2), since + (n + 1) * BLINK_MS))
    }

    /// When sitting idle next changes the lit panel: when it dims, or once
    /// it is dimming, when it goes off.
    fn idle_due(&self) -> Option<u64> {
        let (active, after) = (self.active?, SCREEN_OFF[self.state.screen_off]?);
        if !self.display_on {
            return None;
        }
        Some(if self.bright.on { active + after - dim_span(after) } else { active + after })
    }

    /// Open what can be done with entry `i` of the folder shown, held long
    /// enough to, with a tap of the motor.
    fn held_entry(&mut self, i: usize) -> Vec<Effect> {
        let entry = self.state.files.entry(i).clone();
        self.state.file = Some(entry);
        self.state.asking_delete = false;
        self.open(Screen::FileInfo);
        if self.state.vibrate {
            vec![Effect::Haptic]
        } else {
            Vec::new()
        }
    }

    /// Let time pass: the panel dims and goes off once left alone long
    /// enough, a gliding page moves on, a page past its end springs back,
    /// the bar and its title follow the scroll, and a message past its
    /// deadline goes away.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        // The key waiting for its partner in a screenshot, left alone long
        // enough, does what it does alone.
        if self.chord.is_some_and(|(_, at)| now >= at + CHORD_MS) {
            fx.extend(self.let_chord_go());
        }
        // An entry of the file manager held long enough, under a finger or
        // the power key, opens what can be done with it, a folder as much as
        // a file, with a tap of the motor.
        if let Drag::Press { action: Some(Action::OpenEntry(i)), at, .. } = self.drag {
            if now >= at + LONG_PRESS_MS {
                self.drag = Drag::Held;
                fx.extend(self.held_entry(i));
            }
        }
        if let Some((key, Press::Power { spot: Spot { stop: Stop::Hit(Action::OpenEntry(i)), .. }, at })) = self.button {
            if now >= at + LONG_PRESS_MS {
                self.button = Some((key, Press::Spent));
                fx.extend(self.held_entry(i));
            }
        }
        let active = *self.active.get_or_insert(now);
        if let (true, Some(after)) = (self.display_on, SCREEN_OFF[self.state.screen_off]) {
            let idle = now.saturating_sub(active);
            if idle >= after {
                self.sleep();
            } else if idle >= after - dim_span(after) {
                self.bright.set(false, now);
            }
        }
        if self.toast.as_ref().is_some_and(|m| now >= m.1) {
            self.toast = None;
            self.dirty = true;
        }
        if self.blink_at(now).is_some_and(|(on, _)| on != self.caret_shown) || self.morph.is_some() || self.turn.is_some() {
            self.dirty = true;
        }
        if let Some(p) = self.row_press {
            if p.gone(now) {
                self.row_press = None;
                self.row_art = None;
                self.dirty = true;
            } else if p.changing(now) {
                self.dirty = true;
            }
        }
        if let Some(fling) = self.fling {
            let (y, done) = fling.at(now);
            let next = y.clamp(0.0, self.max_scroll());
            let top = self.top_mut();
            if next != top.1 {
                top.1 = next;
                self.dirty = true;
            }
            if next != y {
                // The glide ran into an end: the speed it had left throws
                // the page past it, and it springs back.
                let speed = fling.velocity(now);
                let period = if speed.abs() <= self.theme().m.dp(HARD_BOUNCE_DP) { 0.4 } else { 0.55 };
                self.bounce = Some(Spring::new(now, 0.0, speed, period));
                self.fling = None;
            } else if done {
                self.fling = None;
                self.settle_fold(now);
            }
        }
        if let Some((glide, pitch, scrolled)) = self.glide {
            let (y, done) = glide.at(now);
            let rows = (y / pitch).trunc() as isize;
            if rows != scrolled {
                self.scroll_rows(rows - scrolled);
            }
            self.glide = if done { None } else { Some((glide, pitch, rows)) };
        }
        if let Some(b) = self.bounce {
            self.dirty = true;
            if b.at(now).1 {
                self.bounce = None;
            }
        }
        if let Some((target, spring)) = self.seek {
            let (off, done) = spring.at(now);
            let at = if done { target } else { (target + off).clamp(0.0, self.max_scroll()) };
            let top = self.top_mut();
            if top.1 != at {
                top.1 = at;
                self.dirty = true;
            }
            if done {
                self.seek = None;
            }
        }
        // The power key held on a swipe-to-confirm control slides its knob.
        if let Some((_, Press::Slide { which, track, at })) = self.button {
            let along = (now.saturating_sub(at) as f32 / SLIDE_MS as f32).min(1.0);
            if along != self.state.confirm {
                self.state.confirm = along;
                self.knob = Some((which, track));
                self.dirty = true;
            }
        }
        // A volume key held moves the light on and on, once the page it
        // last moved on has been drawn.
        if let Some((key, Press::Volume { down, next })) = self.button {
            if now >= next && self.frame.is_some() {
                self.button = Some((key, Press::Volume { down, next: now + REPEAT_MS }));
                fx.extend(self.volume(down, now));
            }
        }
        if let Some((_, _, s)) = self.cue {
            self.dirty = true;
            if s.at(now).1 {
                self.cue = None;
            }
        }
        if self.screen() == Screen::About {
            let look = self.theme().hero.glow;
            match &mut self.glow {
                Some(g) if std::ptr::eq(g.look(), look) => g.advance(now),
                _ => self.glow = Some(GlowClock::new(look, now)),
            }
            if self.glowing() {
                self.dirty = true;
            }
        }
        self.follow_scroll(now);
        fx
    }

    /// Whether the About page's field is on screen, moving.
    fn glowing(&self) -> bool {
        self.glow.is_some() && self.display_on && self.top().1 < self.theme().m.dp(ABOUT_FIELD_DP)
    }

    /// Whether the bar's title and the bar belong over the rendered page
    /// where it is scrolled: the title once the large one has faded, the bar
    /// once the content slides under it. On the About page both come once
    /// the version has faded out.
    fn bar_wanted(&self) -> Option<(bool, bool)> {
        let fr = self.frame.as_ref()?;
        let t = self.theme();
        let scroll = self.top().1;
        Some(if fr.hero.is_some() {
            let gone = scroll >= t.m.dp(ABOUT_LOGO_DP) - widgets::bar_bottom(&t);
            (gone, gone)
        } else {
            let under = (fr.content_top - widgets::bar_bottom(&t) - t.m.dp(20.0)).max(0.0);
            (fr.header && fr.title_lift(scroll) >= t.m.dp(TITLE_FADE_DP), scroll > under)
        })
    }

    /// Bring the bar and its title in or out as the page scrolls.
    fn follow_scroll(&mut self, now: u64) {
        let Some((title, bar)) = self.bar_wanted() else { return };
        self.title.set(title, now);
        self.bar.set(bar, now);
        if self.title.moving(now) || self.bar.moving(now) {
            self.dirty = true;
        }
    }

    /// How far past its end the page shows at `now`: negative past the
    /// top, positive past the bottom.
    fn overscroll(&self, now: u64) -> f32 {
        if self.pull != 0.0 {
            motion::rubber(self.pull, self.h as f32)
        } else {
            self.bounce.map_or(0.0, |b| b.at(now).0)
        }
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    /// The backlight at `now`: off, or this share of the brightness at the
    /// slider's top.
    pub fn panel(&self, now: u64) -> Option<f32> {
        if !self.display_on {
            return None;
        }
        let (full, level) = (self.state.brightness, self.bright.level(now));
        Some(full * level + dim_level(full) * (1.0 - level))
    }

    /// Something was touched or pressed: the panel stays at full
    /// brightness for the screen-off time from now.
    fn stir(&mut self, now: u64) {
        self.active = Some(now);
        self.bright.set(true, now);
    }

    /// Light the panel, as if just touched.
    fn wake(&mut self, now: u64) {
        if !self.display_on {
            self.display_on = true;
            self.dirty = true;
        }
        self.woke_at = Some(now);
        self.stir(now);
    }

    /// Whether the panel lit so recently that a touch or key should only
    /// keep it awake.
    fn just_woke(&self, now: u64) -> bool {
        self.woke_at.is_some_and(|w| now.saturating_sub(w) < WAKE_GUARD_MS)
    }

    /// Turn the panel off. A finger on the glass lets go of what it held,
    /// and the page stops where it is.
    fn sleep(&mut self) {
        self.display_on = false;
        self.bright = full_brightness();
        match std::mem::replace(&mut self.drag, Drag::Idle) {
            Drag::Confirm { .. } => {
                self.state.confirm = 0.0;
                self.refresh();
            }
            Drag::Pattern => {
                self.state.pattern.clear();
                self.refresh();
            }
            _ => {}
        }
        self.drop_button();
        self.adjusting = false;
        self.fling = None;
        self.bounce = None;
        self.seek = None;
        self.pull = 0.0;
        self.cue = None;
        self.morph = None;
        self.row_press = None;
        self.row_art = None;
        self.end_turn();
    }

    /// A page sliding over another comes to rest at once.
    fn end_turn(&mut self) {
        if let Some(turn) = self.turn.take() {
            self.backdrop = Some(turn.left);
            self.dirty = true;
        }
    }

    /// Something laid over the page changed (the clock): compose again.
    pub fn redraw(&mut self) {
        self.dirty = true;
    }

    /// The page's readings changed (battery, storage): render it again.
    pub fn refresh(&mut self) {
        self.frame = None;
        self.stale.clear();
        self.spots = None;
        self.knob = None;
        self.row_art = None;
        self.shown = None;
        self.layer = None;
        self.spare.extend(self.drawn.drain(..).map(|(_, c)| c));
        self.dirty = true;
    }

    /// What the page shows changed only in rows `first..end`: draw those
    /// again, and the rest stays as drawn.
    fn refresh_rows(&mut self, first: u32, end: u32) {
        if self.frame.is_none() {
            return;
        }
        self.stale.push((first, end));
        self.spots = None;
        self.shown = None;
        self.layer = None;
        self.dirty = true;
    }

    /// The data partition is open: every user's own files with it, or
    /// those of `waiting`, the user and the control their screen lock is
    /// answered with, still closed.
    pub fn data_waiting(&mut self, waiting: Option<(u32, Keys)>) {
        self.state.data = match waiting {
            Some((user, keys)) => Data::Locked { user, keys, checking: false },
            None => Data::Open,
        };
        self.state.code.clear();
        self.refresh();
    }

    /// The data partition did not open, and why.
    pub fn data_failed(&mut self, why: String) {
        self.state.data = Data::Failed(why);
        self.refresh();
    }

    /// What the user entered did not open their own files: the page asks
    /// again and says why.
    pub fn unlock_failed(&mut self, why: String, now: u64) {
        if let Data::Locked { checking, .. } = &mut self.state.data {
            *checking = false;
        }
        self.refresh();
        self.report(&why, now);
    }

    /// The runtime read the mount table again, or could not: keep it and
    /// show it if it changed.
    pub fn set_mounts(&mut self, mounts: Result<Vec<MountRow>, String>) {
        if self.state.mounts != mounts {
            self.state.mounts = mounts;
            self.refresh();
        }
    }

    /// The runtime read whether fastbootd runs and how far a computer has
    /// the port: keep it and show it if it changed.
    pub fn set_fastboot(&mut self, running: bool, link: UsbLink) {
        let now = Fastboot { running, link };
        if self.state.fastboot != now {
            self.state.fastboot = now;
            self.refresh();
        }
    }

    /// Fastboot has stopped and the recovery is up: its pages start again
    /// from the home page, which nothing can go back from to fastboot.
    pub fn enter_recovery(&mut self) {
        self.stack = vec![(Screen::Home, 0.0, Focus::Unset)];
        self.settle_page();
        self.refresh();
    }

    /// How many rows and columns the terminal's screen holds.
    pub fn term_size(&self, f: &Fonts) -> (u16, u16) {
        let t = self.theme();
        let l = term::layout(&t, f, t.m.content_top);
        (l.rows, l.cols)
    }

    /// A shell has started for the terminal page, on a blank screen of
    /// `rows` by `cols`.
    pub fn term_started(&mut self, rows: u16, cols: u16) {
        self.term = Some(Term::new(rows, cols));
        self.refresh();
    }

    /// What the shell wrote.
    pub fn term_output(&mut self, bytes: &[u8]) {
        let term = self.term.as_mut().expect("only a shell that started writes");
        term.feed(bytes);
        if self.screen() == Screen::Terminal {
            self.refresh();
        }
    }

    /// The shell has ended as `end` says; its screen says so, and that
    /// Enter starts another.
    pub fn term_ended(&mut self, end: TermEnd) {
        let (l, term) = (self.state.lang, self.term.as_mut().expect("only a shell that started ends"));
        let how = fmt::term_end(l, &end);
        term.feed(tr!(l, "\r\n[{how}。按「回车」重新启动]\r\n", "\r\n[{how}。按「Enter」重新啟動]\r\n", "\r\n[{how}. Press Enter to start it again]\r\n").as_bytes());
        term.running = false;
        if self.screen() == Screen::Terminal {
            self.refresh();
        }
    }

    /// Names are taken in `into` where a copy or move of `path` goes: what
    /// to do with them is the user's to choose.
    pub fn file_clash(&mut self, op: FileOp, path: String, into: String, taken: Vec<String>) {
        self.state.clash = Some((Clip { op, path }, into, taken));
        self.open(Screen::Clash);
    }

    /// Work on files has begun: its page shows how far it comes, in place
    /// of the page of an entry being deleted.
    pub fn file_job_started(&mut self, task: FileTask) {
        if matches!(task, FileTask::Copy { .. } | FileTask::Move { .. }) {
            self.state.clip = None;
        }
        self.state.job = Some(FileJob { task, progress: None, outcome: None, stopping: false });
        // The work's page takes the place of the entry's page, sliding in
        // over it.
        if self.screen() == Screen::FileInfo {
            self.leave(false);
            self.state.file = None;
            self.state.asking_delete = false;
            self.stack.pop();
        }
        self.open(Screen::FileJob);
    }

    /// How far work on files has come, in bytes and in entries.
    pub fn file_progress(&mut self, bytes: (u64, u64), entries: (u64, u64)) {
        let job = self.state.job.as_mut().expect("only work under way moves on");
        job.progress = Some((bytes, entries));
        if self.screen() == Screen::FileJob {
            self.refresh();
        }
    }

    /// Work on files has ended. Done, its page goes and says so; stopped
    /// or failed, the page stays and says why, lit for the user to read.
    pub fn file_job_ended(&mut self, end: JobEnd, now: u64) {
        let job = self.state.job.as_mut().expect("only work under way ends");
        job.stopping = false;
        let done = match (&end, &job.task) {
            (JobEnd::Done, FileTask::Copy { .. }) => Some(["已复制", "已複製", "Copied"]),
            (JobEnd::Done, FileTask::Move { .. }) => Some(["已移动", "已移動", "Moved"]),
            (JobEnd::Done, FileTask::Delete { .. }) => Some(["已删除", "已刪除", "Deleted"]),
            _ => None,
        };
        job.outcome = Some(end);
        match done {
            Some(said) => {
                self.state.job = None;
                if self.screen() == Screen::FileJob {
                    self.leave(true);
                    self.stack.pop();
                    self.settle_page();
                }
                self.refresh();
                self.say(self.state.lang.pick(said), now);
            }
            None => {
                self.wake(now);
                self.refresh();
            }
        }
    }

    /// Giving the data partition's files their labels has started: its page
    /// follows how far it comes.
    pub fn relabel_started(&mut self) {
        self.state.relabel = Some(RelabelJob::default());
        self.refresh();
    }

    /// How far giving the data partition's files their labels has come:
    /// entries looked at, of how many, and how many took a new label.
    pub fn relabel_progress(&mut self, checked: u64, total: u64, changed: u64) {
        let job = self.state.relabel.as_mut().expect("only work under way moves on");
        (job.checked, job.total, job.changed) = (checked, total, changed);
        if self.screen() == Screen::Relabel {
            self.refresh();
        }
    }

    /// Giving the data partition's files their labels has ended: its page
    /// says how, lit for the user to read.
    pub fn relabel_ended(&mut self, end: JobEnd, now: u64) {
        let job = self.state.relabel.as_mut().expect("only work under way ends");
        job.stopping = false;
        job.outcome = Some(end);
        self.wake(now);
        self.refresh();
    }

    /// The partitions for the filesystem page, as the runtime read them,
    /// or why it could not.
    pub fn set_filesystems(&mut self, rows: Result<Vec<FsRow>, String>) {
        self.state.filesystems = Some(rows);
        self.refresh();
    }

    /// The partition the page of one shows.
    fn fs_row(&self) -> Option<&FsRow> {
        self.state.filesystems.as_ref()?.as_ref().ok()?.get(self.state.fs_chosen)
    }

    /// Work on whole partitions has begun. Work the system asked for, which
    /// `asked` says, has its page as the only one; the filesystem page's
    /// opens over it, to go back to once the work has ended.
    pub fn part_work_started(&mut self, kind: PartKind, asked: bool) {
        self.state.part_work = Some(PartWork { kind, asked, items: Vec::new(), output: Vec::new(), outcome: None });
        match asked {
            true => {
                self.stack = vec![(Screen::PartWork, 0.0, Focus::Unset)];
                self.settle_page();
            }
            false => self.open(Screen::PartWork),
        }
        self.refresh();
    }

    /// The partitions the work goes through.
    pub fn part_listed(&mut self, names: Vec<String>) {
        let work = self.state.part_work.as_mut().expect("only work under way lists its partitions");
        work.items = names.into_iter().map(|name| (name, ItemState::Waiting)).collect();
        self.refresh();
    }

    /// The partition at `index` is under way.
    pub fn part_at(&mut self, index: usize) {
        let work = self.state.part_work.as_mut().expect("only work under way moves on");
        work.items[index].1 = ItemState::Working;
        self.refresh();
    }

    /// The partition at `index` ended as `end`, the program working on it
    /// having said `said`.
    pub fn part_ended(&mut self, index: usize, end: ItemEnd, said: &str) {
        let work = self.state.part_work.as_mut().expect("only work under way ends a partition");
        work.items[index].1 = ItemState::Ended(end);
        let lines: Vec<String> = said.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(str::to_string).collect();
        if !lines.is_empty() {
            work.output = lines[lines.len().saturating_sub(SCRIPT_LINES)..].to_vec();
        }
        self.refresh();
    }

    /// The work has ended. What comes next is the user's to choose, and
    /// the panel lights for it, unless the system asked for the work and it
    /// went through every partition: the phone then leaves on its own. The
    /// data partition, worked on, is closed now.
    pub fn part_work_ended(&mut self, outcome: Result<(), PartStop>, now: u64) {
        let work = self.state.part_work.as_mut().expect("only work under way ends");
        let failed = outcome.is_err() || work.items.iter().any(|(_, s)| matches!(s, ItemState::Ended(ItemEnd::Failed(_))));
        let lit = failed || !work.asked;
        let data = work.kind == PartKind::Repair && work.items.iter().any(|(name, _)| name == DATA_PARTITION);
        work.outcome = Some(outcome);
        if data && self.state.data != Data::Erased {
            self.state.data = Data::Closed;
        }
        if lit {
            self.wake(now);
        }
        self.refresh();
    }

    /// The system asked for the package at `name` to be installed before
    /// the recovery started: the installing page opens over the home page,
    /// looking for it. `security` for a security update.
    pub fn start_asked(&mut self, security: bool, name: String) {
        self.begin_install(Install::Starting, Source::Asked { security }, Some(name));
        self.open(Screen::Installing);
    }

    /// The package the system asked for is in the data partition, which is
    /// being opened for it.
    pub fn asked_opening(&mut self) {
        self.state.install = Install::Opening;
        self.state.data = Data::Opening;
        self.refresh();
    }

    /// The package the system asked for is in the own files of `user`,
    /// which wait on their screen lock: the page says to decrypt them, and
    /// lights for it.
    pub fn asked_locked(&mut self, user: u32, now: u64) {
        self.state.install = Install::Locked(user);
        self.wake(now);
        self.refresh();
    }

    /// The system asked for rescue before the recovery started: the
    /// installing page opens over the home page, already waiting for the
    /// computer.
    pub fn start_rescue(&mut self) {
        self.begin_install(Install::Starting, Source::Rescue, None);
        self.open(Screen::Installing);
    }

    /// In rescue, the computer asked for all user data to be erased, and it
    /// is being erased.
    pub fn rescue_wiping(&mut self) {
        self.state.install = Install::Wiping;
        self.refresh();
    }

    /// In rescue, erasing all user data has ended; a data partition erased
    /// whole opens no more until the system makes it afresh.
    pub fn rescue_wiped(&mut self, outcome: Result<(), EraseStop>, now: u64) {
        if outcome.is_ok() {
            self.state.data = Data::Erased;
        }
        self.state.install = Install::Wiped(outcome);
        self.wake(now);
        if self.screen() == Screen::Installing {
            self.refresh();
        }
    }

    /// How the WLAN stands, as the runtime last told it.
    pub fn set_wlan(&mut self, w: Wlan) {
        if self.state.wlan.as_ref() == Some(&w) {
            return;
        }
        self.state.wlan = Some(w);
        if matches!(self.screen(), Screen::Home | Screen::Wlan | Screen::WlanNetwork | Screen::Update) {
            self.refresh();
        }
        // The status bar shows the network joined over every page.
        self.dirty = true;
    }

    /// How fast data went over the network joined in the last second, which
    /// only the status bar shows.
    pub fn set_traffic(&mut self, t: Traffic) {
        if self.state.traffic != t {
            self.state.traffic = t;
            self.dirty = true;
        }
    }

    /// What the status bar shows of the WLAN: the network joined, while it
    /// has an address.
    /// What the status bar shows, its clock reading `clock`: the battery,
    /// the network joined, the data partition's lock, the slot the system
    /// runs from, and the battery's temperature.
    pub fn bar<'a>(&self, clock: &'a str, info: &DeviceInfo) -> Bar<'a> {
        let battery = info.battery.as_ref();
        Bar {
            clock,
            battery: battery.map(|b| (b.level, b.plugged().then_some(if b.quick { Bolt::Quick } else { Bolt::Charging }))),
            wlan: self.wlan_mark(),
            data: match self.state.data {
                Data::Erased => None,
                Data::Open => Some(true),
                Data::Closed | Data::Opening | Data::Failed(_) | Data::Locked { .. } => Some(false),
            },
            slot: info.slot.map(Slot::letter),
            temp: battery.and_then(|b| b.temp),
        }
    }

    pub fn wlan_mark(&self) -> Option<WlanMark> {
        let Some(Joined { stage: Joining::Connected(link), .. }) = self.state.wlan.as_ref()?.joined.as_ref() else { return None };
        let t = self.state.traffic;
        let (speed, unit) = fmt::speed(t.rx + t.tx);
        Some(WlanMark { level: fmt::signal_level(link.signal), receiving: t.rx > 0, sending: t.tx > 0, speed, unit })
    }

    /// What the update server answered, and what internal storage holds of
    /// the full package of the newest system it told of.
    pub fn update_checked(&mut self, answer: Result<Option<Release>, Unanswered>, have: Have) {
        self.state.check = match answer {
            Ok(newest) => Check::Answered(newest),
            Err(why) => Check::Failed(why),
        };
        self.state.have = have;
        if matches!(self.screen(), Screen::Advanced | Screen::Update) {
            self.refresh();
        }
    }

    /// How far downloading the full package has come, and how fast it goes,
    /// in bytes a second.
    pub fn download_progress(&mut self, progress: Progress, rate: u64) {
        let job = self.state.download.as_mut().expect("only a download under way moves on");
        job.progress = Some(progress);
        job.rate = rate;
        if self.screen() == Screen::Update {
            self.refresh();
        }
    }

    /// Downloading the full package has ended, leaving `have` of it: the
    /// update page says how, lit for the user to read.
    pub fn download_ended(&mut self, end: DownloadEnd, have: Have, now: u64) {
        let job = self.state.download.as_mut().expect("only a download under way ends");
        job.stopping = false;
        job.outcome = Some(end);
        self.state.have = have;
        self.wake(now);
        self.refresh();
    }

    /// Open the password page for `network`, empty.
    fn ask_password(&mut self, network: Network) {
        self.state.joining = Some(network);
        self.state.password.clear();
        self.state.show_password = false;
        self.state.layer = Layer::Letters;
        self.open(Screen::WlanPassword);
    }

    /// How renaming the entry the file page shows went: done, the folder
    /// comes back with the entry under its new name; refused, why.
    pub fn renamed(&mut self, outcome: Result<(), RenameRefused>, now: u64) {
        match outcome {
            Ok(()) => {
                self.leave(true);
                while matches!(self.screen(), Screen::Rename | Screen::FileInfo) {
                    self.stack.pop();
                }
                self.state.file = None;
                self.settle_page();
                self.refresh();
                self.say(self.state.lang.pick(["已重命名", "已重新命名", "Renamed"]), now);
            }
            Err(RenameRefused::Taken) => self.say_taken(now),
            Err(RenameRefused::Failed(why)) => self.report(&why, now),
        }
    }

    /// The entry the file page shows was to have the permission bits
    /// `wanted`: those it has now, or why it has not.
    pub fn mode_set(&mut self, wanted: u32, outcome: Result<u32, String>, now: u64) {
        let l = self.state.lang;
        let bits = match outcome {
            Ok(bits) => bits,
            Err(why) => return self.report(&tr!(l, "修改权限失败：{why}", "修改權限失敗：{why}", "Couldn't change the permissions: {why}"), now),
        };
        let entry = self.state.file.as_mut().expect("the permissions page shows an entry");
        entry.mode = (entry.mode & !0o7777) | bits;
        let name = entry.name.clone();
        if let Ok(entries) = &mut self.state.files.entries {
            if let Some(listed) = entries.iter_mut().find(|e| e.name == name) {
                listed.mode = (listed.mode & !0o7777) | bits;
            }
        }
        if bits != wanted {
            self.say(l.pick(["该位置不支持修改权限", "這個位置不支援修改權限", "Permissions can't be changed here"]), now);
        }
        self.refresh();
    }

    /// What the file at `path` holds, read to edit it: none if it holds
    /// more than [`editor::MAX_BYTES`]; or why it could not be read. Text
    /// opens on the editor page, wrapped to its card; anything else is
    /// turned down, saying why.
    pub fn text_opened(&mut self, f: &Fonts, path: String, read: Result<Option<Vec<u8>>, String>, now: u64) {
        let l = self.state.lang;
        let bytes = match read {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return self.say(l.pick(["暂不支持编辑超过 2 MB 的文件", "尚不支援編輯超過 2 MB 的檔案", "Files over 2 MB can't be edited"]), now),
            Err(why) => return self.report(&tr!(l, "读取失败：{why}", "讀取失敗：{why}", "Couldn't read it: {why}"), now),
        };
        let Some(text) = String::from_utf8(bytes).ok().filter(|text| !text.contains('\0')) else {
            return self.say(l.pick(["仅支持编辑 UTF-8 编码的文本文件", "僅支援編輯 UTF-8 編碼的文字檔", "Only UTF-8 text files can be edited"]), now);
        };
        let l = editor::layout(&self.theme(), f);
        self.state.editor = Some(Editor::new(path, &text, l.cols as usize, l.rows as usize));
        self.state.layer = Layer::Letters;
        self.unsaved_warned = None;
        self.open(Screen::Editor);
    }

    /// How writing the file the editor page shows went: done, it stands as
    /// saved; or why not.
    pub fn text_saved(&mut self, outcome: Result<(), String>, now: u64) {
        match outcome {
            Ok(()) => {
                self.state.editor.as_mut().expect("the editor page shows a file").saved();
                self.refresh();
                self.say(self.state.lang.pick(["已保存", "已儲存", "Saved"]), now);
            }
            Err(why) => self.report(&tr!(self.state.lang, "保存失败：{why}", "儲存失敗：{why}", "Couldn't save it: {why}"), now),
        }
    }

    /// The name typed for an entry is another entry's in the folder.
    fn say_taken(&mut self, now: u64) {
        let name = &self.state.new_name;
        let said = tr!(self.state.lang, "该文件夹中已有\"{name}\"", "這個資料夾中已有「{name}」", "This folder already has “{name}”");
        self.say(&said, now);
    }

    /// The system asked for sideload before the recovery started: the page
    /// opens over the home page, already receiving.
    pub fn start_sideload(&mut self) {
        self.begin_install(Install::Starting, Source::Computer, None);
        self.open(Screen::Installing);
    }

    /// Start an install from `source` afresh, of the file named `name` when
    /// it is one on the phone: nothing the last install showed stays.
    fn begin_install(&mut self, install: Install, source: Source, name: Option<String>) {
        let s = &mut self.state;
        (s.install, s.source, s.package_name) = (install, source, name);
        s.script_output.clear();
        s.reflash = None;
    }

    /// What the system asked of the recovery before it started.
    pub fn set_asked(&mut self, asked: Asked) {
        self.state.asked = asked;
        self.refresh();
    }

    /// Erasing has begun, going through `parts`: its page is the only one
    /// until it ends. `reset` for a factory reset, `asked` when the system
    /// asked for it before the recovery started.
    pub fn erase_started(&mut self, reset: bool, asked: bool, parts: Vec<Part>) {
        self.state.erase = Erase { reset, asked, parts, done: 0, progress: None, outcome: None };
        self.stack = vec![(Screen::Erase, 0.0, Focus::Unset)];
        self.settle_page();
        self.refresh();
    }

    /// Erasing has come to `part`.
    pub fn erase_at(&mut self, part: Part) {
        let erase = &mut self.state.erase;
        erase.done = erase.parts.iter().position(|&p| p == part).expect("erasing comes only to the parts it set out with");
        erase.progress = None;
        self.refresh();
    }

    /// The part under way has come `done` entries of `total`.
    pub fn erase_progress(&mut self, done: u64, total: u64) {
        self.state.erase.progress = Some((done, total));
        self.refresh();
    }

    /// Erasing has ended. What comes next is the user's to choose and the
    /// panel lights for it, unless the system asked for the erasing and it
    /// went well: the phone then leaves on its own. A data partition
    /// erased whole opens no more until the system makes it afresh.
    pub fn erase_ended(&mut self, outcome: Result<(), EraseStop>, now: u64) {
        let erase = &mut self.state.erase;
        if outcome.is_ok() {
            erase.done = erase.parts.len();
        }
        erase.progress = None;
        let data_gone = erase.parts.iter().position(|&p| p == Part::Data).is_some_and(|k| k < erase.done);
        if outcome.is_err() || !erase.asked {
            self.wake(now);
        }
        if data_gone {
            self.state.data = Data::Erased;
        }
        self.state.erase.outcome = Some(outcome);
        self.refresh();
    }

    /// What the boot control holds of the slots, as the runtime read it, or
    /// why it could not.
    pub fn set_boot_slots(&mut self, slots: Result<BootSlots, String>) {
        self.state.boot_slots = Some(slots);
        self.refresh();
    }

    /// How switching the slot to start next went: what the boot control
    /// holds of the slots now, or why it did not switch.
    pub fn slot_switched(&mut self, outcome: Result<BootSlots, String>, now: u64) {
        match outcome {
            Ok(slots) => {
                self.state.next_slot = None;
                self.state.boot_slots = Some(Ok(slots));
                let slot = slots.active.letter();
                self.say(&tr!(self.state.lang, "下次将从槽位 {slot} 启动", "下次將從槽位 {slot} 啟動", "Slot {slot} starts next"), now);
                self.refresh();
            }
            Err(why) => self.say(&tr!(self.state.lang, "切换失败：{why}", "切換失敗：{why}", "Couldn't switch: {why}"), now),
        }
    }

    /// What was found in the places looked in for backups: the newest
    /// first, by when each was made, and those that cannot be read after
    /// them, by their folder's name, which is when each was begun.
    pub fn set_backups(&mut self, mut backups: Vec<BackupEntry>) {
        let key = |e: &BackupEntry| (e.held.is_ok(), e.held.as_ref().map_or(0, |h| h.made), e.name.clone());
        backups.sort_by_key(|e| std::cmp::Reverse(key(e)));
        self.state.backups = backups;
        self.refresh();
    }

    /// Look for backups in the places that can be read now.
    fn find_backups(&self) -> Effect {
        Effect::FindBackups { storage: self.state.place_open(Place::Storage), usb: self.state.place_open(Place::UsbDrive) }
    }

    /// Backing up, or restoring, has begun: its page shows how far it comes.
    /// `checks` when each part is read back to check it.
    pub fn backup_started(&mut self, restore: bool, place: Place, checks: bool) {
        self.state.backup_job = Some(BackupJob { restore, place, checks, plan: Vec::new(), at: None, outcome: None, stopping: false });
        self.open(Screen::BackupJob);
    }

    /// The parts the work goes through, once they are measured.
    pub fn backup_plan(&mut self, plan: Vec<(String, u64)>) {
        self.state.backup_job.as_mut().expect("only work under way is measured").plan = plan;
        if self.screen() == Screen::BackupJob {
            self.refresh();
        }
    }

    /// How far the work has come.
    pub fn backup_at(&mut self, at: BackupAt) {
        self.state.backup_job.as_mut().expect("only work under way moves on").at = Some(at);
        if self.screen() == Screen::BackupJob {
            self.refresh();
        }
    }

    /// Backing up or restoring has ended: its page says how, lit for the
    /// user to read.
    pub fn backup_ended(&mut self, end: BackupEnd, now: u64) {
        let job = self.state.backup_job.as_mut().expect("only work under way ends");
        job.stopping = false;
        job.outcome = Some(end);
        self.wake(now);
        self.refresh();
    }

    /// How deleting the backup the restore page shows went: deleted, the
    /// page goes, and the backups left are looked for; not, why.
    pub fn backup_deleted(&mut self, outcome: Result<(), String>, now: u64) -> Vec<Effect> {
        match outcome {
            Ok(()) => {
                self.state.restore = None;
                self.state.asking_delete = false;
                if self.screen() == Screen::RestoreBackup {
                    self.leave(true);
                    self.stack.pop();
                    self.settle_page();
                }
                self.refresh();
                self.say(self.state.lang.pick(["已删除", "已刪除", "Deleted"]), now);
                vec![self.find_backups()]
            }
            Err(why) => {
                self.report(&tr!(self.state.lang, "删除失败：{why}", "刪除失敗：{why}", "Couldn't delete it: {why}"), now);
                Vec::new()
            }
        }
    }

    /// The runtime could not mount or unmount a partition: the mount page
    /// says so, and why.
    pub fn mount_failed(&mut self, failure: MountFailure) {
        self.state.mount_failure = Some(failure);
        self.refresh();
    }

    /// The runtime listed a folder for the file manager at `listed`, in Unix
    /// seconds, or could not: it becomes the folder shown.
    pub fn set_files(&mut self, folder: String, files: Result<Vec<FileRow>, String>, listed: i64) {
        self.show_folder(Screen::Files, folder, files, listed);
    }

    /// The runtime listed a folder of `place` at `listed`, its path from
    /// the place's top, or could not: it becomes the folder shown to choose
    /// a file to install in, with what it holds that may be installed.
    pub fn set_place(&mut self, place: Place, folder: String, files: Result<Vec<FileRow>, String>, listed: i64) {
        let files = files.map(|entries| entries.into_iter().filter(installable).collect());
        self.show_folder(Screen::Choose(place), folder, files, listed);
    }

    /// The runtime found the USB drive plugged in, shown or not, or found
    /// none. Another drive, or none, starts its folders over from the top,
    /// and one shown while its page is up is read at once. The backups
    /// listed are looked for again, and a backup on a drive gone leaves its
    /// page.
    pub fn set_usb_drive(&mut self, drive: Option<UsbDrive>) -> Vec<Effect> {
        if drive == self.state.usb_drive {
            return Vec::new();
        }
        self.state.usb_drive = drive;
        self.state.drive = Folder::new("");
        self.refresh();
        let gone = !self.state.place_open(Place::UsbDrive) && self.state.restore.as_ref().is_some_and(|(e, _)| e.place == Place::UsbDrive);
        match self.screen() {
            Screen::RestoreBackup if gone => {
                self.state.restore = None;
                self.state.asking_delete = false;
                self.state.confirm = 0.0;
                self.leave(true);
                self.stack.pop();
                self.settle_page();
                return vec![self.find_backups()];
            }
            Screen::Restore => return vec![self.find_backups()],
            _ => {}
        }
        let page = Screen::Choose(Place::UsbDrive);
        if self.screen() != page {
            return Vec::new();
        }
        *self.top_mut() = (page, 0.0, Focus::Unset);
        self.fling = None;
        self.bounce = None;
        self.seek = None;
        self.pull = 0.0;
        self.snap = true;
        self.arrive_at(page)
    }

    /// `folder` becomes the one the list on page `screen` shows. A folder
    /// gone back up to shows where it was left, as the stock file manager
    /// does; any other starts from its top.
    fn show_folder(&mut self, screen: Screen, folder: String, files: Result<Vec<FileRow>, String>, listed: i64) {
        let left = &self.state.folder(screen).path;
        if folder != *left && self.screen() == screen {
            // Climbing out of a folder, the keys come back to it; any other
            // folder starts them at its top.
            let came_from = std::path::Path::new(left).strip_prefix(&folder).ok().and_then(|rest| rest.components().next());
            let name = came_from.map(|c| c.as_os_str().to_string_lossy().into_owned());
            let k = name.and_then(|name| files.as_ref().ok()?.iter().position(|e| e.name == name));
            let row = if screen == Screen::Files { Action::OpenEntry } else { Action::ChooseEntry };
            self.top_mut().2 = match k {
                Some(k) => Focus::On(Spot { stop: Stop::Hit(row(k)), group: Group::Page, rect: Rect::new(0.0, 0.0, 0.0, 0.0) }),
                None => Focus::Unset,
            };
            self.top_mut().1 = self.state.folder_mut(screen).scroll_for(&folder);
            self.fling = None;
            self.bounce = None;
            self.seek = None;
            self.pull = 0.0;
            self.snap = true;
        }
        // The entry the file page describes stands as the folder it is in
        // lists it again, as after its file is saved.
        if screen == Screen::Files && folder == self.state.files.path {
            if let (Some(file), Ok(entries)) = (self.state.file.as_mut(), &files) {
                if let Some(listed) = entries.iter().find(|e| e.name == file.name) {
                    *file = listed.clone();
                }
            }
        }
        let shown = self.state.folder_mut(screen);
        shown.path = folder;
        shown.entries = files;
        shown.listed = listed;
        self.refresh();
    }

    /// Render the current page if what it shows has changed: the rows of
    /// it on screen, the rest as they come into view; or, where only some
    /// rows changed, those rows.
    pub fn frame(&mut self, f: &Fonts, info: &DeviceInfo) -> &Frame {
        self.render(f, info, self.w)
    }

    /// [`App::frame`], drawing of a page rendered anew only its columns
    /// from the left edge to `wide`.
    fn render(&mut self, f: &Fonts, info: &DeviceInfo, wide: u32) -> &Frame {
        if self.frame.is_some() && !self.stale.is_empty() {
            let t = self.theme();
            let screen = self.screen();
            for rows in std::mem::take(&mut self.stale) {
                let band = Band { rows, cols: (0, self.cols) };
                let cx = Ctx { t: &t, f, info, state: &self.state, term: self.term.as_ref(), band: Some(band) };
                let fresh = pages::render(screen, &cx);
                let mut canvas = self.frame.take().expect("checked above").canvas;
                assert_eq!(canvas.h, fresh.canvas.h, "a page drawn again in part is as tall as it was");
                canvas.replace(&fresh.canvas);
                self.frame = Some(Frame { canvas, ..fresh });
            }
        }
        if self.frame.is_none() {
            let t = self.theme();
            let screen = self.screen();
            let first = self.top().1.max(0.0) as u32;
            let band = Band { rows: (first, first + self.h), cols: (0, wide.min(self.w)) };
            let cx = Ctx { t: &t, f, info, state: &self.state, term: self.term.as_ref(), band: Some(band) };
            let mut frame = pages::render(screen, &cx);
            // A page drawn in part across holds its rows whole, the columns
            // of them drawn on as they are needed.
            self.cols = frame.canvas.cols().end;
            if self.cols < self.w {
                let mut whole = Canvas::unfilled(self.w, frame.canvas.h, frame.canvas.held());
                whole.replace(&frame.canvas);
                frame.canvas = whole;
            }
            self.height = frame.height(&t);
            self.frame = Some(frame);
            self.clamp_scroll();
            if std::mem::take(&mut self.snap) {
                let (title, bar) = self.bar_wanted().expect("rendered above");
                self.title.jump(title);
                self.bar.jump(bar);
            }
        }
        self.frame.as_ref().expect("rendered above")
    }

    /// The current page rendered, all of it drawn.
    pub fn whole_frame(&mut self, f: &Fonts, info: &DeviceInfo) -> &Frame {
        let h = self.frame(f, info).canvas.h;
        self.draw_cols(f, info, self.w);
        self.draw_rows(f, info, 0.0, h as f32);
        self.frame.as_ref().expect("rendered above")
    }

    /// Draw a part of the current page not drawn yet: the rest of the
    /// columns of its rows drawn, then a band of rows below those or else
    /// above them, for a page at rest to have all of it drawn by the time
    /// it scrolls into view; that done, make the About page field's dither,
    /// for that page to open without making it. Whether any is left to do.
    pub fn draw_ahead(&mut self, f: &Fonts, info: &DeviceInfo) -> bool {
        let Some(frame) = &self.frame else { return false };
        let (held, h) = (frame.canvas.held(), frame.canvas.h);
        if self.cols < self.w {
            self.draw_cols(f, info, self.w);
        } else if held.end < h {
            self.draw_rows(f, info, held.end as f32, (held.end + 1) as f32);
        } else if held.start > 0 {
            self.draw_rows(f, info, (held.start - 1) as f32, held.start as f32);
        } else if self.dither.is_none() {
            self.dither = Some(glow::Dither::new(self.w as usize, self.h as usize));
        }
        self.cols < self.w || self.frame.as_ref().is_some_and(|fr| !fr.canvas.whole()) || self.dither.is_none()
    }

    /// Draw the columns of the current page's rows drawn from where they
    /// were drawn to, on to `wide`.
    fn draw_cols(&mut self, f: &Fonts, info: &DeviceInfo, wide: u32) {
        let wide = wide.min(self.w);
        let Some(frame) = &self.frame else { return };
        if self.cols >= wide {
            return;
        }
        let held = frame.canvas.held();
        let t = self.theme();
        let band = Band { rows: (held.start, held.end), cols: (self.cols, wide) };
        let cx = Ctx { t: &t, f, info, state: &self.state, term: self.term.as_ref(), band: Some(band) };
        let part = pages::render(self.screen(), &cx).canvas;
        self.frame.as_mut().expect("drawn above").canvas.replace(&part);
        self.cols = wide;
        self.layer = None;
    }

    /// Draw the rows `from..to` of the current page it has not drawn yet,
    /// a band at least a quarter of a screen tall at a time; the rows drawn
    /// are drawn across first.
    fn draw_rows(&mut self, f: &Fonts, info: &DeviceInfo, from: f32, to: f32) {
        let Some(frame) = &self.frame else { return };
        let h = frame.canvas.h;
        let (from, to) = ((from.max(0.0) as u32).min(h), (to.max(0.0).ceil() as u32).min(h));
        let held = frame.canvas.held();
        let least = self.h / 4;
        let mut bands = Vec::new();
        if from < held.start {
            bands.push((from.min(held.start.saturating_sub(least)), held.start));
        }
        if to > held.end {
            bands.push((held.end, to.max(held.end + least).min(h)));
        }
        if bands.is_empty() {
            return;
        }
        self.draw_cols(f, info, self.w);
        let t = self.theme();
        let screen = self.screen();
        for rows in bands {
            let band = Band { rows, cols: (0, self.w) };
            let cx = Ctx { t: &t, f, info, state: &self.state, term: self.term.as_ref(), band: Some(band) };
            let part = pages::render(screen, &cx).canvas;
            self.frame.as_mut().expect("drawn above").canvas.join(&part);
        }
        self.layer = None;
    }

    fn max_scroll(&self) -> f32 {
        (self.height - self.h as f32).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        let top = self.top_mut();
        top.1 = top.1.clamp(0.0, max);
    }

    /// Draw what the panel shows now: the visible part of the page, pulled
    /// past an end if it is, its large title fading as it scrolls, the bar
    /// and its title over a scrolled page, the `<`, the page sliding in or
    /// out over or under the page left, the status bar, any message, and
    /// the back cue while a swipe from an edge is under way.
    pub fn compose(&mut self, out: &mut Canvas, f: &Fonts, info: &DeviceInfo, clock: &str, now: u64) {
        // A page opened is drawn from its left edge as it slides in.
        let opening = self.turn.as_ref().is_some_and(|t| !t.back);
        self.render(f, info, if opening { 0 } else { self.w });
        let screen = self.screen();
        let scroll = self.top().1;
        let offset = (scroll + self.overscroll(now)).round();
        let caret_on = match self.frame.as_ref().is_some_and(|fr| fr.caret.is_some()) {
            true => self.caret_up(screen, now),
            false => {
                self.blink = None;
                self.caret_shown = false;
                false
            }
        };
        if let Some(m) = &self.morph {
            if !m.at(now).1 {
                return self.compose_morph(out, f, info, clock, now, caret_on);
            }
            self.morph = None;
        }
        let t = self.theme();
        if let Some((which, track)) = self.knob.take() {
            self.paint_knob(&t, f, which, track);
        }
        if self.toast.as_ref().is_some_and(|m| now >= m.1) {
            self.toast = None;
        }
        let title = self.frame.as_ref().expect("rendered above").title;
        let key = (title, self.state.dark);
        if self.titles.as_ref().is_none_or(|(k, _, _)| *k != key) {
            self.titles = Some((key, Blurs::new(widgets::big_title(&t, f, title)), Blurs::new(widgets::small_title(&t, f, title))));
        }
        if let Some(fold) = self.frame.as_ref().and_then(|fr| fr.fold) {
            let key = (fold.hint, self.state.dark);
            if self.search_words.as_ref().is_none_or(|(k, _)| *k != key) {
                self.search_words = Some((key, widgets::search_words(&t, f, fold.bar, fold.hint)));
            }
        }
        if self.chip.as_ref().is_none_or(|c| c.0 != t.dark) {
            let (art, x, y) = widgets::back_chip(&t);
            self.chip = Some((t.dark, art, x, y));
        }
        let button = self.frame.as_ref().is_some_and(|fr| fr.button.is_some());
        if button && self.button_chip.as_ref().is_none_or(|c| c.0 != t.dark) {
            let (art, x, y) = widgets::button_chip(&t);
            self.button_chip = Some((t.dark, art, x, y));
        }
        let slide = self.slid(now);
        if self.frame.as_ref().is_some_and(|fr| fr.hero.is_none()) {
            // What of the page is on screen, drawn if it has not been yet.
            // While it slides in, that is its columns come in, and a
            // quarter of the screen more each frame, so it is drawn across
            // early in the slide; all of it once a row of it is held.
            let wide = match (slide, &self.turn) {
                (Some((at, _, _)), Some(turn)) if !turn.back && self.row_press.is_none() => {
                    (self.w - at.clamp(0, self.w as i32) as u32).max(self.cols + self.w / 4)
                }
                _ => self.w,
            };
            self.draw_cols(f, info, wide);
            self.draw_rows(f, info, offset, offset + self.h as f32);
        }
        // The row a finger holds, drawn once more as held down when it
        // starts darkening, and only that row: a finger that moves on
        // before then never waits for it.
        if let (Some(press), None) = (self.row_press.filter(|p| now >= p.rise), &self.row_art) {
            let page = &self.frame.as_ref().expect("rendered above").canvas;
            let (x, y, w, h) = row_part(press.rect, page);
            let pressed = t.pressed();
            let band = Band { rows: (y, y + h), cols: (x, x + w) };
            let cx = Ctx { t: &pressed, f, info, state: &self.state, term: self.term.as_ref(), band: Some(band) };
            let down = pages::render(screen, &cx).canvas;
            assert_eq!((down.w, down.h), (page.w, page.h), "a page held down is laid out as it was");
            self.row_art = Some((page.crop(x, y, w, h), down.crop(x, y, w, h)));
        }
        let view = View {
            scroll,
            offset,
            bar: self.bar.level(now),
            title: self.title.level(now),
            light: self.light(offset),
            caret: caret_on,
            row: self.row_press.map_or(0.0, |p| p.level(now)),
            pressing: self.pressing(),
        };
        match slide {
            Some((at, shift, dim)) => {
                // The page shown is drawn on its own canvas, once for as
                // long as it shows the same, and slid from there with the
                // page left.
                // A page opened shows nothing of itself until its edge comes
                // in from the right: the page left alone shows.
                let back = self.turn.as_ref().expect("sliding").back;
                if !back && at >= self.w as i32 {
                    let left = &self.turn.as_ref().expect("sliding").left;
                    out.slide(left, left, at, shift, dim, self.corners);
                } else {
                    let still = self.frame.as_ref().is_some_and(|fr| fr.hero.is_none());
                    let page = match self.layer.take() {
                        Some((shown, c)) if still && shown == view => c,
                        kept => {
                            let mut c = kept.map(|(_, c)| c).unwrap_or_else(|| Canvas::new(out.w, out.h));
                            self.paint_page(&mut c, &t, Some((f, info)), &view);
                            c
                        }
                    };
                    let turn = self.turn.as_ref().expect("sliding");
                    let (front, behind) = if back { (&turn.left, &page) } else { (&page, &turn.left) };
                    out.slide(front, behind, at, shift, dim, self.corners);
                    self.layer = Some((view, page));
                }
            }
            None => {
                self.layer = None;
                self.paint_page(out, &t, Some((f, info)), &view);
            }
        }
        self.shown = Some(view);

        widgets::status_bar(out, &t, f, &self.bar(clock, info), t.pal.status);
        if let Some((text, _)) = &self.toast {
            widgets::toast(out, &t, f, text);
        }
        match self.drag {
            Drag::Edge { start, from_left, armed, x, cy } => {
                let travel = if from_left { x - start } else { start - x };
                widgets::back_cue(out, &t, from_left, cy, widgets::back_cue_depth(&t, travel), armed);
            }
            _ => {
                if let Some((from_left, cy, s)) = self.cue {
                    widgets::back_cue(out, &t, from_left, cy, s.at(now).0.max(0.0), false);
                }
            }
        }
    }

    /// Draw the page shown as `view` has it: all of it but the status bar,
    /// any message and the back cue, which lie over every page. Rows of its
    /// long list come into view are drawn with `fresh`; a page drawn again
    /// as it last showed has them drawn already.
    fn paint_page(&mut self, out: &mut Canvas, t: &Theme, fresh: Option<(&Fonts, &DeviceInfo)>, view: &View) {
        let m = t.m;
        let screen = self.screen();
        let (scroll, offset, pressing) = (view.scroll, view.offset, view.pressing);
        // The light goes over its control, in the part of the screen the
        // control is drawn in, under whatever is laid over that part.
        let shine = |out: &mut Canvas, part: Group| {
            if let Some((g, r, radius, tint)) = view.light {
                if g == part {
                    out.fill_round_rect(r.x, r.y, r.w, r.h, radius, tint);
                }
            }
        };
        let frame = self.frame.as_ref().expect("a page is drawn from its frame");
        let (_, big, small) = self.titles.as_mut().expect("a page's titles are drawn with it");
        // The row a finger holds darkens as part of the page, under what is
        // laid over the page.
        let press_row = |out: &mut Canvas| {
            if let (Some(press), Some((was, is))) = (self.row_press, &self.row_art) {
                let (x, y, _, _) = row_part(press.rect, &frame.canvas);
                out.relay(was, is, x as i32, y as i32 - offset as i32, view.row);
            }
        };

        if let Some(art) = &frame.hero {
            // The field fills the screen and fades as the page scrolls; the
            // cards scroll over it. The version and the author's line fade
            // and shrink away first, then the wordmark.
            let glow = self.glow.as_ref().expect("the About page keeps its field");
            let fade = (1.0 - scroll / m.dp(ABOUT_FIELD_DP)).clamp(0.0, 1.0);
            let top = (widgets::bar_bottom(t) + m.dp(ABOUT_FIELD_DP)) / m.h as f32;
            let field = glow::sample(glow, out.w as usize, out.h as usize, FIELD_CELL, top, t.pal.bg, fade);
            let dither = self.dither.get_or_insert_with(|| glow::Dither::new(out.w as usize, out.h as usize));
            glow::paint(out, &field, fade, &glow::Layer { canvas: &frame.canvas, runs: &art.runs, offset: offset as isize }, dither);
            press_row(out);
            let range = m.dp(ABOUT_LOGO_DP) - widgets::bar_bottom(t);
            let v = (scroll / range).clamp(0.0, 1.0);
            art.version.draw_scaled(out, 0.0, 0.0, 1.0 - 0.1 * v, t.hero.version, 1.0 - v);
            art.author.draw_scaled(out, 0.0, 0.0, 1.0 - 0.1 * v, t.hero.version, 1.0 - v);
            let l = ((scroll - range) / m.dp(ABOUT_LOGO_FADE_DP)).clamp(0.0, 1.0);
            glow::ink_wordmark(out, &field, &glow.look().ink, &art.wordmark, 1.0 - 0.1 * l, 1.0 - l, fade, dither);
        } else {
            // The page rows on screen above a long list's rows, those rows,
            // and the page colour past the ends; every pixel is written
            // once.
            let h = out.h as f32;
            let span = frame.rows.as_ref().map(|r| ((r.edges[0] - offset).clamp(0.0, h), (r.bottom() - offset).clamp(0.0, h)));
            let (list_top, list_end) = span.unwrap_or((h, h));
            let first = (-offset).clamp(0.0, h) as usize;
            let last = (frame.canvas.h as f32 - offset).clamp(first as f32, list_top) as usize;
            if first < last {
                let from = offset as i32 + first as i32;
                out.put_rows(&frame.canvas, from, from + (last - first) as i32, first as i32);
            }
            out.fill_rect(0.0, 0.0, out.w as f32, first as f32, frame.bg);
            out.fill_rect(0.0, last as f32, out.w as f32, list_top - last as f32, frame.bg);
            let below = list_end.max(last as f32);
            out.fill_rect(0.0, below, out.w as f32, h - below, frame.bg);
            if let Some(rows) = &frame.rows {
                // A long list's rows on screen, each drawn when it first
                // shows. Those more than half a screen away are let go, and
                // their canvases drawn over again for the rows coming in, as
                // a list view recycles its rows.
                let near = rows.within(offset - h * NEAR_SCREENS, offset + h * (1.0 + NEAR_SCREENS));
                self.spare.extend(self.drawn.extract_if(.., |(i, _)| !near.contains(i)).map(|(_, c)| c));
                for i in rows.within(offset, offset + h) {
                    let k = match self.drawn.iter().position(|(j, _)| *j == i) {
                        Some(k) => k,
                        None => {
                            let (f, info) = fresh.expect("a page drawn again as it last showed has its rows in view drawn");
                            let mut c = self.spare.pop().unwrap_or_else(|| Canvas::new(out.w, 0));
                            c.resize((rows.edges[i + 1] - rows.edges[i]) as u32);
                            let cx = Ctx { t, f, info, state: &self.state, term: self.term.as_ref(), band: None };
                            pages::row(screen, &cx, i, &mut c);
                            self.drawn.push((i, c));
                            self.drawn.len() - 1
                        }
                    };
                    out.put(&self.drawn[k].1, (rows.edges[i] - offset) as i32);
                }
            } else {
                // Off a long list, its rows' canvases are not kept.
                self.spare.clear();
            }
            press_row(out);
            if let Some(fold) = &frame.fold {
                let (_, words) = self.search_words.as_ref().expect("a folding search bar's words are made with its page");
                fold.draw(out, t, words, offset);
            }
            if frame.header {
                let level = (1.0 - frame.title_lift(scroll) / m.dp(TITLE_FADE_DP)).clamp(0.0, 1.0);
                draw_faded(out, big, 0.0, -frame.title_lift(offset), t.pal.text, level, (1.0 - level) * m.dp(TITLE_BLUR_DP));
            }
        }
        // The search page's round button and the home page's search bar
        // darken at once under a finger.
        if let Some(a) = pressing.filter(|a| matches!(a, Action::CloseSearch | Action::Open(Screen::Search))) {
            let r = frame.hits.iter().find(|h| h.action == a).expect("a button held down is on its page").rect;
            match frame.shown(r, offset) {
                Some(r) if a == Action::CloseSearch => widgets::cancel_held(out, t, r),
                Some(r) => widgets::search_bar_held(out, t, r),
                None => {}
            }
        }
        if let Some(caret) = frame.caret.as_ref().filter(|_| view.caret) {
            out.lay(&caret.art, caret.x, caret.y - offset as i32, 1.0);
        }
        let bar = view.bar;
        let strip = frame.pinned.as_ref().map(|p| (p, pinned_top(t, p, offset)));
        shine(out, Group::Page);
        if bar > 0.0 {
            match strip {
                // The strip under the bar is veiled with it, and the list
                // thins out below the strip.
                Some((p, top)) => widgets::strip_mask(out, t, frame.bg, bar, top.round(), top.round() + p.canvas.h as f32),
                None => {
                    let stops = if frame.hero.is_some() { &widgets::ABOUT_BAR_MASK[..] } else { &widgets::BAR_MASK[..] };
                    widgets::bar_mask(out, t, stops, frame.bg, bar);
                }
            }
        }
        if let Some((p, top)) = strip {
            out.put(&p.canvas, top.round() as i32);
            shine(out, Group::Strip);
        }
        if let Some(d) = &frame.dock {
            out.put(&d.canvas, (out.h - d.canvas.h) as i32);
            shine(out, Group::Dock);
        }
        if frame.back {
            let (_, chip, x, y) = self.chip.as_ref().expect("a page's `<` is drawn with it");
            out.lay(chip, *x, *y, bar);
            if pressing == Some(Action::Back) {
                widgets::back_held(out, t);
            } else {
                widgets::back_arrow(out, t, t.pal.text);
            }
        }
        if let Some((glyph, action)) = frame.button {
            let (_, chip, x, y) = self.button_chip.as_ref().expect("a page's round button is drawn with its chip");
            out.lay(chip, *x, *y, bar);
            if pressing == Some(action) {
                widgets::button_held(out, t, glyph);
            } else {
                widgets::button_glyph(out, t, glyph, t.pal.text);
            }
        }
        if frame.back || frame.button.is_some() {
            shine(out, Group::Fixed);
        }
        let level = view.title;
        let blur = if frame.hero.is_some() { 0.0 } else { (1.0 - level) * m.dp(TITLE_BLUR_DP) };
        draw_faded(out, small, 0.0, 0.0, t.pal.text, level, blur);
    }

    /// Where a page sliding in over the page left, or the page left
    /// sliding off, stands at `now`: how many pixels from the left edge the
    /// page in front stands, how many left of where it rests the page
    /// behind, and how dark that is; nothing once the slide is over. See
    /// [`Turn`].
    fn slid(&mut self, now: u64) -> Option<(i32, i32, f32)> {
        let turn = self.turn.as_mut()?;
        let spring = *turn.spring.get_or_insert_with(|| {
            if turn.back {
                Spring::new(now, -1000.0, 0.0, BACK_PERIOD)
            } else {
                Spring::new(now, 1000.0, -OPEN_SPEED * 1000.0, OPEN_PERIOD)
            }
        });
        let (x, done) = spring.at(now);
        if done {
            self.backdrop = self.turn.take().map(|t| t.left);
            return None;
        }
        // Where the page in front stands, in screen widths from the left
        // edge.
        let along = if turn.back { 1.0 + x / 1000.0 } else { x / 1000.0 };
        let w = self.w as f32;
        Some(((along * w).round() as i32, (BEHIND_SHIFT * w * (1.0 - along)).round() as i32, BEHIND_DIM * (1.0 - along)))
    }

    /// Whether the cursor of the text field on `screen` shows at `now`: it
    /// comes up whole on each page it shows on and at each edit, then
    /// blinks.
    fn caret_up(&mut self, screen: Screen, now: u64) -> bool {
        let key = (screen, self.edits);
        if self.blink.is_none_or(|(s, e, _)| (s, e) != key) {
            self.blink = Some((screen, self.edits, now));
        }
        self.caret_shown = self.blink_at(now).is_some_and(|(on, _)| on);
        self.caret_shown
    }

    /// Draw the search bar on its way between the home page and the search
    /// page; see [`Morph`].
    fn compose_morph(&mut self, out: &mut Canvas, f: &Fonts, info: &DeviceInfo, clock: &str, now: u64, caret_on: bool) {
        let t = self.theme();
        let m = t.m;
        let mut morph = self.morph.take().expect("composing it");
        let (q, _) = morph.at(now);
        let (field, _) = widgets::search_layout(&t);
        let home = Rect::new(m.margin, m.content_top - morph.scroll, m.w as f32 - m.margin * 2.0, m.search_h);
        let along = |a: f32, b: f32| a + (b - a) * q;
        let bar = Rect::new(m.margin, along(home.y, field.y), along(home.w, field.w), m.search_h);
        let rise = bar.y - home.y;
        if !morph.into {
            // The home page's rows coming down with its bar.
            self.draw_rows(f, info, morph.scroll - rise, morph.scroll - rise + m.h as f32);
        }
        let frame = self.frame.as_ref().expect("rendered above");
        out.fill_rect(0.0, 0.0, out.w as f32, out.h as f32, frame.bg);
        if morph.into {
            // The search page's keyboard and what it shows under its
            // field, from its frame; that comes in once the bar is nearly
            // there.
            let (_, keys_top) = *morph.keyboard.get_or_insert_with(|| keyboard_strip(&t, frame));
            let (body, top) = morph.body.get_or_insert_with(|| {
                let top = (field.y + field.h).ceil() as i32;
                let mut body = Canvas::new(frame.canvas.w, (keys_top - top).max(0) as u32);
                body.put_rows(&frame.canvas, top, keys_top, 0);
                (body, top)
            });
            let (delay, span) = MORPH_BODY_MS;
            out.lay(body, 0, *top, (now.saturating_sub(morph.start + delay) as f32 / span as f32).min(1.0));
        } else {
            // The home page under its bar comes down with it.
            let below = m.content_top + m.search_h;
            out.put_rows(&frame.canvas, below.floor() as i32, frame.canvas.h as i32, (below - morph.scroll + rise).floor() as i32);
        }
        let title = morph.title.get_or_insert_with(|| Blurs::new(widgets::big_title(&t, f, pages::HOME_TITLE)));
        let level = (1.0 - q / MORPH_TITLE_GONE).clamp(0.0, 1.0);
        draw_faded(out, title, 0.0, rise - morph.scroll, t.pal.text, level, (1.0 - level) * m.dp(TITLE_BLUR_DP));
        if let Some((keys, top)) = &morph.keyboard {
            out.put(keys, (*top as f32 + (1.0 - q) * keys.h as f32).round() as i32);
        }
        widgets::search_capsule(out, &t, f, bar, self.state.lang.pick(pages::SEARCH_HINT));
        // The cursor shows from the start, going up with the field.
        if let Some(caret) = frame.caret.as_ref().filter(|_| morph.into && caret_on) {
            out.lay(&caret.art, caret.x, caret.y + (bar.y - field.y).round() as i32, 1.0);
        }
        let (disc, cross) = morph.cancel.get_or_insert_with(|| {
            let (disc, cross) = widgets::cancel_art(&t);
            (Blurs::new(disc), Blurs::new(cross))
        });
        let shown = ((q - MORPH_CANCEL.0) / (MORPH_CANCEL.1 - MORPH_CANCEL.0)).clamp(0.0, 1.0);
        let blur = (1.0 - shown) * m.dp(MORPH_CANCEL_BLUR_DP);
        draw_faded(out, disc, 0.0, 0.0, t.pal.search, shown, blur);
        draw_faded(out, cross, 0.0, 0.0, t.pal.text, shown, blur);
        self.morph = Some(morph);
        widgets::status_bar(out, &t, f, &self.bar(clock, info), t.pal.status);
        if let Some((text, _)) = &self.toast {
            widgets::toast(out, &t, f, text);
        }
    }

    /// Put the page back where it rests, for a page just opened or returned
    /// to; its first frame shows the bar as its scroll has it.
    fn settle_page(&mut self) {
        self.height = 0.0;
        self.fling = None;
        self.glide = None;
        self.pull = 0.0;
        self.bounce = None;
        self.seek = None;
        self.adjusting = false;
        self.titles = None;
        self.glow = None;
        self.row_press = None;
        self.row_art = None;
        let hero = self.screen() == Screen::About;
        self.title = title_fade(hero);
        self.bar = bar_fade(hero);
        self.snap = true;
    }

    /// Show the user what the runtime has to say about work it was asked
    /// to do.
    pub fn report(&mut self, text: &str, now: u64) {
        self.say(text, now);
    }

    /// The screenshot [`Effect::Screenshot`] asked for went where `saved`
    /// says, or did not, as the system says why.
    pub fn screenshot_saved(&mut self, saved: Result<Shot, String>, now: u64) {
        let l = self.state.lang;
        let text = match saved {
            Ok(Shot::Album) => l.pick(fmt::SCREENSHOT_SAVED).to_string(),
            Ok(Shot::Held) => l.pick(fmt::SCREENSHOT_HELD).to_string(),
            Err(why) => fmt::screenshot_failed(l, &why),
        };
        self.say(&text, now);
    }

    /// The screenshots held while the owner's storage was closed did not go
    /// into the gallery's once it opened, as the system says why.
    pub fn screenshots_not_kept(&mut self, why: &str, now: u64) {
        self.say(&fmt::screenshots_not_kept(self.state.lang, why), now);
    }

    /// File transfer could not be turned on or off as the switch says: it
    /// shows how it stands, and why.
    pub fn file_transfer_failed(&mut self, on: bool, why: &str, now: u64) {
        self.state.file_transfer = on;
        self.refresh();
        self.say(why, now);
    }

    fn say(&mut self, text: &str, now: u64) {
        self.toast = Some((text.to_string(), now + TOAST_MS));
        self.dirty = true;
    }

    /// Open `screen` over the page shown, sliding in over it.
    fn open(&mut self, screen: Screen) {
        self.leave(false);
        self.push(screen);
    }

    fn push(&mut self, screen: Screen) {
        self.stack.push((screen, 0.0, Focus::Unset));
        self.settle_page();
        self.refresh();
    }

    /// The page shown gives way to another, opened over it or, `back`, gone
    /// back to under it: draw it as it last showed, to slide away. Changes
    /// made before the slide's first frame keep the page that showed; a
    /// page rendered anew since it last showed gives way at once.
    fn leave(&mut self, back: bool) {
        if let Some(turn) = self.turn.as_mut().filter(|t| t.spring.is_none()) {
            turn.back = back;
            return;
        }
        self.end_turn();
        let Some(view) = self.shown.take() else { return };
        let mut left = self.backdrop.take().unwrap_or_else(|| Canvas::new(self.w, self.h));
        // The `<` comes up with the finger, before its page goes.
        let view = View { pressing: None, ..view };
        let t = self.theme();
        self.paint_page(&mut left, &t, None, &view);
        // A page left within the first frames of sliding in, before it was
        // drawn across, shows its colour where it was not drawn yet.
        if let Some(fr) = self.frame.as_ref().filter(|_| self.cols < self.w) {
            left.fill_rect(self.cols as f32, 0.0, (self.w - self.cols) as f32, self.h as f32, fr.bg);
        }
        self.turn = Some(Turn { back, left, spring: None });
        self.dirty = true;
    }

    /// Set the home page's search bar moving up to the search page's field,
    /// or back down, before the page changes: from where a move not yet
    /// done has it, at the speed it has.
    fn start_morph(&mut self, into: bool, now: u64) {
        let (q, speed, title, cancel) = match self.morph.take() {
            Some(m) => {
                let (q, _) = m.at(now);
                let speed = (q - m.at(now.saturating_sub(8)).0) / 0.008;
                (q, speed, m.title, m.cancel)
            }
            None => (if into { 0.0 } else { 1.0 }, 0.0, None, None),
        };
        let target = if into { 1.0 } else { 0.0 };
        // The home page is the page shown, or the one under the search.
        let home = self.stack.len() - if into { 1 } else { 2 };
        // On the way down the keyboard goes with the search page's frame.
        let keyboard = if into { None } else { self.frame.as_ref().map(|fr| keyboard_strip(&self.theme(), fr)) };
        self.morph = Some(Morph {
            into,
            spring: Spring::new(now, (q - target) * 1000.0, speed * 1000.0, MORPH_PERIOD),
            start: now,
            scroll: self.stack[home].1,
            keyboard,
            body: None,
            title,
            cancel,
        });
        self.dirty = true;
    }

    /// Go back a page. Leaving the installing page stops what can be
    /// stopped; while a package is being read or written, the page stays.
    /// In the file manager, back climbs a folder until it reaches the root,
    /// and in a place, until it reaches the place's top.
    fn back(&mut self, now: u64) -> Vec<Effect> {
        let (l, mut fx) = (self.state.lang, Vec::new());
        if self.stack.len() <= 1 {
            return fx;
        }
        let morph = self.screen() == Screen::Search && self.stack[self.stack.len() - 2].0 == Screen::Home;
        if morph {
            self.start_morph(false, now);
        }
        let left = self.screen();
        match left {
            Screen::Mount => self.state.mount_failure = None,
            // A slot picked and not switched to is put down.
            Screen::Slot => self.state.next_slot = None,
            Screen::Files => {
                if let Some(up) = self.state.files.up() {
                    return vec![Effect::ReadDir(up)];
                }
                // Leaving the file manager puts a picked entry down.
                self.state.clip = None;
            }
            Screen::Choose(place) => {
                if let Some(up) = self.state.folder(Screen::Choose(place)).up() {
                    return vec![Effect::ReadPlace(place, up)];
                }
            }
            Screen::FileInfo => {
                self.state.file = None;
                self.state.asking_delete = false;
            }
            // Changes not saved are given up only by going back again while
            // the warning shows.
            Screen::Editor => {
                let changed = self.state.editor.as_ref().expect("the editor page shows a file").changed;
                if changed && self.unsaved_warned.is_none_or(|at| now >= at + TOAST_MS) {
                    self.unsaved_warned = Some(now);
                    self.say(l.pick(["修改尚未保存，再次返回将放弃修改", "修改尚未儲存，再次返回將捨棄修改", "Changes aren't saved. Go back again to discard them"]), now);
                    return fx;
                }
                self.state.editor = None;
                self.unsaved_warned = None;
            }
            // Turning the choice down leaves the entry picked, for another
            // folder.
            Screen::Clash => self.state.clash = None,
            Screen::WlanPassword => {
                self.state.joining = None;
                self.state.password.clear();
            }
            Screen::WlanNetwork => self.state.details = None,
            // A list open over the page closes first.
            Screen::WlanAdd => {
                if let Some(a) = self.state.adding.as_mut().filter(|a| a.choosing) {
                    a.choosing = false;
                    self.refresh();
                    return fx;
                }
                self.state.adding = None;
                self.state.password.clear();
            }
            Screen::FileJob => {
                if self.state.job.as_ref().is_some_and(|j| j.outcome.is_none()) {
                    self.say(l.pick(WAIT_OR_STOP), now);
                    return fx;
                }
                self.state.job = None;
            }
            Screen::Relabel => {
                if self.state.relabel.as_ref().is_some_and(|j| j.outcome.is_none()) {
                    self.say(l.pick(WAIT_OR_STOP), now);
                    return fx;
                }
                self.state.relabel = None;
            }
            // Work on a partition cannot be stopped halfway. Done, the
            // partition may hold another filesystem, and what is to be done
            // to it is chosen afresh.
            Screen::PartWork => {
                if self.state.part_work.as_ref().is_some_and(|w| w.outcome.is_none()) {
                    self.say(l.pick(["请等待完成后再返回", "請等待完成後再返回", "Wait for it to finish before going back"]), now);
                    return fx;
                }
                self.state.part_work = None;
                self.state.fs_action = FsAction::Repair;
            }
            Screen::RestoreBackup => {
                self.state.restore = None;
                self.state.asking_delete = false;
            }
            // A restore that has begun writing cannot be stopped.
            Screen::BackupJob => {
                let job = self.state.backup_job.as_ref().expect("the backup page shows work");
                if job.outcome.is_none() {
                    let writing = job.restore && job.at.is_some_and(|a| !a.checking);
                    let said = if writing { ["正在恢复，请等待恢复完成", "正在還原，請等待還原完成", "Restoring. Wait for it to finish"] } else { WAIT_OR_STOP };
                    self.say(l.pick(said), now);
                    return fx;
                }
                self.state.backup_job = None;
            }
            _ => {}
        }
        if self.screen() == Screen::Installing {
            match self.state.install {
                // Checking a signature only reads the package, and stops
                // when the page is left.
                Install::Checking { stage: Some(Stage::Signature { .. }), .. } => {
                    self.state.install = Install::Ending;
                    fx.push(Effect::Cancel);
                }
                Install::Checking { .. } => {
                    self.say(l.pick(["正在读取文件，请稍后再试", "正在讀取檔案，請稍後再試", "Reading the file. Try again shortly"]), now);
                    return fx;
                }
                Install::Updating { .. } | Install::Scripting { .. } => {
                    self.say(l.pick(["正在安装，请等待安装完成", "正在安裝，請等待安裝完成", "Installing. Wait for it to finish"]), now);
                    return fx;
                }
                Install::Flashing { .. } => {
                    self.say(l.pick(["正在刷入，请等待刷入完成", "正在刷入，請等待刷入完成", "Flashing. Wait for it to finish"]), now);
                    return fx;
                }
                Install::Opening => {
                    self.say(l.pick(DATA_OPENING), now);
                    return fx;
                }
                Install::Wiping => {
                    self.say(l.pick(["正在清除，请等待清除完成", "正在清除，請等待清除完成", "Wiping. Wait for it to finish"]), now);
                    return fx;
                }
                Install::Starting
                | Install::Locked(_)
                | Install::Waiting
                | Install::Choosing(_)
                | Install::Confirming { .. }
                | Install::ConfirmingScript => {
                    self.state.install = Install::Ending;
                    fx.push(Effect::Cancel);
                }
                // Rescue waits on for the computer's next command until
                // its page is left.
                Install::Finished { .. } | Install::Wiped(_) if self.state.source == Source::Rescue => {
                    self.state.install = Install::Ending;
                    fx.push(Effect::Cancel);
                }
                Install::Finished { .. } | Install::Wiped(_) => self.state.install = Install::Idle,
                Install::Idle | Install::Ending => {}
            }
        }
        if !morph {
            self.leave(true);
        }
        self.stack.pop();
        self.state.confirm = 0.0;
        self.settle_page();
        self.refresh();
        // Coming back to the list of backups looks for them again: one may
        // have been made, deleted or opened up to be read since. So does
        // coming back to the partitions after work on one, which may have
        // changed what it holds.
        match self.screen() {
            Screen::Restore => fx.push(self.find_backups()),
            Screen::Filesystem | Screen::Filesystems if left == Screen::PartWork => fx.push(Effect::ReadFilesystems),
            _ => {}
        }
        fx
    }

    /// Hear from the installer. Once the installing page is left, only its
    /// end matters. An image waiting for its partition, an update or a
    /// script waiting to be agreed to, and the end light a dark panel: all
    /// want the user.
    pub fn install(&mut self, ev: Event, now: u64) {
        if self.state.install == Install::Ending {
            if matches!(ev, Event::Finished(_)) {
                self.state.install = Install::Idle;
            }
            return;
        }
        // In rescue, erasing all user data and leaving are the runtime's to
        // carry out, which tells the page of them itself.
        if matches!(ev, Event::Wipe | Event::Reboot(_)) {
            return;
        }
        if matches!(ev, Event::Image { .. } | Event::Ready { .. } | Event::Script | Event::Done(_) | Event::Finished(_)) {
            self.wake(now);
        }
        let install = &mut self.state.install;
        match ev {
            Event::Waiting => *install = Install::Waiting,
            Event::Checking { size } => *install = Install::Checking { size, stage: None },
            Event::Ready { version, wipe } => *install = Install::Confirming { version, wipe },
            Event::Installing { version } => *install = Install::Updating { version, stage: None, overwriting: false },
            Event::Overwriting => {
                if let Install::Updating { overwriting, .. } = install {
                    *overwriting = true;
                }
            }
            Event::Image { kind, size, partitions } => {
                *install = Install::Choosing(Choice { kind, size, partitions, partition: 0, pick: Pick::Current })
            }
            Event::Script => *install = Install::ConfirmingScript,
            Event::Running => {
                *install = Install::Scripting { stage: None, running: true };
                self.state.script_output.clear();
            }
            Event::Print(line) => {
                let shown = &mut self.state.script_output;
                if shown.len() == SCRIPT_LINES {
                    shown.remove(0);
                }
                shown.push(line);
            }
            Event::Progress(stage) => match install {
                Install::Checking { stage: shown, .. }
                | Install::Updating { stage: shown, .. }
                | Install::Flashing { stage: shown, .. }
                | Install::Scripting { stage: shown, .. } => *shown = Some(stage),
                _ => {}
            },
            Event::Reflashed(reflash) => self.state.reflash = Some(reflash),
            Event::Wipe | Event::Reboot(_) => unreachable!("the runtime carries out what rescue asks of it"),
            Event::Done(outcome) | Event::Finished(outcome) => {
                let damage = match (&outcome, &*install) {
                    (Outcome::Failed(_), Install::Updating { overwriting: true, .. }) => Some(Damage::System),
                    (Outcome::Failed(_), Install::Flashing { partition, stage: Some(Stage::Flashing { done, .. }) }) if *done > 0 => {
                        Some(Damage::Partition(partition))
                    }
                    (Outcome::Failed(_), Install::Scripting { running: true, .. }) => Some(Damage::Script),
                    _ => None,
                };
                *install = Install::Finished { outcome, damage };
            }
        }
        if self.screen() == Screen::Installing {
            self.refresh();
        }
    }

    /// What the page shown first asks of the runtime as it shows.
    pub fn arrive(&mut self) -> Vec<Effect> {
        self.arrive_at(self.screen())
    }

    /// What a page asks of the runtime as it opens: what it shows read, or
    /// started. The decrypt page opens the data partition, which is also
    /// how it learns which control a user's screen lock is answered with.
    fn arrive_at(&mut self, s: Screen) -> Vec<Effect> {
        match s {
            Screen::Mount => vec![Effect::ReadMounts],
            Screen::Filesystems => vec![Effect::ReadFilesystems],
            Screen::Slot | Screen::Reboot => vec![Effect::ReadSlots],
            Screen::Restore => vec![self.find_backups()],
            Screen::Files => vec![Effect::ReadDir(self.state.files.path.clone())],
            Screen::Choose(place) if self.state.place_open(place) => vec![Effect::ReadPlace(place, self.state.folder(s).path.clone())],
            Screen::Search => {
                self.state.search.clear();
                self.state.layer = Layer::Letters;
                Vec::new()
            }
            Screen::Terminal => {
                self.state.layer = Layer::Letters;
                if self.term.is_none() {
                    self.shell_folder = "/".into();
                    vec![Effect::StartShell(self.shell_folder.clone())]
                } else {
                    Vec::new()
                }
            }
            Screen::Decrypt if matches!(self.state.data, Data::Closed | Data::Failed(_)) => self.open_data(),
            // As the stock updater does, the page asks the update server as
            // it opens, once a network is joined and nothing has been heard.
            Screen::Update => {
                let mut fx = vec![Effect::ReadUpdate];
                if self.state.update_server && matches!(self.state.check, Check::Idle | Check::Failed(_)) && self.state.online() {
                    self.state.check = Check::Asking;
                    fx.push(Effect::CheckUpdate);
                }
                fx
            }
            Screen::Wlan if self.state.wlan.as_ref().is_some_and(|w| w.power == Power::On) => vec![Effect::ScanWlan],
            _ => Vec::new(),
        }
    }

    /// Where the update stands, as the runtime read it or a merge has come.
    pub fn set_update(&mut self, standing: Standing) {
        if self.state.update != standing {
            self.state.update = standing;
            self.refresh();
        }
    }

    /// Open the data partition, the decrypt page saying so meanwhile.
    fn open_data(&mut self) -> Vec<Effect> {
        self.state.data = Data::Opening;
        self.refresh();
        vec![Effect::OpenData]
    }

    /// Carry out a tapped control.
    fn act(&mut self, action: Action, now: u64) -> Vec<Effect> {
        let l = self.state.lang;
        match action {
            Action::Open(s) => {
                if s == Screen::Search && self.screen() == Screen::Home {
                    // The bar moves up from where it stands unfolded: a
                    // page easing there for the keys gets there at once.
                    if let Some((rest, _)) = self.seek.take() {
                        self.top_mut().1 = rest;
                    }
                    self.start_morph(true, now);
                    self.push(s);
                } else {
                    self.open(s);
                }
                return self.arrive_at(s);
            }
            Action::OpenData => return self.open_data(),
            Action::Skip => {
                self.leave(true);
                if self.stack.len() > 1 {
                    self.stack.pop();
                } else {
                    *self.top_mut() = (Screen::Home, 0.0, Focus::Unset);
                }
                self.settle_page();
                self.refresh();
            }
            Action::Back | Action::CloseSearch => return self.back(now),
            Action::Theme { dark } => {
                if self.state.dark != dark {
                    self.state.dark = dark;
                    self.refresh();
                }
            }
            // The switch moves at once; the runtime then tells how the WLAN
            // came up, or went off.
            Action::Toggle(Switch::Wlan) => {
                let w = self.state.wlan.as_mut().expect("the WLAN switch shows on a phone whose WLAN is brought up");
                let on = matches!(w.power, Power::Off | Power::Failed(_));
                *w = Wlan { power: if on { Power::Starting } else { Power::Off }, ..Wlan::default() };
                self.refresh();
                return vec![Effect::Wlan(on)];
            }
            Action::Toggle(switch) => {
                let on = match switch {
                    Switch::Vibrate => &mut self.state.vibrate,
                    Switch::FileTransfer => &mut self.state.file_transfer,
                    Switch::Compress => &mut self.state.compress,
                    Switch::Verify => &mut self.state.verify,
                    Switch::Signature => &mut self.state.check_signature,
                    Switch::KeepRecovery => &mut self.state.keep_recovery,
                    Switch::ShowPassword => &mut self.state.show_password,
                    Switch::Wlan => unreachable!("the WLAN switch is turned above"),
                };
                *on = !*on;
                let now_on = *on;
                self.refresh();
                if switch == Switch::FileTransfer {
                    return vec![Effect::FileTransfer(now_on)];
                }
            }
            Action::MountToggle(i) => {
                if self.state.mount_failure.take().is_some() {
                    self.refresh();
                }
                let row = &self.state.mounts.as_ref().expect("a partition toggled is one the page lists")[i];
                let base = row.base.clone();
                return vec![if row.mounted { Effect::Unmount(base) } else { Effect::Mount(base) }];
            }
            Action::RebootTarget(i) => {
                self.state.reboot = i;
                self.refresh();
            }
            // Everything takes in every part, so choosing it leaves the
            // parts out, and choosing a part on its own leaves it out.
            Action::WipeItem(item) => {
                let k = WIPES.iter().position(|&w| w == item).expect("the page offers only what it lists");
                let on = !self.state.wipe[k];
                match (on, item) {
                    (true, Wipe::All) => {
                        self.state.wipe = [false; WIPES.len()];
                        self.state.wipe[k] = true;
                    }
                    (true, _) => {
                        self.state.wipe[0] = false;
                        self.state.wipe[k] = true;
                    }
                    (false, _) => self.state.wipe[k] = false,
                }
                self.refresh();
            }
            Action::Confirm(_) => self.say(l.pick(["请按住圆形按钮向右滑动", "請按住圓形按鈕向右滑動", "Hold the round button and slide it right"]), now),
            Action::Slider(_) | Action::Pattern => {}
            Action::Sideload => match self.state.install {
                Install::Idle | Install::Finished { .. } => {
                    self.begin_install(Install::Starting, Source::Computer, None);
                    if self.screen() == Screen::Installing {
                        self.refresh();
                    } else {
                        self.open(Screen::Installing);
                    }
                    return vec![Effect::Sideload];
                }
                _ => self.say(l.pick(INSTALL_UNFINISHED), now),
            },
            Action::FlashPartition(k) => {
                if let Install::Choosing(choice) = &mut self.state.install {
                    choice.partition = k;
                    self.refresh();
                }
            }
            Action::FlashSlots(pick) => {
                if let Install::Choosing(choice) = &mut self.state.install {
                    choice.pick = pick;
                    self.refresh();
                }
            }
            Action::RebootTo(target) => return vec![Effect::Reboot(target)],
            Action::EnterRecovery => return vec![Effect::EnterRecovery],
            Action::Dismiss => {
                self.stack = vec![(Screen::Home, 0.0, Focus::Unset)];
                self.settle_page();
                self.refresh();
                return vec![Effect::Dismiss];
            }
            Action::PickEntry(op) => {
                let path = self.entry_path();
                self.state.clip = Some(Clip { op, path });
                return self.back(now);
            }
            // A folder cannot go inside itself, and an entry moved to the
            // folder it is in stays put: the entry stays picked for another
            // folder.
            Action::Paste => {
                let clip = self.state.clip.clone().expect("only a picked entry is pasted");
                let (path, folder) = (std::path::Path::new(&clip.path), std::path::Path::new(&self.state.files.path));
                let name = path.file_name().map_or_else(|| clip.path.clone(), |n| n.to_string_lossy().into_owned());
                if folder.starts_with(path) {
                    self.say(&tr!(l, "无法将\"{name}\"放入其自身中", "無法將「{name}」放入其本身中", "“{name}” can't go inside itself"), now);
                } else if clip.op == FileOp::Move && path.parent() == Some(folder) {
                    self.say(&tr!(l, "\"{name}\"已在该文件夹中", "「{name}」已在這個資料夾中", "“{name}” is already in this folder"), now);
                } else {
                    return vec![Effect::Paste { op: clip.op, path: clip.path, into: self.state.files.path.clone(), clash: None }];
                }
            }
            Action::Unpick => {
                self.state.clip = None;
                self.refresh();
            }
            Action::RenameEntry => {
                self.state.new_name = self.state.file.as_ref().expect("the file page shows an entry").name.clone();
                self.state.layer = Layer::Letters;
                self.open(Screen::Rename);
            }
            Action::EditEntry => return vec![Effect::OpenText(self.entry_path())],
            Action::InstallEntry => {
                let size = self.state.file.as_ref().expect("the file page shows an entry").size;
                return self.install_opened(self.entry_path(), size, now);
            }
            Action::SaveText => {
                let ed = self.state.editor.as_ref().expect("the editor page shows a file");
                return vec![Effect::SaveText { path: ed.path.clone(), text: ed.text() }];
            }
            Action::OpenPermissions => self.open(Screen::Permissions),
            Action::ModeBit(bit) => {
                let mode = self.state.file.as_ref().expect("the permissions page shows an entry").mode;
                return vec![Effect::SetMode { path: self.entry_path(), mode: (mode ^ bit) & 0o7777 }];
            }
            Action::AskDelete => {
                self.state.asking_delete = true;
                self.state.confirm = 0.0;
                self.refresh();
            }
            Action::Resolve(choice) => {
                let (clip, into, _) = self.state.clash.take().expect("a choice answers a clash");
                self.leave(true);
                self.stack.pop();
                self.settle_page();
                self.refresh();
                // Skipping the one entry picked leaves nothing to do.
                if choice == ClashChoice::Skip {
                    self.state.clip = None;
                    return Vec::new();
                }
                return vec![Effect::Paste { op: clip.op, path: clip.path, into, clash: Some(choice) }];
            }
            Action::StopJob => {
                let job = self.state.job.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopJob];
                }
            }
            Action::StopRelabel => {
                let job = self.state.relabel.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopRelabel];
                }
            }
            Action::OpenFilesystem(k) => {
                self.state.fs_chosen = k;
                self.state.fs_action = FsAction::Repair;
                self.open(Screen::Filesystem);
            }
            Action::ChooseFs(choice) => {
                let row = self.fs_row().expect("the page of a partition shows one that was read");
                let action = match choice {
                    FsChoice::Repair => FsAction::Repair,
                    FsChoice::Remake(k) => FsAction::Remake(row.others[k].clone()),
                };
                self.state.fs_action = action;
                self.refresh();
            }
            Action::ScreenOff(k) => {
                self.state.screen_off = k;
                self.refresh();
            }
            Action::TimeZone(k) => {
                self.state.time_zone = k;
                self.refresh();
            }
            Action::Language(l) => {
                self.state.lang = l;
                self.refresh();
            }
            Action::BackupPart(k) => {
                self.state.backup[k] = !self.state.backup[k];
                self.refresh();
            }
            Action::BackupTo(place) => {
                self.state.backup_to = place;
                self.refresh();
            }
            // All a backup holds is ticked to restore at first.
            Action::OpenBackup(i) => {
                let entry = self.state.backups[i].clone();
                let parts = entry.held.as_ref().map_or([false; 4], |h| h.parts);
                self.state.restore = Some((entry, parts));
                self.state.asking_delete = false;
                self.open(Screen::RestoreBackup);
            }
            Action::RestorePart(k) => {
                let (_, parts) = self.state.restore.as_mut().expect("the restore page shows a backup");
                parts[k] = !parts[k];
                self.refresh();
            }
            Action::StopBackup => {
                let job = self.state.backup_job.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopBackup];
                }
            }
            Action::LogPart(k) => {
                self.state.logs[k] = !self.state.logs[k];
                self.refresh();
            }
            Action::LogsTo(k) => {
                self.state.logs_to = k;
                self.refresh();
            }
            Action::ExportLogs => {
                let ([recovery, kernel], storage) = (self.state.logs, self.state.logs_to == 1);
                if !recovery && !kernel {
                    self.say(l.pick(["请选择要导出的日志", "請選擇要匯出的記錄", "Choose the logs to export"]), now);
                } else if storage && !self.state.data.storage_open() {
                    self.say(l.pick(DECRYPT_FIRST), now);
                } else {
                    return vec![Effect::ExportLogs { recovery, kernel, storage }];
                }
            }
            Action::PickSlot(slot) => {
                self.state.next_slot = Some(slot);
                self.refresh();
            }
            Action::Type(cap) if self.screen() == Screen::Terminal => return self.term_typed(cap),
            Action::Type(cap) if self.screen() == Screen::Editor => self.edit_typed(cap),
            Action::Type(cap) => return self.type_key(cap, now),
            Action::TermKey(key) if self.screen() == Screen::Editor => {
                let ed = self.state.editor.as_mut().expect("the editor page shows a file");
                match key {
                    TermKey::Tab => ed.insert('\t'),
                    TermKey::Left => ed.left(),
                    TermKey::Up => ed.up(),
                    TermKey::Down => ed.down(),
                    TermKey::Right => ed.right(),
                    TermKey::Esc | TermKey::Ctrl => unreachable!("the editor's row has no {key:?}"),
                }
                self.edits += 1;
                self.refresh_text();
            }
            Action::TermKey(key) => {
                let Some(term) = self.term.as_mut() else { return Vec::new() };
                let bytes = term.key(key);
                let running = term.running;
                self.refresh();
                if running && !bytes.is_empty() {
                    return vec![Effect::TermInput(bytes)];
                }
            }
            Action::ClearSearch => {
                self.state.search.clear();
                self.edits += 1;
                self.refresh();
            }
            Action::OpenEntry(i) => {
                let entry = self.state.files.entry(i).clone();
                match entry.kind {
                    FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => {
                        let scroll = self.top().1;
                        self.state.files.leave(scroll);
                        return vec![Effect::ReadDir(self.state.files.inside(&entry.name))];
                    }
                    FileKind::File if matches!(files::file_type(&entry.name), FileType::Text | FileType::Code) => {
                        return vec![Effect::OpenText(self.state.files.inside(&entry.name))];
                    }
                    _ if installs(&entry) => {
                        let path = self.state.files.inside(&entry.name);
                        return self.install_opened(path, entry.size, now);
                    }
                    FileKind::File | FileKind::Link { folder: false, .. } | FileKind::Special => {
                        self.state.file = Some(entry);
                        self.open(Screen::FileInfo);
                    }
                }
            }
            // A shell already running goes to the folder; otherwise one starts
            // there.
            Action::TerminalHere => {
                let folder = self.state.files.path.clone();
                let running = self.term.as_ref().is_some_and(|term| term.running);
                self.open(Screen::Terminal);
                self.state.layer = Layer::Letters;
                if running {
                    return vec![Effect::ShellTo(folder)];
                }
                self.shell_folder = folder.clone();
                return vec![Effect::StartShell(folder)];
            }
            Action::ChooseEntry(i) => {
                let screen = self.screen();
                let Screen::Choose(place) = screen else { unreachable!("only a place's folder is chosen in") };
                let folder = self.state.folder(screen);
                let entry = folder.entry(i).clone();
                let path = folder.inside(&entry.name);
                match entry.kind {
                    FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => {
                        let scroll = self.top().1;
                        self.state.folder_mut(screen).leave(scroll);
                        return vec![Effect::ReadPlace(place, path)];
                    }
                    FileKind::File | FileKind::Link { folder: false, .. } => return self.install_file(place, path, entry.size, now),
                    FileKind::Special => unreachable!("a place lists no special files"),
                }
            }
            Action::OpenAncestor(depth) => {
                let screen = self.screen();
                if let Some(path) = self.state.folder(screen).ancestor(depth) {
                    return vec![match screen {
                        Screen::Choose(place) => Effect::ReadPlace(place, path),
                        _ => Effect::ReadDir(path),
                    }];
                }
            }
            Action::JoinNetwork(i) => {
                let w = self.state.wlan.as_ref().expect("networks show on a phone whose WLAN is brought up");
                // A scan may have changed the list since the page was drawn.
                let Some(network) = w.networks.get(i).cloned() else { return Vec::new() };
                if w.joined.as_ref().is_some_and(|j| j.ssid == network.ssid) {
                    return self.act(Action::OpenJoined, now);
                }
                match network.security {
                    Security::Wep => self.say(l.pick(["暂不支持 WEP 加密的网络", "尚不支援 WEP 加密的網路", "WEP networks aren't supported"]), now),
                    Security::Enterprise => {
                        self.say(l.pick(["暂不支持需要账号登录的网络", "尚不支援需要帳號登入的網路", "Networks that need an account sign-in aren't supported"]), now)
                    }
                    s if s.password() => self.ask_password(network),
                    _ => {
                        return vec![Effect::JoinWlan {
                            ssid: network.ssid,
                            name: network.name,
                            security: network.security,
                            password: Secret::default(),
                        }]
                    }
                }
            }
            Action::NetworkDetails(i) => {
                let w = self.state.wlan.as_ref().expect("networks show on a phone whose WLAN is brought up");
                let Some(network) = w.networks.get(i).cloned() else { return Vec::new() };
                self.state.details = Some(network);
                self.open(Screen::WlanNetwork);
            }
            Action::OpenJoined => {
                let w = self.state.wlan.as_ref().expect("a network is chosen on a phone whose WLAN is brought up");
                let Some(j) = w.joined.clone() else { return Vec::new() };
                // As found, where it still is; as it was chosen otherwise.
                let found = w.networks.iter().find(|n| n.ssid == j.ssid).cloned();
                let network = found.unwrap_or(Network {
                    ssid: j.ssid,
                    name: j.name,
                    signal: 0,
                    frequency: 0,
                    bands: Default::default(),
                    security: j.security,
                });
                if j.stage == Joining::WrongPassword {
                    self.ask_password(network);
                } else {
                    self.state.details = Some(network);
                    self.open(Screen::WlanNetwork);
                }
            }
            Action::LeaveNetwork => {
                let mut fx = self.back(now);
                fx.push(Effect::LeaveWlan);
                return fx;
            }
            Action::Rescan => return vec![Effect::ScanWlan],
            Action::AddNetwork => {
                self.state.adding = Some(Adding { name: String::new(), security: Security::Open, field: Field::Name, choosing: false });
                self.state.password.clear();
                self.state.show_password = false;
                self.state.layer = Layer::Letters;
                self.open(Screen::WlanAdd);
            }
            Action::TypeInto(field) => {
                let a = self.state.adding.as_mut().expect("the fields are on the page that adds a network");
                if a.field != field {
                    a.field = field;
                    self.refresh();
                }
            }
            Action::ChooseSecurity => {
                self.state.adding.as_mut().expect("the protection is on the page that adds a network").choosing = true;
                // The page draws the row held down while its list is open.
                self.row_press = None;
                self.row_art = None;
                self.refresh();
            }
            Action::PickSecurity(security) => {
                let a = self.state.adding.as_mut().expect("the protections open on the page that adds a network");
                a.security = security;
                a.choosing = false;
                // With no password to type, the keyboard types the name.
                if !security.password() {
                    a.field = Field::Name;
                }
                self.refresh();
            }
            Action::CloseChoices => {
                self.state.adding.as_mut().expect("the protections open on the page that adds a network").choosing = false;
                self.refresh();
            }
            // Asking takes a network, which the WLAN page joins.
            Action::CheckUpdate => {
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if self.state.check != Check::Asking {
                    self.state.check = Check::Asking;
                    self.refresh();
                    return vec![Effect::CheckUpdate];
                }
            }
            // The package goes to internal storage, which the decrypt page
            // opens, over a network the WLAN page joins.
            Action::Download => {
                let release = self.state.newest().expect("the download is offered for the newest system").clone();
                if !self.state.data.storage_open() {
                    self.open(Screen::Decrypt);
                    return self.arrive_at(Screen::Decrypt);
                }
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if !self.state.downloading() {
                    self.state.download = Some(DownloadJob { progress: None, rate: 0, outcome: None, stopping: false });
                    self.refresh();
                    return vec![Effect::Download(release)];
                }
            }
            Action::StopDownload => {
                let job = self.state.download.as_mut().expect("only a download under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopDownload];
                }
            }
            Action::InstallDownloaded => {
                let name = self.state.newest().expect("the package installed is the newest system's").name.clone();
                let Have::Whole(size) = self.state.have else { unreachable!("only a whole package is offered to install") };
                return self.install_file(Place::Storage, format!("{DOWNLOADS}/{name}"), size, now);
            }
            Action::Unavailable(text) => self.say(text, now),
        }
        Vec::new()
    }

    /// The text the keyboard types into on this page.
    fn typing(&mut self) -> &mut String {
        match self.screen() {
            Screen::Search => &mut self.state.search,
            Screen::Decrypt => &mut self.state.code,
            Screen::Rename => &mut self.state.new_name,
            Screen::WlanPassword => &mut self.state.password,
            Screen::WlanAdd => {
                let a = self.state.adding.as_mut().expect("the page adds a network");
                match a.field {
                    Field::Name => &mut a.name,
                    Field::Password => &mut self.state.password,
                }
            }
            s => panic!("{s:?} has no text field"),
        }
    }

    /// Where the entry the file page shows is.
    fn entry_path(&self) -> String {
        let entry = self.state.file.as_ref().expect("the file page shows an entry");
        self.state.files.inside(&entry.name)
    }

    /// Install the file of `size` bytes at `path` in `place`, its page
    /// following how it goes; one install at a time.
    fn install_file(&mut self, place: Place, path: String, size: u64, now: u64) -> Vec<Effect> {
        let name = file_name(&path);
        self.install_named(Source::File(place), name, size, Effect::InstallFile(place, path), now)
    }

    /// Install the file of `size` bytes at the full `path`, opened in the
    /// file manager, as a file chosen in a place installs.
    fn install_opened(&mut self, path: String, size: u64, now: u64) -> Vec<Effect> {
        let source = Source::Opened { drive: std::path::Path::new(&path).starts_with(USB_DRIVE) };
        let name = file_name(&path);
        self.install_named(source, name, size, Effect::InstallPath(path), now)
    }

    /// Install the file `name` of `size` bytes from `source`, `start` asking
    /// the runtime to, its page following how it goes; one install at a
    /// time.
    fn install_named(&mut self, source: Source, name: Option<String>, size: u64, start: Effect, now: u64) -> Vec<Effect> {
        match self.state.install {
            Install::Idle | Install::Finished { .. } => {
                self.begin_install(Install::Checking { size, stage: None }, source, name);
                self.open(Screen::Installing);
                vec![start]
            }
            _ => {
                self.say(self.state.lang.pick(INSTALL_UNFINISHED), now);
                Vec::new()
            }
        }
    }

    /// A key of the keyboard on the terminal page. The layer keys change
    /// the keyboard; every other key goes to the shell, and once the shell
    /// has ended, Enter starts another.
    fn term_typed(&mut self, cap: Cap) -> Vec<Effect> {
        match cap {
            Cap::Shift => self.state.layer = if self.state.layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted },
            Cap::Show(layer) => self.state.layer = layer,
            _ => {}
        }
        let shifted = self.state.layer == Layer::Shifted;
        if shifted && matches!(cap, Cap::Char(_)) {
            self.state.layer = Layer::Letters;
        }
        self.refresh();
        let Some(term) = self.term.as_mut() else { return Vec::new() };
        if !term.running {
            return if cap == Cap::Enter { vec![Effect::StartShell(self.shell_folder.clone())] } else { Vec::new() };
        }
        let bytes = term.typed(cap, shifted);
        if bytes.is_empty() {
            Vec::new()
        } else {
            vec![Effect::TermInput(bytes)]
        }
    }

    /// Send what was entered at the screen lock of the user whose own
    /// files wait. It is taken as it is sent, so the dots clear while it is
    /// checked, and nothing more is entered until it has been.
    fn unlock(&mut self, code: String) -> Vec<Effect> {
        let Data::Locked { user, keys, .. } = self.state.data else { unreachable!("only a screen lock waiting takes what is entered") };
        self.state.data = Data::Locked { user, keys, checking: true };
        self.refresh();
        vec![Effect::Unlock { user, code }]
    }

    /// A key of the keyboard on the editor page: a character or a space
    /// goes in at the cursor, Enter breaks the line there and backspace
    /// takes away what is before it; the layer keys change the keyboard.
    fn edit_typed(&mut self, cap: Cap) {
        let layer = self.state.layer;
        let ed = self.state.editor.as_mut().expect("the editor page shows a file");
        match cap {
            Cap::Char(ch) if layer == Layer::Shifted => {
                ed.insert(ch.to_ascii_uppercase());
                self.state.layer = Layer::Letters;
            }
            Cap::Char(ch) => ed.insert(ch),
            Cap::Space => ed.insert(' '),
            Cap::Backspace => ed.backspace(),
            Cap::Enter => ed.newline(),
            Cap::Shift => self.state.layer = if layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted },
            Cap::Show(shown) => self.state.layer = shown,
        }
        self.edits += 1;
        // The keyboard stays as it was while its keys do.
        if self.state.layer == layer {
            self.refresh_text();
        } else {
            self.refresh();
        }
    }

    /// The editor's text or cursor changed: draw again what lies above the
    /// keyboard, which stays as it was.
    fn refresh_text(&mut self) {
        let t = self.theme();
        self.refresh_rows(0, (t.m.h as f32 - widgets::keyboard_height(&t)).floor() as u32);
    }

    /// A tap at page `x`, `y` on the editor's card puts the cursor between
    /// the characters nearest it.
    fn place_caret(&mut self, f: &Fonts, x: f32, y: f32) {
        let l = editor::layout(&self.theme(), f);
        if !l.card.contains(x, y) {
            return;
        }
        let ed = self.state.editor.as_mut().expect("the editor page shows a file");
        let row = (((y - l.y0) / l.pitch).floor().max(0.0) as usize).min(l.rows as usize - 1);
        ed.place(ed.top() + row, (x - l.x0) / l.cell);
        self.edits += 1;
        self.refresh_text();
    }

    /// Scroll the terminal's screen or the editor's text as a finger goes
    /// from `last` to `y`: a row for each row's height.
    fn scroll_history(&mut self, last: f32, y: f32, carry: f32, pitch: f32) -> Drag {
        let carry = carry + (y - last);
        let rows = (carry / pitch).trunc();
        if rows != 0.0 {
            self.scroll_rows(rows as isize);
        }
        Drag::History { last: y, carry: carry - rows * pitch, pitch }
    }

    /// Scroll as a finger going down `rows` rows' heights does, or up for
    /// fewer than none: the terminal's screen back to what the shell wrote
    /// before, the editor's text to the lines before.
    fn scroll_rows(&mut self, rows: isize) {
        if self.screen() == Screen::Editor {
            self.state.editor.as_mut().expect("the editor page shows a file").scroll(-rows);
            self.refresh_text();
        } else if let Some(term) = self.term.as_mut() {
            term.scroll(rows);
            self.refresh();
        }
    }

    /// A key of the on-screen keyboard. A capital lasts one letter; return
    /// opens the first result of a search, and runs a terminal's command.
    fn type_key(&mut self, cap: Cap, now: u64) -> Vec<Effect> {
        let (l, layer) = (self.state.lang, self.state.layer);
        match cap {
            Cap::Char(ch) => {
                let shifted = self.state.layer == Layer::Shifted;
                self.typing().push(if shifted { ch.to_ascii_uppercase() } else { ch });
                self.edits += 1;
                if shifted {
                    self.state.layer = Layer::Letters;
                }
            }
            Cap::Space => {
                self.typing().push(' ');
                self.edits += 1;
            }
            Cap::Backspace => {
                self.typing().pop();
                self.edits += 1;
            }
            Cap::Shift => {
                self.state.layer = if self.state.layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted };
            }
            Cap::Show(layer) => self.state.layer = layer,
            Cap::Enter => match self.screen() {
                Screen::Search => {
                    if let Some(first) = search::find_on(&self.state.search, l, self.state.wlan.is_some()).first() {
                        return self.act(Action::Open(first.screen), now);
                    }
                }
                Screen::Decrypt if self.state.code.is_empty() => self.say(l.pick(["请输入锁屏密码", "請輸入螢幕鎖定密碼", "Enter the screen lock"]), now),
                Screen::Decrypt => {
                    let code = std::mem::take(&mut self.state.code);
                    return self.unlock(code);
                }
                // A name the folder cannot take is turned down here, in
                // the user's words; an unchanged one leaves the page.
                Screen::Rename => {
                    let name = self.state.new_name.clone();
                    let current = &self.state.file.as_ref().expect("the file page shows an entry").name;
                    let taken = self.state.files.entries.as_ref().is_ok_and(|entries| entries.iter().any(|e| e.name == name));
                    match name.as_str() {
                        "" => self.say(l.pick(["请输入名称", "請輸入名稱", "Enter a name"]), now),
                        "." | ".." => self.say(l.pick(["名称不能为\".\"或\"..\"", "名稱不能是「.」或「..」", "A name can't be “.” or “..”"]), now),
                        _ if name.contains('/') => self.say(l.pick(["名称不能包含\"/\"", "名稱不能包含「/」", "A name can't hold “/”"]), now),
                        _ if name == *current => return self.back(now),
                        _ if taken => self.say_taken(now),
                        _ => return vec![Effect::Rename { path: self.entry_path(), name }],
                    }
                }
                // A password the network's protection cannot take is turned
                // down here; one it can goes, and the WLAN page shows how
                // joining goes.
                Screen::WlanPassword => {
                    let network = self.state.joining.clone().expect("the password page is for a network");
                    match refused(l, network.security, &self.state.password) {
                        Some(why) => self.say(why, now),
                        None => {
                            let password = Secret(std::mem::take(&mut self.state.password));
                            let mut fx = self.back(now);
                            fx.push(Effect::JoinWlan { ssid: network.ssid, name: network.name, security: network.security, password });
                            return fx;
                        }
                    }
                }
                // So is a name no network can have; the keyboard then types
                // into the field turned down.
                Screen::WlanAdd => {
                    let a = self.state.adding.as_mut().expect("the page adds a network");
                    let (field, why) = match a.name.len() {
                        0 => (Field::Name, Some(l.pick(["请输入网络名称", "請輸入網路名稱", "Enter the network's name"]).to_string())),
                        n if n > SSID_MAX => (
                            Field::Name,
                            Some(tr!(l, "网络名称不能超过 {SSID_MAX} 个字符", "網路名稱不能超過 {SSID_MAX} 個字元", "A network's name can't be over {SSID_MAX} characters")),
                        ),
                        _ if a.security.password() => (Field::Password, refused(l, a.security, &self.state.password).map(str::to_string)),
                        _ => (Field::Name, None),
                    };
                    match why {
                        Some(why) => {
                            if a.field != field {
                                a.field = field;
                                self.refresh();
                            }
                            self.say(&why, now)
                        }
                        None => {
                            let (name, security) = (a.name.clone(), a.security);
                            let password = match security.password() {
                                true => Secret(std::mem::take(&mut self.state.password)),
                                false => Secret::default(),
                            };
                            let mut fx = self.back(now);
                            fx.push(Effect::JoinWlan { ssid: name.clone().into_bytes(), name, security, password });
                            return fx;
                        }
                    }
                }
                other => unreachable!("{other:?} shows no keyboard"),
            },
        }
        // What is typed shows above the keyboard, which stays as it was
        // while its keys do.
        if matches!(cap, Cap::Char(_) | Cap::Space | Cap::Backspace) && self.state.layer == layer {
            let t = self.theme();
            self.refresh_rows(0, (t.m.h as f32 - widgets::keyboard_height(&t)).floor() as u32);
        } else {
            self.refresh();
        }
        Vec::new()
    }

    pub fn touch(&mut self, ev: Touch, f: &Fonts, now: u64) -> Vec<Effect> {
        if !self.display_on {
            // A finger on the dark panel lights it, and does nothing else
            // until it lifts.
            if matches!(ev, Touch::Down(..)) {
                self.wake(now);
                self.drag = Drag::Waking;
            }
            return Vec::new();
        }
        // A finger landing just after the panel lit only keeps it awake.
        if matches!(ev, Touch::Down(..)) && self.just_woke(now) {
            self.stir(now);
            self.drag = Drag::Waking;
            return Vec::new();
        }
        // A finger on the glass puts the keys' light out, whatever a key
        // held down was doing comes to nothing, and a page sliding comes to
        // rest.
        if matches!(ev, Touch::Down(..)) {
            if self.keys {
                self.keys = false;
                self.adjusting = false;
                self.dirty = true;
            }
            self.drop_button();
            self.end_turn();
        }
        self.stir(now);
        let t = self.theme();
        let m = t.m;
        let (slop, edge) = (m.dp(8.0), m.dp(20.0));
        let scroll = self.top().1;
        let mut fx = Vec::new();
        let pressing = self.pressing();
        match ev {
            Touch::Down(x, y) => {
                self.tracker.clear();
                self.tracker.add(now, y);
                let side = if x < edge {
                    Some(true)
                } else if x > m.w as f32 - edge {
                    Some(false)
                } else {
                    None
                };
                // The first page has nothing to go back to: its edges are
                // like the rest of it.
                let at_edge = side.filter(|_| self.stack.len() > 1);
                // A swipe in from an edge goes back over whatever the page is
                // doing: a finger landing there leaves a gliding or springing
                // page alone until it drags the page, and presses nothing.
                if at_edge.is_some() && self.in_motion(now) {
                    self.drag = Drag::Press { x, y, action: None, edge: at_edge, at: now };
                    return fx;
                }
                // A terminal's screen or the editor's text gliding stops
                // under a finger landing on it, which holds it to scroll on.
                let glided = self.glide.take().filter(|(glide, ..)| !glide.at(now).1);
                let history = self.frame.as_ref().and_then(|fr| fr.scrollback).filter(|(r, _)| r.contains(x, y + scroll));
                if let (Some(_), Some((_, pitch))) = (glided, history) {
                    self.drag = Drag::History { last: y, carry: 0.0, pitch };
                    return fx;
                }
                // Anywhere else it stops such a page and holds it; the
                // control under the finger is not pressed.
                if self.catch(now) {
                    self.drag = Drag::Scroll { last: y };
                    return fx;
                }
                // The `<` stays put while the page scrolls; under the bar
                // only it answers. A pinned strip answers where it shows,
                // over the rows beneath it, and so does a docked one.
                let back = widgets::back_rect(&t);
                let strip = self.frame.as_ref().and_then(|fr| fr.pinned.as_ref()).map(|p| (p, pinned_top(&t, p, scroll)));
                let on_strip = strip.filter(|&(p, top)| y >= top && y < top + p.canvas.h as f32);
                let dock = self.frame.as_ref().and_then(|fr| fr.dock.as_ref()).map(|d| (d, self.h as f32 - d.canvas.h as f32));
                let on_dock = dock.filter(|&(_, top)| y >= top);
                let button = self.frame.as_ref().and_then(|fr| fr.button).map(|(_, a)| (a, widgets::button_rect(&t)));
                let hit = if self.frame.as_ref().is_some_and(|fr| fr.back) && back.contains(x, y) {
                    Some((Action::Back, back))
                } else if let Some(button) = button.filter(|(_, r)| r.contains(x, y)) {
                    Some(button)
                } else if let Some((d, top)) = on_dock {
                    d.hits.iter().find(|h| h.rect.contains(x, y - top)).map(|h| (h.action, Rect { y: h.rect.y + top, ..h.rect }))
                } else if let Some((p, top)) = on_strip {
                    p.hits.iter().find(|h| h.rect.contains(x, y - top)).map(|h| (h.action, Rect { y: h.rect.y + top, ..h.rect }))
                } else if self.bar.on && y < widgets::bar_bottom(&t) {
                    None
                } else {
                    // A search bar folding away answers only on what is
                    // left of it.
                    let fr = self.frame.as_ref();
                    fr.and_then(|fr| fr.hit(x, y + scroll).filter(|h| fr.shown(h.rect, scroll).is_some_and(|r| r.contains(x, y))))
                        .map(|h| (h.action, h.rect))
                };
                let action = hit.map(|h| h.0);
                // A row of the page darkens behind its words a moment after
                // the finger lands on it, if the finger stays.
                let row = hit
                    .filter(|&(a, r)| self.frame.as_ref().is_some_and(|fr| fr.hits.contains(&Hit { rect: r, action: a })))
                    .filter(|&(a, r)| row_wide(&t, r) && !matches!(a, Action::Open(Screen::Search) | Action::Confirm(_) | Action::TypeInto(_)))
                    .map(|(_, r)| r);
                self.drag = match hit {
                    Some((Action::Slider(which), track)) => {
                        self.set_slider(&t, which, track, x);
                        Drag::Slider(which, track)
                    }
                    Some((Action::Confirm(which), track)) if self.on_knob(track, x) => {
                        let knob_x = track.x + (track.w - track.h) * self.state.confirm;
                        Drag::Confirm { which, track, grab: x - knob_x }
                    }
                    Some((Action::Pattern, _)) => {
                        self.state.pattern.clear();
                        self.trace(&t, x, y + scroll);
                        Drag::Pattern
                    }
                    _ => {
                        if let Some(rect) = row {
                            self.row_press = Some(RowPress { rect, rise: now + ROW_PRESS_DELAY_MS, fall: None });
                            self.row_art = None;
                        }
                        Drag::Press { x, y, action, edge: at_edge, at: now }
                    }
                };
            }
            Touch::Move(x, y) => match std::mem::replace(&mut self.drag, Drag::Idle) {
                Drag::Press { x: x0, y: y0, action, edge: at_edge, at } => {
                    self.tracker.add(now, y);
                    let (dx, dy) = (x - x0, y - y0);
                    self.drag = match at_edge {
                        Some(from_left) if dx.abs() > slop && dx.abs() > dy.abs() => {
                            self.dirty = true;
                            Drag::Edge { start: x0, from_left, armed: false, x, cy: y0 - m.dp(CUE_LIFT_DP) }
                        }
                        _ if dy.abs() > slop => {
                            let history = self.frame.as_ref().and_then(|fr| fr.scrollback).filter(|(r, _)| r.contains(x0, y0 + scroll));
                            if let Some((_, pitch)) = history {
                                self.scroll_history(y0, y, 0.0, pitch)
                            } else {
                                // A page still gliding or springing back is
                                // caught where it has got to; one at rest
                                // follows the finger from where it came down.
                                if !self.catch(now) {
                                    self.drag_by(y0 - y);
                                }
                                Drag::Scroll { last: y }
                            }
                        }
                        _ => Drag::Press { x: x0, y: y0, action, edge: at_edge, at },
                    };
                    if !matches!(self.drag, Drag::Press { .. }) {
                        self.let_row_go(now);
                    }
                }
                Drag::Scroll { last } => {
                    self.tracker.add(now, y);
                    self.drag_by(last - y);
                    self.drag = Drag::Scroll { last: y };
                }
                Drag::Edge { start, from_left, armed, cy, .. } => {
                    let travel = if from_left { x - start } else { start - x };
                    let ready = travel > m.dp(BACK_SWIPE_DP);
                    // A tap of the motor as the `<` appears, as the system
                    // gives.
                    if ready && !armed && self.state.vibrate {
                        fx.push(Effect::Haptic);
                    }
                    self.drag = Drag::Edge { start, from_left, armed: ready, x, cy };
                    self.dirty = true;
                }
                Drag::Slider(which, track) => {
                    self.set_slider(&t, which, track, x);
                    self.drag = Drag::Slider(which, track);
                }
                Drag::Confirm { which, track, grab } => {
                    let travel = (track.w - track.h).max(1.0);
                    self.state.confirm = ((x - grab - track.x) / travel).clamp(0.0, 1.0);
                    self.repaint_confirm(&t, f, which, track);
                    self.drag = Drag::Confirm { which, track, grab };
                }
                Drag::Pattern => {
                    self.trace(&t, x, y + scroll);
                    self.drag = Drag::Pattern;
                }
                Drag::History { last, carry, pitch } => {
                    self.tracker.add(now, y);
                    self.drag = self.scroll_history(last, y, carry, pitch);
                }
                Drag::Waking => self.drag = Drag::Waking,
                Drag::Held => self.drag = Drag::Held,
                Drag::Idle => {}
            },
            Touch::Up => match std::mem::replace(&mut self.drag, Drag::Idle) {
                Drag::Press { action: Some(a), .. } => {
                    if let Some(p) = &mut self.row_press {
                        p.lift(now);
                    }
                    fx.extend(self.act(a, now));
                }
                Drag::Press { x, y, action: None, .. } if self.screen() == Screen::Editor => self.place_caret(f, x, y + scroll),
                Drag::History { pitch, .. } => {
                    // A finger lifted while still moving sends the rows
                    // gliding on.
                    let speed = self.tracker.velocity(now);
                    if speed.abs() >= m.dp(MIN_FLING_DP) {
                        let speed = speed.clamp(-m.dp(MAX_FLING_DP), m.dp(MAX_FLING_DP));
                        self.glide = Some((Fling::new(now, 0.0, speed, self.px_per_dp * 160.0), pitch, 0));
                    }
                }
                Drag::Scroll { .. } if self.pull != 0.0 => {
                    // A page let go past its end springs back.
                    self.bounce = Some(Spring::new(now, motion::rubber(self.pull, self.h as f32), 0.0, 0.4));
                    self.pull = 0.0;
                    self.dirty = true;
                }
                Drag::Scroll { .. } => {
                    // A finger lifted while still moving sends the page
                    // gliding on. One lifted slower leaves it where it is,
                    // but for a search bar part folded, which settles
                    // folded or unfolded; so does one lifted in the fold
                    // whose glide would end in it too.
                    let speed = -self.tracker.velocity(now);
                    let folding = |s: f32| self.frame.as_ref().is_some_and(|fr| fr.settle(s) != s);
                    let glide = (speed.abs() >= m.dp(MIN_FLING_DP)).then(|| {
                        let speed = speed.clamp(-m.dp(MAX_FLING_DP), m.dp(MAX_FLING_DP));
                        Fling::new(now, scroll, speed, self.px_per_dp * 160.0)
                    });
                    match glide {
                        Some(glide) if !(folding(scroll) && folding(glide.at(u64::MAX).0)) => self.fling = Some(glide),
                        _ => self.settle_fold(now),
                    }
                }
                Drag::Edge { armed: true, .. } => fx.extend(self.back(now)),
                Drag::Edge { start, from_left, x, cy, .. } => {
                    // Short of going back, the cue sinks back into the edge.
                    let travel = if from_left { x - start } else { start - x };
                    let depth = widgets::back_cue_depth(&t, travel);
                    self.cue = Some((from_left, cy, Spring::new(now, depth, 0.0, 0.15)));
                    self.dirty = true;
                }
                Drag::Confirm { which, track, .. } => {
                    let done = self.state.confirm > SLID;
                    self.state.confirm = 0.0;
                    self.repaint_confirm(&t, f, which, track);
                    if done {
                        fx.extend(self.confirmed(which, now));
                    }
                }
                Drag::Pattern => fx.extend(self.drawn(now)),
                // Let go, the strength slider taps the motor as strongly as
                // it was left at.
                Drag::Slider(Slider::Vibration, _) if self.state.vibrate => fx.push(Effect::Haptic),
                _ => {}
            },
        }
        if self.pressing() != pressing {
            self.dirty = true;
        }
        // A key of the on-screen keyboard, or a choice of a list open over
        // the page, shows held down while the finger rests on it.
        let held = match self.drag {
            Drag::Press { action: Some(a @ (Action::Type(_) | Action::PickSecurity(_))), .. } => Some(a),
            _ => None,
        };
        if held != self.state.held {
            let was = std::mem::replace(&mut self.state.held, held);
            // Only the rows of what is let go and what is held change.
            let rects: Option<Vec<Rect>> = self
                .frame
                .as_ref()
                .and_then(|fr| [was, held].into_iter().flatten().map(|a| fr.hits.iter().find(|h| h.action == a).map(|h| h.rect)).collect());
            match rects {
                Some(rects) => rects.into_iter().for_each(|r| self.refresh_rows(r.y.floor() as u32, (r.y + r.h).ceil() as u32)),
                None => self.refresh(),
            }
        }
        fx
    }

    /// The button a finger holds down, which shows held at once: `<`, the
    /// search page's round button, or the home page's search bar.
    fn pressing(&self) -> Option<Action> {
        let button = self.frame.as_ref().and_then(|fr| fr.button).map(|(_, a)| a);
        match self.drag {
            Drag::Press { action: Some(a @ (Action::Back | Action::CloseSearch | Action::Open(Screen::Search))), .. } => Some(a),
            Drag::Press { action: Some(a), .. } if Some(a) == button => Some(a),
            _ => None,
        }
    }

    /// The finger left the row it held, scrolling the page or swiping: a
    /// row not darkening yet never does, and one darkening fades back.
    fn let_row_go(&mut self, now: u64) {
        let Some(p) = self.row_press else { return };
        if now < p.rise {
            self.row_press = None;
            self.row_art = None;
        } else if p.fall.is_none() {
            self.row_press = Some(RowPress { fall: Some(now), ..p });
        }
        self.dirty = true;
    }

    /// No touchscreen answers: the keys work the pages, their light shown
    /// from now on.
    pub fn touchscreen_missing(&mut self) {
        self.keys = true;
        self.dirty = true;
    }

    /// A button went down. With the keys working the page, the volume keys
    /// move the light from control to control and the power key works the
    /// one lit as it comes back up; otherwise the first volume press lights
    /// a control, and the power key turns the panel off. The power key and
    /// volume down together take a screenshot instead: whichever of the two
    /// goes down first waits `CHORD_MS` for the other, or until it is let
    /// go, and then does what it does alone, as pressed when it went down.
    pub fn key(&mut self, key: Key, now: u64) -> Vec<Effect> {
        // Any button stops a gliding page, and brings a sliding one to rest.
        self.fling = None;
        self.end_turn();
        if let Some((first, at)) = self.chord {
            if matches!((first, key), (Key::Power, Key::VolumeDown) | (Key::VolumeDown, Key::Power)) && now < at + CHORD_MS {
                return self.screenshot(key, now);
            }
        }
        let mut fx = self.let_chord_go();
        if !self.display_on {
            // Any button lights the dark panel, and does only that.
            self.wake(now);
            self.button = Some((key, Press::Spent));
            return fx;
        }
        // A button pressed just after the panel lit only keeps it awake, so
        // a power press meant to wake it does not turn it back off.
        if self.just_woke(now) {
            self.stir(now);
            self.button = Some((key, Press::Spent));
            return fx;
        }
        self.stir(now);
        if matches!(key, Key::Power | Key::VolumeDown) {
            self.chord = Some((key, now));
            return fx;
        }
        fx.extend(self.key_down(key, now));
        fx
    }

    /// The power key and volume down went down together: what the panel
    /// shows is saved as a picture, with a tap of the motor, and neither key
    /// does anything more until let go.
    fn screenshot(&mut self, key: Key, now: u64) -> Vec<Effect> {
        self.chord = None;
        self.stir(now);
        self.button = Some((key, Press::Spent));
        let mut fx = Vec::new();
        if self.state.vibrate {
            fx.push(Effect::Haptic);
        }
        fx.push(Effect::Screenshot { storage: self.state.place_open(Place::Storage) });
        fx
    }

    /// The key waiting for its partner in a screenshot does what it does
    /// alone, as pressed when it went down.
    fn let_chord_go(&mut self) -> Vec<Effect> {
        match self.chord.take() {
            Some((key, at)) => self.key_down(key, at),
            None => Vec::new(),
        }
    }

    /// What a button going down does on its own.
    fn key_down(&mut self, key: Key, now: u64) -> Vec<Effect> {
        match key {
            Key::Back => {
                self.button = Some((key, Press::Spent));
                self.act(Action::Back, now)
            }
            Key::VolumeUp | Key::VolumeDown => {
                let down = key == Key::VolumeDown;
                self.button = Some((key, Press::Volume { down, next: now + REPEAT_DELAY_MS }));
                self.volume(down, now)
            }
            // A press before the page it lands on is drawn is lost, rather
            // than taken for one on nothing.
            Key::Power if self.keys && self.frame.is_none() => {
                self.button = Some((key, Press::Spent));
                Vec::new()
            }
            Key::Power => {
                match self.keys.then(|| self.lit()).flatten() {
                    // Held on a swipe-to-confirm control, the key slides
                    // its knob along as a finger would.
                    Some(Spot { stop: Stop::Hit(Action::Confirm(which)), rect, .. }) => {
                        self.button = Some((key, Press::Slide { which, track: rect, at: now }))
                    }
                    Some(spot) => self.button = Some((key, Press::Power { spot, at: now })),
                    None => {
                        self.button = Some((key, Press::Spent));
                        self.sleep();
                    }
                }
                self.dirty = true;
                Vec::new()
            }
        }
    }

    /// A button came back up: the power key works the control it went down
    /// on, or confirms with the knob it slid far enough. Let go before its
    /// partner in a screenshot came, the key waiting for it first does what
    /// it does alone.
    pub fn key_up(&mut self, key: Key, now: u64) -> Vec<Effect> {
        let mut fx = match self.chord {
            Some((first, _)) if first == key => self.let_chord_go(),
            _ => Vec::new(),
        };
        let Some((down, press)) = self.button else { return fx };
        if down != key {
            return fx;
        }
        self.button = None;
        self.dirty = true;
        match press {
            Press::Power { spot, .. } => fx.extend(self.work(spot, now)),
            Press::Slide { which, track, .. } => {
                let done = self.state.confirm > SLID;
                self.let_knob_go(which, track);
                if done {
                    fx.extend(self.confirmed(which, now));
                } else {
                    self.say(self.state.lang.pick(["请按住电源键", "請按住電源鍵", "Hold the power key"]), now);
                }
            }
            Press::Volume { .. } | Press::Spent => {}
        }
        fx
    }

    /// Whatever the button held down was doing comes to nothing until it is
    /// let go: a knob it slid goes back.
    fn drop_button(&mut self) {
        let Some((key, press)) = self.button else { return };
        if let Press::Slide { which, track, .. } = press {
            self.let_knob_go(which, track);
        }
        self.button = Some((key, Press::Spent));
    }

    /// Let go of a swipe-to-confirm control's knob, which goes back to the
    /// start of its track.
    fn let_knob_go(&mut self, which: Confirm, track: Rect) {
        self.state.confirm = 0.0;
        self.knob = Some((which, track));
        self.dirty = true;
    }

    /// What a swipe-to-confirm control slid to its end does.
    fn confirmed(&mut self, which: Confirm, now: u64) -> Vec<Effect> {
        let (l, mut fx) = (self.state.lang, Vec::new());
        match which {
            Confirm::Reboot => fx.push(Effect::Reboot(TARGETS[self.state.reboot])),
            Confirm::Wipe => {
                let chosen: Vec<Wipe> = WIPES.iter().zip(self.state.wipe).filter(|(_, on)| *on).map(|(&w, _)| w).collect();
                match chosen.is_empty() {
                    true => self.say(l.pick(["请选择要清除的内容", "請選擇要清除的內容", "Choose what to wipe"]), now),
                    false => fx.push(Effect::Wipe(chosen)),
                }
            }
            Confirm::Install | Confirm::InstallWipe => {
                if let Install::Confirming { version, .. } = &self.state.install {
                    self.state.install = Install::Updating { version: version.clone(), stage: None, overwriting: false };
                    fx.push(Effect::Install);
                    self.refresh();
                }
            }
            Confirm::RunScript => {
                if self.state.install == Install::ConfirmingScript {
                    self.state.install = Install::Scripting { stage: None, running: false };
                    fx.push(Effect::Install);
                    self.refresh();
                }
            }
            Confirm::Backup => {
                let s = &self.state;
                let refused = if !s.backup.contains(&true) {
                    Some(["请选择要备份的内容", "請選擇要備份的內容", "Choose what to back up"])
                } else if s.backup[2] && s.data != Data::Open {
                    Some(["备份 data 分区需先完成「解密」", "備份 data 分割區需先完成「解密」", "Decrypt before backing up the data partition"])
                } else if s.place_open(s.backup_to) {
                    None
                } else {
                    Some(match (s.backup_to, &s.usb_drive) {
                        (Place::Storage, _) if s.data == Data::Erased => DATA_ERASED,
                        (Place::Storage, _) => DECRYPT_FIRST,
                        (Place::UsbDrive, Some(UsbDrive::Unreadable(_))) => USB_DRIVE_UNREADABLE,
                        (Place::UsbDrive, _) => USB_DRIVE_WANTED,
                    })
                };
                let backup = Effect::Backup { parts: s.backup, place: s.backup_to, compress: s.compress, verify: s.verify };
                match refused {
                    Some(why) => self.say(l.pick(why), now),
                    None => fx.push(backup),
                }
            }
            Confirm::Restore => {
                let (entry, parts) = self.state.restore.as_ref().expect("the restore page shows a backup");
                let restore = Effect::Restore { place: entry.place, name: entry.name.clone(), parts: *parts };
                if !parts.contains(&true) {
                    self.say(l.pick(["请选择要恢复的内容", "請選擇要還原的內容", "Choose what to restore"]), now);
                } else if parts[2] && self.state.data != Data::Open {
                    self.say(l.pick(["恢复 data 分区需先完成「解密」", "還原 data 分割區需先完成「解密」", "Decrypt before restoring the data partition"]), now);
                } else {
                    fx.push(restore);
                }
            }
            Confirm::DeleteBackup => {
                let (entry, _) = self.state.restore.as_ref().expect("the restore page shows a backup");
                fx.push(Effect::DeleteBackup { place: entry.place, name: entry.name.clone() });
            }
            Confirm::Slot => match (&self.state.boot_slots, self.state.next_slot) {
                (Some(Ok(slots)), Some(pick)) if pick != slots.active => fx.push(Effect::SetSlot(pick)),
                (Some(Ok(slots)), _) => {
                    let slot = slots.active.letter();
                    let said = tr!(l, "下次已从槽位 {slot} 启动", "下次已從槽位 {slot} 啟動", "Slot {slot} already starts next");
                    self.say(&said, now);
                }
                _ => self.say(l.pick(["无法读取启动槽位", "無法讀取啟動槽位", "Couldn't read the boot slots"]), now),
            },
            Confirm::FactoryReset => fx.push(Effect::FactoryReset),
            Confirm::Merge => {
                if self.state.update == Standing::Waiting {
                    self.state.update = Standing::Merging(0.0);
                    self.refresh();
                    fx.push(Effect::Merge);
                }
            }
            Confirm::DeleteEntry => fx.push(Effect::Delete(self.entry_path())),
            // The partition has to be open; whether its users have unlocked
            // their own files makes no difference.
            Confirm::Relabel => match self.state.data {
                Data::Open | Data::Locked { .. } => fx.push(Effect::Relabel),
                Data::Opening => self.say(l.pick(DATA_OPENING), now),
                Data::Erased => self.say(l.pick(DATA_ERASED), now),
                Data::Closed | Data::Failed(_) => {
                    self.say(l.pick(["请先在「解密」中打开 data 分区", "請先在「解密」中開啟 data 分割區", "Open the data partition in Decrypt first"]), now)
                }
            },
            // The data partition is reached through its own key whether or
            // not it is open, but not while it is being opened or erased.
            Confirm::Filesystem => {
                let row = self.fs_row().expect("the page of a partition shows one that was read");
                let data = row.name == DATA_PARTITION;
                match (&self.state.fs_action, &self.state.data) {
                    (_, Data::Opening) if data => self.say(l.pick(DATA_OPENING), now),
                    (FsAction::Repair, Data::Erased) if data => self.say(l.pick(DATA_ERASED), now),
                    (action, _) => fx.push(Effect::Filesystem { name: row.name.clone(), action: action.clone() }),
                }
            }
            Confirm::Flash => {
                if let Install::Choosing(choice) = &self.state.install {
                    let partition = choice.partitions[choice.partition];
                    fx.push(Effect::Flash { partition, pick: choice.pick });
                    self.state.install = Install::Flashing { partition, stage: None };
                    self.refresh();
                }
            }
        }
        fx
    }

    /// Work the control the keys are on, as touching it does. The
    /// brightness slider takes the volume keys until the power key lets it
    /// go, and a pattern's dots are joined one press at a time, a second
    /// press on the last one joined ending the pattern.
    fn work(&mut self, spot: Spot, now: u64) -> Vec<Effect> {
        let l = self.state.lang;
        match spot.stop {
            Stop::Back => self.act(Action::Back, now),
            Stop::Button(action) => self.act(action, now),
            Stop::Hit(Action::Slider(which)) => {
                self.adjusting = !self.adjusting;
                if self.adjusting {
                    let said = match which {
                        Slider::Brightness => ["按音量键调节亮度，按电源键完成", "按音量鍵調整亮度，按電源鍵完成", "Set the brightness with the volume keys, then press the power key"],
                        Slider::Vibration => {
                            ["按音量键调节振动强度，按电源键完成", "按音量鍵調整震動強度，按電源鍵完成", "Set the vibration with the volume keys, then press the power key"]
                        }
                    };
                    self.say(l.pick(said), now);
                }
                Vec::new()
            }
            Stop::Hit(action) => self.act(action, now),
            Stop::Dot(i) if self.state.pattern.last() == Some(&i) => self.drawn(now),
            Stop::Dot(i) => {
                if self.state.pattern.is_empty() {
                    self.say(l.pick(["连接完成后，在最后一个点上再按一次电源键", "連接完成後，在最後一個點上再按一次電源鍵", "When the pattern is done, press the power key again on its last dot"]), now);
                }
                self.join(&self.theme(), i);
                Vec::new()
            }
        }
    }

    /// A pattern drawn to its end: four dots or more are sent to be
    /// checked, read as the number of each dot counted from one across the
    /// rows; fewer are wiped and asked for again.
    fn drawn(&mut self, now: u64) -> Vec<Effect> {
        let drawn = std::mem::take(&mut self.state.pattern);
        self.repaint_pattern(&self.theme());
        if drawn.len() < 4 {
            self.say(self.state.lang.pick(["请至少连接 4 个点", "請至少連接 4 個點", "Connect at least 4 dots"]), now);
            return Vec::new();
        }
        self.unlock(drawn.iter().map(|i| (b'1' + *i as u8) as char).collect())
    }

    /// A volume key: the first lights the control the keys were last on
    /// if it shows, or else the first in view; each after moves the light
    /// down the page or up it, or moves a slider that takes the keys, the
    /// strength slider tapping the motor as strongly as it was moved to.
    fn volume(&mut self, down: bool, now: u64) -> Vec<Effect> {
        if self.frame.is_none() {
            return Vec::new();
        }
        if self.adjusting {
            if let Some(Spot { stop: Stop::Hit(Action::Slider(which)), rect, .. }) = self.lit() {
                let level = self.level(which) + if down { -SLIDER_STEP } else { SLIDER_STEP };
                self.set_level(&self.theme(), which, rect, level);
                if which == Slider::Vibration && self.state.vibrate {
                    return vec![Effect::Haptic];
                }
            }
            return Vec::new();
        }
        if self.keys {
            self.step(down, now);
            return Vec::new();
        }
        self.keys = true;
        self.dirty = true;
        let scroll = self.resting();
        let shows = self.lit().is_some_and(|spot| self.shows(&spot, scroll));
        if !shows {
            self.top_mut().2 = Focus::Unset;
            self.lit();
        }
        Vec::new()
    }

    /// Move the light to the next control down the page, or up it. A
    /// control far off is scrolled to half a screen at a time, the light
    /// staying on its control while that shows; past the last control the
    /// light goes round to the first, and past the first to the last.
    fn step(&mut self, down: bool, now: u64) {
        let scroll = self.resting();
        let current = self.lit();
        if self.spots().is_none() {
            return;
        }
        let spots = self.spots.clone().expect("made above");
        let cur = current.and_then(|c| focus::find(&spots, &c));
        let (top, bottom) = self.band(scroll);
        let (max, n) = (self.max_scroll(), spots.len());
        let page = |k: usize| spots[k].group == Group::Page;
        let held = |k: usize| matches!(spots[k].group, Group::Fixed | Group::Strip);
        let docked = |k: usize| spots[k].group == Group::Dock;
        // The page's own controls not wholly above the band, or below it.
        let from_top = |k: usize| page(k) && self.place(&spots[k], scroll).is_some_and(|r| r.y + r.h > top);
        let from_bottom = |k: usize| page(k) && self.place(&spots[k], scroll).is_some_and(|r| r.y < bottom);
        enum To {
            Spot(usize),
            Scroll,
            Round,
        }
        let to = if down {
            // Past the page's own: the rest of the page, then the bottom
            // strip, then round.
            let past = || {
                if scroll < max {
                    To::Scroll
                } else {
                    (0..n).find(|&k| docked(k)).map_or(To::Round, To::Spot)
                }
            };
            match cur {
                Some(i) if page(i) => (i + 1..n).find(|&k| page(k)).map_or_else(past, To::Spot),
                Some(i) if docked(i) => (i + 1..n).find(|&k| docked(k)).map_or(if scroll < max { To::Scroll } else { To::Round }, To::Spot),
                Some(i) => (i + 1..n).find(|&k| held(k)).or_else(|| (0..n).find(|&k| from_top(k))).map_or_else(past, To::Spot),
                None => (0..n).find(|&k| from_top(k)).map_or_else(past, To::Spot),
            }
        } else {
            // Before the page's own: the rest of the page, then the `<` and
            // the strip at the top, then round.
            let before = || {
                if scroll > 0.0 {
                    To::Scroll
                } else {
                    (0..n).rev().find(|&k| held(k)).map_or(To::Round, To::Spot)
                }
            };
            match cur {
                Some(i) if page(i) => (0..i).rev().find(|&k| page(k)).map_or_else(before, To::Spot),
                Some(i) if docked(i) => (0..i).rev().find(|&k| docked(k)).or_else(|| (0..n).rev().find(|&k| from_bottom(k))).map_or_else(before, To::Spot),
                Some(i) => (0..i).rev().find(|&k| held(k)).map_or(if scroll > 0.0 { To::Scroll } else { To::Round }, To::Spot),
                None => (0..n).rev().find(|&k| from_bottom(k)).map_or_else(before, To::Spot),
            }
        };
        match to {
            To::Spot(k) => {
                let s = spots[k];
                if s.group == Group::Page {
                    let target = self.reveal(&s, scroll);
                    if (target - scroll).abs() > (bottom - top) / 2.0 {
                        return self.scroll_on(down, &spots, cur, now);
                    }
                    self.seek_to(target, now);
                }
                self.top_mut().2 = Focus::On(s);
            }
            To::Scroll => return self.scroll_on(down, &spots, cur, now),
            To::Round if n == 0 => return,
            // Going round, the page goes to its top or its end, as far as
            // its control there shows.
            To::Round => {
                let s = spots[if down { 0 } else { n - 1 }];
                let end = if down { 0.0 } else { max };
                let target = if self.shows(&s, end) { end } else { self.reveal(&s, end) };
                self.seek_to(target, now);
                self.top_mut().2 = Focus::On(s);
            }
        }
        self.dirty = true;
    }

    /// Scroll the page half a screen down or up for the keys. The light
    /// stays on its control while that shows, and goes to the first in view
    /// the way the page went when it does not.
    fn scroll_on(&mut self, down: bool, spots: &[Spot], cur: Option<usize>, now: u64) {
        let scroll = self.resting();
        let (top, bottom) = self.band(scroll);
        let half = (bottom - top) / 2.0;
        let target = if down { (scroll + half).min(self.max_scroll()) } else { (scroll - half).max(0.0) };
        // A page coming to rest with its search bar part folded folds it
        // whole or not at all, as one let go there does.
        let target = self.frame.as_ref().map_or(target, |fr| fr.settle(target));
        self.seek_to(target, now);
        if cur.is_some_and(|i| self.shows(&spots[i], target)) {
            return;
        }
        let band = self.band(target);
        let in_view = |k: &usize| spots[*k].group == Group::Page && self.place(&spots[*k], target).is_some_and(|r| within(r, band));
        let next = if down { (0..spots.len()).find(in_view) } else { (0..spots.len()).rev().find(in_view) };
        self.top_mut().2 = next.map_or(Focus::Off, |k| Focus::On(spots[k]));
    }

    /// The page's controls in the order the keys go through them, once the
    /// page is drawn.
    fn spots(&mut self) -> Option<&[Spot]> {
        if self.spots.is_none() {
            let t = self.theme();
            self.spots = Some(focus::spots(&t, self.frame.as_ref()?));
        }
        self.spots.as_deref()
    }

    /// The control the keys are on, once the page is drawn: the one they
    /// were on, found again, or the first in view when they have not been
    /// anywhere on the page yet or theirs has gone from it.
    fn lit(&mut self) -> Option<Spot> {
        let scroll = self.resting();
        self.spots()?;
        let spots = self.spots.as_deref().expect("made above");
        let found = match self.top().2 {
            Focus::Off => return None,
            Focus::On(at) => focus::find(spots, &at),
            Focus::Unset => None,
        };
        let spot = spots[found.or_else(|| self.first_in_view(spots, scroll))?];
        self.stack.last_mut().expect("the page stack always holds a page").2 = Focus::On(spot);
        Some(spot)
    }

    /// The first of the page's own controls in view at `scroll`, or else
    /// the bottom strip's first, or else the `<` or the top strip's first.
    fn first_in_view(&self, spots: &[Spot], scroll: f32) -> Option<usize> {
        let band = self.band(scroll);
        let k = spots.iter().position(|s| s.group == Group::Page && self.place(s, scroll).is_some_and(|r| within(r, band)));
        k.or_else(|| spots.iter().position(|s| s.group == Group::Dock))
            .or_else(|| spots.iter().position(|s| matches!(s.group, Group::Fixed | Group::Strip)))
    }

    /// Where on screen `s` is with the page `offset` pixels down; nowhere
    /// for a search bar folded away.
    fn place(&self, s: &Spot, offset: f32) -> Option<Rect> {
        let fr = self.frame.as_ref().expect("a control is placed on the page drawn");
        let dy = match s.group {
            Group::Fixed => 0.0,
            Group::Strip => pinned_top(&self.theme(), fr.pinned.as_ref().expect("the strip holds its controls"), offset),
            Group::Page => return fr.shown(s.rect, offset),
            Group::Dock => self.h as f32 - fr.dock.as_ref().expect("the dock holds its controls").canvas.h as f32,
        };
        Some(Rect { y: s.rect.y + dy, ..s.rect })
    }

    /// The band of the screen the page's own controls show in with the page
    /// at `scroll`: under the bar, or under the strip held there, down to
    /// the strip held at the bottom.
    fn band(&self, scroll: f32) -> (f32, f32) {
        let t = self.theme();
        let fr = self.frame.as_ref().expect("the band is the page drawn's");
        let top = match &fr.pinned {
            Some(p) => pinned_top(&t, p, scroll) + p.canvas.h as f32,
            None => widgets::bar_bottom(&t),
        };
        (top, self.h as f32 - fr.dock.as_ref().map_or(0.0, |d| d.canvas.h as f32))
    }

    /// Whether `s` shows, in part at least, with the page at `scroll`.
    fn shows(&self, s: &Spot, scroll: f32) -> bool {
        s.group != Group::Page || self.place(s, scroll).is_some_and(|r| within(r, self.band(scroll)))
    }

    /// How far down to scroll the page from `scroll` to show `s` whole, a
    /// margin clear of the band's edges, or as much of it from its top as
    /// fits. A search bar under the large title shows only unfolded, at
    /// the page's top, and a page scrolled down to show any other control
    /// goes on to fold the bar away whole.
    fn reveal(&self, s: &Spot, scroll: f32) -> f32 {
        let fr = self.frame.as_ref().expect("a control is revealed on the page drawn");
        let fold = fr.fold.filter(|_| s.group == Group::Page);
        if fold.is_some_and(|f| f.bar == s.rect) {
            return 0.0;
        }
        let margin = self.theme().m.margin;
        let (top, bottom) = self.band(scroll);
        let (top, bottom) = (top + margin, bottom - margin);
        let r = self.place(s, scroll).expect("only a search bar folded away has no place");
        let d = if r.y < top {
            r.y - top
        } else if r.y + r.h > bottom {
            (r.y + r.h - bottom).min(r.y - top)
        } else {
            0.0
        };
        let target = scroll + d;
        let target = fold.map_or(target, |f| if target > 0.0 && target < f.span { f.span } else { target });
        target.clamp(0.0, self.max_scroll())
    }

    /// Where the page comes to rest: where the keys are scrolling it to, or
    /// where it is.
    fn resting(&self) -> f32 {
        self.seek.map_or(self.top().1, |(target, _)| target)
    }

    /// A page come to rest with its search bar part folded eases on from
    /// there to fold the bar away or unfold it whole, whichever is nearer.
    fn settle_fold(&mut self, now: u64) {
        let Some(fr) = self.frame.as_ref() else { return };
        let at = self.top().1;
        let rest = fr.settle(at).min(self.max_scroll());
        if rest != at {
            self.seek = Some((rest, Spring::new(now, at - rest, 0.0, FOLD_PERIOD)));
            self.dirty = true;
        }
    }

    /// Scroll the page to `target` for the keys, easing there.
    fn seek_to(&mut self, target: f32, now: u64) {
        let at = self.top().1;
        self.fling = None;
        self.seek = (target != at).then(|| (target, Spring::new(now, at - target, 0.0, SEEK_PERIOD)));
        self.dirty = true;
    }

    /// The light on the control the keys are on while they work the page:
    /// the part of the screen its control is drawn in, where the light
    /// falls with the page `offset` pixels down, how round it is, and its
    /// tint, stronger while the power key is held down on it.
    fn light(&mut self, offset: f32) -> Option<(Group, Rect, f32, Rgba)> {
        if !self.keys {
            return None;
        }
        let spot = self.lit()?;
        let t = self.theme();
        let keypad = self.screen() == Screen::Decrypt && matches!(self.state.data, Data::Locked { keys: Keys::Digits, .. });
        let (r, radius) = focus::lit(&t, spot.stop, self.place(&spot, offset)?, keypad);
        let held = matches!(self.button, Some((_, Press::Power { .. } | Press::Slide { .. })));
        let alpha = if held || self.adjusting { LIGHT_HELD_ALPHA } else { LIGHT_ALPHA };
        Some((spot.group, r, radius, t.pal.accent.with_alpha(alpha)))
    }

    /// Whether the page is gliding, springing back, or easing on of its own
    /// accord at `now`.
    fn in_motion(&self, now: u64) -> bool {
        self.fling.is_some_and(|fl| !fl.at(now).1) || self.bounce.is_some_and(|b| !b.at(now).1) || self.seek.is_some_and(|(_, s)| !s.at(now).1)
    }

    /// Stop a page gliding, springing back or easing on of its own accord,
    /// where it has got to, for a finger to hold: one past its end stays
    /// out there as the finger's pull. Whether there was one.
    fn catch(&mut self, now: u64) -> bool {
        let glide = self.fling.take().is_some_and(|fl| !fl.at(now).1);
        let seeking = self.seek.take().is_some_and(|(_, s)| !s.at(now).1);
        let spring = self.bounce.take().map(|b| b.at(now)).filter(|&(_, done)| !done);
        if let Some((offset, _)) = spring {
            self.pull = motion::unrubber(offset, self.h as f32);
        }
        glide || seeking || spring.is_some()
    }

    /// Move the page `d` pixels under a dragging finger (positive moves it
    /// up). Past an end the finger's travel goes into the pull, which the
    /// page follows with growing resistance; coming back undoes the pull
    /// before the page scrolls again.
    fn drag_by(&mut self, d: f32) {
        self.dirty = true;
        let mut d = d;
        if self.pull != 0.0 {
            let pull = self.pull + d;
            if pull * self.pull > 0.0 {
                self.pull = pull;
                return;
            }
            self.pull = 0.0;
            d = pull;
        }
        let max = self.max_scroll();
        let top = self.top_mut();
        let next = top.1 + d;
        top.1 = next.clamp(0.0, max);
        self.pull = next - top.1;
    }

    fn on_knob(&self, track: Rect, x: f32) -> bool {
        let knob_x = track.x + (track.w - track.h) * self.state.confirm;
        x >= knob_x && x <= knob_x + track.h
    }

    fn set_slider(&mut self, t: &Theme, which: Slider, track: Rect, x: f32) {
        self.set_level(t, which, track, (x - track.x) / track.w);
    }

    /// Where a slider stands, as a share of its span.
    fn level(&self, which: Slider) -> f32 {
        match which {
            Slider::Brightness => self.state.brightness,
            Slider::Vibration => self.state.vibration,
        }
    }

    /// Set a slider to `level` of its span, the brightness no lower than
    /// its floor, and draw the slider at `track` on the page again.
    fn set_level(&mut self, t: &Theme, which: Slider, track: Rect, level: f32) {
        let value = match which {
            Slider::Brightness => &mut self.state.brightness,
            Slider::Vibration => &mut self.state.vibration,
        };
        *value = level.clamp(if which == Slider::Brightness { MIN_BRIGHTNESS } else { 0.0 }, 1.0);
        let value = *value;
        if let Some(fr) = self.frame.as_mut() {
            fr.canvas.fill_rect(track.x, track.y, track.w, track.h, t.pal.card);
            widgets::slider(&mut fr.canvas, t, track.x, track.y, track.w, value);
        }
        self.layer = None;
        self.dirty = true;
    }

    fn repaint_confirm(&mut self, t: &Theme, f: &Fonts, which: Confirm, track: Rect) {
        self.paint_knob(t, f, which, track);
        self.dirty = true;
    }

    /// Draw the swipe-to-confirm control at `track` on the page again, its
    /// knob where it has come to.
    fn paint_knob(&mut self, t: &Theme, f: &Fonts, which: Confirm, track: Rect) {
        let (label, danger) = pages::confirm_label(which, &self.state);
        if let Some(fr) = self.frame.as_mut() {
            fr.canvas.fill_rect(track.x, track.y, track.w, track.h, t.pal.bg);
            widgets::confirm(&mut fr.canvas, t, f, track.y, label, danger, self.state.confirm);
        }
        self.layer = None;
    }

    /// Add the dot under the finger to the pattern being drawn.
    fn trace(&mut self, t: &Theme, x: f32, y: f32) {
        let Some((cx, cy, span)) = self.frame.as_ref().and_then(|fr| fr.pattern) else { return };
        let reach = t.m.dp(widgets::PATTERN_REACH_DP);
        let dots = widgets::pattern_dots(cx, cy, span);
        if let Some(i) = dots.iter().position(|&(dx, dy)| (dx - x).hypot(dy - y) < reach) {
            self.join(t, i);
        }
    }

    /// Join dot `i` to the pattern being drawn, after the dot it passes
    /// straight over on its way from the last one, as the system's pattern
    /// lock joins that one too. A dot joined once is not joined again.
    fn join(&mut self, t: &Theme, i: usize) {
        let drawn = &mut self.state.pattern;
        if drawn.contains(&i) {
            return;
        }
        if let Some(&last) = drawn.last() {
            let (row, col) = ((last / 3) as i32, (last % 3) as i32);
            let (rows, cols) = ((i / 3) as i32 - row, (i % 3) as i32 - col);
            // Two rows or columns away, and not a knight's move.
            let row = if rows.abs() == 2 && cols.abs() != 1 { row + rows.signum() } else { row };
            let col = if cols.abs() == 2 && rows.abs() != 1 { col + cols.signum() } else { col };
            let over = (row * 3 + col) as usize;
            if !drawn.contains(&over) {
                drawn.push(over);
            }
        }
        drawn.push(i);
        self.repaint_pattern(t);
    }

    fn repaint_pattern(&mut self, t: &Theme) {
        let Some(fr) = self.frame.as_mut() else { return };
        let Some((cx, cy, span)) = fr.pattern else { return };
        let r = span / 2.0 + t.m.dp(16.0);
        fr.canvas.fill_rect(cx - r, cy - r, r * 2.0, r * 2.0, t.pal.bg);
        widgets::pattern_lock(&mut fr.canvas, t, cx, cy, span, &self.state.pattern);
        self.layer = None;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::over;
    use crate::fmt;
    use skyward_info::{Boot, Charge, Cpu, Storage};
    use skyward_install::{Failure, Refusal};
    use skyward_net::reach::Reach;
    use skyward_vab::SnapshotState;

    // A Xiaomi 13 Ultra panel: 1440 × 3200 at 560 dpi.
    const W: u32 = 1440;
    const H: u32 = 3200;
    const PPD: f32 = 3.5;

    fn fonts() -> Fonts {
        crate::font::bundled()
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            model: "Xiaomi 13 Ultra".into(),
            codename: "ishtar".into(),
            soc: None,
            cpu: Cpu { cores: 8, max_khz: Some(3_187_200) },
            ram_kib: 15_556_972,
            panel: Some((W, H)),
            screen: Some(6.73),
            os_version: None,
            android: None,
            locale: None,
            security_patch: None,
            kernel: "5.15.178".into(),
            storage: Storage { total: 496_550_785_024, free: None },
            battery: None,
            slot: None,
            snapshot: SnapshotState::None,
            boot: Boot { unlocked: None, verified: None },
        }
    }

    #[test]
    fn a_page_drawn_a_band_at_a_time_is_the_page_drawn_whole() {
        let (f, i) = (fonts(), info());
        let state = State::default();
        // The About page lies over its field and is drawn whole.
        let screens = [
            Screen::Decrypt,
            Screen::Home,
            Screen::Settings,
            Screen::Reboot,
            Screen::Wipe,
            Screen::Mount,
            Screen::Install,
            Screen::Choose(Place::Storage),
            Screen::Specs,
            Screen::ScreenOff,
            Screen::Search,
            Screen::Backup,
            Screen::Restore,
            Screen::Advanced,
            Screen::Files,
            Screen::Terminal,
            Screen::Language,
            Screen::TimeZone,
            Screen::Logs,
            Screen::Update,
            Screen::License,
            Screen::FontLicense,
            Screen::Slot,
            Screen::Fastboot,
            Screen::Erase,
            Screen::CantStart,
            Screen::Unhandled,
        ];
        for t in [Theme::light(W, H, PPD), Theme::dark(W, H, PPD).pressed()] {
            for screen in screens {
                let cx = |band| Ctx { t: &t, f: &f, info: &i, state: &state, term: None, band };
                let whole = pages::render(screen, &cx(None)).canvas;
                let row = whole.w as usize * 4;
                let differs = |c: &Canvas| c.data.chunks(row).zip(whole.data.chunks(row)).position(|(a, b)| a != b);
                // Cuts through cards, text and icons, and a band one row
                // tall, the bands joined one under another.
                let mut cuts: Vec<u32> = [0, 1, 377, 1201, 3000, 3001, 4444, u32::MAX].map(|k| k.min(whole.h)).to_vec();
                cuts.dedup();
                let mut joined: Option<Canvas> = None;
                for rows in cuts.windows(2) {
                    let part = pages::render(screen, &cx(Some(Band { rows: (rows[0], rows[1]), cols: (0, W) }))).canvas;
                    assert_eq!((part.h, part.held()), (whole.h, rows[0]..rows[1]), "{screen:?}: a band is laid out as the whole");
                    match &mut joined {
                        Some(c) => c.join(&part),
                        None => joined = Some(part),
                    }
                }
                let joined = joined.expect("a page has rows");
                assert!(joined.whole(), "{screen:?}: the bands make the whole");
                assert_eq!(differs(&joined), None, "{screen:?}: the first row drawn otherwise in bands");
                // The page drawn across a part at a time, as one sliding
                // in, into a canvas holding all of it: cuts through cards
                // and text, and a part one column wide.
                let mut across = Canvas::unfilled(W, whole.h, 0..whole.h);
                for cols in [0, 1, 333, 720, W - 1, W].windows(2) {
                    let part = pages::render(screen, &cx(Some(Band { rows: (0, whole.h), cols: (cols[0], cols[1]) }))).canvas;
                    assert_eq!(part.cols(), cols[0]..cols[1], "{screen:?}: a part across holds its columns");
                    across.replace(&part);
                }
                assert_eq!(differs(&across), None, "{screen:?}: the first row drawn otherwise across");
            }
        }
    }

    #[test]
    fn a_page_at_rest_draws_the_rows_below_the_screen_a_band_at_a_time() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.compose(&mut Canvas::new(W, H), &f, &i, "12:00", 0);
        assert_eq!(app.frame.as_ref().unwrap().canvas.held(), 0..H, "the page shown has the rows on screen drawn");
        let mut bands = 1;
        while app.draw_ahead(&f, &i) {
            bands += 1;
        }
        let t = app.theme();
        let whole = pages::render(Screen::Settings, &Ctx { t: &t, f: &f, info: &i, state: &app.state, term: None, band: None }).canvas;
        let page = &app.frame.as_ref().unwrap().canvas;
        assert!(whole.h > H + H / 4, "a page well over a screen long, drawn in {bands} more bands");
        assert!(page.whole() && page.data == whole.data, "the page drawn a band at a time is the page drawn whole");
    }

    #[test]
    fn a_page_typed_on_draws_again_only_what_changed_and_shows_what_it_would_drawn_anew() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Search, W, H, PPD);
        let mut out = Canvas::new(W, H);
        app.compose(&mut out, &f, &i, "12:00", 0);
        let anew = |app: &App| {
            let t = app.theme();
            pages::render(Screen::Search, &Ctx { t: &t, f: &f, info: &i, state: &app.state, term: None, band: None })
        };
        for cap in [Cap::Char('b'), Cap::Char('f'), Cap::Space, Cap::Backspace, Cap::Shift, Cap::Char('q')] {
            let (x, y) = spot(&mut app, &f, &i, Action::Type(cap));
            app.touch(Touch::Down(x, y), &f, 0);
            assert!(app.frame.is_some() && !app.stale.is_empty() || cap == Cap::Shift, "{cap:?}: the key held is drawn again on its own");
            app.compose(&mut out, &f, &i, "12:00", 0);
            assert!(app.frame.as_ref().unwrap().canvas.data == anew(&app).canvas.data, "{cap:?} held down");
            app.touch(Touch::Up, &f, 0);
            app.compose(&mut out, &f, &i, "12:00", 0);
            let (drawn, fresh) = (app.frame.as_ref().unwrap(), anew(&app));
            assert!(drawn.canvas.data == fresh.canvas.data, "{cap:?} typed");
            assert_eq!(drawn.hits.iter().map(|h| h.action).collect::<Vec<_>>(), fresh.hits.iter().map(|h| h.action).collect::<Vec<_>>(), "{cap:?} typed");
        }
        assert_eq!(app.state.search, "bfQ");
    }

    /// Where on screen the first control doing `action` sits.
    /// Where on screen the control doing `action` is: among the page's
    /// controls, on its pinned strip, or a row of its long list.
    fn spot(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action) -> (f32, f32) {
        let (t, scroll) = (app.theme(), app.top().1);
        let fr = app.frame(f, i);
        if let Some(h) = fr.hits.iter().find(|h| h.action == action) {
            let (x, y) = h.rect.centre();
            return (x, y - scroll);
        }
        if let Some(p) = &fr.pinned {
            if let Some(h) = p.hits.iter().find(|h| h.action == action) {
                let (x, y) = h.rect.centre();
                return (x, y + pinned_top(&t, p, scroll));
            }
        }
        if let Some(d) = &fr.dock {
            if let Some(h) = d.hits.iter().find(|h| h.action == action) {
                let (x, y) = h.rect.centre();
                return (x, y + H as f32 - d.canvas.h as f32);
            }
        }
        let rows = fr.rows.as_ref().expect("control on the page");
        let answer = rows.action.expect("control on the page");
        let k = (0..rows.count()).find(|&k| answer(k) == action).expect("control on the page");
        let (x, y) = rows.rect(k, fr.canvas.w as f32).centre();
        (x, y - scroll)
    }

    fn tap(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action) -> Vec<Effect> {
        tap_at(app, f, i, action, 0)
    }

    fn tap_at(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action, now: u64) -> Vec<Effect> {
        let (x, y) = spot(app, f, i, action);
        let mut fx = app.touch(Touch::Down(x, y), f, now);
        fx.extend(app.touch(Touch::Up, f, now));
        fx
    }

    /// Tap the `<`, which stays at the top of the screen.
    fn tap_back(app: &mut App, f: &Fonts, i: &DeviceInfo, now: u64) {
        app.frame(f, i);
        let (x, y) = widgets::back_rect(&app.theme()).centre();
        app.touch(Touch::Down(x, y), f, now);
        app.touch(Touch::Up, f, now);
    }

    /// Tap the round button at the right end of the bar.
    fn tap_button(app: &mut App, f: &Fonts, i: &DeviceInfo, now: u64) -> Vec<Effect> {
        app.frame(f, i);
        let (x, y) = widgets::button_rect(&app.theme()).centre();
        let mut fx = app.touch(Touch::Down(x, y), f, now);
        fx.extend(app.touch(Touch::Up, f, now));
        fx
    }


    #[test]
    fn rows_open_pages_and_back_returns() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Settings));
        assert_eq!(app.screen(), Screen::Settings);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.screen(), Screen::Home);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home, "back on the first page stays there");
    }

    #[test]
    fn the_back_control_stays_put_while_the_page_scrolls() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Settings));
        app.frame(&f, &i);
        app.touch(Touch::Down(700.0, 2400.0), &f, 0);
        app.touch(Touch::Move(700.0, 2200.0), &f, 0);
        app.touch(Touch::Move(700.0, 2100.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.top().1, 300.0);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.screen(), Screen::Home);
    }

    /// `screen` opened from the home page, so there is a page to go back
    /// to.
    fn opened(screen: Screen, f: &Fonts, i: &DeviceInfo) -> App {
        let mut app = App::new(Screen::Home, W, H, PPD);
        tap(&mut app, f, i, Action::Open(screen));
        app
    }

    #[test]
    fn home_opens_the_mount_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Mount)), vec![Effect::ReadMounts], "the page shows the partitions as they are now");
        assert_eq!(app.screen(), Screen::Mount);
    }

    /// A battery at `level`, as `charge` has it, 35.2 °C.
    fn battery_at(level: u8, charge: Charge) -> skyward_info::Battery {
        skyward_info::Battery { level, charge, temp: Some(352), health: None, cycles: None, design: None, full: None, voltage: None, quick: false }
    }

    #[test]
    fn the_home_card_tells_of_a_battery_too_low_to_install_on() {
        let f = fonts();
        // Where the first row under the card starts: the card is that much
        // taller with a notice in it.
        let under = |i: &DeviceInfo| {
            let mut app = App::new(Screen::Home, W, H, PPD);
            app.frame(&f, i).hits.iter().find(|h| h.action == Action::Open(Screen::Install)).expect("an install row").rect.y
        };
        let plain = under(&info());
        assert!(under(&DeviceInfo { battery: Some(battery_at(19, Charge::Discharging)), ..info() }) > plain, "short of 20% unplugged");
        assert_eq!(under(&DeviceInfo { battery: Some(battery_at(20, Charge::Discharging)), ..info() }), plain);
        assert!(under(&DeviceInfo { battery: Some(battery_at(14, Charge::Charging)), ..info() }) > plain, "short of 15% charging");
        assert_eq!(under(&DeviceInfo { battery: Some(battery_at(16, Charge::Charging)), ..info() }), plain);
        let (title, note) = fmt::battery_low(Lang::Hans, 14, true);
        assert_eq!((title.as_str(), note.as_str()), ("电量仅剩 14%", "充电至 15% 以上才能安装"));
    }

    #[test]
    fn the_home_card_tells_of_a_restart_into_the_other_slot_and_opens_the_slot_page() {
        let f = fonts();
        let i = DeviceInfo { slot: Some(Slot::B), ..info() };
        let mut app = App::new(Screen::Home, W, H, PPD);
        let slots = |active| BootSlots { active, bootable: [true, true], successful: [true, true] };
        app.set_boot_slots(Ok(slots(Slot::B)));
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::Open(Screen::Slot)), "the slot it runs from starts next");
        app.set_boot_slots(Ok(slots(Slot::A)));
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Slot)), vec![Effect::ReadSlots]);
        assert_eq!(app.screen(), Screen::Slot);
        assert_eq!(fmt::next_boot(Lang::Hans, Slot::A, Slot::B), ("重启后从 A 槽位启动".into(), "当前运行的是 B 槽位".into()));
    }

    #[test]
    fn the_status_bar_carries_the_slot_the_data_partitions_lock_and_the_temperature() {
        let i = DeviceInfo { slot: Some(Slot::B), battery: Some(battery_at(64, Charge::Charging)), ..info() };
        let mut app = App::new(Screen::Home, W, H, PPD);
        let bar = app.bar("12:00", &i);
        assert_eq!((bar.slot, bar.temp, bar.battery), (Some("B"), Some(352), Some((64, Some(Bolt::Charging)))), "the temperature to the tenth");
        let quick = DeviceInfo { battery: Some(skyward_info::Battery { quick: true, ..battery_at(64, Charge::Charging) }), ..info() };
        assert_eq!(app.bar("12:00", &quick).battery, Some((64, Some(Bolt::Quick))), "a quick charger draws the stock bar's quick bolt");
        let out = DeviceInfo { battery: Some(battery_at(64, Charge::Discharging)), ..info() };
        assert_eq!(app.bar("12:00", &out).battery, Some((64, None)), "no bolt with the cable out");
        let lock = |app: &mut App, data: Data| {
            app.state.data = data;
            app.bar("12:00", &i).data
        };
        assert_eq!(lock(&mut app, Data::Locked { user: 0, keys: Keys::Digits, checking: false }), Some(false), "shut while its files are out of reach");
        assert_eq!(lock(&mut app, Data::Open), Some(true));
        assert_eq!(lock(&mut app, Data::Erased), None, "nothing to open once wiped");
        let none = app.bar("12:00", &info());
        assert_eq!((none.slot, none.temp), (None, None), "a phone without slots or a fuel gauge");
    }

    /// A screen the home page's rows fit on with a little room to spare.
    const SNUG: u32 = 3000;

    #[test]
    fn a_home_page_grown_by_an_update_notice_scrolls_down_to_its_last_row() {
        let f = fonts();
        let plain = info();
        let mut app = App::new(Screen::Home, W, SNUG, PPD);
        let fr = app.frame(&f, &plain);
        let reboot = fr.hits.iter().find(|h| h.action == Action::Open(Screen::Reboot)).expect("a reboot row").rect;
        assert!(reboot.y + reboot.h <= SNUG as f32, "without the notice the rows fit the screen");
        let i = DeviceInfo { snapshot: SnapshotState::Snapshotted, ..info() };
        let mut app = App::new(Screen::Home, W, SNUG, PPD);
        let fr = app.frame(&f, &i);
        let reboot = fr.hits.iter().find(|h| h.action == Action::Open(Screen::Reboot)).expect("a reboot row").rect;
        assert!(reboot.y + reboot.h > SNUG as f32, "the notice pushes the last row below the screen");
        assert!(reboot.y + reboot.h <= fr.canvas.h as f32, "the page is drawn down to it");
        app.touch(Touch::Down(700.0, 2800.0), &f, 0);
        app.touch(Touch::Move(700.0, 2000.0), &f, 0);
        app.touch(Touch::Move(700.0, 1200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        for ms in (0..3000).step_by(8) {
            app.tick(ms);
        }
        let (_, y) = spot(&mut app, &f, &i, Action::Open(Screen::Reboot));
        assert!(y < SNUG as f32, "scrolled up into view");
        tap_at(&mut app, &f, &i, Action::Open(Screen::Reboot), 3000);
        assert_eq!(app.screen(), Screen::Reboot);
    }

    #[test]
    fn the_cursor_blinks_and_comes_up_whole_at_each_edit() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        let mut screen = Canvas::new(W, H);
        let mut shows = |app: &mut App, now: u64| {
            app.compose(&mut screen, &f, &i, "12:00", now);
            app.caret_shown
        };
        assert!(shows(&mut app, 1000), "it comes up whole on the page");
        assert_eq!(app.deadline(1200), Some(1500), "the loop wakes when it turns");
        assert!(!shows(&mut app, 1500), "half a second on it hides");
        assert!(shows(&mut app, 2000));
        assert!(!shows(&mut app, 2600));
        tap(&mut app, &f, &i, Action::Type(Cap::Char('b')));
        assert!(shows(&mut app, 2700), "an edit brings it back whole at once");
        assert!(!shows(&mut app, 3200));
        tap(&mut app, &f, &i, Action::CloseSearch);
        assert_eq!(app.screen(), Screen::Home, "the round button leaves the search");
        assert!(!shows(&mut app, 3300), "no cursor off the page");
        assert!(app.deadline(3300).is_none_or(|d| d > 10_000), "nor a wake for it, only for the screen going off");
    }

    #[test]
    fn the_search_bar_moves_up_into_the_search_page_and_back_down() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let m = t.m;
        let mut app = App::new(Screen::Home, W, H, PPD);
        let mut screen = Canvas::new(W, H);
        // The bar's top along a column through it, clear of the title and
        // the placeholder: the first row in the field's grey.
        let x = m.dp(300.0) as u32;
        let top = |screen: &Canvas| {
            (m.dp(40.0) as u32..H).find(|&y| screen.data[((y * W + x) * 4) as usize] == t.pal.search.r).map(|y| y as f32)
        };
        let grey = |screen: &Canvas, y: f32| screen.data[((y as u32 * W + W / 2) * 4) as usize];
        let (home, field) = (m.content_top, widgets::search_layout(&t).0.y);
        // The device card under the home page's bar.
        let card = m.content_top + m.search_h + m.after_search + m.dp(20.0);
        app.compose(&mut screen, &f, &i, "12:00", 0);
        assert_eq!(grey(&screen, card), t.pal.card.r);

        tap_at(&mut app, &f, &i, Action::Open(Screen::Search), 1000);
        assert_eq!(app.screen(), Screen::Search);
        assert_eq!(app.deadline(1000), Some(0), "it moves frame by frame");
        app.compose(&mut screen, &f, &i, "12:00", 1067);
        let along = crate::motion::settle(67, MORPH_PERIOD);
        let expected = home + (field - home) * along;
        let at = top(&screen).expect("the bar is on screen");
        assert!((at - expected).abs() <= 2.0, "up {along} of the way after 67 ms: at {at}, not {expected}");
        assert_eq!(grey(&screen, card), t.pal.bg.r, "the home page's cards go at once");
        app.compose(&mut screen, &f, &i, "12:00", 2500);
        assert!(app.morph.is_none(), "settled, the search page shows as itself");
        assert_eq!(top(&screen), Some(field.ceil()));

        tap_at(&mut app, &f, &i, Action::CloseSearch, 3000);
        assert_eq!(app.screen(), Screen::Home);
        app.compose(&mut screen, &f, &i, "12:00", 3067);
        let expected = field + (home - field) * along;
        let at = top(&screen).expect("the bar is on screen");
        assert!((at - expected).abs() <= 2.0, "down {along} of the way after 67 ms: at {at}, not {expected}");
        assert_eq!(grey(&screen, card + (at - home)), t.pal.card.r, "the cards come down under the bar");
        app.compose(&mut screen, &f, &i, "12:00", 4500);
        assert!(app.morph.is_none());
        assert_eq!(top(&screen), Some(home.ceil()));
    }

    /// A screen too short for the home page's rows, which scrolls well past
    /// the fold of its search bar.
    const SHORT: u32 = 2600;

    #[test]
    fn the_home_search_bar_folds_away_under_its_still_title_before_the_title_goes() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, SHORT, PPD);
        let m = t.m;
        let mut app = App::new(Screen::Home, W, SHORT, PPD);
        let fold = app.frame(&f, &i).fold.expect("the home page's search bar folds");
        assert!((fold.span - (m.search_h + m.after_search)).abs() < 1e-3, "the bar and the gap under it");
        let mut screen = Canvas::new(W, SHORT);
        // Down a column through the bar clear of its words: the rows in the
        // bar's grey, and the first row of the device card's white.
        let x = m.dp(300.0) as u32;
        let px = |screen: &Canvas, y: u32| screen.data[((y * W + x) * 4) as usize];
        let bar = |screen: &Canvas| (0..SHORT).filter(|&y| px(screen, y) == t.pal.search.r).collect::<Vec<_>>();
        let card = |screen: &Canvas| (fold.bar.y as u32..SHORT).find(|&y| px(screen, y) == t.pal.card.r).expect("the card shows");
        // The large title's band.
        let (top, bottom) = ((m.title_top - m.dp(14.0)) as u32, fold.bar.y as u32);
        let title = |screen: &Canvas| screen.crop(0, top, W, bottom - top).data.clone();

        app.compose(&mut screen, &f, &i, "12:00", 0);
        let (whole, still, first) = (bar(&screen), title(&screen), card(&screen));
        assert_eq!((whole[0], *whole.last().expect("the bar shows")), (fold.bar.y.ceil() as u32, (fold.bar.y + fold.bar.h).floor() as u32 - 1));

        let half = (fold.bar.h / 2.0).round();
        scroll_to(&mut app, &f, half, 1000);
        app.compose(&mut screen, &f, &i, "12:00", 1800);
        let rows = bar(&screen);
        assert_eq!(rows[0], whole[0], "its top stays");
        assert_eq!(*rows.last().expect("half the bar shows"), (fold.bar.y + fold.bar.h - half).floor() as u32 - 1, "its foot rises with the page");
        assert_eq!(card(&screen), first - half as u32, "the content follows the finger");
        assert!(title(&screen) == still, "the title holds still");

        scroll_to(&mut app, &f, fold.span, 3000);
        app.compose(&mut screen, &f, &i, "12:00", 3800);
        assert!(bar(&screen).is_empty(), "folded away");
        assert_eq!(card(&screen), first - fold.span.round() as u32, "the gap under it closed");
        assert!(title(&screen) == still, "the title holds still till then");

        scroll_to(&mut app, &f, fold.span + m.dp(10.0), 5000);
        app.compose(&mut screen, &f, &i, "12:00", 5800);
        assert!(title(&screen) != still, "then it goes up with the page");
        assert!(!app.title.on, "the bar's title waits for the large one to fade");
        scroll_to(&mut app, &f, fold.span + m.dp(TITLE_FADE_DP) + 2.0, 7000);
        assert!(app.title.on);
    }

    #[test]
    fn a_home_page_let_go_with_its_search_bar_part_folded_folds_it_whole_or_not_at_all() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, SHORT, PPD);
        let span = app.frame(&f, &i).fold.expect("the home page's search bar folds").span;
        let settle = |app: &mut App, from: u64| {
            for ms in (from..from + 2000).step_by(8) {
                app.tick(ms);
            }
            app.top().1
        };
        scroll_to(&mut app, &f, span * (FOLD_SETTLE - 0.1), 0);
        assert_eq!(settle(&mut app, 800), 0.0, "short of the mark it unfolds");
        scroll_to(&mut app, &f, span * (FOLD_SETTLE + 0.1), 3000);
        assert_eq!(settle(&mut app, 3800), span, "past it it folds away");
        scroll_to(&mut app, &f, span + 50.0, 6000);
        assert_eq!(settle(&mut app, 6800), span + 50.0, "a page scrolled past the fold stays");

        // A light flick down, a thousand pixels a second, let go past the
        // fold, that glides to rest with the bar part folded.
        scroll_to(&mut app, &f, span + 86.0, 8000);
        app.touch(Touch::Down(10.0, 1000.0), &f, 9000);
        for k in 1..=7u64 {
            app.touch(Touch::Move(10.0, 1000.0 + 8.0 * k as f32), &f, 9000 + 8 * k);
        }
        app.touch(Touch::Up, &f, 9056);
        let glide = app.fling.expect("it glides");
        let (rest, _) = glide.at(u64::MAX);
        assert!(rest > 0.0 && rest < span, "this one comes to rest in the fold: {rest}");
        let folded = if rest < span * FOLD_SETTLE { 0.0 } else { span };
        assert_eq!(settle(&mut app, 9056), folded, "it settles there the same way");

        // Let go in the fold while still moving, too slowly to glide out of
        // it: the bar settles at once from where it was let go.
        scroll_to(&mut app, &f, 0.0, 12_000);
        app.touch(Touch::Down(10.0, 1000.0), &f, 13_000);
        for k in 1..=5u64 {
            app.touch(Touch::Move(10.0, 1000.0 - 30.0 * k as f32), &f, 13_000 + 8 * k);
        }
        for k in 1..=14u64 {
            app.touch(Touch::Move(10.0, 850.0 + 2.0 * k as f32), &f, 13_040 + 8 * k);
        }
        app.touch(Touch::Up, &f, 13_152);
        assert_eq!(app.top().1, 122.0);
        assert!(app.fling.is_none(), "no glide");
        assert_eq!(app.seek.map(|(rest, _)| rest), Some(span), "it was let go past the mark");
        assert_eq!(settle(&mut app, 13_152), span);

        // A flick that carries the page out of the fold glides on.
        scroll_to(&mut app, &f, 0.0, 16_000);
        app.touch(Touch::Down(10.0, 1000.0), &f, 17_000);
        for k in 1..=4u64 {
            app.touch(Touch::Move(10.0, 1000.0 - 30.0 * k as f32), &f, 17_000 + 8 * k);
        }
        app.touch(Touch::Up, &f, 17_032);
        let (end, _) = app.fling.expect("it glides").at(u64::MAX);
        assert!(end > span, "on past the fold: {end}");
    }

    #[test]
    fn a_search_bar_folded_away_answers_no_touch() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let fold = app.frame(&f, &i).fold.expect("the home page's search bar folds");
        scroll_to(&mut app, &f, fold.span, 0);
        app.tick(3000);
        // Where the page carried the bar's middle had the bar scrolled with
        // it: under the large title.
        let (x, y) = fold.bar.centre();
        app.touch(Touch::Down(x, y - fold.span), &f, 3000);
        assert_eq!(app.pressing(), None, "nothing shows held");
        app.touch(Touch::Up, &f, 3000);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn the_keys_unfold_the_search_bar_to_light_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let span = app.frame(&f, &i).fold.expect("the home page's search bar folds").span;
        scroll_to(&mut app, &f, span, 0);
        app.tick(3000);
        press(&mut app, Key::VolumeDown, 3000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))), "a bar folded away is not in view");
        press(&mut app, Key::VolumeUp, 3000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        for ms in (3000..5000).step_by(8) {
            app.tick(ms);
        }
        assert_eq!(app.top().1, 0.0, "the page went back up to unfold it");
        press(&mut app, Key::Power, 5000);
        assert_eq!(app.screen(), Screen::Search);
    }

    #[test]
    fn a_search_bar_pulled_down_past_the_top_comes_down_whole_with_its_title() {
        let fold = Fold { bar: Rect::new(36.0, 503.0, 1128.0, 131.0), hint: "", span: 169.0 };
        assert_eq!(fold.shown(-30.0), Some(Rect::new(36.0, 533.0, 1128.0, 131.0)));
        assert_eq!(fold.lift(-30.0), -30.0);
        assert_eq!(fold.shown(31.0), Some(Rect::new(36.0, 503.0, 1128.0, 100.0)));
        assert_eq!(fold.shown(131.0), None);
        assert_eq!((fold.lift(150.0), fold.lift(200.0)), (0.0, 31.0));
    }

    /// The grey of the screen's pixel at (`x`, `y`).
    fn grey_at(c: &Canvas, x: f32, y: f32) -> u8 {
        c.data[((y as u32 * c.w + x as u32) * 4) as usize]
    }

    /// The darkest grey of the screen within `r`.
    fn darkest(c: &Canvas, r: Rect) -> u8 {
        let (x0, y0, x1, y1) = (r.x as u32, r.y as u32, (r.x + r.w) as u32, (r.y + r.h) as u32);
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| c.data[((y * c.w + x) * 4) as usize])).min().expect("a part of the screen")
    }

    /// A copy of the screen.
    fn shot(c: &Canvas) -> Canvas {
        c.clone()
    }

    /// How a slide opening a page, or going `back`, stands `ms` into it:
    /// where the page in front stands, how far left the page behind is, and
    /// how much of its light the page behind keeps, in 256ths.
    fn slid(back: bool, ms: u64) -> (u32, u32, u32) {
        let (x, _) = if back { Spring::new(0, -1000.0, 0.0, BACK_PERIOD) } else { Spring::new(0, 1000.0, -OPEN_SPEED * 1000.0, OPEN_PERIOD) }.at(ms);
        let along = if back { 1.0 + x / 1000.0 } else { x / 1000.0 };
        let front = (along * W as f32).round() as u32;
        let behind = (BEHIND_SHIFT * W as f32 * (1.0 - along)).round() as u32;
        let keep = ((1.0 - BEHIND_DIM * (1.0 - along)) * 256.0).round() as u32;
        (front, behind, keep)
    }

    /// Check row `y` of `mid`: the page `ahead` from `front` on, and left of
    /// it the page `under`, `behind` pixels left of rest and keeping `keep`
    /// 256ths of its light.
    fn check_slide(mid: &Canvas, ahead: &Canvas, under: &Canvas, y: u32, (front, behind, keep): (u32, u32, u32)) {
        for x in 0..W {
            let at = |c: &Canvas, x: u32| c.data[((y * W + x) * 4) as usize] as u32;
            let want = if x >= front { at(ahead, x - front) } else { (at(under, x + behind) * keep) >> 8 };
            let got = at(mid, x);
            assert!(got.abs_diff(want) <= 1, "column {x} of row {y}: {got}, not {want} (front {front}, behind {behind})");
        }
    }

    #[test]
    fn a_page_opened_slides_in_over_the_page_it_opens_from_and_back_off_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        let home = shot(&c);
        tap_at(&mut app, &f, &i, Action::Open(Screen::Mount), 1000);
        assert_eq!(app.screen(), Screen::Mount);
        assert_eq!(app.deadline(1000), Some(0), "it slides frame by frame");
        // The slide sets off with its first frame, the page opened still
        // past the right edge.
        app.compose(&mut c, &f, &i, "12:00", 1000);
        let first = shot(&c);
        app.compose(&mut c, &f, &i, "12:00", 1050);
        let mid = shot(&c);
        app.compose(&mut c, &f, &i, "12:00", 3000);
        assert!(app.turn.is_none(), "come to rest, the page left is let go");
        assert_ne!(app.deadline(3000), Some(0));
        let mount = shot(&c);
        for y in [H / 3, H / 2, H - 200] {
            check_slide(&first, &mount, &home, y, (W, 0, 256));
            check_slide(&mid, &mount, &home, y, slid(false, 50));
        }
        let (front, _, _) = slid(false, 50);
        assert!((front as f32 / W as f32 - 0.728).abs() < 0.01, "about three quarters of the way in after 50 ms: {front}");

        tap_back(&mut app, &f, &i, 4000);
        assert_eq!(app.screen(), Screen::Home);
        app.compose(&mut c, &f, &i, "12:00", 4000);
        let first = shot(&c);
        app.compose(&mut c, &f, &i, "12:00", 4060);
        let mid = shot(&c);
        app.compose(&mut c, &f, &i, "12:00", 6000);
        assert!(app.turn.is_none());
        let home = shot(&c);
        for y in [H / 3, H / 2, H - 200] {
            check_slide(&first, &mount, &home, y, (0, W / 4, ((1.0 - BEHIND_DIM) * 256.0).round() as u32));
            check_slide(&mid, &mount, &home, y, slid(true, 60));
        }
    }

    #[test]
    fn a_page_sliding_has_its_leading_corners_rounded_as_the_screens_are() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.set_screen_corners(140.0);
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        let home = shot(&c);
        tap_at(&mut app, &f, &i, Action::Open(Screen::Mount), 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        app.compose(&mut c, &f, &i, "12:00", 1050);
        let mid = shot(&c);
        app.compose(&mut c, &f, &i, "12:00", 3000);
        let mount = shot(&c);
        let (front, behind, keep) = slid(false, 50);
        let at = |c: &Canvas, x: u32, y: u32| c.data[((y * W + x) * 4) as usize] as u32;
        let y = H - 3;
        assert_eq!(at(&mid, front + 5, y), (at(&home, front + 5 + behind, y) * keep) >> 8, "the corner's tip shows the page behind");
        assert_eq!(at(&mid, front + 140, y), at(&mount, 140, y), "past the corner's curve the page in front is whole");
        assert_eq!(at(&mid, front + 5, H / 2), at(&mount, 5, H / 2), "between the corners its edge is straight");
    }

    #[test]
    fn a_finger_brings_a_sliding_page_to_rest_and_a_page_never_shown_gives_way_at_once() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        tap_at(&mut app, &f, &i, Action::Open(Screen::Mount), 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        app.touch(Touch::Down(W as f32 / 2.0, H as f32 / 2.0), &f, 1030);
        assert!(app.turn.is_none(), "a finger brings it to rest");
        app.touch(Touch::Up, &f, 1060);

        // A page rendered anew since it last showed is not drawn as it
        // stands: what replaces it shows at once.
        app.refresh();
        tap_back(&mut app, &f, &i, 2000);
        assert_eq!(app.screen(), Screen::Home);
        assert!(app.turn.is_none());
    }

    #[test]
    fn a_row_held_darkens_behind_its_words_a_moment_after_the_finger_lands() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Home, W, H, PPD);
        let mut c = Canvas::new(W, H);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Backup)).expect("on the home page").rect;
        // The card between the row's words and its arrow.
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        app.compose(&mut c, &f, &i, "12:00", 0);
        let ink = darkest(&c, row);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r);

        app.touch(Touch::Down(x, y), &f, 1000);
        let rise = 1000 + ROW_PRESS_DELAY_MS;
        assert_eq!(app.deadline(1000), Some(rise), "nothing moves until the row starts darkening");
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert!(app.row_art.is_none(), "a finger that may yet scroll does not wait for the row drawn held down");
        app.compose(&mut c, &f, &i, "12:00", rise);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r, "it has not started yet");
        app.compose(&mut c, &f, &i, "12:00", rise + 30);
        let partway = grey_at(&c, x, y);
        assert!(partway < 255 && partway > 225, "darkening after 30 ms: {partway}");
        app.compose(&mut c, &f, &i, "12:00", 2000);
        assert_eq!(grey_at(&c, x, y), 225, "as dark as the stock settings' row held down");
        assert_eq!(darkest(&c, row), ink, "its words keep their ink");
        assert_ne!(app.deadline(2000), Some(0), "held still, nothing moves");

        // The finger moves on to scroll: the row fades back.
        app.touch(Touch::Move(x, y + t.m.dp(30.0)), &f, 2100);
        assert_eq!(app.deadline(2100), Some(0));
        app.touch(Touch::Up, &f, 2200);
        app.tick(3000);
        assert!(app.row_press.is_none() && app.row_art.is_none(), "faded, it is let go");
        assert_eq!(app.screen(), Screen::Home, "a scroll opens nothing");
    }

    #[test]
    fn a_row_scrolled_away_from_before_it_darkens_never_shows_held() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Home, W, H, PPD);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Backup)).expect("on the home page").rect;
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 1000);
        app.touch(Touch::Move(x, y + t.m.dp(30.0)), &f, 1050);
        assert!(app.row_press.is_none());
    }

    #[test]
    fn a_row_tapped_quickly_shows_held_from_the_lift_and_fades_back() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let mut c = Canvas::new(W, H);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Toggle(Switch::Vibrate)).expect("on the settings page").rect;
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        tap_at(&mut app, &f, &i, Action::Toggle(Switch::Vibrate), 1000);
        assert!(!app.state.vibrate, "the tap worked the switch");
        app.compose(&mut c, &f, &i, "12:00", 1040);
        let held = grey_at(&c, x, y);
        assert!(held < t.pal.card.r, "held from the lift: {held}");
        app.tick(1040 + QUICK_TAP_MS + 1000);
        assert!(app.row_press.is_none(), "and let go once faded");
        app.compose(&mut c, &f, &i, "12:00", 2200);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r);
    }

    #[test]
    fn the_back_control_and_the_search_buttons_darken_at_once_under_a_finger() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut c = Canvas::new(W, H);

        // `<`: a disc behind it, the `<` lighter on it.
        let mut app = opened(Screen::Mount, &f, &i);
        let (cx, cy, r) = widgets::back_disc(&t);
        let (bx, by) = widgets::back_rect(&t).centre();
        let around = Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
        app.compose(&mut c, &f, &i, "12:00", 5000);
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), t.pal.bg.r);
        assert_eq!(darkest(&c, around), 0);
        app.touch(Touch::Down(bx, by), &f, 6000);
        app.compose(&mut c, &f, &i, "12:00", 6000);
        let disc = over(t.pal.button_press, t.pal.bg).r;
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), disc, "darker at once");
        let arrow = darkest(&c, around);
        assert!(arrow.abs_diff(85) <= 2, "the stock `<` held down is 85, not {arrow}");
        // A finger moving off lets it go.
        app.touch(Touch::Move(bx, by + t.m.dp(40.0)), &f, 6050);
        app.compose(&mut c, &f, &i, "12:00", 6050);
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), t.pal.bg.r);
        app.touch(Touch::Up, &f, 6100);
        assert_eq!(app.screen(), Screen::Mount);

        // The search page's round button.
        let mut app = App::new(Screen::Search, W, H, PPD);
        app.frame(&f, &i);
        let (_, button) = widgets::search_layout(&t);
        let (x, y) = button.centre();
        app.touch(Touch::Down(x, y), &f, 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert_eq!(grey_at(&c, x, y + button.h * 0.35), over(t.pal.button_press, t.pal.search).r);
        let cross = darkest(&c, button);
        assert!(cross.abs_diff(70) <= 2, "the stock cross held down is 70, not {cross}");

        // The home page's search bar, clear of its words.
        let mut app = App::new(Screen::Home, W, H, PPD);
        let bar = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Search)).expect("on the home page").rect;
        let (x, y) = (bar.x + bar.w * 0.8, bar.y + bar.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert_eq!(grey_at(&c, x, y), over(t.pal.button_press, t.pal.search).r);
    }

    #[test]
    fn an_unreadable_partition_table_leaves_nothing_to_toggle() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Mount, &f, &i);
        app.set_mounts(Err("read the metadata in super: Input/output error (os error 5)".into()));
        assert!(app.frame(&f, &i).hits.is_empty());
    }

    #[test]
    fn partitions_mount_while_an_update_waits_to_merge() {
        let (f, mut i) = (fonts(), info());
        i.snapshot = SnapshotState::Snapshotted;
        let mut app = opened(Screen::Mount, &f, &i);
        app.set_mounts(Ok(vec![
            MountRow { base: "vendor".into(), point: "/vendor".into(), mounted: false, holders: Vec::new() },
            MountRow { base: "odm".into(), point: "/odm".into(), mounted: true, holders: Vec::new() },
        ]));
        let frame = app.frame(&f, &i);
        assert_eq!(frame.hits.iter().map(|h| h.action).collect::<Vec<_>>(), [Action::MountToggle(0), Action::MountToggle(1)]);
        assert_eq!(tap(&mut app, &f, &i, Action::MountToggle(0)), vec![Effect::Mount("vendor".into())], "through its snapshot");
    }

    #[test]
    fn an_update_waiting_in_the_slot_merges_with_a_swipe_and_shows_how_far_it_has_come() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        app.set_update(Standing::Waiting);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate], "the page reads where the update stands");
        assert_eq!(app.screen(), Screen::Update);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Merge), vec![Effect::Merge]);
        assert_eq!(app.state.update, Standing::Merging(0.0), "the page turns to the merge at once");
        assert!(app.frame(&f, &i).hits.is_empty(), "nothing to swipe while it merges");
        app.set_update(Standing::Merging(0.5));
        assert!(app.frame(&f, &i).hits.is_empty());
        app.set_update(Standing::Merged);
        assert!(app.frame(&f, &i).hits.is_empty(), "merged, nothing is left to do here");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Advanced);
        app.set_update(Standing::Installed(Slot::A));
        tap(&mut app, &f, &i, Action::Open(Screen::Update));
        assert!(app.frame(&f, &i).hits.is_empty(), "an update in the other slot merges once that slot has started");
    }

    #[test]
    fn the_home_pages_word_on_a_waiting_update_opens_its_page() {
        let (f, mut i) = (fonts(), info());
        i.snapshot = SnapshotState::Snapshotted;
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate]);
        assert_eq!(app.screen(), Screen::Update);
    }

    #[test]
    fn the_mount_page_toggles_each_partition() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Mount, &f, &i);
        app.set_mounts(Ok(vec![
            MountRow { base: "system".into(), point: "/mnt/system".into(), mounted: true, holders: Vec::new() },
            MountRow { base: "vendor".into(), point: "/vendor".into(), mounted: false, holders: Vec::new() },
        ]));
        let frame = app.frame(&f, &i);
        assert!(frame.hits.iter().any(|h| h.action == Action::MountToggle(0)), "the first partition has a toggle");
        assert!(frame.hits.iter().any(|h| h.action == Action::MountToggle(1)), "the second partition has a toggle");
        assert!(!frame.hits.iter().any(|h| matches!(h.action, Action::Toggle(_))), "partitions mount read-only");
        // A mounted partition unmounts, an unmounted one mounts.
        assert_eq!(app.act(Action::MountToggle(0), 0), vec![Effect::Unmount("system".into())]);
        assert_eq!(app.act(Action::MountToggle(1), 0), vec![Effect::Mount("vendor".into())]);
    }

    #[test]
    fn a_partition_in_use_cannot_be_unmounted() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Mount, &f, &i);
        let held = vec!["touch_report".to_string(), "touchsensor".to_string()];
        app.set_mounts(Ok(vec![
            MountRow { base: "odm".into(), point: "/odm".into(), mounted: true, holders: held },
            MountRow { base: "vendor".into(), point: "/vendor".into(), mounted: true, holders: Vec::new() },
        ]));
        let frame = app.frame(&f, &i);
        assert!(matches!(frame.hits[0].action, Action::Unavailable(_)), "the programs using odm keep it");
        assert_eq!(frame.hits[1].action, Action::MountToggle(1));
        let busy = frame.hits[0].action;
        assert!(tap(&mut app, &f, &i, busy).is_empty());
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(TOAST_MS), "it says why");
    }

    #[test]
    fn a_failed_mount_stays_on_the_page_until_the_next_try() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Mount, &f, &i);
        app.set_mounts(Ok(vec![MountRow { base: "vendor".into(), point: "/vendor".into(), mounted: false, holders: Vec::new() }]));
        let failure = MountFailure { mounting: true, base: "vendor".into(), detail: "mount vendor_b: no filesystem".into() };
        app.mount_failed(failure.clone());
        assert_eq!(app.state.mount_failure, Some(failure.clone()));
        assert_eq!(app.act(Action::MountToggle(0), 0), vec![Effect::Mount("vendor".into())]);
        assert_eq!(app.state.mount_failure, None, "trying again clears it");
        app.mount_failed(failure);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(app.state.mount_failure, None, "leaving the page clears it");
    }

    #[test]
    fn the_mount_page_notes_when_there_is_nothing_to_mount() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Mount, &f, &i);
        let frame = app.frame(&f, &i);
        assert!(frame.hits.is_empty(), "no partition rows, and nothing else to touch");
    }

    #[test]
    fn swipe_from_the_left_edge_goes_back() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(90.0, 1605.0), &f, 0);
        app.touch(Touch::Move(320.0, 1610.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn a_short_edge_swipe_does_not_go_back() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.screen(), Screen::Reboot);
    }

    #[test]
    fn the_first_page_has_no_edge_swipe_back() {
        let f = fonts();
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 8);
        assert!(app.touch(Touch::Move(400.0, 1600.0), &f, 16).is_empty(), "no tap of the motor");
        assert!(!matches!(app.drag, Drag::Edge { .. }), "no cue");
        app.touch(Touch::Up, &f, 24);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(app.deadline(24), app.idle_due(), "nothing left on screen to settle");
    }

    #[test]
    fn the_back_cue_taps_once_as_it_arms_and_sinks_when_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        let ready = app.theme().m.dp(BACK_SWIPE_DP);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        assert!(app.touch(Touch::Move(80.0, 1600.0), &f, 8).is_empty());
        assert_eq!(app.touch(Touch::Move(20.0 + ready, 1600.0), &f, 16), vec![Effect::Haptic]);
        assert!(app.touch(Touch::Move(60.0 + ready, 1600.0), &f, 24).is_empty(), "one tap while armed");
        app.touch(Touch::Move(40.0, 1600.0), &f, 32);
        app.touch(Touch::Up, &f, 40);
        assert_eq!(app.screen(), Screen::Reboot, "pulled back in, it does not go back");
        assert_eq!(app.deadline(40), Some(0), "the cue is sinking back");
        app.tick(1000);
        assert_eq!(app.deadline(1000), app.idle_due());
    }

    #[test]
    fn the_back_cue_stays_quiet_with_touch_vibration_off() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.state.vibrate = false;
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 8);
        assert!(app.touch(Touch::Move(400.0, 1600.0), &f, 16).is_empty());
    }

    #[test]
    fn the_back_cue_keeps_to_the_height_the_swipe_began_at() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        let at = 1600.0 - app.theme().m.dp(CUE_LIFT_DP);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1610.0), &f, 8);
        app.touch(Touch::Move(200.0, 1300.0), &f, 16);
        app.touch(Touch::Move(150.0, 1900.0), &f, 24);
        let Drag::Edge { cy, .. } = app.drag else { panic!("still swiping in from the edge") };
        assert_eq!(cy, at, "the finger wandered up and down");
        app.touch(Touch::Move(40.0, 2100.0), &f, 32);
        app.touch(Touch::Up, &f, 40);
        assert_eq!(app.cue.as_ref().map(|c| c.1), Some(at), "it sinks back where it was");
    }

    #[test]
    fn pulling_past_the_top_resists_and_springs_back() {
        let f = fonts();
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        app.touch(Touch::Down(700.0, 1000.0), &f, 0);
        app.touch(Touch::Move(700.0, 1300.0), &f, 100);
        app.touch(Touch::Move(700.0, 1800.0), &f, 600);
        let shown = app.overscroll(600);
        assert!(shown < 0.0 && shown > -700.0, "the page follows less than the finger: {shown}");
        assert_eq!(app.top().1, 0.0);
        // Dragging back first undoes the pull.
        app.touch(Touch::Move(700.0, 1400.0), &f, 1000);
        assert!(app.overscroll(1000) > shown);
        app.touch(Touch::Up, &f, 1400);
        assert!(app.overscroll(1400) < 0.0);
        assert_eq!(app.deadline(1400), Some(0));
        app.tick(1500);
        assert!(app.overscroll(1500) > app.overscroll(1400), "on its way back");
        app.tick(3000);
        assert_eq!(app.overscroll(3000), 0.0);
        assert_eq!(app.deadline(3000), app.idle_due());
    }

    #[test]
    fn a_page_pulled_past_its_end_can_be_caught() {
        let f = fonts();
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        app.touch(Touch::Down(700.0, 1000.0), &f, 0);
        app.touch(Touch::Move(700.0, 1500.0), &f, 500);
        app.touch(Touch::Up, &f, 1000);
        app.tick(1050);
        let held = app.overscroll(1050);
        app.touch(Touch::Down(700.0, 1200.0), &f, 1050);
        app.tick(1500);
        assert!((app.overscroll(1500) - held).abs() < 1.0, "the finger holds it where it was");
        app.touch(Touch::Up, &f, 1600);
        app.tick(4000);
        assert_eq!(app.overscroll(4000), 0.0);
    }

    #[test]
    fn scrolling_back_down_past_the_top_pulls_from_the_top() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        app.touch(Touch::Down(700.0, 2000.0), &f, 0);
        app.touch(Touch::Move(700.0, 1700.0), &f, 100);
        assert_eq!(app.top().1, 300.0);
        app.touch(Touch::Move(700.0, 2100.0), &f, 500);
        assert_eq!(app.top().1, 0.0);
        assert!(app.overscroll(500) < 0.0, "the last 100 px pull the page down");
    }

    #[test]
    fn dragging_scrolls_and_does_not_tap() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let (x, y) = spot(&mut app, &f, &i, Action::Toggle(Switch::Vibrate));
        assert!(y < H as f32, "the switch is on the first screen");
        app.touch(Touch::Down(x, y), &f, 0);
        app.touch(Touch::Move(x, y - 100.0), &f, 0);
        app.touch(Touch::Move(x, y - 300.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert!(app.state.vibrate, "the switch was not flipped");
        assert_eq!(app.top().1, 300.0);
    }

    #[test]
    fn the_file_transfer_switch_has_the_runtime_follow_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        assert!(app.state.file_transfer, "on until turned off");
        let (_, y) = spot(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer));
        if y > H as f32 * 0.9 {
            scroll_to(&mut app, &f, y - H as f32 * 0.8, 0);
        }
        assert_eq!(tap_at(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer), 2000), [Effect::FileTransfer(false)]);
        assert!(!app.state.file_transfer);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer), 3000), [Effect::FileTransfer(true)]);
        app.file_transfer_failed(false, "无法开启文件传输", 4000);
        assert!(!app.state.file_transfer, "the switch shows how it stands");
    }

    /// Flick the page up from `start`: the finger travels `step` px every
    /// 8 ms five times and lifts 8 ms after its last move. Returns when it
    /// lifted.
    fn flick(app: &mut App, f: &Fonts, start: u64, step: f32) -> u64 {
        app.touch(Touch::Down(700.0, 2400.0), f, start);
        for k in 1..=5u64 {
            app.touch(Touch::Move(700.0, 2400.0 - step * k as f32), f, start + 8 * k);
        }
        let lift = start + 48;
        app.touch(Touch::Up, f, lift);
        lift
    }

    #[test]
    fn a_flick_glides_on_and_comes_to_rest() {
        let f = fonts();
        // Every reading there is, for a page long enough to glide on.
        let battery = skyward_info::Battery {
            level: 80,
            charge: Charge::Discharging,
            temp: Some(300),
            health: Some("Good".into()),
            cycles: Some(169),
            design: Some(5_500_000),
            full: Some(5_095_000),
            voltage: Some(4_200_000),
            quick: false,
        };
        let i = DeviceInfo {
            os_version: Some("OS3.0.305.0.WMACNXM".into()),
            android: Some("16".into()),
            battery: Some(battery),
            ..info()
        };
        let mut app = App::new(Screen::Specs, W, H, PPD);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 20.0);
        let lifted = app.top().1;
        assert_eq!(lifted, 100.0);
        assert_eq!(app.deadline(lift), Some(0), "the page moves on after the lift");
        app.tick(lift + 50);
        assert!(app.top().1 > lifted);
        for ms in (100..5000).step_by(8) {
            app.tick(lift + ms);
        }
        assert_eq!(app.deadline(lift + 5000), app.idle_due(), "the glide comes to rest");
        let rest = app.top().1;
        assert!(rest > lifted + 300.0 && rest < app.max_scroll(), "{rest}");
    }

    #[test]
    fn a_page_drawn_again_as_work_goes_on_keeps_its_scroll() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.backup_started(false, Place::Storage, true);
        app.backup_plan((0..20).map(|k| (format!("part{k}"), 1_000_000)).collect());
        let at = |done| BackupAt { item: 0, done, of: 1_000_000, checking: false };
        app.backup_at(at(0));
        app.frame(&f, &i);
        // A finger drags the page up while the work tells how far it has
        // come, each time drawing the page again.
        app.touch(Touch::Down(700.0, 2400.0), &f, 0);
        for k in 1..=4u64 {
            app.touch(Touch::Move(700.0, 2400.0 - 200.0 * k as f32), &f, 8 * k);
        }
        let dragged = app.top().1;
        assert!(dragged > 0.0);
        app.backup_at(at(500_000));
        app.touch(Touch::Move(700.0, 1400.0), &f, 40);
        assert_eq!((app.top().1, app.pull), (dragged + 200.0, 0.0), "the page goes on from where it was");
        app.touch(Touch::Up, &f, 48);
        let lifted = app.top().1;
        app.backup_at(at(600_000));
        app.tick(98);
        assert!(app.top().1 > lifted, "a glide goes on through the page being drawn again");
        assert_eq!(app.overscroll(98), 0.0, "and does not spring back from the top");
    }

    #[test]
    fn a_glide_into_the_end_swings_past_it_and_back() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 80.0);
        let mut furthest: f32 = 0.0;
        for ms in (8..2000).step_by(8) {
            app.tick(lift + ms);
            furthest = furthest.max(app.overscroll(lift + ms));
        }
        assert_eq!(app.top().1, app.max_scroll());
        assert!(furthest > app.theme().m.dp(8.0), "it ran on past the end: {furthest}");
        assert_eq!(app.overscroll(lift + 2000), 0.0);
        assert_eq!(app.deadline(lift + 2000), app.idle_due());
    }

    #[test]
    fn a_finger_that_stops_before_lifting_leaves_the_page_still() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        app.touch(Touch::Down(700.0, 2400.0), &f, 1000);
        app.touch(Touch::Move(700.0, 2200.0), &f, 1008);
        app.touch(Touch::Move(700.0, 2000.0), &f, 1016);
        app.touch(Touch::Up, &f, 1100);
        assert!(app.fling.is_none());
        assert_eq!(app.top().1, 400.0);
    }

    #[test]
    fn touching_a_gliding_page_stops_it_without_pressing() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 20.0);
        app.tick(lift + 30);
        assert!(app.fling.is_some(), "the page is gliding");
        let held = app.top().1;
        let (x, y) = spot(&mut app, &f, &i, Action::Toggle(Switch::Vibrate));
        assert!(y > 0.0 && y < H as f32, "the switch is on screen");
        app.touch(Touch::Down(x, y), &f, lift + 40);
        app.tick(lift + 500);
        assert_eq!(app.top().1, held, "the page stopped under the finger");
        app.touch(Touch::Up, &f, lift + 520);
        assert!(app.state.vibrate, "the switch was not flipped");
        assert!(app.fling.is_none());
    }

    #[test]
    fn an_edge_swipe_goes_back_over_a_gliding_page() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 20.0);
        app.tick(lift + 30);
        app.touch(Touch::Down(10.0, 1600.0), &f, lift + 40);
        app.touch(Touch::Move(90.0, 1605.0), &f, lift + 48);
        let under = app.top().1;
        app.tick(lift + 64);
        assert!(app.top().1 > under, "the page glides on under the swipe");
        app.touch(Touch::Move(400.0, 1610.0), &f, lift + 72);
        app.touch(Touch::Up, &f, lift + 80);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn a_finger_dragging_from_the_edge_catches_a_gliding_page() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 20.0);
        app.tick(lift + 30);
        app.touch(Touch::Down(10.0, 1600.0), &f, lift + 40);
        app.tick(lift + 48);
        assert!(app.fling.is_some(), "landing at the edge leaves the glide going");
        app.touch(Touch::Move(12.0, 1500.0), &f, lift + 56);
        assert!(app.fling.is_none(), "dragging the page catches it");
        let held = app.top().1;
        app.touch(Touch::Move(12.0, 1450.0), &f, lift + 64);
        assert_eq!(app.top().1, held + 50.0, "and it follows the finger from there");
    }

    #[test]
    fn a_button_stops_a_gliding_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        let lift = flick(&mut app, &f, 1000, 20.0);
        assert!(app.fling.is_some(), "the page is gliding");
        app.key(Key::Back, lift + 20);
        assert!(app.fling.is_none());
    }

    /// Drag the page to scroll `to`, resting before lifting so it does not
    /// glide, and let the time pass. The finger first goes 100 px, past the
    /// touch slop, and comes back to the mark. It runs down the page's
    /// margin, clear of the sliders, which a finger landing on moves
    /// instead of the page.
    fn scroll_to(app: &mut App, f: &Fonts, to: f32, now: u64) {
        let (from, x) = (app.top().1, app.theme().m.margin / 2.0);
        app.touch(Touch::Down(x, 2400.0), f, now);
        app.touch(Touch::Move(x, 2300.0), f, now + 200);
        app.touch(Touch::Move(x, 2400.0 - (to - from)), f, now + 400);
        app.touch(Touch::Up, f, now + 800);
        app.tick(now + 800);
        assert!((app.top().1 - to).abs() < 0.01, "{} {to}", app.top().1);
    }

    #[test]
    fn the_bar_title_comes_after_the_large_title_fades_and_the_bar_once_content_slides_under() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let fr = app.frame(&f, &i);
        let content_top = fr.content_top;
        let m = app.theme().m;
        scroll_to(&mut app, &f, m.dp(TITLE_FADE_DP) - 2.0, 0);
        assert!(!app.title.on && !app.bar.on);
        scroll_to(&mut app, &f, m.dp(TITLE_FADE_DP) + 2.0, 1000);
        assert!(app.title.on, "the large title is gone");
        assert!(!app.bar.on, "the content is not under the bar yet");
        assert!(app.title.level(1900) < 1.0 && app.title.level(1900) > 0.0, "it eases in");
        assert_eq!(app.title.level(3000), 1.0);
        let under = content_top - widgets::bar_bottom(&app.theme()) - m.dp(20.0);
        scroll_to(&mut app, &f, under + 10.0, 4000);
        assert!(app.bar.on);
        assert_eq!(app.bar.level(4800 + 100), 1.0, "the bar comes in 100 ms");
        scroll_to(&mut app, &f, 0.0, 6000);
        assert!(!app.title.on && !app.bar.on);
    }

    #[test]
    fn the_about_page_brings_title_and_bar_together_once_the_version_fades() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::About, W, H, PPD);
        app.frame(&f, &i);
        let t = app.theme();
        let gone = t.m.dp(ABOUT_LOGO_DP) - widgets::bar_bottom(&t);
        scroll_to(&mut app, &f, gone - 4.0, 0);
        assert!(!app.title.on && !app.bar.on);
        scroll_to(&mut app, &f, gone + 4.0, 1000);
        assert!(app.title.on && app.bar.on);
        assert!((app.bar.level(1800 + 75) - 0.5).abs() < 0.01, "a straight 150 ms fade");
    }

    #[test]
    fn a_scrolled_page_gone_back_to_has_its_bar_from_its_first_frame() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Settings));
        app.frame(&f, &i);
        let end = app.max_scroll();
        scroll_to(&mut app, &f, end, 0);
        app.tick(2000);
        assert_eq!((app.title.level(2000), app.bar.level(2000)), (1.0, 1.0), "scrolled down, the bar is up");
        tap_at(&mut app, &f, &i, Action::Open(Screen::Language), 3000);
        assert_eq!(app.screen(), Screen::Language);
        tap_back(&mut app, &f, &i, 4000);
        assert_eq!(app.screen(), Screen::Settings);
        let mut c = Canvas::new(W, H);
        app.tick(4008);
        app.compose(&mut c, &f, &i, "12:00", 4008);
        assert_eq!(app.top().1, end, "the page is where it was left");
        assert_eq!((app.title.level(4008), app.bar.level(4008)), (1.0, 1.0), "its bar is up from the first frame");
    }

    #[test]
    fn theme_previews_switch_the_theme() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        tap(&mut app, &f, &i, Action::Theme { dark: true });
        assert!(app.state.dark);
        assert!(app.theme().dark);
        tap(&mut app, &f, &i, Action::Theme { dark: false });
        assert!(!app.state.dark);
    }

    #[test]
    fn dragging_the_track_sets_brightness() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Slider(Slider::Brightness)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        assert!(app.touch(Touch::Down(track.x + track.w * 0.25, y), &f, 0).is_empty());
        assert_eq!(app.panel(0), Some(0.25));
        assert!(app.touch(Touch::Move(track.x + track.w * 0.75, y + 40.0), &f, 0).is_empty());
        assert_eq!(app.panel(0), Some(0.75));
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "the panel shows the brightness");
        assert_eq!(app.state.brightness, 0.75);
    }

    #[test]
    fn the_strength_slider_taps_the_motor_as_strongly_as_it_is_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Slider(Slider::Vibration)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        assert!(app.touch(Touch::Down(track.x + track.w * 0.25, y), &f, 0).is_empty());
        assert!(app.touch(Touch::Move(track.x + track.w * 0.8, y + 40.0), &f, 0).is_empty(), "nothing taps while the finger moves");
        assert_eq!(app.touch(Touch::Up, &f, 0), vec![Effect::Haptic], "let go, the motor taps");
        assert!((app.state.vibration - 0.8).abs() < 1e-5);
        assert!(app.touch(Touch::Down(track.x - 50.0, y), &f, 0).is_empty());
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "touched beside the track, the slider stays");
        app.state.vibrate = false;
        app.touch(Touch::Down(track.x, y), &f, 0);
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "with touch vibration off, the motor stays still");
        assert_eq!(app.state.vibration, 0.0, "the strength still moves, down to the slider's left end");
    }

    #[test]
    fn swiping_the_knob_reboots_to_the_chosen_target() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        tap(&mut app, &f, &i, Action::RebootTarget(3));
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::Reboot)).unwrap().rect;
        let y = track.centre().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 0);
        app.touch(Touch::Move(track.x + track.w / 2.0, y), &f, 0);
        app.touch(Touch::Move(track.x + track.w, y), &f, 0);
        let fx = app.touch(Touch::Up, &f, 0);
        assert_eq!(fx, vec![Effect::Reboot(Target::Bootloader)]);
        assert_eq!(app.state.confirm, 0.0, "the knob springs back");
    }

    #[test]
    fn letting_go_halfway_does_not_reboot() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::Reboot)).unwrap().rect;
        let y = track.centre().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 0);
        app.touch(Touch::Move(track.x + track.w / 2.0, y), &f, 0);
        assert!(app.touch(Touch::Up, &f, 0).is_empty());
    }

    #[test]
    fn a_control_that_cannot_work_explains_itself() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Mount, W, H, PPD);
        app.set_mounts(Ok(vec![MountRow { base: "odm".into(), point: "/odm".into(), mounted: true, holders: vec!["qseecomd".into()] }]));
        tap(&mut app, &f, &i, Action::Unavailable("无法卸载正在使用的分区"));
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("无法卸载正在使用的分区"));
        // Once the row tapped has faded back, only the message waits.
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(TOAST_MS));
        app.tick(TOAST_MS);
        assert_eq!(app.deadline(TOAST_MS), app.idle_due());
    }

    #[test]
    fn the_language_chosen_is_spoken_on_every_page() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        tap(&mut app, &f, &i, Action::Open(Screen::Language));
        assert_eq!(app.frame(&f, &i).title, "语言");
        tap(&mut app, &f, &i, Action::Language(Lang::En));
        assert_eq!((app.state.lang, app.screen()), (Lang::En, Screen::Language), "the page stays");
        assert_eq!(app.frame(&f, &i).title, "Language");
        tap(&mut app, &f, &i, Action::Language(Lang::Hant));
        assert_eq!(app.frame(&f, &i).title, "語言");
        tap_back(&mut app, &f, &i, 0);
        app.tick(1000);
        assert_eq!((app.screen(), app.frame(&f, &i).title), (Screen::Settings, "設定"), "and so does the page gone back to");
        app.set_mounts(Ok(vec![MountRow { base: "odm".into(), point: "/odm".into(), mounted: true, holders: vec!["qseecomd".into()] }]));
        tap(&mut app, &f, &i, Action::Open(Screen::Language));
        tap(&mut app, &f, &i, Action::Language(Lang::En));
        tap_back(&mut app, &f, &i, 0);
        app.tick(2000);
        tap_back(&mut app, &f, &i, 2000);
        app.tick(3000);
        tap(&mut app, &f, &i, Action::Open(Screen::Mount));
        tap(&mut app, &f, &i, Action::Unavailable("A partition in use can't be unmounted"));
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("A partition in use can't be unmounted"), "as do its messages");
    }

    /// The installing page, opened from the install page, with the
    /// installer started and waiting for the computer.
    fn sideloading(f: &Fonts, i: &DeviceInfo) -> App {
        let mut app = opened(Screen::Install, f, i);
        assert_eq!(tap(&mut app, f, i, Action::Sideload), vec![Effect::Sideload]);
        assert_eq!(app.screen(), Screen::Installing);
        assert_eq!(app.state.install, Install::Starting);
        app.install(Event::Waiting, 0);
        app
    }

    #[test]
    fn leaving_while_waiting_cancels_and_the_end_clears_it() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        assert_eq!(app.state.install, Install::Waiting);
        app.frame(&f, &i);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(90.0, 1600.0), &f, 8);
        app.touch(Touch::Move(400.0, 1600.0), &f, 16);
        assert_eq!(app.touch(Touch::Up, &f, 24), vec![Effect::Cancel]);
        assert_eq!(app.screen(), Screen::Install);
        assert_eq!(app.state.install, Install::Ending);
        // Whatever the installer was in the middle of, only its end counts.
        app.install(Event::Checking { size: 4096 }, 0);
        assert_eq!(app.state.install, Install::Ending);
        tap(&mut app, &f, &i, Action::Sideload);
        assert_eq!(app.screen(), Screen::Install, "no second installer while the first is ending");
        app.install(Event::Finished(Outcome::Cancelled), 0);
        assert_eq!(app.state.install, Install::Idle);
    }

    #[test]
    fn an_image_goes_where_it_is_sent_once_swiped() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 201_326_592 }, 0);
        let kind = ImageKind::Boot { kernel: true, ramdisk: skyward_install::Ramdisk::Recovery };
        app.install(Event::Image { kind, size: 201_326_592, partitions: vec!["recovery", "boot"] }, 0);
        // The image lit the panel, so the user looks and taps past the wake
        // guard.
        tap_at(&mut app, &f, &i, Action::FlashPartition(1), 1000);
        tap_at(&mut app, &f, &i, Action::FlashSlots(Pick::Both), 1000);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::Flash)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 1000);
        app.touch(Touch::Move(track.x + track.w, y), &f, 1000);
        assert_eq!(app.touch(Touch::Up, &f, 1000), vec![Effect::Flash { partition: "boot", pick: Pick::Both }]);
        assert_eq!(app.state.install, Install::Flashing { partition: "boot", stage: None });
        app.install(Event::Progress(Stage::Flashing { done: 10, total: 40 }), 1000);
        assert_eq!(app.state.install, Install::Flashing { partition: "boot", stage: Some(Stage::Flashing { done: 10, total: 40 }) });
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Installing, "a partition being written keeps the page");
        app.install(Event::Finished(Outcome::Flashed { partition: "boot", slots: vec![Slot::A, Slot::B] }), 1000);
        assert!(matches!(app.state.install, Install::Finished { damage: None, .. }), "a flash that worked broke nothing");
        assert_eq!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 2000), vec![Effect::Reboot(Target::System)]);
        app.key(Key::Back, 2000);
        assert_eq!(app.screen(), Screen::Install);
        assert_eq!(app.state.install, Install::Idle);
    }

    #[test]
    fn an_update_shows_its_steps_and_cannot_be_left() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 7_103_043_868 }, 0);
        app.install(Event::Installing { version: Some("OS3.0.306.0.WMACNXM".into()) }, 0);
        app.install(Event::Overwriting, 0);
        app.install(Event::Progress(Stage::Writing { done: 1 << 30, total: 7 << 30 }), 0);
        assert!(matches!(app.state.install, Install::Updating { overwriting: true, stage: Some(_), .. }));
        let shown = app.frame(&f, &i).hits.len();
        assert_eq!(shown, 0, "nothing on the page to tap while installing");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Installing);
        assert_eq!(app.deadline(0), Some(TOAST_MS), "it says why");
        app.install(Event::Finished(Outcome::Failed(Failure::Engine(9))), 0);
        assert!(matches!(app.state.install, Install::Finished { damage: Some(Damage::System), .. }));
        // Sending again starts over on the same page, past the wake guard.
        assert_eq!(tap_at(&mut app, &f, &i, Action::Sideload, 1000), vec![Effect::Sideload]);
        assert_eq!(app.state.install, Install::Starting);
        assert_eq!(app.screen(), Screen::Installing);
    }

    #[test]
    fn a_refused_package_or_an_update_that_kept_the_system_broke_nothing() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 10 }, 0);
        app.install(Event::Finished(Outcome::Refused(Refusal::Unknown)), 0);
        assert_eq!(app.state.install, Install::Finished { outcome: Outcome::Refused(Refusal::Unknown), damage: None });
        tap_at(&mut app, &f, &i, Action::Sideload, 1000);
        app.install(Event::Waiting, 1000);
        app.install(Event::Installing { version: None }, 1000);
        app.install(Event::Finished(Outcome::Failed(Failure::Engine(21))), 1000);
        assert!(matches!(app.state.install, Install::Finished { damage: None, .. }));
    }

    #[test]
    fn an_image_that_failed_part_way_names_its_partition() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Image { kind: ImageKind::Dtbo, size: 1 << 20, partitions: vec!["dtbo"] }, 0);
        app.state.install = Install::Flashing { partition: "dtbo", stage: None };
        app.install(Event::Progress(Stage::Flashing { done: 1 << 20, total: 4 << 20 }), 0);
        app.install(Event::Finished(Outcome::Failed(Failure::Disconnected)), 0);
        assert!(matches!(app.state.install, Install::Finished { damage: Some(Damage::Partition("dtbo")), .. }));
    }

    #[test]
    fn fastboot_is_chosen_and_restarted_to_like_any_target() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        let fastboot = TARGETS.iter().position(|t| *t == Target::Fastboot).unwrap();
        tap(&mut app, &f, &i, Action::RebootTarget(fastboot));
        assert_eq!(app.state.reboot, fastboot);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Reboot), vec![Effect::Reboot(Target::Fastboot)]);
    }

    /// Press `key` and let it go.
    fn press(app: &mut App, key: Key, now: u64) -> Vec<Effect> {
        let mut fx = app.key(key, now);
        fx.extend(app.key_up(key, now));
        fx
    }

    /// The control the keys are on, on the page drawn.
    fn lit(app: &mut App, f: &Fonts, i: &DeviceInfo) -> Option<Stop> {
        app.frame(f, i);
        app.lit().map(|s| s.stop)
    }

    /// Move the light down the page until it is on `stop`.
    fn light_on(app: &mut App, f: &Fonts, i: &DeviceInfo, stop: Stop) {
        for _ in 0..100 {
            press(app, Key::VolumeDown, 0);
            if app.keys && lit(app, f, i) == Some(stop) {
                return;
            }
        }
        panic!("the keys never came to {stop:?}");
    }

    #[test]
    fn the_keys_reach_the_last_row_of_a_home_page_grown_by_an_update_notice() {
        let f = fonts();
        let i = DeviceInfo { snapshot: SnapshotState::Snapshotted, ..info() };
        let mut app = App::new(Screen::Home, W, SNUG, PPD);
        let (_, y) = spot(&mut app, &f, &i, Action::Open(Screen::Reboot));
        assert!(y > SNUG as f32, "below the screen to start with");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Reboot)));
        for ms in (0..3000).step_by(8) {
            app.tick(ms);
        }
        let (_, y) = spot(&mut app, &f, &i, Action::Open(Screen::Reboot));
        assert!(y < SNUG as f32, "the page scrolled the lit row into view");
        press(&mut app, Key::Power, 3000);
        assert_eq!(app.screen(), Screen::Reboot);
    }

    #[test]
    fn the_fastboot_page_is_worked_with_the_keys() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Fastboot, W, H, PPD);
        let choice = |k: usize| Some(Stop::Hit(FASTBOOT_CHOICES[k].1));
        assert_eq!(lit(&mut app, &f, &i), choice(0), "the first choice is lit as the page opens");
        assert!(press(&mut app, Key::VolumeDown, 0).is_empty());
        assert_eq!(lit(&mut app, &f, &i), choice(1), "down moves to the next choice");
        assert!(app.key(Key::Power, 0).is_empty(), "power takes it as it comes up");
        assert_eq!(app.key_up(Key::Power, 0), vec![Effect::EnterRecovery]);
        assert!(app.display_on(), "power leaves the panel on here");
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), choice(FASTBOOT_CHOICES.len() - 1), "up from the first goes round to the last");
        assert_eq!(press(&mut app, Key::Power, 0), vec![Effect::Reboot(Target::PowerOff)]);
        assert_eq!(tap(&mut app, &f, &i, Action::RebootTo(Target::Bootloader)), vec![Effect::Reboot(Target::Bootloader)], "a touch takes a choice too");
        assert!(press(&mut app, Key::Back, 0).is_empty());
        assert_eq!(app.screen(), Screen::Fastboot, "there is nothing to go back to");
    }

    #[test]
    fn the_recovery_comes_up_from_fastboot_at_the_home_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Fastboot, W, H, PPD);
        app.frame(&f, &i);
        app.enter_recovery();
        assert_eq!(app.screen(), Screen::Home);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home, "fastboot is not behind it");
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "the keys go on working the pages");
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert!(press(&mut app, Key::Power, 0).is_empty());
        assert!(!app.display_on(), "with the light put out, power turns the panel off");
    }

    #[test]
    fn without_a_touchscreen_the_first_control_is_lit_at_once() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert_eq!(app.light(0.0), None, "a page opens to the touch");
        app.touchscreen_missing();
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.light(0.0).is_some(), "lit before any key is pressed");
        assert!(app.take_dirty(), "and shown");
    }

    #[test]
    fn the_volume_keys_light_the_controls_down_the_page_and_up_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert!(!app.keys, "a page opens to the touch");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "the first press lights the first control in view");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))));
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Reboot))), "up from the first goes round to the last");
        app.tick(5000);
        let reboot = app.lit().expect("lit");
        assert!(app.shows(&reboot, app.top().1), "scrolled into view");
        press(&mut app, Key::VolumeDown, 6000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "down from the last goes round to the first");
        app.tick(10_000);
        assert_eq!(app.top().1, 0.0, "at the top of the page again");
    }

    #[test]
    fn the_power_key_works_the_control_lit_as_it_comes_up() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Settings)));
        assert!(app.key(Key::Power, 0).is_empty());
        assert_eq!(app.screen(), Screen::Home, "nothing happens as the key goes down");
        app.key_up(Key::Power, 0);
        assert_eq!(app.screen(), Screen::Settings, "the row opens its page as the key comes up");
        let first = lit(&mut app, &f, &i);
        assert!(first.is_some() && first != Some(Stop::Back), "the page's first control is lit, not the `<`: {first:?}");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Settings))), "back on the page, the keys are where they left it");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Settings)));
        press(&mut app, Key::Power, 0);
        assert_eq!(lit(&mut app, &f, &i), first);
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Back), "up from the page's first control comes to the `<`");
        press(&mut app, Key::Power, 0);
        assert_eq!(app.screen(), Screen::Home, "the `<` goes back");
    }

    #[test]
    fn past_the_last_control_the_keys_scroll_on_half_a_screen_a_press() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Installing, W, H, PPD);
        app.state.install = Install::Finished { outcome: Outcome::Ran { wipe_cache: false }, damage: None };
        app.state.script_output = (0..300).map(|k| format!("- line {k}")).collect();
        app.frame(&f, &i);
        let (max, (top, bottom)) = (app.max_scroll(), app.band(0.0));
        let half = (bottom - top) / 2.0;
        assert!(max > half * 4.0, "the script's output runs on below the controls: {max}");
        let last = *app.spots().expect("drawn").last().expect("controls on the page");
        light_on(&mut app, &f, &i, last.stop);
        let mut at = app.resting();
        let mut presses = 0;
        while at < max {
            press(&mut app, Key::VolumeDown, 0);
            assert_eq!(app.resting(), (at + half).min(max), "half a screen a press");
            at = app.resting();
            presses += 1;
            assert!(presses < 40);
            if !app.shows(&last, at) {
                assert_eq!(lit(&mut app, &f, &i), None, "with no control in view, nothing is lit");
            }
        }
        press(&mut app, Key::VolumeDown, 0);
        let first = app.spots().expect("drawn")[0].stop;
        assert_eq!((lit(&mut app, &f, &i), app.resting()), (Some(first), 0.0), "past the end the light goes round to the top");
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(last.stop), "and up from it round to the last control");
        assert!(app.shows(&last, app.resting()), "which shows");
        let resting = app.resting();
        app.tick(5000);
        assert_eq!(app.top().1, resting, "where the page eased to");
    }

    #[test]
    fn the_power_key_held_slides_a_swipe_control_and_confirms_it_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Confirm(Confirm::Reboot)));
        assert!(press(&mut app, Key::Power, 0).is_empty());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("请按住电源键"), "a short press says how");
        app.key(Key::Power, 1000);
        assert_eq!(app.deadline(1000), Some(1000 + CHORD_MS), "it waits for volume down first");
        app.tick(1000 + CHORD_MS);
        assert_eq!(app.deadline(1000 + CHORD_MS), Some(0), "then the knob moves while the key is held");
        app.tick(1000 + SLIDE_MS / 2);
        assert!((app.state.confirm - 0.5).abs() < 0.01, "half way after half the time: {}", app.state.confirm);
        assert!(app.key_up(Key::Power, 1000 + SLIDE_MS / 2).is_empty(), "let go short of the end, nothing happens");
        assert_eq!(app.state.confirm, 0.0, "and the knob goes back");
        app.key(Key::Power, 3000);
        app.tick(3000 + SLIDE_MS * 2);
        assert_eq!(app.state.confirm, 1.0, "the knob stops at the end");
        assert_eq!(app.key_up(Key::Power, 3000 + SLIDE_MS * 2), vec![Effect::Reboot(TARGETS[0])], "let go there, it confirms");
        assert_eq!(app.state.confirm, 0.0);
        app.key(Key::Power, 6000);
        app.tick(6000 + SLIDE_MS);
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 6000 + SLIDE_MS);
        assert_eq!(app.state.confirm, 0.0, "a touch lets the knob go");
        assert!(app.key_up(Key::Power, 6000 + SLIDE_MS).is_empty(), "and the key comes up to nothing");
    }

    #[test]
    fn the_brightness_slider_takes_the_volume_keys_until_the_power_key_lets_it_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Slider(Slider::Brightness)));
        assert_eq!(app.state.brightness, SLIDER_START, "it starts a step from its left end");
        press(&mut app, Key::Power, 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("按音量键调节亮度，按电源键完成"));
        assert!(press(&mut app, Key::VolumeUp, 0).is_empty(), "the panel shows the brightness");
        assert!((app.state.brightness - (SLIDER_START + SLIDER_STEP)).abs() < 1e-5, "up turns it up");
        press(&mut app, Key::VolumeDown, 0);
        assert!((app.state.brightness - SLIDER_START).abs() < 1e-5, "down turns it down");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(app.state.brightness, MIN_BRIGHTNESS, "no lower than the slider's floor");
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Slider(Slider::Brightness))), "the light stays on the slider");
        press(&mut app, Key::Power, 0);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(app.state.brightness, MIN_BRIGHTNESS, "let go, the slider keeps its level");
        assert_ne!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Slider(Slider::Brightness))), "and the keys move the light again");
    }

    #[test]
    fn the_strength_slider_taps_the_motor_at_each_step_of_the_keys() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Slider(Slider::Vibration)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("按音量键调节振动强度，按电源键完成"));
        assert_eq!(press(&mut app, Key::VolumeDown, 0), vec![Effect::Haptic], "each step taps at the strength reached");
        assert_eq!(app.state.vibration, 0.0, "it starts a step from its left end, the lightest");
        for _ in 0..20 {
            press(&mut app, Key::VolumeUp, 0);
        }
        assert_eq!(app.state.vibration, 1.0, "up to its right end, the strongest");
        press(&mut app, Key::Power, 0);
        assert!(press(&mut app, Key::VolumeDown, 0).is_empty(), "let go, the keys move the light");
        assert_eq!(app.state.vibration, 1.0);
    }

    #[test]
    fn the_keys_join_a_patterns_dots_and_end_it_on_the_last_one_joined() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Pattern)));
        app.frame(&f, &i);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(0)), "the grid's first dot is lit first");
        assert!(press(&mut app, Key::Power, 0).is_empty());
        assert_eq!(app.state.pattern, [0]);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("连接完成后，在最后一个点上再按一次电源键"));
        press(&mut app, Key::Power, 0);
        assert!(app.state.pattern.is_empty(), "ended at one dot, the pattern is wiped");
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("请至少连接 4 个点"));
        press(&mut app, Key::Power, 0);
        press(&mut app, Key::VolumeDown, 0);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(2)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2], "the dot passed straight over is joined on the way");
        for _ in 0..6 {
            press(&mut app, Key::VolumeDown, 0);
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(8)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2, 5, 8]);
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2, 5, 8, 7, 6], "two along the row joins the one between first");
        press(&mut app, Key::VolumeDown, 0);
        assert!(press(&mut app, Key::Power, 0).is_empty(), "a dot joined already is not joined again");
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(press(&mut app, Key::Power, 0), vec![Effect::Unlock { user: 0, code: "1236987".into() }], "the last one joined ends it");
    }

    #[test]
    fn the_keys_type_a_screen_locks_code_on_its_keypad() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Digits)));
        app.frame(&f, &i);
        for ch in ['2', '5', '8', '0'] {
            light_on(&mut app, &f, &i, Stop::Hit(Action::Type(Cap::Char(ch))));
            press(&mut app, Key::Power, 0);
        }
        assert_eq!(app.state.code, "2580");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Type(Cap::Enter)));
        assert_eq!(press(&mut app, Key::Power, 0), vec![Effect::Unlock { user: 0, code: "2580".into() }]);
    }

    #[test]
    fn the_power_key_held_on_an_entry_opens_what_can_be_done_with_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        app.set_files("/".into(), Ok(vec![entry("vendor", folder(12)), entry("init.rc", FileKind::File)]), LISTED);
        app.frame(&f, &i);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))));
        assert!(app.key(Key::Power, 1000).is_empty());
        assert_eq!(app.deadline(1000), Some(1000 + CHORD_MS), "it waits for volume down first");
        assert!(app.tick(1000 + CHORD_MS).is_empty());
        assert_eq!(app.deadline(1000 + CHORD_MS), Some(1000 + LONG_PRESS_MS), "the press is timed from when it went down");
        assert!(app.tick(1000 + LONG_PRESS_MS - 1).is_empty());
        assert_eq!(app.tick(1000 + LONG_PRESS_MS), vec![Effect::Haptic], "held long enough, the folder's page opens");
        assert_eq!(app.screen(), Screen::FileInfo);
        assert_eq!(app.state.file.as_ref().map(|e| e.name.as_str()), Some("vendor"));
        assert!(app.key_up(Key::Power, 1000 + LONG_PRESS_MS + 100).is_empty(), "and the key comes up to nothing");
        app.key(Key::Back, 2000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))));
        assert_eq!(press(&mut app, Key::Power, 3000), vec![Effect::ReadDir("/vendor".into())], "a short press goes into the folder");
    }

    #[test]
    fn a_touch_puts_the_light_out_and_a_volume_key_brings_it_back() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::About)));
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        assert!(app.light(0.0).is_some(), "lit while the keys work the page");
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert!(app.light(0.0).is_none(), "a touch puts the light out");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))), "the keys come back where they were");
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        press(&mut app, Key::Power, 0);
        assert!(!app.display_on(), "with the light out, power turns the panel off");
        press(&mut app, Key::VolumeDown, 1000);
        assert!(app.display_on(), "a button lights the panel");
        assert!(!app.keys, "and does only that");
    }

    #[test]
    fn a_volume_key_held_moves_the_light_on_and_on() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        app.key(Key::VolumeDown, 0);
        assert_eq!(app.deadline(0), Some(CHORD_MS), "it waits for the power key first");
        app.tick(CHORD_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert_eq!(app.deadline(CHORD_MS), Some(REPEAT_DELAY_MS), "and repeats as from when it went down");
        app.tick(REPEAT_DELAY_MS - 1);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "not held long enough yet");
        app.tick(REPEAT_DELAY_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        app.tick(REPEAT_DELAY_MS + REPEAT_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))));
        app.key_up(Key::VolumeDown, REPEAT_DELAY_MS + REPEAT_MS + 10);
        app.tick(REPEAT_DELAY_MS + REPEAT_MS * 5);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))), "let go, it stops");
    }

    #[test]
    fn climbing_out_of_a_folder_the_keys_come_back_to_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        let folders: Vec<FileRow> = (0..300).map(|k| entry(&format!("dir{k:03}"), folder(k))).collect();
        app.set_files("/".into(), Ok(folders.clone()), LISTED);
        app.frame(&f, &i);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))), "the first entry, below the path");
        for _ in 0..40 {
            press(&mut app, Key::VolumeDown, 0);
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(40))));
        app.tick(5000);
        let row = app.lit().expect("lit");
        assert!(app.shows(&row, app.top().1), "scrolled into view");
        assert_eq!(press(&mut app, Key::Power, 5000), vec![Effect::ReadDir("/dir040".into())]);
        app.set_files("/dir040".into(), Ok(vec![entry("a", FileKind::File)]), LISTED);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))), "a folder opened lights its first entry");
        assert_eq!(app.key(Key::Back, 6000), vec![Effect::ReadDir("/".into())]);
        app.set_files("/".into(), Ok(folders), LISTED);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(40))), "back up, the folder left is lit");
    }

    #[test]
    fn the_cant_start_page_retries_looks_around_or_erases() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::CantStart, W, H, PPD);
        app.set_asked(Asked { reason: Some("RescueParty".into()), ..Asked::default() });
        assert_eq!(tap(&mut app, &f, &i, Action::RebootTo(Target::System)), vec![Effect::Reboot(Target::System)]);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::FactoryReset), vec![Effect::FactoryReset]);
        assert_eq!(tap(&mut app, &f, &i, Action::Dismiss), vec![Effect::Dismiss]);
        assert_eq!(app.screen(), Screen::Home);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home, "the system's question is not behind it");
    }

    #[test]
    fn erasing_goes_part_by_part_and_offers_a_way_out_once_it_stops() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::CantStart, W, H, PPD);
        app.erase_started(true, true, vec![Part::Data, Part::Metadata]);
        assert_eq!(app.screen(), Screen::Erase);
        assert!(app.frame(&f, &i).hits.is_empty(), "nothing is to be done while erasing");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Erase, "there is nothing to go back to");
        app.erase_at(Part::Metadata);
        assert_eq!(app.state.erase.done, 1);
        press(&mut app, Key::Power, 0);
        assert!(!app.display_on());
        app.erase_ended(Err(EraseStop::Failed(Part::Metadata, "make_f2fs ended with exit status: 1".into())), 1000);
        assert!(app.display_on(), "what comes next is the user's to choose");
        assert_eq!(app.state.data, Data::Erased, "the data partition went before /metadata failed");
        let later = 2000;
        assert_eq!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::PowerOff), later), vec![Effect::Reboot(Target::PowerOff)]);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Dismiss, later), vec![Effect::Dismiss]);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn erasing_that_ends_well_leaves_nothing_to_choose() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.erase_started(false, true, vec![Part::Cache]);
        app.erase_ended(Ok(()), 0);
        assert_eq!(app.state.erase.done, 1, "every part is erased");
        assert!(app.frame(&f, &i).hits.is_empty(), "the runtime starts the system at once");
        assert_eq!(app.state.data, Data::Closed, "the data partition was not part of it");
    }

    #[test]
    fn the_wipe_page_erases_everything_or_the_parts_chosen() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Wipe, &f, &i);
        let chosen = |app: &App| WIPES.iter().zip(app.state.wipe).filter(|(_, on)| *on).map(|(&w, _)| w).collect::<Vec<_>>();
        assert!(swipe(&mut app, &f, &i, Confirm::Wipe).is_empty(), "nothing is chosen at first");
        for part in [Wipe::Updates, Wipe::Logs, Wipe::Art, Wipe::SystemCaches, Wipe::AppCaches, Wipe::Media, Wipe::AppData] {
            tap(&mut app, &f, &i, Action::WipeItem(part));
        }
        let parts = vec![Wipe::AppData, Wipe::Media, Wipe::AppCaches, Wipe::SystemCaches, Wipe::Art, Wipe::Logs, Wipe::Updates];
        assert_eq!(chosen(&app), parts, "every part the page lists can be chosen with the others");
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Wipe), vec![Effect::Wipe(parts)], "in the order listed");
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::All));
        assert_eq!(chosen(&app), [Wipe::All], "everything takes the parts in");
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::Art));
        assert_eq!(chosen(&app), [Wipe::Art], "a part on its own leaves everything out");
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Wipe), vec![Effect::Wipe(vec![Wipe::Art])]);
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::Art));
        assert!(chosen(&app).is_empty(), "tapped again, a part is left out");
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::All));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Wipe), vec![Effect::Wipe(vec![Wipe::All])]);
    }

    #[test]
    fn the_cache_partition_is_offered_only_on_a_phone_that_has_one() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Wipe, &f, &i);
        let offered = |app: &mut App| app.frame(&f, &i).hits.iter().any(|h| h.action == Action::WipeItem(Wipe::Cache));
        assert!(!offered(&mut app));
        app.state.cache_partition = true;
        app.refresh();
        assert!(offered(&mut app));
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::Cache));
        tap(&mut app, &f, &i, Action::WipeItem(Wipe::Media));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Wipe), vec![Effect::Wipe(vec![Wipe::Media, Wipe::Cache])], "in the order listed");
    }

    #[test]
    fn erasing_the_user_chose_tells_its_progress_and_waits_for_them_at_the_end() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.erase_started(false, false, vec![Part::Media, Part::Art]);
        app.erase_progress(30, 120);
        assert_eq!(app.state.erase.progress, Some((30, 120)));
        app.erase_at(Part::Art);
        assert_eq!(app.state.erase.progress, None, "the next part starts from nothing");
        press(&mut app, Key::Power, 0);
        assert!(!app.display_on());
        app.erase_ended(Ok(()), 1000);
        assert!(app.display_on(), "how it ended waits to be read");
        assert_eq!(app.state.data, Data::Closed, "the data partition itself stays");
        let later = 2000;
        assert_eq!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), later), vec![Effect::Reboot(Target::System)]);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Dismiss, later), vec![Effect::Dismiss]);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn parts_waiting_on_a_locked_user_erase_nothing_and_send_the_user_home_to_decrypt() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.data = Data::Locked { user: 0, keys: Keys::Digits, checking: false };
        app.erase_started(false, false, vec![Part::AppCaches, Part::Logs]);
        app.erase_ended(Err(EraseStop::Locked(vec![0])), 0);
        assert_eq!(app.state.erase.done, 0, "nothing was erased");
        assert_eq!(app.state.data, Data::Locked { user: 0, keys: Keys::Digits, checking: false }, "the partition stays as it was");
        assert_eq!(tap_at(&mut app, &f, &i, Action::Dismiss, 1000), vec![Effect::Dismiss]);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Open(Screen::Decrypt), 1000), Vec::new(), "the screen lock is asked for at once");
        assert_eq!(app.screen(), Screen::Decrypt);
    }

    #[test]
    fn a_data_partition_erased_whole_is_not_opened_again() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.data = Data::Open;
        app.erase_started(true, false, vec![Part::Data, Part::Metadata]);
        app.erase_ended(Ok(()), 0);
        assert_eq!(app.state.data, Data::Erased);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Dismiss, 1000), vec![Effect::Dismiss]);
        assert!(tap_at(&mut app, &f, &i, Action::Open(Screen::Decrypt), 1000).is_empty(), "nothing opens until the system makes it afresh");
        assert_eq!(app.screen(), Screen::Decrypt);
    }

    #[test]
    fn an_update_that_erases_all_user_data_waits_to_be_agreed_to() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 7_103_043_868 }, 0);
        app.install(Event::Ready { version: Some("OS3.0.306.0.WMACNXM".into()), wipe: true }, 0);
        assert_eq!(app.state.install, Install::Confirming { version: Some("OS3.0.306.0.WMACNXM".into()), wipe: true });
        // The question lit the panel, so the user swipes past the wake guard.
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::InstallWipe)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 1000);
        app.touch(Touch::Move(track.x + track.w, y), &f, 1000);
        assert_eq!(app.touch(Touch::Up, &f, 1000), vec![Effect::Install]);
        assert_eq!(app.state.install, Install::Updating { version: Some("OS3.0.306.0.WMACNXM".into()), stage: None, overwriting: false });
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Installing, "an update being written keeps the page");

        let mut app = sideloading(&f, &i);
        app.install(Event::Ready { version: None, wipe: true }, 0);
        assert_eq!(app.key(Key::Back, 1000), vec![Effect::Cancel], "turning it down cancels before anything is written");
        assert_eq!(app.state.install, Install::Ending);
        assert_eq!(app.screen(), Screen::Install);
    }

    #[test]
    fn a_zip_runs_its_install_script_once_agreed_to_and_shows_what_it_says() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        app.data_waiting(None);
        tap(&mut app, &f, &i, Action::Open(Screen::Choose(Place::Storage)));
        app.set_place(Place::Storage, String::new(), Ok(vec![entry("rom.zip", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::ChooseEntry(0)), vec![Effect::InstallFile(Place::Storage, "rom.zip".into())]);
        assert_eq!(app.state.package_name.as_deref(), Some("rom.zip"), "the question names the file");
        app.install(Event::Script, 0);
        assert_eq!(app.state.install, Install::ConfirmingScript);
        // The question lit the panel, so the user swipes past the wake guard.
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::RunScript)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 1000);
        app.touch(Touch::Move(track.x + track.w, y), &f, 1000);
        assert_eq!(app.touch(Touch::Up, &f, 1000), vec![Effect::Install]);
        assert_eq!(app.state.install, Install::Scripting { stage: None, running: false });
        app.install(Event::Running, 1000);
        for line in ["- Flashing boot", " ", "- Done"] {
            app.install(Event::Print(line.into()), 1000);
        }
        app.install(Event::Progress(Stage::Script { thousandths: 500 }), 1000);
        assert_eq!(app.state.install, Install::Scripting { stage: Some(Stage::Script { thousandths: 500 }), running: true });
        let fr = app.frame(&f, &i);
        assert!(fr.hits.is_empty(), "nothing on the page to tap while the script runs");
        assert_eq!(fr.rows.as_ref().map(Rows::count), Some(3), "a row to each line it showed");
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Installing, "a script running keeps the page");
        assert_eq!(app.deadline(1000), Some(1000 + TOAST_MS), "it says why");
        app.install(Event::Finished(Outcome::Ran { wipe_cache: false }), 1000);
        assert_eq!(app.state.install, Install::Finished { outcome: Outcome::Ran { wipe_cache: false }, damage: None });
        let fr = app.frame(&f, &i);
        let rows = fr.rows.as_ref().expect("what it showed stays on the page");
        assert_eq!(rows.count(), 3);
        assert!(fr.hit(W as f32 / 2.0, rows.rect(1, W as f32).centre().1).is_none(), "its lines answer no touch");
        assert_eq!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 2000), vec![Effect::Reboot(Target::System)]);
    }

    #[test]
    fn a_script_turned_down_never_runs_and_one_that_failed_may_have_written_anywhere() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Script, 0);
        assert_eq!(app.key(Key::Back, 1000), vec![Effect::Cancel], "turning it down cancels before it runs");
        assert_eq!((app.screen(), &app.state.install), (Screen::Install, &Install::Ending));

        let mut app = sideloading(&f, &i);
        app.install(Event::Script, 0);
        app.state.install = Install::Scripting { stage: None, running: false };
        let full = "extract META-INF/com/google/android/update-binary: No space left on device (os error 28)";
        app.install(Event::Finished(Outcome::Failed(Failure::Error(full.into()))), 0);
        assert!(matches!(app.state.install, Install::Finished { damage: None, .. }), "a script that never ran wrote nothing");
        tap_at(&mut app, &f, &i, Action::Sideload, 1000);
        app.install(Event::Waiting, 1000);
        app.install(Event::Script, 1000);
        app.state.install = Install::Scripting { stage: None, running: false };
        app.install(Event::Running, 1000);
        app.install(Event::Print("! Could not write vendor_b".into()), 1000);
        app.install(Event::Finished(Outcome::Failed(Failure::Script(1))), 1000);
        assert_eq!(app.state.install, Install::Finished { outcome: Outcome::Failed(Failure::Script(1)), damage: Some(Damage::Script) });
        assert_eq!(app.state.script_output, ["! Could not write vendor_b"], "what it showed before it failed stays");
        assert_eq!(tap_at(&mut app, &f, &i, Action::Sideload, 2000), vec![Effect::Sideload]);
        assert!(app.state.script_output.is_empty(), "sending again starts afresh");
    }

    #[test]
    fn a_script_page_keeps_the_latest_lines_its_script_showed() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Script, 0);
        app.install(Event::Running, 0);
        for k in 0..SCRIPT_LINES + 2 {
            app.install(Event::Print(format!("line {k}")), 0);
        }
        assert_eq!(app.state.script_output.len(), SCRIPT_LINES);
        assert_eq!(app.state.script_output[0], "line 2");
        assert_eq!(app.frame(&f, &i).rows.as_ref().map(Rows::count), Some(SCRIPT_LINES));
    }

    #[test]
    fn the_install_page_offers_to_keep_skyward_where_the_phone_has_recovery_partitions() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        let offered = |app: &mut App| app.frame(&f, &i).hits.iter().any(|h| h.action == Action::Toggle(Switch::KeepRecovery));
        assert!(!offered(&mut app), "a phone whose recovery lives in boot");
        app.state.recovery_partitions = true;
        app.refresh();
        assert!(offered(&mut app));
        assert!(app.state.keep_recovery, "on until turned off");
        tap(&mut app, &f, &i, Action::Toggle(Switch::KeepRecovery));
        assert!(!app.state.keep_recovery);
    }

    #[test]
    fn the_install_page_offers_to_check_a_zips_signature_off_until_turned_on() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        assert!(app.frame(&f, &i).hits.iter().any(|h| h.action == Action::Toggle(Switch::Signature)), "on any phone");
        assert!(!app.state.check_signature);
        tap(&mut app, &f, &i, Action::Toggle(Switch::Signature));
        assert!(app.state.check_signature);
    }

    #[test]
    fn a_signature_being_checked_shows_how_far_and_leaving_stops_it() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 4096 }, 0);
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Installing, "a package being looked at keeps the page");
        let checking = Stage::Signature { done: 1024, total: 4000 };
        app.install(Event::Progress(checking.clone()), 1000);
        assert_eq!(app.state.install, Install::Checking { size: 4096, stage: Some(checking) });
        assert!(app.frame(&f, &i).hits.is_empty(), "nothing on the page to tap while it is checked");
        assert_eq!(app.key(Key::Back, 2000), vec![Effect::Cancel], "checking only reads, so leaving stops it");
        assert_eq!((app.screen(), &app.state.install), (Screen::Install, &Install::Ending));
        app.install(Event::Finished(Outcome::Cancelled), 2000);
        assert_eq!(app.state.install, Install::Idle);
    }

    #[test]
    fn how_skyward_went_back_stays_until_the_next_install() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Script, 0);
        app.install(Event::Running, 0);
        let writing = Stage::Reflashing { done: 1 << 20, total: 4 << 20 };
        app.install(Event::Progress(writing.clone()), 0);
        assert_eq!(app.state.install, Install::Scripting { stage: Some(writing), running: true });
        let why = "write /dev/block/by-name/recovery_a: Input/output error (os error 5)";
        app.install(Event::Reflashed(Reflash::Failed(why.into())), 0);
        app.install(Event::Finished(Outcome::Failed(Failure::Script(1))), 0);
        assert_eq!(app.state.reflash, Some(Reflash::Failed(why.into())));
        tap_at(&mut app, &f, &i, Action::Sideload, 1000);
        assert_eq!(app.state.reflash, None, "sending again starts afresh");

        app.install(Event::Waiting, 1000);
        app.install(Event::Installing { version: None }, 1000);
        app.install(Event::Progress(Stage::Reflashing { done: 1, total: 2 }), 1000);
        assert!(matches!(app.state.install, Install::Updating { stage: Some(Stage::Reflashing { .. }), .. }), "an update writes it back too");
        app.install(Event::Reflashed(Reflash::Written(vec![Slot::A])), 1000);
        app.install(Event::Finished(Outcome::Installed { slot: Slot::A, wipe: false }), 1000);
        assert_eq!(app.state.reflash, Some(Reflash::Written(vec![Slot::A])));
    }

    #[test]
    fn work_the_recovery_does_not_do_is_shown_with_where_to_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Unhandled, W, H, PPD);
        app.set_asked(Asked { unhandled: Some("--update_package=/data/ota_package/update.zip".into()), ..Asked::default() });
        assert_eq!(tap(&mut app, &f, &i, Action::RebootTo(Target::System)), vec![Effect::Reboot(Target::System)]);
        assert_eq!(tap(&mut app, &f, &i, Action::Dismiss), vec![Effect::Dismiss]);
        assert_eq!(app.screen(), Screen::Home);
    }

    /// Drag the knob of a swipe-to-confirm track all the way across.
    fn swipe(app: &mut App, f: &Fonts, i: &DeviceInfo, which: Confirm) -> Vec<Effect> {
        let track = app.frame(f, i).hits.iter().find(|h| h.action == Action::Confirm(which)).expect("the track is on the page").rect;
        let y = track.centre().1 - app.top().1;
        let mut fx = app.touch(Touch::Down(track.x + track.h / 2.0, y), f, 0);
        fx.extend(app.touch(Touch::Move(track.x + track.w / 2.0, y), f, 0));
        fx.extend(app.touch(Touch::Move(track.x + track.w, y), f, 0));
        fx.extend(app.touch(Touch::Up, f, 0));
        fx
    }

    #[test]
    fn every_unfinished_entry_now_opens_its_page() {
        let (f, i) = (fonts(), DeviceInfo { slot: Some(Slot::B), ..info() });
        let routes: [(Screen, Screen); 13] = [
            (Screen::Home, Screen::Search),
            (Screen::Home, Screen::Backup),
            (Screen::Home, Screen::Restore),
            (Screen::Home, Screen::Advanced),
            (Screen::Advanced, Screen::Files),
            (Screen::Advanced, Screen::Terminal),
            (Screen::Advanced, Screen::Logs),
            (Screen::Settings, Screen::Language),
            (Screen::Settings, Screen::TimeZone),
            (Screen::Settings, Screen::Terminal),
            (Screen::About, Screen::Logs),
            (Screen::Specs, Screen::License),
            (Screen::Specs, Screen::FontLicense),
        ];
        for (from, to) in routes {
            let mut app = App::new(from, W, H, PPD);
            tap(&mut app, &f, &i, Action::Open(to));
            assert_eq!(app.screen(), to, "from {from:?}");
        }
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Slot));
        assert_eq!(app.screen(), Screen::Slot);
    }

    #[test]
    fn logs_are_exported_as_ticked_where_they_can_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Logs, W, H, PPD);
        let both = Effect::ExportLogs { recovery: true, kernel: true, storage: false };
        assert_eq!(tap(&mut app, &f, &i, Action::ExportLogs), vec![both], "both, to the temporary folder, at first");
        tap(&mut app, &f, &i, Action::LogPart(0));
        tap(&mut app, &f, &i, Action::LogsTo(1));
        assert!(tap(&mut app, &f, &i, Action::ExportLogs).is_empty(), "internal storage waits for the data partition");
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(TOAST_MS), "and says so");
        app.data_waiting(None);
        assert_eq!(tap(&mut app, &f, &i, Action::ExportLogs), vec![Effect::ExportLogs { recovery: false, kernel: true, storage: true }]);
        tap(&mut app, &f, &i, Action::LogPart(1));
        assert!(tap(&mut app, &f, &i, Action::ExportLogs).is_empty(), "nothing ticked, nothing saved");
    }

    #[test]
    fn a_time_zone_chosen_is_the_one_kept() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::TimeZone));
        assert_eq!(app.state.time_zone, 0, "China until another is chosen");
        let london = TIME_ZONES.iter().position(|&(_, _, name)| name == "Europe/London").expect("London is listed");
        tap(&mut app, &f, &i, Action::TimeZone(london));
        assert_eq!(app.state.time_zone, london);
        tap(&mut app, &f, &i, Action::TimeZone(0));
        assert_eq!(app.state.time_zone, 0, "and China again");
    }

    #[test]
    fn a_backup_takes_what_is_ticked_to_where_it_is_sent() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Backup, &f, &i);
        assert_eq!(app.state.backup, [true, true, false, true]);
        assert!(swipe(&mut app, &f, &i, Confirm::Backup).is_empty(), "internal storage waits for the data partition");
        assert_eq!(app.deadline(0), Some(TOAST_MS), "and says so");
        // The owner's files open first; another user's still wait.
        app.data_waiting(Some((10, Keys::Digits)));
        let first = Effect::Backup { parts: [true, true, false, true], place: Place::Storage, compress: false, verify: true };
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Backup), vec![first]);
        tap(&mut app, &f, &i, Action::BackupPart(2));
        assert!(swipe(&mut app, &f, &i, Confirm::Backup).is_empty(), "the data partition is backed up only with every user's files open");
        app.data_waiting(None);
        tap(&mut app, &f, &i, Action::BackupPart(0));
        assert_eq!(app.state.backup, [false, true, true, true]);
        tap(&mut app, &f, &i, Action::BackupTo(Place::UsbDrive));
        tap(&mut app, &f, &i, Action::Toggle(Switch::Compress));
        tap(&mut app, &f, &i, Action::Toggle(Switch::Verify));
        assert!(swipe(&mut app, &f, &i, Confirm::Backup).is_empty(), "no drive is plugged in");
        app.set_usb_drive(Some(UsbDrive::Shown));
        let second = Effect::Backup { parts: [false, true, true, true], place: Place::UsbDrive, compress: true, verify: false };
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Backup), vec![second]);
        for k in 1..4 {
            tap(&mut app, &f, &i, Action::BackupPart(k));
        }
        assert!(swipe(&mut app, &f, &i, Confirm::Backup).is_empty(), "nothing ticked, nothing backed up");
    }

    /// A backup found in `place`, made at `made`, holding `parts`.
    fn found(place: Place, made: i64, parts: [bool; 4]) -> BackupEntry {
        let held = Held {
            made,
            size: 6_200_000_000,
            parts,
            device: "ishtar".into(),
            system: Some("OS3.0.305.0.WMACNXM".into()),
            slot: Some(Slot::B),
        };
        BackupEntry { place, name: fmt::stamp(made, 0), held: Ok(held) }
    }

    #[test]
    fn backups_found_are_listed_newest_first_and_those_unread_last() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Restore)), vec![Effect::FindBackups { storage: false, usb: false }]);
        let unread = BackupEntry { place: Place::Storage, name: "20261021-150000".into(), held: Err("read backup.toml: not found".into()) };
        let (old, new) = (found(Place::UsbDrive, 1_792_000_000, [true; 4]), found(Place::Storage, 1_792_348_260, [true; 4]));
        app.set_backups(vec![unread.clone(), old.clone(), new.clone()]);
        assert_eq!(app.state.backups, [new, old, unread]);
        let opens = app.frame(&f, &i).hits.iter().filter(|h| matches!(h.action, Action::OpenBackup(_))).count();
        assert_eq!(opens, 3, "each opens, one that cannot be read to be deleted");
    }

    #[test]
    fn a_backup_found_opens_to_choose_what_to_restore() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Restore, &f, &i);
        assert!(app.frame(&f, &i).hits.is_empty(), "nothing found, nothing to open");
        app.set_backups(vec![found(Place::Storage, 1_792_348_260, [true, true, false, true])]);
        tap(&mut app, &f, &i, Action::OpenBackup(0));
        assert_eq!(app.screen(), Screen::RestoreBackup);
        let parts: Vec<Action> =
            app.frame(&f, &i).hits.iter().map(|h| h.action).filter(|a| matches!(a, Action::RestorePart(_))).collect();
        assert_eq!(parts, [Action::RestorePart(0), Action::RestorePart(1), Action::RestorePart(3)], "only what the backup holds");
        tap(&mut app, &f, &i, Action::RestorePart(1));
        assert_eq!(app.state.restore.as_ref().map(|r| r.1), Some([true, false, false, true]));
        let name = fmt::stamp(1_792_348_260, 0);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Restore), vec![Effect::Restore {
            place: Place::Storage,
            name: name.clone(),
            parts: [true, false, false, true]
        }]);
        for k in [0, 3] {
            tap(&mut app, &f, &i, Action::RestorePart(k));
        }
        assert!(swipe(&mut app, &f, &i, Confirm::Restore).is_empty(), "nothing ticked, nothing restored");
        assert_eq!(app.key(Key::Back, 0), vec![Effect::FindBackups { storage: false, usb: false }], "back on the list, it is looked for again");
        assert_eq!((app.screen(), app.state.restore.clone()), (Screen::Restore, None));
    }

    #[test]
    fn a_data_backup_is_restored_only_with_every_users_files_open() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Restore, &f, &i);
        app.set_backups(vec![found(Place::Storage, 1_792_348_260, [false, false, true, false])]);
        tap(&mut app, &f, &i, Action::OpenBackup(0));
        assert!(swipe(&mut app, &f, &i, Confirm::Restore).is_empty());
        assert_eq!(app.deadline(0), Some(TOAST_MS), "the page says to decrypt first");
        app.data_waiting(None);
        let fx = swipe(&mut app, &f, &i, Confirm::Restore);
        assert!(matches!(&fx[..], [Effect::Restore { parts: [false, false, true, false], .. }]), "{fx:?}");
    }

    #[test]
    fn a_backup_made_on_another_phone_is_not_restored_here() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Restore, &f, &i);
        let mut other = found(Place::Storage, 1_792_348_260, [true; 4]);
        other.held.as_mut().unwrap().device = "aurora".into();
        let unread = BackupEntry { place: Place::Storage, name: "20261021-150000".into(), held: Err("read backup.toml: not found".into()) };
        app.set_backups(vec![other, unread]);
        for k in 0..2 {
            tap(&mut app, &f, &i, Action::OpenBackup(k));
            let actions: Vec<Action> = app.frame(&f, &i).hits.iter().map(|h| h.action).collect();
            assert!(!actions.contains(&Action::Confirm(Confirm::Restore)) && actions.contains(&Action::AskDelete), "{k}: {actions:?}");
            app.key(Key::Back, 0);
        }
    }

    #[test]
    fn a_backup_is_deleted_once_a_swipe_confirms_it() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Restore, &f, &i);
        app.data_waiting(None);
        let entry = found(Place::Storage, 1_792_348_260, [true; 4]);
        app.set_backups(vec![entry.clone()]);
        tap(&mut app, &f, &i, Action::OpenBackup(0));
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::Confirm(Confirm::DeleteBackup)), "asked for first");
        tap(&mut app, &f, &i, Action::AskDelete);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::DeleteBackup), vec![Effect::DeleteBackup { place: Place::Storage, name: entry.name }]);
        assert!(app.backup_deleted(Err("Permission denied".into()), 0).is_empty());
        assert_eq!(app.screen(), Screen::RestoreBackup, "not deleted, the page stays and says why");
        assert_eq!(app.backup_deleted(Ok(()), 0), vec![Effect::FindBackups { storage: true, usb: false }]);
        assert_eq!((app.screen(), app.state.restore.clone()), (Screen::Restore, None));
    }

    #[test]
    fn a_backup_on_a_drive_pulled_out_leaves_its_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.set_usb_drive(Some(UsbDrive::Shown));
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Restore)), vec![Effect::FindBackups { storage: false, usb: true }]);
        app.set_backups(vec![found(Place::UsbDrive, 1_792_348_260, [true; 4])]);
        tap(&mut app, &f, &i, Action::OpenBackup(0));
        assert_eq!(app.set_usb_drive(None), vec![Effect::FindBackups { storage: false, usb: false }]);
        assert_eq!((app.screen(), app.state.restore.clone()), (Screen::Restore, None));
        assert_eq!(app.set_usb_drive(Some(UsbDrive::Shown)), vec![Effect::FindBackups { storage: false, usb: true }], "one plugged in is looked in");
    }

    #[test]
    fn backing_up_shows_how_far_it_has_come_and_stops_when_asked() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Backup, &f, &i);
        app.data_waiting(None);
        app.backup_started(false, Place::Storage, true);
        assert_eq!(app.screen(), Screen::BackupJob);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::BackupJob, "the page stays while the backup goes on");
        app.backup_plan(vec![("boot".into(), 200), ("system".into(), 800)]);
        app.backup_at(BackupAt { item: 1, done: 400, of: 800, checking: false });
        assert_eq!(tap(&mut app, &f, &i, Action::StopBackup), vec![Effect::StopBackup]);
        assert!(app.state.backup_job.as_ref().unwrap().stopping);
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::StopBackup), "only one stop is asked for");
        app.backup_ended(BackupEnd::Stopped, 0);
        tap_at(&mut app, &f, &i, Action::Back, 1000);
        assert_eq!((app.screen(), app.state.backup_job.clone()), (Screen::Backup, None));
    }

    #[test]
    fn a_restore_checks_its_backup_first_and_goes_on_to_its_end_once_writing() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Restore, &f, &i);
        app.set_backups(vec![found(Place::Storage, 1_792_348_260, [true; 4])]);
        tap(&mut app, &f, &i, Action::OpenBackup(0));
        app.backup_started(true, Place::Storage, true);
        app.backup_plan(vec![("boot".into(), 200), ("data".into(), 800)]);
        app.backup_at(BackupAt { item: 0, done: 100, of: 200, checking: true });
        let stops = |app: &mut App| app.frame(&f, &i).hits.iter().any(|h| h.action == Action::StopBackup);
        assert!(stops(&mut app), "nothing is written while the backup is checked");
        app.backup_at(BackupAt { item: 0, done: 0, of: 400, checking: false });
        assert!(!stops(&mut app), "what has begun to be written is not stopped");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::BackupJob);
        app.backup_ended(BackupEnd::Restored { unmade: vec!["/data/user/10".into()] }, 0);
        assert_eq!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 1000), vec![Effect::Reboot(Target::System)]);
    }

    #[test]
    fn the_slot_page_switches_to_the_slot_picked() {
        let (f, i) = (fonts(), DeviceInfo { slot: Some(Slot::B), ..info() });
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Slot)), vec![Effect::ReadSlots], "read as the page opens");
        let held = BootSlots { active: Slot::B, bootable: [false, true], successful: [true, true] };
        app.set_boot_slots(Ok(held));
        assert!(swipe(&mut app, &f, &i, Confirm::Slot).is_empty());
        assert_eq!(app.deadline(0), Some(TOAST_MS), "B starts next already");
        tap(&mut app, &f, &i, Action::PickSlot(Slot::A));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Slot), vec![Effect::SetSlot(Slot::A)]);
        let switched = BootSlots { active: Slot::A, bootable: [true, true], ..held };
        app.slot_switched(Ok(switched), 0);
        assert_eq!((app.state.boot_slots.clone(), app.state.next_slot), (Some(Ok(switched)), None), "the pick is done with");
        tap(&mut app, &f, &i, Action::PickSlot(Slot::B));
        app.key(Key::Back, 0);
        assert_eq!(app.state.next_slot, None, "a pick left unswitched is put down");
    }

    #[test]
    fn the_slot_stays_as_an_update_in_flight_set_it() {
        let (f, i) = (fonts(), DeviceInfo { slot: Some(Slot::B), snapshot: SnapshotState::Snapshotted, ..info() });
        let mut app = App::new(Screen::Slot, W, H, PPD);
        app.set_boot_slots(Ok(BootSlots { active: Slot::A, bootable: [true, true], successful: [false, true] }));
        assert!(app.frame(&f, &i).hits.iter().all(|h| !matches!(h.action, Action::Confirm(_))), "nothing to swipe");
    }

    /// Whether the page shows a key of the keypad or keyboard to tap.
    fn keys_shown(app: &mut App, f: &Fonts, i: &DeviceInfo) -> bool {
        app.frame(f, i).hits.iter().any(|h| matches!(h.action, Action::Type(_)))
    }

    #[test]
    fn the_decrypt_page_opens_the_partition_and_then_asks_with_the_users_own_control() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Decrypt)), vec![Effect::OpenData], "no swipe first");
        assert_eq!(app.state.data, Data::Opening);
        assert!(!keys_shown(&mut app, &f, &i) && app.frame(&f, &i).pattern.is_none(), "nothing to answer yet");
        app.data_waiting(Some((0, Keys::Pattern)));
        assert!(app.frame(&f, &i).pattern.is_some(), "the owner set a pattern, so the grid shows");
        app.data_waiting(Some((0, Keys::Digits)));
        assert!(keys_shown(&mut app, &f, &i), "for a code, the keypad");
        app.key(Key::Back, 0);
        assert!(tap(&mut app, &f, &i, Action::Open(Screen::Decrypt)).is_empty(), "open already, it is not opened again");
        let mut first = App::new(Screen::Decrypt, W, H, PPD);
        assert_eq!(first.arrive(), vec![Effect::OpenData], "a recovery that starts on the page opens it as well");
    }

    #[test]
    fn leaving_the_decrypt_page_goes_back_to_where_it_was_opened_from() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Decrypt, &f, &i);
        tap(&mut app, &f, &i, Action::Skip);
        assert_eq!((app.screen(), app.stack.len()), (Screen::Home, 1), "back on the home page, with no other behind it");
        let mut first = App::new(Screen::Decrypt, W, H, PPD);
        first.arrive();
        tap(&mut first, &f, &i, Action::Skip);
        assert_eq!((first.screen(), first.stack.len()), (Screen::Home, 1), "a recovery that started on the page goes on to the home page");
    }

    #[test]
    fn the_code_typed_on_the_keypad_is_sent_once_and_checked_before_more_is_taken() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Digits)));
        for ch in "1234".chars() {
            tap(&mut app, &f, &i, Action::Type(Cap::Char(ch)));
        }
        assert_eq!(app.state.code, "1234");
        tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        assert_eq!(app.state.code, "123");
        let fx = tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        assert_eq!(fx, vec![Effect::Unlock { user: 0, code: "123".into() }]);
        assert_eq!(app.state.code, "", "what was typed is gone once it is sent");
        assert_eq!(app.state.data, Data::Locked { user: 0, keys: Keys::Digits, checking: true });
        assert!(!keys_shown(&mut app, &f, &i), "nothing more is taken while it is checked");
        app.unlock_failed("the secure world does not recognise that credential".into(), 0);
        assert!(keys_shown(&mut app, &f, &i), "turned down, it is asked for again");
        assert_eq!(app.deadline(0), Some(TOAST_MS), "with why");
        app.data_waiting(None);
        assert_eq!(app.state.data, Data::Open);
    }

    #[test]
    fn nothing_typed_is_not_sent() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Digits)));
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(app.deadline(0), Some(TOAST_MS), "the page says to type one first");
    }

    #[test]
    fn a_pattern_is_sent_as_the_numbers_of_the_dots_it_joins() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Pattern)));
        let (cx, cy, span) = app.frame(&f, &i).pattern.expect("the grid is on the page");
        let dots = widgets::pattern_dots(cx, cy, span);
        let scroll = app.top().1;
        app.touch(Touch::Down(dots[0].0, dots[0].1 - scroll), &f, 0);
        for &which in &[1usize, 4, 7] {
            app.touch(Touch::Move(dots[which].0, dots[which].1 - scroll), &f, 0);
        }
        let fx = app.touch(Touch::Up, &f, 0);
        assert_eq!(fx, vec![Effect::Unlock { user: 0, code: "1258".into() }], "the dots counted from one across the rows");
        assert!(app.state.pattern.is_empty(), "the grid is cleared once it is sent");
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::Pattern), "nothing more is drawn while it is checked");
    }

    #[test]
    fn a_pattern_too_short_to_be_one_is_not_sent() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Pattern)));
        let (cx, cy, span) = app.frame(&f, &i).pattern.expect("the grid is on the page");
        let dots = widgets::pattern_dots(cx, cy, span);
        let scroll = app.top().1;
        app.touch(Touch::Down(dots[0].0, dots[0].1 - scroll), &f, 0);
        app.touch(Touch::Move(dots[1].0, dots[1].1 - scroll), &f, 0);
        let fx = app.touch(Touch::Up, &f, 0);
        assert!(fx.is_empty());
        assert_eq!(app.deadline(0), Some(TOAST_MS), "the page says how many dots it takes");
    }

    #[test]
    fn a_partition_that_did_not_open_says_why_and_is_tried_again() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.arrive();
        app.data_failed("KeyMint is not running".into());
        assert_eq!(tap(&mut app, &f, &i, Action::OpenData), vec![Effect::OpenData]);
        assert_eq!(app.state.data, Data::Opening);
    }

    /// Type `text` on the keyboard of the page showing, one tap a key, in
    /// the letters layer.
    fn type_text(app: &mut App, f: &Fonts, i: &DeviceInfo, text: &str) {
        for ch in text.chars() {
            tap(app, f, i, Action::Type(Cap::Char(ch)));
        }
    }

    #[test]
    fn search_finds_by_pinyin_typed_on_its_own_keyboard() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        type_text(&mut app, &f, &i, "bf");
        assert_eq!(app.state.search, "bf");
        let opens: Vec<Action> = app.frame(&f, &i).hits.iter().map(|h| h.action).filter(|a| matches!(a, Action::Open(_))).collect();
        assert_eq!(opens, [Action::Open(Screen::Backup)]);
        tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        assert_eq!(app.state.search, "b");
        tap(&mut app, &f, &i, Action::ClearSearch);
        assert_eq!(app.state.search, "");
        type_text(&mut app, &f, &i, "xpsj");
        tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        assert_eq!(app.screen(), Screen::ScreenOff, "return opens the first result");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Search);
    }

    #[test]
    fn shift_makes_one_capital_and_the_layers_switch() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        tap(&mut app, &f, &i, Action::Type(Cap::Shift));
        type_text(&mut app, &f, &i, "ab");
        assert_eq!(app.state.search, "Ab");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Digits)));
        type_text(&mut app, &f, &i, "1");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Symbols)));
        type_text(&mut app, &f, &i, "|");
        tap(&mut app, &f, &i, Action::Type(Cap::Space));
        assert_eq!(app.state.search, "Ab1| ");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Letters)));
        assert_eq!(app.state.layer, Layer::Letters);
    }

    #[test]
    fn a_key_shows_held_down_while_touched() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        let (x, y) = spot(&mut app, &f, &i, Action::Type(Cap::Char('q')));
        app.touch(Touch::Down(x, y), &f, 0);
        assert_eq!(app.state.key_held(), Some(Cap::Char('q')));
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.state.held, None);
        assert_eq!(app.state.search, "q");
    }

    #[test]
    fn the_terminal_sends_every_key_to_its_shell() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Terminal)), vec![Effect::StartShell("/".into())], "the page asks for a shell the first time");
        let (rows, cols) = app.term_size(&f);
        assert!(rows >= 20 && cols >= 40, "a screen of {rows} rows by {cols}");
        app.term_started(rows, cols);
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Char('l'))), vec![Effect::TermInput(b"l".to_vec())]);
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::TermInput(b"\r".to_vec())]);
        assert!(tap(&mut app, &f, &i, Action::TermKey(TermKey::Ctrl)).is_empty());
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Char('c'))), vec![Effect::TermInput(vec![0x03])]);
        assert_eq!(tap(&mut app, &f, &i, Action::TermKey(TermKey::Up)), vec![Effect::TermInput(b"\x1b[A".to_vec())]);
        app.term_output(b"ishtar:/ # ");
        tap_back(&mut app, &f, &i, 0);
        assert!(tap(&mut app, &f, &i, Action::Open(Screen::Terminal)).is_empty(), "the same shell goes on");
        app.term_ended(TermEnd::Exited(0));
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Char('l'))).is_empty(), "nothing runs to take the keys");
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::StartShell("/".into())]);
    }

    #[test]
    fn the_file_managers_button_opens_the_terminal_in_the_folder_shown() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/sdcard/Download".into(), Ok(vec![entry("a.txt", FileKind::File)]), 0);
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Terminal, Action::TerminalHere)));
        assert_eq!(tap_button(&mut app, &f, &i, 0), vec![Effect::StartShell("/sdcard/Download".into())], "a shell starts there");
        assert_eq!(app.screen(), Screen::Terminal);
        let (rows, cols) = app.term_size(&f);
        app.term_started(rows, cols);
        app.term_ended(TermEnd::Exited(0));
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::StartShell("/sdcard/Download".into())], "Enter starts it again there");
        app.term_started(rows, cols);
        tap_back(&mut app, &f, &i, 0);
        app.set_files("/data/adb".into(), Ok(Vec::new()), 0);
        assert_eq!(tap_button(&mut app, &f, &i, 0), vec![Effect::ShellTo("/data/adb".into())], "the shell running goes there");
        assert_eq!(app.screen(), Screen::Terminal);
    }

    #[test]
    fn the_bars_round_button_darkens_under_a_finger_on_the_frames_drawn() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/data/adb".into(), Ok(vec![entry("a.txt", FileKind::File)]), LISTED);
        let t = app.theme();
        let (cx, cy, _) = widgets::button_disc(&t);
        let k = (((cy - t.m.dp(5.0)) as u32 * W + (cx + t.m.dp(5.0)) as u32) * 4) as usize;
        let mut c = Canvas::new(W, H);
        let mut pixel = |app: &mut App, now: u64| {
            app.tick(now);
            app.compose(&mut c, &f, &i, "12:00", now);
            c.data[k..k + 4].to_vec()
        };
        let (x, y) = widgets::button_rect(&t).centre();
        let rest = pixel(&mut app, 0);
        app.touch(Touch::Down(x, y), &f, 100);
        assert_ne!(pixel(&mut app, 100), rest, "held down, its disc darkens");
        assert_eq!(app.touch(Touch::Up, &f, 150), vec![Effect::StartShell("/data/adb".into())]);
        for now in (150..1000).step_by(8) {
            pixel(&mut app, now);
        }
        assert_eq!(app.screen(), Screen::Terminal, "the terminal slides in");
        app.text_opened(&f, "/data/adb/a.txt".into(), Ok(Some(b"a".to_vec())), 1000);
        app.state.editor.as_mut().expect("the editor page shows a file").insert('b');
        app.refresh();
        for now in (1000..1500).step_by(8) {
            pixel(&mut app, now);
        }
        app.touch(Touch::Down(x, y), &f, 1500);
        pixel(&mut app, 1500);
        assert_eq!(app.touch(Touch::Up, &f, 1550), vec![Effect::SaveText { path: "/data/adb/a.txt".into(), text: "ba".into() }]);
    }

    #[test]
    fn a_drag_on_the_terminal_scrolls_back_through_what_the_shell_wrote() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Terminal, W, H, PPD);
        let (rows, cols) = app.term_size(&f);
        app.term_started(rows, cols);
        let many: String = (0..100).map(|n| format!("line {n}\r\n")).collect();
        app.term_output(many.as_bytes());
        let (card, pitch) = app.frame(&f, &i).scrollback.expect("the terminal's screen scrolls back");
        let (x, y) = (card.x + card.w / 2.0, card.y + card.h / 3.0);
        app.touch(Touch::Down(x, y), &f, 0);
        app.touch(Touch::Move(x, y + pitch * 5.5), &f, 10);
        app.touch(Touch::Up, &f, 20);
        assert_eq!(app.term.as_ref().expect("the shell started").screen().scrollback(), 5, "down shows five older rows");
        assert_eq!(app.top().1, 0.0, "the page itself stays put");
    }

    fn entry(name: &str, kind: FileKind) -> FileRow {
        FileRow { name: name.into(), kind, size: 4096, modified: 1_792_348_260, mode: 0o040_755, uid: 0, gid: 0 }
    }

    /// When the tests list folders: a day after the entries changed.
    const LISTED: i64 = 1_792_348_260 + 86_400;

    fn folder(items: usize) -> FileKind {
        FileKind::Folder { items: Some(items) }
    }

    #[test]
    fn the_file_manager_walks_folders_and_back_up_to_the_root() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Files)), vec![Effect::ReadDir("/".into())]);
        let link = FileKind::Link { target: "/storage/self/primary".into(), folder: true };
        app.set_files("/".into(), Ok(vec![entry("sdcard", link), entry("vendor", folder(12)), entry("init.rc", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(1)), vec![Effect::ReadDir("/vendor".into())]);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(0)), vec![Effect::ReadDir("/sdcard".into())], "a link to a folder opens like one");
        app.set_files("/vendor".into(), Ok(vec![entry("etc", folder(3))]), LISTED);
        assert_eq!(app.key(Key::Back, 0), vec![Effect::ReadDir("/".into())], "back climbs a folder");
        assert_eq!(app.screen(), Screen::Files);
        app.set_files("/".into(), Ok(vec![entry("vmlinux", FileKind::File), entry("init.rc", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(1)), vec![Effect::OpenText("/init.rc".into())], "a text file opens to edit");
        assert_eq!(app.screen(), Screen::Files);
        tap(&mut app, &f, &i, Action::OpenEntry(0));
        assert_eq!(app.screen(), Screen::FileInfo);
        assert_eq!(app.state.file.as_ref().map(|e| e.name.as_str()), Some("vmlinux"));
        app.key(Key::Back, 0);
        assert_eq!(app.state.file, None);
        assert!(app.key(Key::Back, 0).is_empty(), "at the root, back leaves");
        assert_eq!(app.screen(), Screen::Advanced);
    }

    #[test]
    fn a_file_in_internal_storage_is_chosen_once_the_storage_is_open() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        // Closed, the storage leads to the decrypt page, which comes back.
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Decrypt)), vec![Effect::OpenData]);
        app.data_waiting(None);
        tap(&mut app, &f, &i, Action::Skip);
        assert_eq!(app.screen(), Screen::Install);
        let storage = Screen::Choose(Place::Storage);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(storage)), vec![Effect::ReadPlace(Place::Storage, String::new())]);
        let listed = vec![
            entry(".thumbnails", folder(9)),
            entry("Download", folder(3)),
            entry("OTA.ZIP", FileKind::File),
            entry("boot.img", FileKind::File),
            entry("notes.txt", FileKind::File),
            entry(".old.zip", FileKind::File),
            entry("fifo", FileKind::Special),
        ];
        app.set_place(Place::Storage, String::new(), Ok(listed), LISTED);
        let shown: Vec<&str> = app.state.storage.entries.as_ref().expect("listed").iter().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["Download", "OTA.ZIP", "boot.img"], "folders and what can be installed, hidden ones left out");
        assert_eq!(tap(&mut app, &f, &i, Action::ChooseEntry(0)), vec![Effect::ReadPlace(Place::Storage, "Download".into())]);
        app.set_place(Place::Storage, "Download".into(), Ok(vec![entry("ota.zip", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::ChooseEntry(0)), vec![Effect::InstallFile(Place::Storage, "Download/ota.zip".into())]);
        assert_eq!((app.screen(), app.state.source), (Screen::Installing, Source::File(Place::Storage)));
        assert_eq!(app.state.install, Install::Checking { size: 4096, stage: None }, "the file is read at once");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Installing, "a file being read keeps the page");
        app.install(Event::Finished(Outcome::Refused(Refusal::Unknown)), 0);
        tap_at(&mut app, &f, &i, Action::Back, 1000);
        assert_eq!((app.screen(), &app.state.install), (storage, &Install::Idle), "back to choose another");
        assert_eq!(app.key(Key::Back, 1000), vec![Effect::ReadPlace(Place::Storage, String::new())], "back climbs to the top");
        app.set_place(Place::Storage, String::new(), Ok(Vec::new()), LISTED);
        assert!(app.key(Key::Back, 1000).is_empty());
        assert_eq!(app.screen(), Screen::Install, "from the top, back leaves");
    }

    #[test]
    fn a_usb_drive_shows_its_files_as_soon_as_it_is_plugged_in() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        let page = Screen::Choose(Place::UsbDrive);
        assert!(tap(&mut app, &f, &i, Action::Open(page)).is_empty(), "with no drive, nothing to read");
        assert_eq!(app.screen(), page);
        assert!(app.frame(&f, &i).rows.is_none(), "the page waits for one");
        assert_eq!(app.set_usb_drive(Some(UsbDrive::Shown)), vec![Effect::ReadPlace(Place::UsbDrive, String::new())], "read once shown");
        app.set_place(Place::UsbDrive, String::new(), Ok(vec![entry("ROMs", folder(2))]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::ChooseEntry(0)), vec![Effect::ReadPlace(Place::UsbDrive, "ROMs".into())]);
        app.set_place(Place::UsbDrive, "ROMs".into(), Ok(vec![entry("boot.img", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::ChooseEntry(0)), vec![Effect::InstallFile(Place::UsbDrive, "ROMs/boot.img".into())]);
        assert_eq!(app.state.source, Source::File(Place::UsbDrive));
        app.install(Event::Finished(Outcome::Failed(Failure::Unreadable("read /mnt/usb/ROMs/boot.img: Input/output error".into()))), 0);
        tap_at(&mut app, &f, &i, Action::Back, 1000);
        assert_eq!(app.screen(), page, "back to choose another");
        assert!(app.set_usb_drive(None).is_empty());
        assert_eq!(app.state.drive.path, "", "a drive pulled out leaves nothing of its folders");
        assert!(app.frame(&f, &i).rows.is_none(), "the page waits for another");
        assert!(app.key(Key::Back, 1000).is_empty());
        assert_eq!(app.screen(), Screen::Install);
        let unreadable = Some(UsbDrive::Unreadable("sdg holds no filesystem the kernel reads".into()));
        assert!(app.set_usb_drive(unreadable).is_empty(), "another page reads nothing");
        assert!(tap_at(&mut app, &f, &i, Action::Open(page), 1000).is_empty(), "nothing on the drive can be read");
    }

    #[test]
    fn a_file_waits_for_the_install_before_it_to_end() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Install, &f, &i);
        app.data_waiting(None);
        tap(&mut app, &f, &i, Action::Open(Screen::Choose(Place::Storage)));
        app.set_place(Place::Storage, String::new(), Ok(vec![entry("ota.zip", FileKind::File)]), LISTED);
        app.state.install = Install::Ending;
        assert!(tap(&mut app, &f, &i, Action::ChooseEntry(0)).is_empty());
        assert_eq!(app.screen(), Screen::Choose(Place::Storage));
        assert_eq!(app.deadline(0), Some(TOAST_MS), "it says to wait");
    }

    /// The file manager in /sdcard, listing a folder and a file.
    fn in_sdcard(f: &Fonts, i: &DeviceInfo) -> App {
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, f, i, Action::Open(Screen::Files));
        app.set_files("/sdcard".into(), Ok(vec![entry("DCIM", folder(2)), entry("a.zip", FileKind::File)]), LISTED);
        app
    }

    /// Hold the file manager's entry at `index` until the page of what can
    /// be done with it opens.
    fn hold_entry(app: &mut App, f: &Fonts, i: &DeviceInfo, index: usize) {
        let (x, y) = spot(app, f, i, Action::OpenEntry(index));
        app.touch(Touch::Down(x, y), f, 0);
        app.tick(LONG_PRESS_MS);
        app.touch(Touch::Up, f, LONG_PRESS_MS);
        assert_eq!(app.screen(), Screen::FileInfo);
    }

    #[test]
    fn an_entry_held_opens_what_can_be_done_with_it_a_folder_too() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        let (x, y) = spot(&mut app, &f, &i, Action::OpenEntry(0));
        app.touch(Touch::Down(x, y), &f, 1000);
        assert_eq!(app.deadline(1000), Some(1000 + LONG_PRESS_MS), "the press is timed");
        assert!(app.tick(1000 + LONG_PRESS_MS - 1).is_empty());
        assert_eq!(app.tick(1000 + LONG_PRESS_MS), vec![Effect::Haptic], "held long enough, it opens with a tap of the motor");
        assert_eq!(app.screen(), Screen::FileInfo);
        assert_eq!(app.state.file.as_ref().map(|e| e.name.as_str()), Some("DCIM"));
        assert!(app.touch(Touch::Up, &f, 1500).is_empty(), "lifting does not open the folder as well");
        assert_eq!(app.screen(), Screen::FileInfo);
    }

    #[test]
    fn a_picked_entry_goes_into_the_folder_the_bar_is_used_in() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        assert!(tap(&mut app, &f, &i, Action::PickEntry(FileOp::Copy)).is_empty());
        assert_eq!(app.screen(), Screen::Files, "back to the folders to pick one");
        assert_eq!(app.state.clip, Some(Clip { op: FileOp::Copy, path: "/sdcard/a.zip".into() }));
        assert!(app.frame(&f, &i).dock.is_some(), "the bar is held at the bottom");
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(0)), vec![Effect::ReadDir("/sdcard/DCIM".into())]);
        app.set_files("/sdcard/DCIM".into(), Ok(Vec::new()), LISTED);
        let paste = Effect::Paste { op: FileOp::Copy, path: "/sdcard/a.zip".into(), into: "/sdcard/DCIM".into(), clash: None };
        assert_eq!(tap(&mut app, &f, &i, Action::Paste), vec![paste]);
        app.file_job_started(FileTask::Copy { path: "/sdcard/a.zip".into(), into: "/sdcard/DCIM".into() });
        assert_eq!(app.state.clip, None, "the entry is put down once the copy starts");
        assert_eq!(app.screen(), Screen::FileJob);
        app.file_progress((50, 100), (0, 1));
        assert_eq!(app.key(Key::Back, 0), Vec::new());
        assert_eq!(app.screen(), Screen::FileJob, "the page stays while the copy runs");
        app.file_job_ended(JobEnd::Done, 0);
        assert_eq!(app.screen(), Screen::Files);
        assert_eq!(app.state.job, None);
    }

    #[test]
    fn a_folder_goes_nowhere_inside_itself_and_a_move_nowhere_it_is_already() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        let paste = |app: &mut App, op: FileOp, path: &str| {
            app.state.clip = Some(Clip { op, path: path.into() });
            app.toast = None;
            app.refresh();
            tap(app, &f, &i, Action::Paste)
        };
        assert!(paste(&mut app, FileOp::Copy, "/sdcard").is_empty());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("无法将\"sdcard\"放入其自身中"));
        assert!(paste(&mut app, FileOp::Move, "/sdcard/a.zip").is_empty());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("\"a.zip\"已在该文件夹中"));
        assert!(app.state.clip.is_some(), "the entry stays picked, for another folder");
        let copy = Effect::Paste { op: FileOp::Copy, path: "/sdcard/a.zip".into(), into: "/sdcard".into(), clash: None };
        assert_eq!(paste(&mut app, FileOp::Copy, "/sdcard/a.zip"), vec![copy], "a copy where the entry is makes a second one");
    }

    #[test]
    fn a_taken_name_waits_for_the_user_to_choose() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        app.state.clip = Some(Clip { op: FileOp::Move, path: "/tmp/a.zip".into() });
        app.file_clash(FileOp::Move, "/tmp/a.zip".into(), "/sdcard".into(), vec!["a.zip".into()]);
        assert_eq!(app.screen(), Screen::Clash);
        let paste = Effect::Paste { op: FileOp::Move, path: "/tmp/a.zip".into(), into: "/sdcard".into(), clash: Some(ClashChoice::KeepBoth) };
        assert_eq!(tap(&mut app, &f, &i, Action::Resolve(ClashChoice::KeepBoth)), vec![paste]);
        assert_eq!(app.screen(), Screen::Files);
        app.file_clash(FileOp::Move, "/tmp/a.zip".into(), "/sdcard".into(), vec!["a.zip".into()]);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Files, "turning the choice down goes back to the folder");
        assert!(app.state.clip.is_some(), "with the entry still picked, for another folder");
        app.file_clash(FileOp::Move, "/tmp/a.zip".into(), "/sdcard".into(), vec!["a.zip".into()]);
        assert!(tap(&mut app, &f, &i, Action::Resolve(ClashChoice::Skip)).is_empty(), "skipping the entry leaves nothing to do");
        assert_eq!(app.screen(), Screen::Files);
        assert_eq!(app.state.clip, None, "and nothing picked");
    }

    #[test]
    fn renaming_takes_the_name_typed() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::RenameEntry);
        assert_eq!(app.screen(), Screen::Rename);
        assert_eq!(app.state.new_name, "a.zip", "the name it has to start from");
        for _ in 0..4 {
            tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        }
        type_text(&mut app, &f, &i, "b");
        let rename = Effect::Rename { path: "/sdcard/a.zip".into(), name: "ab".into() };
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![rename]);
        app.renamed(Err(RenameRefused::Failed("rename /sdcard/a.zip to ab: No space left on device (os error 28)".into())), 0);
        assert_eq!(app.screen(), Screen::Rename, "a name refused can be typed again");
        app.toast = None;
        app.renamed(Err(RenameRefused::Taken), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("该文件夹中已有\"ab\""), "as the folder compares names");
        app.renamed(Ok(()), 0);
        assert_eq!(app.screen(), Screen::Files, "renamed, the folder shows again");
    }

    #[test]
    fn permissions_switch_a_bit_at_a_time_and_show_what_the_entry_took() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::OpenPermissions);
        assert_eq!(app.screen(), Screen::Permissions);
        let bits: Vec<u32> = app.frame(&f, &i).hits.iter().filter_map(|h| if let Action::ModeBit(b) = h.action { Some(b) } else { None }).collect();
        assert_eq!(bits, [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001, 0o4000, 0o2000, 0o1000]);
        let set = |mode| vec![Effect::SetMode { path: "/sdcard/a.zip".into(), mode }];
        assert_eq!(tap(&mut app, &f, &i, Action::ModeBit(0o002)), set(0o757), "everyone may write");
        app.mode_set(0o757, Ok(0o757), 0);
        assert_eq!(app.state.file.as_ref().unwrap().mode, 0o040_757, "its type stays");
        assert_eq!(tap(&mut app, &f, &i, Action::ModeBit(0o100)), set(0o657), "and back off");
        app.toast = None;
        app.mode_set(0o4757, Ok(0o757), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("该位置不支持修改权限"), "a filesystem that kept none of it");
        app.mode_set(0o755, Err("give /sdcard/a.zip the permissions 0755: Read-only file system (os error 30)".into()), 0);
        assert!(app.toast.as_ref().is_some_and(|t| t.0.starts_with("修改权限失败：give /sdcard/a.zip")), "{:?}", app.toast);
        assert_eq!(app.state.file.as_ref().unwrap().mode, 0o040_757, "as it last was");
        tap_back(&mut app, &f, &i, 0);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.state.files.entries.as_ref().unwrap()[1].mode, 0o040_757, "the folder knows it too");
    }

    #[test]
    fn a_links_page_offers_no_permissions_of_its_own() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        app.state.file = Some(entry("latest", FileKind::Link { target: "DCIM".into(), folder: true }));
        app.open(Screen::FileInfo);
        assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::OpenPermissions));
        app.state.file = Some(entry("a.zip", FileKind::File));
        app.refresh();
        assert!(app.frame(&f, &i).hits.iter().any(|h| h.action == Action::OpenPermissions));
    }

    fn toast(app: &App) -> Option<&str> {
        app.toast.as_ref().map(|t| t.0.as_str())
    }

    #[test]
    fn a_text_file_opens_to_edit_and_saves_what_was_typed() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        app.set_files("/data/adb".into(), Ok(vec![entry("run.sh", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(0)), vec![Effect::OpenText("/data/adb/run.sh".into())]);
        app.text_opened(&f, "/data/adb/run.sh".into(), Ok(Some(b"echo hi\n".to_vec())), 0);
        assert_eq!(app.screen(), Screen::Editor);
        let frame = app.frame(&f, &i);
        assert_eq!(frame.button, None, "nothing waits to be saved");
        assert!(frame.caret.is_some());
        let keys: Vec<Action> = frame.hits.iter().map(|h| h.action).filter(|a| matches!(a, Action::TermKey(_))).collect();
        assert_eq!(keys, editor::EDITOR_KEYS.map(Action::TermKey));
        tap(&mut app, &f, &i, Action::Type(Cap::Shift));
        tap(&mut app, &f, &i, Action::Type(Cap::Char('x')));
        assert_eq!(app.state.layer, Layer::Letters, "a capital lasts one letter");
        tap(&mut app, &f, &i, Action::Type(Cap::Space));
        tap(&mut app, &f, &i, Action::TermKey(TermKey::Tab));
        tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Done, Action::SaveText)));
        let save = vec![Effect::SaveText { path: "/data/adb/run.sh".into(), text: "X \t\necho hi\n".into() }];
        assert_eq!(tap_button(&mut app, &f, &i, 0), save);
        app.text_saved(Err("rename /data/adb/.run.sh.0.saving to /data/adb/run.sh: Read-only file system (os error 30)".into()), 0);
        assert!(toast(&app).is_some_and(|t| t.starts_with("保存失败：rename")), "{:?}", app.toast);
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Done, Action::SaveText)), "still to save");
        assert_eq!(tap_button(&mut app, &f, &i, 0), save);
        app.text_saved(Ok(()), 0);
        assert_eq!(toast(&app), Some("已保存"));
        assert_eq!(app.frame(&f, &i).button, None);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.screen(), Screen::Files, "a file saved leaves at once");
        assert_eq!(app.state.editor, None);
    }

    #[test]
    fn leaving_the_editor_with_changes_unsaved_takes_going_back_again() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/tmp".into(), Ok(vec![entry("a.txt", FileKind::File)]), LISTED);
        app.text_opened(&f, "/tmp/a.txt".into(), Ok(Some(b"a".to_vec())), 0);
        tap(&mut app, &f, &i, Action::Type(Cap::Char('b')));
        tap_back(&mut app, &f, &i, 1000);
        assert_eq!(app.screen(), Screen::Editor, "the first back only warns");
        assert_eq!(toast(&app), Some("修改尚未保存，再次返回将放弃修改"));
        tap_back(&mut app, &f, &i, 1000 + TOAST_MS);
        assert_eq!(app.screen(), Screen::Editor, "once the warning is gone, back warns again");
        tap_back(&mut app, &f, &i, 1000 + TOAST_MS + 500);
        assert_eq!(app.screen(), Screen::Files, "back again while it shows leaves");
        assert_eq!(app.state.editor, None, "without the changes");
    }

    #[test]
    fn files_the_editor_does_not_take_are_turned_down_saying_why() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        let refused = [
            (Ok(None), "暂不支持编辑超过 2 MB 的文件"),
            (Ok(Some(vec![0xff, 0xfe, b'a'])), "仅支持编辑 UTF-8 编码的文本文件"),
            (Ok(Some(b"\x7fELF\x02\x01\x01\0".to_vec())), "仅支持编辑 UTF-8 编码的文本文件"),
        ];
        for (read, said) in refused {
            app.toast = None;
            app.text_opened(&f, "/sdcard/a.bin".into(), read, 0);
            assert_eq!(app.screen(), Screen::Files);
            assert_eq!(toast(&app), Some(said));
        }
        app.text_opened(&f, "/sdcard/a.bin".into(), Err("open /sdcard/a.bin: Permission denied (os error 13)".into()), 0);
        assert_eq!(toast(&app), Some("读取失败：open /sdcard/a.bin: Permission denied (os error 13)"));
        assert_eq!(app.state.editor, None);
    }

    #[test]
    fn the_file_page_edits_a_file_and_nothing_else() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        assert_eq!(tap(&mut app, &f, &i, Action::EditEntry), vec![Effect::OpenText("/sdcard/a.zip".into())]);
        for kind in [folder(2), FileKind::Link { target: "a.zip".into(), folder: false }, FileKind::Special] {
            app.state.file = Some(entry("other", kind));
            app.refresh();
            assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::EditEntry), "{:?}", app.state.file);
        }
    }

    #[test]
    fn the_file_page_shows_a_file_as_saved() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        app.text_opened(&f, "/sdcard/a.zip".into(), Ok(Some(b"x".to_vec())), 0);
        let saved = FileRow { size: 9, modified: LISTED, ..entry("a.zip", FileKind::File) };
        app.set_files("/sdcard".into(), Ok(vec![entry("DCIM", folder(2)), saved.clone()]), LISTED);
        assert_eq!(app.state.file, Some(saved));
    }

    #[test]
    fn a_package_or_an_image_opened_in_the_file_manager_installs_and_another_is_chosen_there() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(1)), vec![Effect::InstallPath("/sdcard/a.zip".into())]);
        assert_eq!((app.screen(), app.state.source), (Screen::Installing, Source::Opened { drive: false }));
        assert_eq!(app.state.package_name.as_deref(), Some("a.zip"), "the page names the file");
        app.install(Event::Finished(Outcome::Refused(Refusal::Unknown)), 0);
        tap_at(&mut app, &f, &i, Action::Back, 1000);
        assert_eq!((app.screen(), &app.state.install), (Screen::Files, &Install::Idle), "another file is chosen where this one was");
        let drive = format!("{USB_DRIVE}/ROMs");
        app.set_files(drive.clone(), Ok(vec![entry("boot.img", FileKind::File)]), LISTED);
        assert_eq!(tap_at(&mut app, &f, &i, Action::OpenEntry(0), 2000), vec![Effect::InstallPath(format!("{drive}/boot.img"))]);
        assert_eq!(app.state.source, Source::Opened { drive: true }, "the page warns about pulling the drive out");
    }

    #[test]
    fn a_file_opened_as_an_install_ends_waits_for_it() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        tap(&mut app, &f, &i, Action::OpenEntry(1));
        app.install(Event::Image { kind: ImageKind::Dtbo, size: 1 << 20, partitions: vec!["dtbo"] }, 0);
        assert_eq!(app.key(Key::Back, 1000), vec![Effect::Cancel]);
        assert_eq!(app.screen(), Screen::Files);
        assert!(tap_at(&mut app, &f, &i, Action::OpenEntry(1), 2000).is_empty());
        assert_eq!(toast(&app), Some("上一次安装尚未结束，请稍后再试"));
    }

    #[test]
    fn the_file_page_installs_what_the_installer_takes_and_nothing_else() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        assert_eq!(tap(&mut app, &f, &i, Action::InstallEntry), vec![Effect::InstallPath("/sdcard/a.zip".into())]);
        assert_eq!(app.screen(), Screen::Installing);
        let link = FileKind::Link { target: "/sdcard/a.zip".into(), folder: false };
        let taken = [entry("boot.IMG", FileKind::File), entry("b.zip", link)];
        let left = [entry("a.txt", FileKind::File), entry("img", FileKind::File), entry("c.zip", folder(1)), entry("d.zip", FileKind::Special)];
        for (e, offered) in taken.into_iter().map(|e| (e, true)).chain(left.into_iter().map(|e| (e, false))) {
            app.state.file = Some(e);
            app.state.install = Install::Idle;
            app.stack = vec![(Screen::FileInfo, 0.0, Focus::Unset)];
            app.refresh();
            let has = app.frame(&f, &i).hits.iter().any(|h| h.action == Action::InstallEntry);
            assert_eq!(has, offered, "{:?}", app.state.file);
        }
    }

    #[test]
    fn relabeling_starts_once_the_data_partition_is_open_and_its_page_follows_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Relabel));
        assert_eq!(app.screen(), Screen::Relabel);
        assert!(swipe(&mut app, &f, &i, Confirm::Relabel).is_empty());
        assert_eq!(toast(&app), Some("请先在「解密」中打开 data 分区"));
        app.state.data = Data::Locked { user: 0, keys: Keys::Digits, checking: false };
        app.refresh();
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Relabel), vec![Effect::Relabel], "open, though a user has yet to unlock");
        app.state.data = Data::Open;
        app.refresh();
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Relabel), vec![Effect::Relabel]);
        app.relabel_started();
        assert!(!app.frame(&f, &i).back, "no going back while it works");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Relabel);
        assert_eq!(toast(&app), Some("请等待完成或停止后再返回"));
        app.relabel_progress(0, 0, 0);
        app.relabel_progress(10, 400, 2);
        assert_eq!(tap(&mut app, &f, &i, Action::StopRelabel), vec![Effect::StopRelabel]);
        assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::StopRelabel), "stopping, it is not asked again");
        app.relabel_progress(12, 400, 3);
        app.relabel_ended(JobEnd::Stopped, 0);
        let job = app.state.relabel.clone().expect("the page shows how it ended");
        assert_eq!((job.checked, job.total, job.changed, job.outcome), (12, 400, 3, Some(JobEnd::Stopped)));
        assert!(app.frame(&f, &i).back);
        tap_at(&mut app, &f, &i, Action::Back, 2000);
        assert_eq!(app.screen(), Screen::Advanced);
        assert_eq!(app.state.relabel, None);
    }

    /// The filesystem page's partitions on a phone with a cache partition:
    /// the data partition closed, and a cache partition its table lets hold
    /// either filesystem.
    fn fs_rows() -> Vec<FsRow> {
        let row = |name: &str, filesystem: Option<&str>, others: &[&str]| FsRow {
            name: name.into(),
            mount_point: format!("/{name}"),
            filesystem: filesystem.map(str::to_string),
            size: 64 << 20,
            others: others.iter().map(|o| o.to_string()).collect(),
        };
        vec![row("data", None, &[]), row("metadata", Some("f2fs"), &[]), row("cache", Some("ext4"), &["f2fs"])]
    }

    #[test]
    fn the_filesystem_page_does_to_a_partition_what_is_chosen_and_reads_them_again_after() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Filesystems)), vec![Effect::ReadFilesystems]);
        app.set_filesystems(Ok(fs_rows()));
        tap(&mut app, &f, &i, Action::OpenFilesystem(2));
        assert_eq!(app.screen(), Screen::Filesystem);
        assert_eq!(app.state.fs_action, FsAction::Repair, "a repair, which changes nothing the partition holds, to begin with");
        tap(&mut app, &f, &i, Action::ChooseFs(FsChoice::Remake(0)));
        assert_eq!(app.state.fs_action, FsAction::Remake("f2fs".into()));
        let remake = Effect::Filesystem { name: "cache".into(), action: FsAction::Remake("f2fs".into()) };
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Filesystem), vec![remake]);
        app.part_work_started(PartKind::Remake("f2fs".into()), false);
        assert_eq!(app.screen(), Screen::PartWork);
        app.part_listed(vec!["cache".into()]);
        app.part_at(0);
        assert!(!app.frame(&f, &i).back, "no going back while it works");
        app.key(Key::Back, 0);
        assert_eq!((app.screen(), toast(&app)), (Screen::PartWork, Some("请等待完成后再返回")));
        app.part_ended(0, ItemEnd::Made, "");
        app.part_work_ended(Ok(()), 0);
        assert!(app.frame(&f, &i).back);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Back, 2000), vec![Effect::ReadFilesystems], "the partition may hold another now");
        assert_eq!(app.screen(), Screen::Filesystem);
        assert_eq!(app.state.part_work, None);
        assert_eq!(app.state.fs_action, FsAction::Repair, "what to do to it is chosen afresh");
    }

    #[test]
    fn work_on_the_data_partition_waits_for_it_to_open_and_leaves_it_closed() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Filesystems, W, H, PPD);
        app.set_filesystems(Ok(fs_rows()));
        tap(&mut app, &f, &i, Action::OpenFilesystem(0));
        tap(&mut app, &f, &i, Action::ChooseFs(FsChoice::Repair));
        app.state.data = Data::Opening;
        app.refresh();
        assert!(swipe(&mut app, &f, &i, Confirm::Filesystem).is_empty());
        assert_eq!(toast(&app), Some("正在打开 data 分区，请稍后再试"));
        app.state.data = Data::Open;
        app.refresh();
        let repair = Effect::Filesystem { name: "data".into(), action: FsAction::Repair };
        assert_eq!(swipe(&mut app, &f, &i, Confirm::Filesystem), vec![repair]);
        app.part_work_started(PartKind::Repair, false);
        app.part_listed(vec!["data".into()]);
        app.part_at(0);
        app.part_ended(0, ItemEnd::Fixed, "[FSCK] valid_block_count matching with CP   [Fail]\n\nDone: 3.2 secs\n");
        app.part_work_ended(Ok(()), 0);
        let work = app.state.part_work.clone().expect("the page shows how it ended");
        assert_eq!(work.items, [("data".to_string(), ItemState::Ended(ItemEnd::Fixed))]);
        assert_eq!(work.output, ["[FSCK] valid_block_count matching with CP   [Fail]", "Done: 3.2 secs"], "blank lines left out");
        assert_eq!(app.state.data, Data::Closed, "worked on, the partition is closed");
    }

    #[test]
    fn partition_work_the_system_asked_for_is_the_only_page_and_offers_somewhere_to_go_only_when_it_fell_short() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.part_work_started(PartKind::Disable, true);
        assert_eq!(app.stack.len(), 1);
        assert_eq!(app.screen(), Screen::PartWork);
        app.part_listed(vec!["/dev/block/by-name/userdata".into(), "/dev/block/by-name/metadata".into()]);
        app.part_at(0);
        app.part_ended(0, ItemEnd::Erased, "");
        app.part_at(1);
        app.part_ended(1, ItemEnd::Erased, "");
        app.part_work_ended(Ok(()), 0);
        let reboot = |app: &mut App| app.frame(&f, &i).hits.iter().any(|h| h.action == Action::RebootTo(Target::System));
        assert!(!reboot(&mut app), "done, the phone leaves on its own");
        app.part_work_started(PartKind::Disable, true);
        app.part_work_ended(Err(PartStop::Refused(Refusal::Unverified)), 0);
        assert!(reboot(&mut app), "refused, the user chooses where to go");
        let frame = app.frame(&f, &i);
        assert!(!frame.back);
        assert!(
            frame.hits.iter().any(|h| h.action == Action::EnterRecovery),
            "the recovery, which ran without the slot's partitions, brings them up to go on"
        );
    }

    #[test]
    fn an_update_the_system_asked_for_waits_for_the_files_that_hold_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.start_asked(false, "/sdcard/update.zip".into());
        assert_eq!((app.screen(), app.state.source), (Screen::Installing, Source::Asked { security: false }));
        app.asked_opening();
        app.key(Key::Back, 0);
        assert_eq!((app.screen(), toast(&app)), (Screen::Installing, Some("正在打开 data 分区，请稍后再试")));
        app.asked_locked(0, 0);
        assert_eq!(app.state.install, Install::Locked(0));
        assert!(app.frame(&f, &i).hits.iter().any(|h| h.action == Action::Open(Screen::Decrypt)));
        assert_eq!(app.key(Key::Back, 5000), vec![Effect::Cancel], "leaving gives the update up");
        app.install(Event::Finished(Outcome::Cancelled), 5000);
        assert_eq!(app.state.install, Install::Idle);
    }

    #[test]
    fn rescue_stays_for_one_command_after_another_until_its_page_is_left() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.start_rescue();
        assert_eq!((app.screen(), app.state.source), (Screen::Installing, Source::Rescue));
        app.install(Event::Waiting, 0);
        app.install(Event::Checking { size: 4096 }, 0);
        app.install(Event::Done(Outcome::Installed { slot: Slot::B, wipe: false }), 0);
        assert!(matches!(app.state.install, Install::Finished { .. }));
        app.rescue_wiping();
        app.key(Key::Back, 5000);
        assert_eq!((app.screen(), toast(&app)), (Screen::Installing, Some("正在清除，请等待清除完成")));
        app.rescue_wiped(Ok(()), 6000);
        assert_eq!((app.state.install.clone(), app.state.data.clone()), (Install::Wiped(Ok(())), Data::Erased));
        app.frame(&f, &i);
        assert_eq!(app.key(Key::Back, 12000), vec![Effect::Cancel], "leaving ends rescue");
        assert_eq!(app.state.install, Install::Ending);
    }

    fn network(name: &str, signal: i32, security: Security) -> Network {
        let bands = skyward_net::wlan::Bands::of(5805);
        Network { ssid: name.as_bytes().to_vec(), name: name.into(), signal, frequency: 5805, bands, security }
    }

    fn joined(name: &str, stage: Joining) -> Wlan {
        let n = network(name, -34, Security::Psk);
        Wlan {
            power: Power::On,
            networks: vec![n.clone()],
            joined: Some(skyward_net::wlan::Joined { ssid: n.ssid, name: n.name, security: n.security, stage }),
        }
    }

    fn connected(name: &str) -> Wlan {
        use std::net::Ipv4Addr;
        joined(
            name,
            Joining::Connected(skyward_net::wlan::Link {
                address: Ipv4Addr::new(192, 168, 1, 23),
                mask: Ipv4Addr::new(255, 255, 255, 0),
                router: Some(Ipv4Addr::new(192, 168, 1, 1)),
                dns: vec![Ipv4Addr::new(192, 168, 1, 1)],
                signal: -40,
                frequency: 5805,
                speed: Some(866),
                generation: Some(5),
                mac: "02:00:00:00:00:04".into(),
                reach: Some(Reach::Online),
            }),
        )
    }

    #[test]
    fn the_wlan_page_joins_a_network_with_its_password_and_leaves_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let entry = |app: &mut App| app.frame(&f, &i).hits.iter().any(|h| h.action == Action::Open(Screen::Wlan));
        assert!(!entry(&mut app), "a phone whose WLAN is not brought up has no WLAN");
        app.state.wlan = Some(Wlan::default());
        app.refresh();
        assert!(entry(&mut app));
        assert!(app.act(Action::Open(Screen::Wlan), 0).is_empty(), "a WLAN that is off is not scanned");
        assert_eq!(tap(&mut app, &f, &i, Action::Toggle(Switch::Wlan)), vec![Effect::Wlan(true)]);
        assert_eq!(app.state.wlan.as_ref().unwrap().power, Power::Starting, "the switch moves at once");
        let nearby = vec![network("Skyward", -34, Security::Psk), network("Skyward-Open", -72, Security::Open), network("Skyward-WEP", -60, Security::Wep)];
        app.set_wlan(Wlan { power: Power::On, networks: nearby, joined: None });
        tap(&mut app, &f, &i, Action::JoinNetwork(2));
        assert_eq!(toast(&app), Some("暂不支持 WEP 加密的网络"));
        let cafe = Effect::JoinWlan { ssid: b"Skyward-Open".to_vec(), name: "Skyward-Open".into(), security: Security::Open, password: Secret::default() };
        assert_eq!(tap(&mut app, &f, &i, Action::JoinNetwork(1)), vec![cafe], "an open network is joined at once");
        tap(&mut app, &f, &i, Action::JoinNetwork(0));
        assert_eq!(app.screen(), Screen::WlanPassword);
        let type_in = |app: &mut App, text: &str| {
            for ch in text.chars() {
                tap(app, &f, &i, Action::Type(Cap::Char(ch)));
            }
        };
        type_in(&mut app, "secret");
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(toast(&app), Some("请输入 8 到 63 位密码"));
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Digits)));
        type_in(&mut app, "12");
        let fx = tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        let join = Effect::JoinWlan { ssid: b"Skyward".to_vec(), name: "Skyward".into(), security: Security::Psk, password: Secret("secret12".into()) };
        assert_eq!(format!("{join:?}"), r#"JoinWlan { ssid: [83, 107, 121, 119, 97, 114, 100], name: "Skyward", security: Psk, password: Secret(8 characters) }"#, "what is printed leaves the password out");
        assert_eq!(fx, vec![join]);
        assert_eq!(app.screen(), Screen::Wlan, "the WLAN page shows how joining goes");
        assert_eq!((app.state.password.as_str(), app.state.joining.as_ref()), ("", None), "the password is not kept");
        app.set_wlan(connected("Skyward"));
        assert_eq!(fmt::wlan_value(Lang::Hans, app.state.wlan.as_ref().unwrap()), "Skyward");
        tap(&mut app, &f, &i, Action::OpenJoined);
        assert_eq!(app.screen(), Screen::WlanNetwork);
        assert_eq!(tap(&mut app, &f, &i, Action::LeaveNetwork), vec![Effect::LeaveWlan]);
        assert_eq!(app.screen(), Screen::Wlan);
        assert_eq!(tap(&mut app, &f, &i, Action::Toggle(Switch::Wlan)), vec![Effect::Wlan(false)]);
        assert_eq!(app.state.wlan, Some(Wlan::default()));
    }

    #[test]
    fn a_refused_password_is_asked_for_again_from_the_networks_row() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.wlan = Some(joined("Skyward", Joining::WrongPassword));
        app.act(Action::Open(Screen::Wlan), 0);
        tap(&mut app, &f, &i, Action::OpenJoined);
        assert_eq!(app.screen(), Screen::WlanPassword);
        assert_eq!(app.state.joining.as_ref().map(|n| n.name.as_str()), Some("Skyward"));
        app.key(Key::Back, 0);
        app.set_wlan(joined("Skyward", Joining::Joining));
        tap(&mut app, &f, &i, Action::OpenJoined);
        assert_eq!(app.screen(), Screen::WlanNetwork, "a network being joined shows how that goes");
        assert_eq!(app.state.details.as_ref().map(|n| n.name.as_str()), Some("Skyward"));
        app.key(Key::Back, 0);
        assert_eq!(app.state.details, None);
    }

    #[test]
    fn the_status_bar_shows_the_network_joined_and_how_fast_data_goes_without_the_page_drawn_again() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.set_wlan(Wlan { power: Power::On, networks: Vec::new(), joined: None });
        assert!(app.wlan_mark().is_none(), "no network joined");
        app.set_wlan(joined("Skyward", Joining::Addressing));
        assert!(app.wlan_mark().is_none(), "not while it has no address");
        app.set_wlan(connected("Skyward"));
        let mark = app.wlan_mark().unwrap();
        assert_eq!((mark.level, mark.speed.as_str(), mark.unit, mark.receiving, mark.sending), (3, "0.00", "KB/s", false, false));
        app.frame(&f, &i);
        app.dirty = false;
        app.set_traffic(Traffic { rx: 42_000, tx: 0 });
        assert!(app.dirty && app.frame.is_some(), "the status bar is drawn again, and the page as it was");
        let mark = app.wlan_mark().unwrap();
        assert_eq!((mark.speed.as_str(), mark.receiving, mark.sending), ("41.0", true, false));
    }

    #[test]
    fn a_network_is_added_by_hand_with_its_name_protection_and_password() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.wlan = Some(Wlan { power: Power::On, networks: Vec::new(), joined: None });
        app.act(Action::Open(Screen::Wlan), 0);
        tap(&mut app, &f, &i, Action::AddNetwork);
        assert_eq!(app.screen(), Screen::WlanAdd, "the list ends with adding one, found networks or none");
        let type_in = |app: &mut App, text: &str| {
            for ch in text.chars() {
                tap(app, &f, &i, Action::Type(Cap::Char(ch)));
            }
        };
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(toast(&app), Some("请输入网络名称"));
        type_in(&mut app, "skyward");
        let frame = app.frame(&f, &i);
        assert!(!frame.hits.iter().any(|h| h.action == Action::TypeInto(Field::Password)), "an open network takes no password");
        // The protections open over the page, which then answers no touch
        // but closes them.
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        let frame = app.frame(&f, &i);
        assert!(frame.hits.iter().all(|h| matches!(h.action, Action::CloseChoices | Action::PickSecurity(_))));
        assert!(frame.caret.is_none(), "no cursor shows through the list");
        let below = (W as f32 / 2.0, H as f32 - 20.0);
        app.touch(Touch::Down(below.0, below.1), &f, 0);
        app.touch(Touch::Up, &f, 0);
        let a = app.state.adding.clone().unwrap();
        assert_eq!((a.name.as_str(), a.security, a.choosing), ("skyward", Security::Open, false), "a touch off the list types nothing");
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        app.key(Key::Back, 0);
        assert_eq!((app.screen(), app.state.adding.as_ref().unwrap().choosing), (Screen::WlanAdd, false), "back closes the list first");
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        tap(&mut app, &f, &i, Action::PickSecurity(Security::Psk));
        let a = app.state.adding.clone().unwrap();
        assert_eq!((a.security, a.choosing, a.field), (Security::Psk, false, Field::Name));
        // The keyboard types into the field touched.
        tap(&mut app, &f, &i, Action::TypeInto(Field::Password));
        type_in(&mut app, "secret");
        tap(&mut app, &f, &i, Action::TypeInto(Field::Name));
        assert_eq!((app.state.adding.as_ref().unwrap().name.as_str(), app.state.password.as_str()), ("skyward", "secret"));
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(toast(&app), Some("请输入 8 到 63 位密码"));
        assert_eq!(app.state.adding.as_ref().unwrap().field, Field::Password, "the keyboard types into the field turned down");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Digits)));
        type_in(&mut app, "12");
        let fx = tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        let join = Effect::JoinWlan { ssid: b"skyward".to_vec(), name: "skyward".into(), security: Security::Psk, password: Secret("secret12".into()) };
        assert_eq!(fx, vec![join]);
        assert_eq!(app.screen(), Screen::Wlan, "the WLAN page shows how joining goes");
        assert_eq!((app.state.adding.as_ref(), app.state.password.as_str()), (None, ""), "nothing typed is kept");
    }

    #[test]
    fn a_network_added_by_hand_is_turned_down_for_a_name_too_long_and_joined_open_without_its_password() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.wlan = Some(Wlan { power: Power::On, networks: vec![network("Skyward-Open", -72, Security::Open)], joined: None });
        app.act(Action::Open(Screen::Wlan), 0);
        tap(&mut app, &f, &i, Action::AddNetwork);
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        tap(&mut app, &f, &i, Action::PickSecurity(Security::Sae));
        tap(&mut app, &f, &i, Action::TypeInto(Field::Password));
        tap(&mut app, &f, &i, Action::Type(Cap::Char('x')));
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        tap(&mut app, &f, &i, Action::PickSecurity(Security::Open));
        assert_eq!(app.state.adding.as_ref().unwrap().field, Field::Name, "with the password field gone, the keyboard types the name");
        for _ in 0..33 {
            tap(&mut app, &f, &i, Action::Type(Cap::Char('a')));
        }
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(toast(&app), Some("网络名称不能超过 32 个字符"));
        tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        let fx = tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        let name = "a".repeat(32);
        assert_eq!(fx, vec![Effect::JoinWlan { ssid: name.clone().into_bytes(), name, security: Security::Open, password: Secret::default() }]);
    }

    #[test]
    fn the_keys_open_a_list_of_choices_take_one_and_close_it_going_back() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.wlan = Some(Wlan { power: Power::On, networks: Vec::new(), joined: None });
        app.act(Action::Open(Screen::Wlan), 0);
        app.act(Action::AddNetwork, 0);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::ChooseSecurity));
        press(&mut app, Key::Power, 0);
        assert!(app.state.adding.as_ref().unwrap().choosing);
        let t = app.theme();
        let stops: Vec<Stop> = focus::spots(&t, app.frame(&f, &i)).iter().map(|s| s.stop).collect();
        let pick = |s| Stop::Hit(Action::PickSecurity(s));
        assert_eq!(stops, [Stop::Back, pick(Security::Open), pick(Security::Psk), pick(Security::Sae)], "only the list and back");
        light_on(&mut app, &f, &i, pick(Security::Sae));
        press(&mut app, Key::Power, 0);
        let a = app.state.adding.clone().unwrap();
        assert_eq!((a.security, a.choosing), (Security::Sae, false));
        light_on(&mut app, &f, &i, Stop::Hit(Action::ChooseSecurity));
        press(&mut app, Key::Power, 0);
        press(&mut app, Key::Back, 0);
        assert_eq!((app.screen(), app.state.adding.as_ref().unwrap().choosing), (Screen::WlanAdd, false));
    }

    #[test]
    fn a_choice_shows_held_down_while_touched() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.wlan = Some(Wlan { power: Power::On, networks: Vec::new(), joined: None });
        app.act(Action::Open(Screen::Wlan), 0);
        tap(&mut app, &f, &i, Action::AddNetwork);
        tap(&mut app, &f, &i, Action::ChooseSecurity);
        let wpa3 = Action::PickSecurity(Security::Sae);
        let rect = app.frame(&f, &i).hits.iter().find(|h| h.action == wpa3).unwrap().rect;
        // Clear of the words, where only the list's own colour shows.
        let at = |app: &mut App| {
            let c = &app.frame(&f, &i).canvas;
            let (x, y) = ((rect.x + rect.w) as u32 - 20, (rect.y + rect.h / 2.0) as u32);
            let k = ((y - c.held().start) * c.w + x) as usize * 4;
            Rgba::rgb(c.data[k], c.data[k + 1], c.data[k + 2])
        };
        let t = app.theme();
        assert_eq!(at(&mut app), t.pal.popup);
        let (x, y) = spot(&mut app, &f, &i, wpa3);
        app.touch(Touch::Down(x, y), &f, 0);
        assert_eq!(app.state.held, Some(wpa3));
        assert_eq!(at(&mut app), over(t.pal.row_press, t.pal.popup));
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.state.held, None);
        assert_eq!(app.state.adding.as_ref().unwrap().security, Security::Sae);
    }

    #[test]
    fn a_networks_round_button_opens_its_details_and_its_row_joins_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let nearby = vec![network("Skyward", -34, Security::Psk), network("Skyward-Open", -72, Security::Open)];
        app.state.wlan = Some(Wlan { power: Power::On, networks: nearby, joined: None });
        app.act(Action::Open(Screen::Wlan), 0);
        assert_eq!(tap(&mut app, &f, &i, Action::Rescan), vec![Effect::ScanWlan]);
        tap(&mut app, &f, &i, Action::NetworkDetails(1));
        assert_eq!(app.screen(), Screen::WlanNetwork);
        assert_eq!(app.state.details.as_ref().map(|n| n.name.as_str()), Some("Skyward-Open"));
        assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::LeaveNetwork), "a network not chosen has nothing to forget");
        app.key(Key::Back, 0);
        // The round button sits on its row, and is what a touch on it finds.
        let fr = app.frame(&f, &i);
        let row = fr.hits.iter().find(|h| h.action == Action::JoinNetwork(1)).expect("the row").rect;
        let round = fr.hits.iter().find(|h| h.action == Action::NetworkDetails(1)).expect("its round button").rect;
        assert!(row.contains(round.x + round.w / 2.0, round.y + round.h / 2.0));
        assert_eq!(fr.hit(round.x + round.w / 2.0, round.y + round.h / 2.0).map(|h| h.action), Some(Action::NetworkDetails(1)));
        assert_eq!(fr.hit(row.x + row.w / 3.0, row.y + row.h / 2.0).map(|h| h.action), Some(Action::JoinNetwork(1)));
    }

    fn newest() -> Release {
        Release {
            version: "OS3.0.310.0.WMACNXM".into(),
            name: "ishtar-ota_full-OS3.0.310.0.WMACNXM-user-16.0-0123456789.zip".into(),
            urls: vec!["https://ultimateota.d.miui.com/OS3.0.310.0.WMACNXM/ishtar-ota_full.zip".into()],
            md5: "0123456789abcdef0123456789abcdef".into(),
            size: "7.1G".into(),
            length: Some(7_572_375_802),
        }
    }

    #[test]
    fn the_update_page_asks_for_the_newest_system_and_downloads_its_full_package_to_install() {
        let f = fonts();
        let i = DeviceInfo { os_version: Some("OS3.0.308.0.WMACNXM".into()), ..info() };
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        app.state.update_server = true;
        app.state.wlan = Some(Wlan::default());
        app.refresh();
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate], "nothing is asked without a network");
        assert!(tap(&mut app, &f, &i, Action::CheckUpdate).is_empty());
        assert_eq!(app.screen(), Screen::Wlan, "asking takes a network, which the WLAN page joins");
        app.key(Key::Back, 0);
        app.set_wlan(connected("Skyward"));
        assert_eq!(tap(&mut app, &f, &i, Action::CheckUpdate), vec![Effect::CheckUpdate]);
        assert_eq!(app.state.check, Check::Asking);
        app.update_checked(Ok(Some(newest())), Have::Nothing);
        app.state.data = Data::Locked { user: 0, keys: Keys::Digits, checking: false };
        app.refresh();
        assert!(tap(&mut app, &f, &i, Action::Download).is_empty());
        assert_eq!(app.screen(), Screen::Decrypt, "the package goes to internal storage, which opens with the owner's files");
        app.state.data = Data::Open;
        app.act(Action::Skip, 0);
        assert_eq!(app.screen(), Screen::Update);
        assert_eq!(tap(&mut app, &f, &i, Action::Download), vec![Effect::Download(newest())]);
        app.download_progress(Progress::Checking { done: 0, total: 7_572_375_802 }, 0);
        app.download_progress(Progress::Downloading { done: 1_000_000_000, total: 7_572_375_802 }, 12_000_000);
        assert!(app.state.downloading());
        assert_eq!(tap(&mut app, &f, &i, Action::StopDownload), vec![Effect::StopDownload]);
        assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::StopDownload), "stopping, it is not asked again");
        // An end lights the panel, and a touch just after only keeps it lit.
        app.download_ended(DownloadEnd::Stopped, Have::Part(1_000_000_000), 0);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Download, 2000), vec![Effect::Download(newest())], "a download stopped carries on");
        let damaged = skyward_net::download::Failure::Damaged;
        app.download_ended(DownloadEnd::Failed(damaged.clone()), Have::Nothing, 3000);
        assert_eq!(fmt::download_failure(Lang::Hans, &damaged), "完整更新包校验失败，已删除，请重新下载。");
        assert_eq!(tap_at(&mut app, &f, &i, Action::Download, 5000), vec![Effect::Download(newest())]);
        app.download_ended(DownloadEnd::Downloaded, Have::Whole(7_572_375_802), 6000);
        let fx = tap_at(&mut app, &f, &i, Action::InstallDownloaded, 8000);
        assert_eq!(fx, vec![Effect::InstallFile(Place::Storage, format!("downloaded_rom/{}", newest().name))]);
        assert_eq!(app.screen(), Screen::Installing);
    }

    #[test]
    fn the_update_page_asks_the_server_as_it_opens_once_a_network_is_joined() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        app.state.wlan = Some(connected("Skyward"));
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate], "a phone whose server is not known asks nothing");
        app.key(Key::Back, 0);
        app.state.update_server = true;
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate, Effect::CheckUpdate]);
        app.update_checked(Err(Unanswered::Failed("update.miui.com did not answer".into())), Have::Nothing);
        app.key(Key::Back, 0);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Update)), vec![Effect::ReadUpdate, Effect::CheckUpdate], "a failed check is tried again");
    }

    #[test]
    fn a_tap_on_the_text_moves_the_cursor_and_a_drag_or_a_flick_scrolls_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        app.text_opened(&f, "/tmp/long.txt".into(), Ok(Some(text.into_bytes())), 0);
        let (card, pitch) = app.frame(&f, &i).scrollback.expect("the editor's text scrolls");
        let l = editor::layout(&app.theme(), &f);
        let ed = |app: &App| app.state.editor.clone().expect("the editor page shows a file");
        app.touch(Touch::Down(l.x0 + 4.2 * l.cell, l.y0 + 2.5 * l.pitch), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(ed(&app).cursor(), (2, 4), "after \"line\" on the third row");
        let (x, y) = (card.x + card.w / 2.0, card.y + card.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 100);
        app.touch(Touch::Move(x, y - pitch * 5.5), &f, 110);
        app.touch(Touch::Up, &f, 400);
        assert_eq!(ed(&app).top(), 5, "up shows the lines after");
        assert_eq!(ed(&app).cursor(), (2, 4), "a drag leaves the cursor where it was");
        assert_eq!(app.top().1, 0.0, "the page itself stays put");
        tap(&mut app, &f, &i, Action::TermKey(TermKey::Down));
        assert_eq!((ed(&app).cursor(), ed(&app).top()), ((3, 4), 3), "moving the cursor brings it into view");
        app.touch(Touch::Down(x, y), &f, 1000);
        for k in 1..=5 {
            app.touch(Touch::Move(x, y - pitch * k as f32), &f, 1000 + k * 8);
        }
        app.touch(Touch::Up, &f, 1045);
        let lifted = ed(&app).top();
        app.tick(1200);
        let gliding = ed(&app).top();
        assert!(gliding > lifted + 5, "a flick glides on: {lifted} to {gliding}");
        app.touch(Touch::Down(x, y), &f, 1210);
        app.touch(Touch::Up, &f, 1220);
        app.tick(2000);
        assert_eq!(ed(&app).top(), gliding, "a finger on it stops it there");
        assert_eq!(ed(&app).cursor(), (3, 4), "and puts no cursor down");
    }

    #[test]
    fn a_name_the_folder_cannot_take_is_turned_down_before_renaming() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::RenameEntry);
        let refused =
            [("", "请输入名称"), ("..", "名称不能为\".\"或\"..\""), ("x/y", "名称不能包含\"/\""), ("DCIM", "该文件夹中已有\"DCIM\"")];
        for (name, said) in refused {
            app.state.new_name = name.into();
            app.toast = None;
            assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty(), "{name:?} is not renamed to");
            assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some(said));
            assert_eq!(app.screen(), Screen::Rename);
        }
        app.state.new_name = "a.zip".into();
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(app.screen(), Screen::FileInfo, "the name left as it was, the page goes");
    }

    #[test]
    fn deleting_asks_with_a_swipe_and_runs_on_its_page() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::Confirm(Confirm::DeleteEntry)));
        tap(&mut app, &f, &i, Action::AskDelete);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::DeleteEntry), vec![Effect::Delete("/sdcard/a.zip".into())]);
        app.file_job_started(FileTask::Delete { path: "/sdcard/a.zip".into() });
        assert_eq!(app.screen(), Screen::FileJob);
        assert_eq!(tap(&mut app, &f, &i, Action::StopJob), vec![Effect::StopJob]);
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::FileJob, "it stays until stopped");
        app.file_job_ended(JobEnd::Stopped, 0);
        assert_eq!(app.screen(), Screen::FileJob, "how it ended stays to be read");
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Files, "back past the entry's page, which was for an entry being deleted");
    }

    #[test]
    fn work_that_ends_on_its_page_takes_the_title_of_how_it_ended() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        app.file_job_started(FileTask::Copy { path: "/sdcard/a.zip".into(), into: "/tmp".into() });
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        app.file_job_ended(JobEnd::Stopped, 0);
        app.compose(&mut c, &f, &i, "12:00", 16);
        assert_eq!(app.titles.as_ref().map(|((title, _), _, _)| *title), Some("复制未完成"));
    }

    #[test]
    fn an_unreadable_folder_offers_nothing_to_open() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/data".into(), Err("read /data: Permission denied (os error 13)".into()), LISTED);
        let fr = app.frame(&f, &i);
        assert!(fr.hits.is_empty() && fr.rows.is_none());
    }

    #[test]
    fn a_long_folder_scrolls_to_its_last_entry_in_one_list() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        let many: Vec<FileRow> = (0..5000).map(|k| entry(&format!("IMG_{k:05}.jpg"), FileKind::File)).collect();
        app.set_files("/sdcard/DCIM/Camera".into(), Ok(many), LISTED);
        let rows = app.frame(&f, &i).rows.as_ref().expect("the entries are listed").count();
        assert_eq!(rows, 5000, "every entry has its row");
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        let end = app.max_scroll();
        scroll_to(&mut app, &f, end, 1000);
        app.compose(&mut c, &f, &i, "12:00", 2000);
        assert!(app.drawn.len() < 40, "only the rows near the screen are kept drawn: {}", app.drawn.len());
        assert!(app.drawn.iter().any(|(k, _)| *k == 4999), "the last row shows");
        tap_at(&mut app, &f, &i, Action::OpenEntry(4999), 3000);
        assert_eq!(app.screen(), Screen::FileInfo);
        assert_eq!(app.state.file.as_ref().map(|e| e.name.as_str()), Some("IMG_04999.jpg"));
    }

    #[test]
    fn a_folder_gone_back_up_to_shows_where_it_was_left() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        let folders: Vec<FileRow> = (0..300).map(|k| entry(&format!("dir{k:03}"), folder(k))).collect();
        app.set_files("/".into(), Ok(folders.clone()), LISTED);
        app.frame(&f, &i);
        scroll_to(&mut app, &f, 9000.0, 0);
        let (_, y) = spot(&mut app, &f, &i, Action::OpenEntry(36));
        assert!(y > 0.0 && y < H as f32, "row 36 is on screen at {y}");
        assert_eq!(tap_at(&mut app, &f, &i, Action::OpenEntry(36), 2000), vec![Effect::ReadDir("/dir036".into())]);
        app.set_files("/dir036".into(), Ok(vec![entry("a", FileKind::File)]), LISTED);
        assert_eq!(app.top().1, 0.0, "a folder opened starts from its top");
        assert_eq!(app.key(Key::Back, 3000), vec![Effect::ReadDir("/".into())]);
        app.set_files("/".into(), Ok(folders), LISTED);
        assert_eq!(app.top().1, 9000.0, "back up, the folder is where it was left");
    }

    #[test]
    fn a_folder_opened_or_gone_back_up_to_has_its_bar_from_its_first_frame() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        let folders: Vec<FileRow> = (0..300).map(|k| entry(&format!("dir{k:03}"), folder(k))).collect();
        app.set_files("/".into(), Ok(folders.clone()), LISTED);
        app.frame(&f, &i);
        scroll_to(&mut app, &f, 9000.0, 0);
        app.tick(2000);
        assert_eq!((app.title.level(2000), app.bar.level(2000)), (1.0, 1.0), "scrolled down, the bar is up");
        // The runtime lists a folder as it is asked to, then goes round its
        // loop: time passes, and the next frame is the folder's first.
        let mut c = Canvas::new(W, H);
        tap_at(&mut app, &f, &i, Action::OpenEntry(36), 3000);
        app.set_files("/dir036".into(), Ok(vec![entry("a", FileKind::File)]), LISTED);
        app.tick(3008);
        app.compose(&mut c, &f, &i, "12:00", 3008);
        assert_eq!((app.title.level(3008), app.bar.level(3008)), (0.0, 0.0), "a folder opened at its top has only its large title");
        app.tick(4008);
        assert_eq!(app.key(Key::Back, 4100), vec![Effect::ReadDir("/".into())]);
        app.set_files("/".into(), Ok(folders), LISTED);
        app.tick(4108);
        app.compose(&mut c, &f, &i, "12:00", 4108);
        assert_eq!((app.title.level(4108), app.bar.level(4108)), (1.0, 1.0), "back up, the list is under the bar from the first frame");
        assert_ne!(app.deadline(4108), Some(0), "nothing eases in after it");
    }

    #[test]
    fn the_path_goes_up_from_its_capsules_even_held_under_the_bar() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        let many: Vec<FileRow> = (0..200).map(|k| entry(&format!("lib{k:03}.so"), FileKind::File)).collect();
        app.set_files("/system/lib64".into(), Ok(many), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenAncestor(1)), vec![Effect::ReadDir("/system".into())]);
        scroll_to(&mut app, &f, 6000.0, 0);
        app.tick(2000);
        let t = app.theme();
        let (_, y) = spot(&mut app, &f, &i, Action::OpenAncestor(0));
        assert!(y > widgets::bar_bottom(&t) && y < widgets::bar_bottom(&t) + t.m.dp(51.0), "the capsules stay under the bar: {y}");
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 2000);
        let (x, _) = spot(&mut app, &f, &i, Action::OpenAncestor(0));
        let px = |c: &Canvas, x: f32, y: f32| c.data[(y as usize * W as usize + x as usize) * 4..][..3].to_vec();
        assert_eq!(px(&c, x - t.m.dp(20.0), y), vec![243, 244, 247], "the root's capsule shows over the list");
        assert_eq!(tap_at(&mut app, &f, &i, Action::OpenAncestor(0), 3000), vec![Effect::ReadDir("/".into())]);
        // The strip's empty end covers the rows beneath it.
        app.touch(Touch::Down(W as f32 - t.m.dp(40.0), y), &f, 4000);
        assert!(app.touch(Touch::Up, &f, 4000).is_empty());
        assert_eq!(app.screen(), Screen::Files);
    }

    #[test]
    fn rows_are_found_by_where_they_lie() {
        let rows = Rows { edges: vec![100.0, 352.0, 682.0, 934.0], action: Some(Action::OpenEntry) };
        assert_eq!(rows.count(), 3);
        assert_eq!(rows.at(99.0), None);
        assert_eq!(rows.at(100.0), Some(0));
        assert_eq!(rows.at(352.0), Some(1));
        assert_eq!(rows.at(933.0), Some(2));
        assert_eq!(rows.at(934.0), None);
        assert_eq!(rows.within(0.0, 100.0), 0..0);
        assert_eq!(rows.within(0.0, 101.0), 0..1);
        assert_eq!(rows.within(351.0, 700.0), 0..3);
        assert_eq!(rows.within(700.0, 5000.0), 2..3);
        assert_eq!(rows.within(934.0, 5000.0), 3..3);
    }

    #[test]
    fn power_key_turns_the_panel_off_and_on() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert!(app.key(Key::Power, 0).is_empty());
        assert_eq!(app.panel(0), Some(SLIDER_START), "it waits for volume down first");
        assert_eq!(app.deadline(0), Some(CHORD_MS));
        assert!(app.tick(CHORD_MS).is_empty());
        assert_eq!(app.panel(CHORD_MS), None, "held on alone, it turns the panel off");
        assert!(app.key_up(Key::Power, CHORD_MS + 50).is_empty());
        assert!(app.key(Key::Power, CHORD_MS + 60).is_empty());
        assert_eq!(app.panel(CHORD_MS + 60), Some(SLIDER_START));
        assert!(app.key_up(Key::Power, CHORD_MS + 100).is_empty());
        assert!(app.key(Key::Power, 1000).is_empty());
        assert!(app.key_up(Key::Power, 1050).is_empty());
        assert_eq!(app.panel(1050), None, "let go sooner, it turns the panel off then");
    }

    #[test]
    fn the_power_key_and_volume_down_together_take_a_screenshot() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        app.state.vibrate = true;
        assert!(app.key(Key::Power, 0).is_empty());
        let shot = vec![Effect::Haptic, Effect::Screenshot { storage: false }];
        assert_eq!(app.key(Key::VolumeDown, CHORD_MS - 1), shot, "volume down follows in time");
        assert_eq!(app.chord, None, "nothing is left waiting");
        assert!(app.tick(1000).is_empty());
        assert!(app.key_up(Key::Power, 1000).is_empty());
        assert!(app.key_up(Key::VolumeDown, 1010).is_empty());
        assert!(app.display_on(), "the power key did not turn the panel off");
        assert!(!app.keys, "nor did volume down light a control");
        // Volume down first works the same, the other key coming up first.
        assert!(app.key(Key::VolumeDown, 2000).is_empty());
        assert_eq!(app.key(Key::Power, 2100), shot);
        assert!(app.key_up(Key::VolumeDown, 2150).is_empty());
        assert!(app.key_up(Key::Power, 2200).is_empty());
        assert!(app.display_on());
        assert!(!app.keys);
        // Without a tap of the motor where the settings turn it off, and into
        // the gallery once internal storage is open.
        app.state.vibrate = false;
        app.data_waiting(None);
        app.key(Key::Power, 3000);
        assert_eq!(app.key(Key::VolumeDown, 3010), vec![Effect::Screenshot { storage: true }]);
    }

    #[test]
    fn the_power_key_and_volume_down_too_far_apart_each_do_their_own() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert!(app.key(Key::VolumeDown, 0).is_empty());
        assert!(!app.keys, "volume down waits for the power key first");
        assert!(app.tick(CHORD_MS).is_empty());
        assert!(app.keys, "then lights a control on its own");
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.key(Key::Power, CHORD_MS + 10).is_empty(), "the power key came too late");
        assert!(app.key_up(Key::VolumeDown, CHORD_MS + 20).is_empty());
        assert!(app.tick(CHORD_MS * 2 + 10).is_empty());
        assert_eq!(app.screen(), Screen::Home, "the power key waits on the control lit until it comes up");
        app.key_up(Key::Power, CHORD_MS * 3);
        assert_eq!(app.screen(), Screen::Search, "and then works it");
    }

    #[test]
    fn a_screenshot_says_where_it_went() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.screenshot_saved(Ok(Shot::Album), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图已保存到相册"));
        app.screenshot_saved(Ok(Shot::Held), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图已暂存，解密后存入相册"));
        app.screenshot_saved(Err("No space left on device".into()), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图保存失败：No space left on device"));
        app.screenshots_not_kept("Read-only file system", 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("暂存的截图未能存入相册：Read-only file system"));
    }

    /// With the default two minutes, when the panel dims and when it goes
    /// off.
    const DIMS: u64 = 113_000;
    const OFF: u64 = 120_000;

    #[test]
    fn left_alone_the_panel_dims_then_goes_off() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        assert_eq!(app.deadline(0), Some(DIMS));
        app.tick(DIMS - 1);
        assert_eq!(app.panel(DIMS - 1), Some(SLIDER_START));
        app.tick(DIMS);
        assert_eq!(app.deadline(DIMS), Some(DIMS + BACKLIGHT_STEP_MS), "the backlight steps down a frame at a time");
        let midway = app.panel(DIMS + 250).unwrap();
        assert!(midway < SLIDER_START && midway > dim_level(SLIDER_START), "{midway}");
        assert_eq!(app.panel(DIMS + 500), Some(dim_level(SLIDER_START)));
        assert_eq!(app.deadline(DIMS + 500), Some(OFF), "dim until it goes off");
        app.tick(OFF);
        assert_eq!(app.panel(OFF), None);
        assert!(!app.display_on());
        assert_eq!(app.deadline(OFF), None, "nothing to wait for on a dark panel");
    }

    #[test]
    fn time_before_the_first_tick_does_not_count_as_idle() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        // The UI started long after boot; the idle clock begins at the
        // first tick, not at zero.
        app.tick(5_000_000);
        assert_eq!(app.panel(5_000_000), Some(SLIDER_START));
        assert_eq!(app.deadline(5_000_000), Some(5_000_000 + DIMS));
        app.tick(5_000_000 + OFF);
        assert_eq!(app.panel(5_000_000 + OFF), None);
    }

    #[test]
    fn the_dim_level_sits_below_the_brightness_set() {
        assert_eq!(dim_level(1.0), DIM_CEILING);
        assert_eq!(dim_level(0.06), MIN_BRIGHTNESS);
        assert_eq!(dim_level(0.03), MIN_BRIGHTNESS);
        assert_eq!(dim_level(0.01), 0.01, "never brighter than the level set");
        assert_eq!(dim_span(15_000), 3000);
        assert_eq!(dim_span(120_000), DIM_MS);
    }

    #[test]
    fn a_touch_while_dim_brightens_and_still_counts() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        app.tick(DIMS);
        assert_eq!(app.panel(DIMS + 500), Some(dim_level(SLIDER_START)));
        let (x, y) = spot(&mut app, &f, &i, Action::Open(Screen::Settings));
        app.touch(Touch::Down(x, y), &f, DIMS + 600);
        app.touch(Touch::Up, &f, DIMS + 650);
        assert_eq!(app.screen(), Screen::Settings, "the tap counts");
        assert_eq!(app.panel(DIMS + 800), Some(SLIDER_START), "back to full brightness");
        assert_eq!(app.deadline(DIMS + 800), Some(DIMS + 650 + DIMS), "the wait starts over");
    }

    #[test]
    fn a_touch_on_the_dark_panel_only_lights_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let (x, y) = spot(&mut app, &f, &i, Action::Open(Screen::Settings));
        press(&mut app, Key::Power, 0);
        assert!(app.touch(Touch::Down(x, y), &f, 100).is_empty());
        assert_eq!(app.panel(100), Some(SLIDER_START));
        assert!(app.touch(Touch::Move(x, y - 300.0), &f, 120).is_empty());
        assert!(app.touch(Touch::Up, &f, 140).is_empty());
        assert_eq!(app.screen(), Screen::Home, "the waking tap pressed nothing");
        assert_eq!(app.top().1, 0.0, "nor scrolled the page");
        tap_at(&mut app, &f, &i, Action::Open(Screen::Settings), 600);
        assert_eq!(app.screen(), Screen::Settings, "a tap past the guard does");
    }

    #[test]
    fn a_key_lights_the_dark_panel_and_does_nothing_else() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        press(&mut app, Key::Power, 0);
        assert!(app.key(Key::Back, 10).is_empty());
        assert!(app.display_on());
        assert_eq!(app.screen(), Screen::Settings, "back did not go back");
    }

    #[test]
    fn a_stray_press_just_after_the_panel_lights_only_keeps_it_awake() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        press(&mut app, Key::Power, 0);
        // A key wakes the panel; the next, within the guard, does not sleep
        // it again or act.
        app.key(Key::Power, 100);
        assert!(app.display_on());
        assert!(app.key(Key::Power, 300).is_empty());
        assert!(app.display_on(), "a second power press did not turn it off");
        assert!(app.key(Key::Back, 350).is_empty());
        assert_eq!(app.screen(), Screen::Reboot, "back did not go back");
        // Past the guard, power turns it off as usual.
        press(&mut app, Key::Power, 600);
        assert!(!app.display_on());
    }

    #[test]
    fn a_stray_touch_just_after_an_install_lights_the_panel_presses_nothing() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        press(&mut app, Key::Power, 0);
        app.install(Event::Finished(Outcome::Installed { slot: Slot::A, wipe: false }), 100);
        assert!(app.display_on(), "the finished install lit the panel");
        // A finger reaching for the phone lands on 重启到系统; within the
        // guard it only keeps the panel awake.
        assert!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 200).is_empty());
        assert_eq!(app.screen(), Screen::Installing, "the stray tap did not reboot");
        // Past the guard the same tap reboots.
        assert_eq!(
            tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 700),
            vec![Effect::Reboot(Target::System)]
        );
    }

    #[test]
    fn a_finger_held_as_the_panel_goes_off_lets_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::Reboot)).unwrap().rect;
        let y = track.centre().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 0);
        app.touch(Touch::Move(track.x + track.w * 0.6, y), &f, 10);
        assert!(app.state.confirm > 0.0);
        press(&mut app, Key::Power, 20);
        assert_eq!(app.state.confirm, 0.0, "the knob is back at the start");
        app.touch(Touch::Move(track.x + track.w, y), &f, 30);
        assert!(app.touch(Touch::Up, &f, 40).is_empty(), "no reboot from a dark panel");
        assert!(!app.display_on());
    }

    #[test]
    fn never_leaves_the_panel_on() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.screen_off = SCREEN_OFF.iter().position(Option::is_none).unwrap();
        app.tick(0);
        assert_eq!(app.deadline(0), None);
        let day = 24 * 3600 * 1000;
        app.tick(day);
        assert_eq!(app.panel(day), Some(SLIDER_START));
    }

    #[test]
    fn the_installer_lights_the_panel_when_it_needs_the_user() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        press(&mut app, Key::Power, 0);
        app.install(Event::Checking { size: 1 << 20 }, 10);
        assert!(!app.display_on(), "reading the package does not light it");
        app.install(Event::Image { kind: ImageKind::Dtbo, size: 1 << 20, partitions: vec!["dtbo"] }, 20);
        assert!(app.display_on(), "an image waiting for its partition does");
        assert_eq!(app.deadline(20), Some(20 + DIMS));
        app.state.install = Install::Flashing { partition: "dtbo", stage: None };
        press(&mut app, Key::Power, 500);
        app.install(Event::Progress(Stage::Flashing { done: 1, total: 2 }), 510);
        assert!(!app.display_on(), "progress does not");
        app.install(Event::Finished(Outcome::Flashed { partition: "dtbo", slots: vec![Slot::A] }), 520);
        assert!(app.display_on(), "the end does");
    }

    #[test]
    fn an_install_ending_after_its_page_was_left_keeps_the_panel_dark() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        assert_eq!(app.key(Key::Back, 0), vec![Effect::Cancel]);
        press(&mut app, Key::Power, 10);
        app.install(Event::Finished(Outcome::Cancelled), 20);
        assert!(!app.display_on());
        assert_eq!(app.state.install, Install::Idle);
    }

    #[test]
    fn choosing_how_long_the_screen_stays_on() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        tap(&mut app, &f, &i, Action::Open(Screen::ScreenOff));
        assert_eq!(app.screen(), Screen::ScreenOff);
        let frame = app.frame(&f, &i);
        assert_eq!(frame.title, "息屏时间");
        assert!(frame.back);
        // The choices sit in order, top to bottom.
        let actions: Vec<Action> = frame.hits.iter().map(|h| h.action).collect();
        assert_eq!(actions, (0..SCREEN_OFF.len()).map(Action::ScreenOff).collect::<Vec<_>>());
        tap(&mut app, &f, &i, Action::ScreenOff(0));
        assert_eq!(app.state.screen_off, 0);
        assert_eq!(app.screen(), Screen::ScreenOff, "the page stays");
        // Fifteen seconds, dim for the last three, once the row tapped has
        // faded back.
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(12_000));
        app.tick(15_000);
        assert!(!app.display_on());
    }
}
