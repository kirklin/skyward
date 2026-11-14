//! The wipe page and the page that follows erasing.

use super::*;

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

impl App {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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
}
