//! The filesystem pages: the partitions whose filesystem can be repaired
//! or made afresh, and the page that follows work on whole partitions.

use super::*;

/// The data partition's name, as the filesystem page lists it.
pub const DATA_PARTITION: &str = "data";

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

impl App {
    /// The partitions for the filesystem page, as the runtime read them,
    /// or why it could not.
    pub fn set_filesystems(&mut self, rows: Result<Vec<FsRow>, String>) {
        self.state.filesystems = Some(rows);
        self.refresh();
    }

    /// The partition the page of one shows.
    pub(super) fn fs_row(&self) -> Option<&FsRow> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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
}
