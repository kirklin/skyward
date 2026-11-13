//! The install pages: where a package comes from, and how installing it
//! stands, as the installer tells.

use super::*;

/// What an install is told while the last one has not ended.
pub const INSTALL_UNFINISHED: Text = ["上一次安装尚未结束，请稍后再试", "上一次安裝尚未結束，請稍後再試", "The last install hasn't ended. Try again shortly"];

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
    /// A system to install beside the installed one waits for the room its
    /// root takes to be chosen.
    SystemChoosing(SystemChoice),
    /// The system is being installed: its name, and how far.
    Systeming { name: String, stage: Option<Stage> },
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

impl App {
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

    /// The system asked for sideload before the recovery started: the page
    /// opens over the home page, already receiving.
    pub fn start_sideload(&mut self) {
        self.begin_install(Install::Starting, Source::Computer, None);
        self.open(Screen::Installing);
    }

    /// Start an install from `source` afresh, of the file named `name` when
    /// it is one on the phone: nothing the last install showed stays.
    pub(super) fn begin_install(&mut self, install: Install, source: Source, name: Option<String>) {
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
        if matches!(ev, Event::Image { .. } | Event::Ready { .. } | Event::Script | Event::System { .. } | Event::Done(_) | Event::Finished(_)) {
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
            Event::System { name, version, sizes } => *install = Install::SystemChoosing(SystemChoice { name, version, sizes, size: 0 }),
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
                | Install::Scripting { stage: shown, .. }
                | Install::Systeming { stage: shown, .. } => *shown = Some(stage),
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

    /// Install the file of `size` bytes at `path` in `place`, its page
    /// following how it goes; one install at a time.
    pub(super) fn install_file(&mut self, place: Place, path: String, size: u64, now: u64) -> Vec<Effect> {
        let name = file_name(&path);
        self.install_named(Source::File(place), name, size, Effect::InstallFile(place, path), now)
    }

    /// Install the file of `size` bytes at the full `path`, opened in the
    /// file manager, as a file chosen in a place installs.
    pub(super) fn install_opened(&mut self, path: String, size: u64, now: u64) -> Vec<Effect> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use skyward_install::Failure;

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
}
