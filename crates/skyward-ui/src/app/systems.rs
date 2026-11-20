//! The slot page, the page of the two systems, and where an update in the
//! slot stands.

use super::*;

/// A system to install beside the installed one, and the room its root
/// takes so far.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemChoice {
    pub name: String,
    pub version: String,
    /// The sizes there is room for, the least first.
    pub sizes: Vec<u64>,
    /// Which of them is chosen.
    pub size: usize,
}

/// The system that starts, as the runtime last read the systems.
pub fn starting(systems: &Overview) -> Who {
    if systems.other_starts {
        Who::Other
    } else {
        Who::Installed
    }
}

/// Work on the systems the runtime carries out for the systems page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemsWork {
    /// Writing this system's boot set into the boot partitions; the phone
    /// restarts into it once that is done.
    Switching(Who),
    /// Taking the other system away.
    Removing,
}

impl App {
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

    /// The systems, as the runtime read them, or why it could not; work on
    /// them that was under way has ended.
    pub fn set_systems(&mut self, systems: Result<Overview, String>) {
        self.state.systems = Some(systems);
        self.state.systems_work = None;
        self.refresh();
    }

    /// Why the work on the systems the page asked for did not go: the
    /// switch did not happen, or the other system was not taken away.
    pub fn systems_work_failed(&mut self, why: &str, now: u64) {
        let l = self.state.lang;
        let said = match self.state.systems_work.take() {
            Some(SystemsWork::Removing) => tr!(l, "删除失败：{why}", "刪除失敗：{why}", "Couldn't remove: {why}"),
            _ => tr!(l, "切换失败：{why}", "切換失敗：{why}", "Couldn't switch: {why}"),
        };
        self.say(&said, now);
        self.refresh();
    }

    /// The second system's start stopped, for `why`, as its first program
    /// told it: the systems page shows it, over the home page.
    pub fn system_stopped(&mut self, why: String) {
        self.state.system_notice = Some(why);
        self.open(Screen::Systems);
    }

    /// A switch to `to` a system asked for before the recovery started is
    /// under way: the systems page shows it, over the home page.
    pub fn switch_asked(&mut self, to: Who) {
        self.state.systems_work = Some(SystemsWork::Switching(to));
        self.open(Screen::Systems);
    }

    /// Where the update stands, as the runtime read it or a merge has come.
    pub fn set_update(&mut self, standing: Standing) {
        if self.state.update != standing {
            self.state.update = standing;
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

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

    /// The systems with a second one installed, the main one starting.
    fn two_systems() -> Overview {
        Overview {
            installed: skyward_systems::Named { name: "HyperOS 3".into(), version: Some("OS3.0.308.0.WMACNXM".into()) },
            other: Some(skyward_systems::Other { id: "ubuntu".into(), name: "Ubuntu Touch".into(), version: "24.04".into(), size: 16 << 30 }),
            other_starts: false,
            changed: false,
            held: skyward_systems::Held::Recorded,
        }
    }

    /// The systems page opened from the home page of a phone that can hold
    /// a second system, with the systems read.
    fn systems_page(f: &Fonts, i: &DeviceInfo, systems: Overview) -> App {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.dual_boot = true;
        assert_eq!(tap(&mut app, f, i, Action::Open(Screen::Systems)), vec![Effect::ReadSystems], "read as the page opens");
        app.set_systems(Ok(systems));
        app
    }

    fn swipes(app: &mut App, f: &Fonts, i: &DeviceInfo) -> bool {
        app.frame(f, i).hits.iter().any(|h| matches!(h.action, Action::Confirm(_)))
    }

    #[test]
    fn the_home_page_offers_the_systems_where_the_phone_can_hold_a_second() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::Open(Screen::Systems)), "no init_boot partitions in both slots");
        let app = systems_page(&f, &i, two_systems());
        assert_eq!(app.screen(), Screen::Systems);
        let named = |systems| {
            let mut app = App::new(Screen::Home, W, H, PPD);
            (app.state.dual_boot, app.state.systems) = (true, Some(Ok(systems)));
            let t = app.theme();
            let cx = Ctx { t: &t, f: &f, info: &i, state: &app.state, term: None, band: None };
            pages::render(Screen::Home, &cx).canvas.data.clone()
        };
        assert!(named(two_systems()) != named(Overview { other_starts: true, ..two_systems() }), "the home page names the one that starts");
    }

    #[test]
    fn the_systems_page_switches_to_the_system_picked() {
        let (f, i) = (fonts(), info());
        let mut app = systems_page(&f, &i, two_systems());
        assert!(!swipes(&mut app, &f, &i), "the main system starts already");
        tap(&mut app, &f, &i, Action::PickSystem(Who::Other));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::SwitchSystem), vec![Effect::SwitchSystem(Who::Other)]);
        assert_eq!((app.state.systems_work, app.state.system_pick), (Some(SystemsWork::Switching(Who::Other)), None));
        assert!(app.frame(&f, &i).hits.is_empty(), "nothing to tap while the boot partitions are written");
        // The card tapped darkens and comes back, as a row of a list does.
        app.tick(1000);
        app.key(Key::Back, 1000);
        assert_eq!((app.screen(), app.deadline(1000)), (Screen::Systems, Some(1000 + TOAST_MS)), "the page stays until they are, and says why");
        app.systems_work_failed("the store's header does not check", 0);
        assert_eq!(app.state.systems_work, None);
        app.set_systems(Ok(Overview { other_starts: true, ..two_systems() }));
        tap(&mut app, &f, &i, Action::PickSystem(Who::Installed));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::SwitchSystem), vec![Effect::SwitchSystem(Who::Installed)]);
        app.set_systems(Ok(two_systems()));
        tap(&mut app, &f, &i, Action::PickSystem(Who::Other));
        app.key(Key::Back, 0);
        assert_eq!((app.screen(), app.state.system_pick), (Screen::Home, None), "a pick left unswitched is put down");
    }

    /// The page as drawn for `systems`, top to bottom.
    fn systems_drawn(f: &Fonts, i: &DeviceInfo, systems: Overview) -> Vec<u8> {
        let mut app = App::new(Screen::Systems, W, H, PPD);
        (app.state.dual_boot, app.state.systems) = (true, Some(Ok(systems)));
        let t = app.theme();
        let cx = Ctx { t: &t, f, info: i, state: &app.state, term: None, band: None };
        pages::render(Screen::Systems, &cx).canvas.data.clone()
    }

    /// A computer wrote the boot partitions over fastboot while they held
    /// the second system's boot images: the page says so, and the main
    /// system's saved images go over what is there only once it is picked
    /// and swiped.
    #[test]
    fn boot_partitions_written_by_another_hand_are_told_of_and_written_over_once_swiped() {
        use skyward_systems::Held;
        let (f, i) = (fonts(), info());
        let second_starts = Overview { other_starts: true, ..two_systems() };
        let rewritten = Overview { held: Held::Rewritten, ..second_starts.clone() };
        assert!(systems_drawn(&f, &i, rewritten.clone()) != systems_drawn(&f, &i, second_starts), "the page says they were written since");
        let mut app = systems_page(&f, &i, rewritten);
        assert!(!swipes(&mut app, &f, &i), "nothing is written until the main system is picked");
        tap(&mut app, &f, &i, Action::PickSystem(Who::Installed));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::SwitchSystem), vec![Effect::SwitchSystem(Who::Installed)]);
    }

    /// /metadata was erased while the boot partitions held a second
    /// system's boot images: nothing is kept of a second system, and the
    /// page neither shows the main system as the one that starts nor has
    /// anything to switch.
    #[test]
    fn boot_partitions_nothing_is_kept_of_are_not_shown_as_the_main_systems() {
        use skyward_systems::Held;
        let (f, i) = (fonts(), info());
        let none_kept = Overview { other: None, ..two_systems() };
        let unrecorded = Overview { held: Held::Unrecorded, ..none_kept.clone() };
        assert!(systems_drawn(&f, &i, unrecorded.clone()) != systems_drawn(&f, &i, none_kept), "the page says what the partitions hold");
        let mut app = systems_page(&f, &i, unrecorded);
        tap(&mut app, &f, &i, Action::PickSystem(Who::Installed));
        assert!(!swipes(&mut app, &f, &i), "nothing Skyward keeps could be written back");
    }

    #[test]
    fn the_second_system_waits_for_an_update_in_flight_and_the_main_one_does_not() {
        let (f, i) = (fonts(), DeviceInfo { snapshot: SnapshotState::Snapshotted, ..info() });
        let mut app = systems_page(&f, &i, two_systems());
        tap(&mut app, &f, &i, Action::PickSystem(Who::Other));
        assert!(!swipes(&mut app, &f, &i), "nothing to swipe");
        app.set_systems(Ok(Overview { other_starts: true, ..two_systems() }));
        tap(&mut app, &f, &i, Action::PickSystem(Who::Installed));
        assert_eq!(swipe(&mut app, &f, &i, Confirm::SwitchSystem), vec![Effect::SwitchSystem(Who::Installed)], "back to the main system to merge it");
    }

    #[test]
    fn the_second_system_is_removed_once_asked_and_swiped() {
        let (f, i) = (fonts(), info());
        let mut app = systems_page(&f, &i, two_systems());
        assert!(app.frame(&f, &i).hits.iter().any(|h| h.action == Action::Open(Screen::Decrypt)), "the data partition is opened first");
        app.state.data = Data::Open;
        app.refresh();
        tap(&mut app, &f, &i, Action::AskDelete);
        assert_eq!(swipe(&mut app, &f, &i, Confirm::RemoveSystem), vec![Effect::RemoveSystem]);
        assert_eq!((app.state.systems_work, app.state.asking_delete), (Some(SystemsWork::Removing), false));
        app.set_systems(Ok(Overview { other: None, ..two_systems() }));
        assert_eq!(app.state.systems_work, None);
        assert!(app.frame(&f, &i).hits.iter().all(|h| h.action != Action::AskDelete), "nothing left to remove");
        tap(&mut app, &f, &i, Action::Open(Screen::Install));
        assert_eq!(app.screen(), Screen::Install, "the empty place opens the install page");
    }

    #[test]
    fn a_system_installs_beside_the_main_one_in_the_room_chosen() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        app.install(Event::Checking { size: 3 << 30 }, 0);
        app.install(Event::System { name: "Ubuntu Touch".into(), version: "24.04".into(), sizes: vec![3 << 30, 8 << 30, 16 << 30] }, 0);
        // The package lit the panel, so the user looks and taps past the
        // wake guard.
        tap_at(&mut app, &f, &i, Action::SystemSize(1), 1000);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::InstallSystem)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 1000);
        app.touch(Touch::Move(track.x + track.w, y), &f, 1000);
        assert_eq!(app.touch(Touch::Up, &f, 1000), vec![Effect::InstallSystem { size: 8 << 30 }]);
        assert_eq!(app.state.install, Install::Systeming { name: "Ubuntu Touch".into(), stage: None });
        app.install(Event::Progress(Stage::Root { done: 1 << 30, total: 6 << 30 }), 1000);
        assert_eq!(app.state.install, Install::Systeming { name: "Ubuntu Touch".into(), stage: Some(Stage::Root { done: 1 << 30, total: 6 << 30 }) });
        app.key(Key::Back, 1000);
        assert_eq!(app.screen(), Screen::Installing, "the root being written keeps the page");
        app.install(Event::Finished(Outcome::SystemInstalled { name: "Ubuntu Touch".into() }), 1000);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Open(Screen::Systems), 2000), vec![Effect::ReadSystems]);
        assert_eq!(app.screen(), Screen::Systems);
    }

    #[test]
    fn a_switch_asked_for_shows_before_the_systems_are_read() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.dual_boot = true;
        app.switch_asked(Who::Installed);
        assert_eq!((app.screen(), app.state.systems_work), (Screen::Systems, Some(SystemsWork::Switching(Who::Installed))));
        assert!(app.frame(&f, &i).hits.is_empty());
    }

    #[test]
    fn a_stopped_second_system_says_why_on_its_page_and_from_the_home_card() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.dual_boot = true;
        app.system_stopped("no partition userdata showed within 15s".into());
        assert_eq!(app.screen(), Screen::Systems);
        app.set_systems(Ok(Overview { other_starts: true, ..two_systems() }));
        let page = app.frame(&f, &i);
        assert!(page.hits.iter().any(|h| h.action == Action::PickSystem(Who::Installed)), "the main system is there to switch back to");
        assert!(!page.hits.iter().any(|h| h.action == Action::Confirm(Confirm::SwitchSystem)), "nothing is switched until one is picked");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home);
        let opens = app.frame(&f, &i).hits.iter().filter(|h| h.action == Action::Open(Screen::Systems)).count();
        assert_eq!(opens, 2, "the card says so above the entry to the systems");
    }
}
