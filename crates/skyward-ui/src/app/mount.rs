//! The mount page: the slot's partitions, each mounted or not.

use super::*;

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

impl App {
    /// The runtime read the mount table again, or could not: keep it and
    /// show it if it changed.
    pub fn set_mounts(&mut self, mounts: Result<Vec<MountRow>, String>) {
        if self.state.mounts != mounts {
            self.state.mounts = mounts;
            self.refresh();
        }
    }

    /// The runtime could not mount or unmount a partition: the mount page
    /// says so, and why.
    pub fn mount_failed(&mut self, failure: MountFailure) {
        self.state.mount_failure = Some(failure);
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn home_opens_the_mount_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Mount)), vec![Effect::ReadMounts], "the page shows the partitions as they are now");
        assert_eq!(app.screen(), Screen::Mount);
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
}
