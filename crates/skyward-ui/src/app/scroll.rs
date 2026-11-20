//! Where the page stands as it scrolls: dragged by a finger, gliding on,
//! springing back from past its end, or eased along for the keys.

use super::*;

impl App {
    /// Where the page comes to rest: where the keys are scrolling it to, or
    /// where it is.
    pub(super) fn resting(&self) -> f32 {
        self.seek.map_or(self.top().1, |(target, _)| target)
    }

    /// A page come to rest with its search bar part folded eases on from
    /// there to fold the bar away or unfold it whole, whichever is nearer.
    pub(super) fn settle_fold(&mut self, now: u64) {
        let Some(fr) = self.frame.as_ref() else { return };
        let at = self.top().1;
        let rest = fr.settle(at).min(self.max_scroll());
        if rest != at {
            self.seek = Some((rest, Spring::new(now, at - rest, 0.0, FOLD_PERIOD)));
            self.dirty = true;
        }
    }

    /// Scroll the page to `target` for the keys, easing there.
    pub(super) fn seek_to(&mut self, target: f32, now: u64) {
        let at = self.top().1;
        self.fling = None;
        self.seek = (target != at).then(|| (target, Spring::new(now, at - target, 0.0, SEEK_PERIOD)));
        self.dirty = true;
    }

    /// Whether the page is gliding, springing back, or easing on of its own
    /// accord at `now`.
    pub(super) fn in_motion(&self, now: u64) -> bool {
        self.fling.is_some_and(|fl| !fl.at(now).1) || self.bounce.is_some_and(|b| !b.at(now).1) || self.seek.is_some_and(|(_, s)| !s.at(now).1)
    }

    /// Stop a page gliding, springing back or easing on of its own accord,
    /// where it has got to, for a finger to hold: one past its end stays
    /// out there as the finger's pull. Whether there was one.
    pub(super) fn catch(&mut self, now: u64) -> bool {
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
    pub(super) fn drag_by(&mut self, d: f32) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use skyward_info::Charge;

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
            plugged: false,
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
}
