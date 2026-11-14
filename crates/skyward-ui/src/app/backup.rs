//! The backup and restore pages, and the page that follows either at work.

use super::*;

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

impl App {
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
    pub(super) fn find_backups(&self) -> Effect {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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
}
