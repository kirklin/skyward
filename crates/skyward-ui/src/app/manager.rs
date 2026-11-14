//! The file manager: the folders it walks, the places a file to install is
//! chosen from, and work on files as its page follows it.

use super::*;

/// Where the USB drive plugged in is shown.
pub const USB_DRIVE: &str = "/mnt/usb";

/// What a USB drive plugged in that cannot be read is said to be, and what
/// is asked for while none is plugged in.
pub const USB_DRIVE_UNREADABLE: Text = ["无法读取该 U 盘", "無法讀取這個 USB 隨身碟", "This USB drive can't be read"];
pub const USB_DRIVE_WANTED: Text = ["请通过 OTG 连接 U 盘", "請透過 OTG 連接 USB 隨身碟", "Connect a USB drive through OTG"];

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
    pub(super) fn entry(&self, i: usize) -> &FileRow {
        &self.entries.as_ref().expect("an entry touched is one the list shows")[i]
    }

    /// Where the entry of this name in the folder is.
    pub(super) fn inside(&self, name: &str) -> String {
        std::path::Path::new(&self.path).join(name).to_string_lossy().into_owned()
    }

    /// The folder this one is in; none for the root.
    pub(super) fn up(&self) -> Option<String> {
        std::path::Path::new(&self.path).parent().map(|p| p.to_string_lossy().into_owned())
    }

    /// The folder this many levels below the root, by its place in the
    /// path shown over the list; none for this folder itself.
    pub(super) fn ancestor(&self, depth: usize) -> Option<String> {
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
    pub(super) fn leave(&mut self, scroll: f32) {
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
pub(super) fn file_name(path: &str) -> Option<String> {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned())
}

impl App {
    /// Open what can be done with entry `i` of the folder shown, held long
    /// enough to, with a tap of the motor.
    pub(super) fn held_entry(&mut self, i: usize) -> Vec<Effect> {
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
        if screen == Screen::Files
            && folder == self.state.files.path
            && let (Some(file), Ok(entries)) = (self.state.file.as_mut(), &files)
            && let Some(listed) = entries.iter().find(|e| e.name == file.name)
        {
            *file = listed.clone();
        }
        let shown = self.state.folder_mut(screen);
        shown.path = folder;
        shown.entries = files;
        shown.listed = listed;
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use skyward_install::Failure;

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
}
