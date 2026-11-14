//! The update page: asking the phone maker's server for the newest system
//! and downloading its full package.

use super::*;

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

impl App {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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
}
