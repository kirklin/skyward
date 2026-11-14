//! The page that gives the data partition's files their SELinux labels.

use super::*;

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

impl App {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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
}
