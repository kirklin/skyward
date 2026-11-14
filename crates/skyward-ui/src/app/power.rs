//! The panel: lit, dimmed and off, what lights it again, and the charging
//! screen a cable going in brings up.

use super::*;

impl App {
    pub fn display_on(&self) -> bool {
        self.display_on
    }

    /// The backlight at `now`: off, or this share of the brightness at the
    /// slider's top, which holds as the content goes dark after the panel
    /// is told to go off.
    pub fn panel(&self, now: u64) -> Option<f32> {
        if !self.display_on {
            return self.darkening.filter(|_| self.curtain.level(now) > 0.0);
        }
        let (full, level) = (self.state.brightness, self.bright.level(now));
        Some(full * level + dim_level(full) * (1.0 - level))
    }

    /// Something was touched or pressed: the panel stays at full
    /// brightness for the screen-off time from now.
    pub(super) fn stir(&mut self, now: u64) {
        self.active = Some(now);
        self.bright.set(true, now);
    }

    /// Light the panel, as if just touched: the content comes up from as
    /// dark as it is.
    pub(super) fn wake(&mut self, now: u64) {
        if !self.display_on {
            self.display_on = true;
            self.dirty = true;
            self.darkening = None;
            self.curtain.set(true, now);
        }
        self.woke_at = Some(now);
        self.stir(now);
    }

    /// A charger or a computer was attached, `cable_in`, or taken away: the
    /// dark panel lights, and a lit one stays at full brightness for the
    /// screen-off time from now, as on the phone's own system. A cable
    /// going in brings up the charging screen, over the page or on the
    /// panel lit for it, which then goes dark again.
    pub fn plugged(&mut self, now: u64, cable_in: bool) {
        if cable_in {
            let dark = !self.display_on;
            let mut shown = self.charging.take().map_or(Fade::hidden(CHARGE_SHOW, CHARGE_HIDE), |c| c.shown);
            if dark {
                shown.jump(true);
            } else {
                shown.set(true, now);
            }
            self.charging = Some(Charging { since: now, shown, dark_after: dark });
            self.dirty = true;
        }
        if self.display_on {
            self.stir(now);
        } else {
            self.wake(now);
        }
    }

    /// The charging screen has gone, and what was drawn for it with it.
    pub(super) fn end_charging(&mut self) {
        self.charging = None;
        self.charge_art = None;
        self.charge_canvas = None;
    }

    /// Whether the charging screen shows, or is coming up.
    pub(super) fn charge_shown(&self) -> bool {
        self.charging.as_ref().is_some_and(|c| c.shown.on)
    }

    /// Send the charging screen away for the page, and keep the panel lit.
    pub(super) fn dismiss_charging(&mut self, now: u64) {
        if let Some(c) = &mut self.charging {
            c.shown.set(false, now);
            c.dark_after = false;
        }
        self.stir(now);
    }

    /// Whether the panel lit so recently that a touch or key should only
    /// keep it awake.
    pub(super) fn just_woke(&self, now: u64) -> bool {
        self.woke_at.is_some_and(|w| now.saturating_sub(w) < WAKE_GUARD_MS)
    }

    /// Turn the panel off: the content goes dark under the backlight it has
    /// now, then the panel goes off. A finger on the glass lets go of what
    /// it held, and the page stops where it is.
    pub(super) fn sleep(&mut self, now: u64) {
        self.darkening = self.panel(now);
        self.curtain.set(false, now);
        self.display_on = false;
        self.bright = full_brightness();
        match std::mem::replace(&mut self.drag, Drag::Idle) {
            Drag::Confirm { .. } => {
                self.state.confirm = 0.0;
                self.refresh();
            }
            Drag::Pattern => {
                self.state.pattern.clear();
                self.refresh();
            }
            _ => {}
        }
        self.drop_button();
        self.adjusting = false;
        self.fling = None;
        self.bounce = None;
        self.seek = None;
        self.pull = 0.0;
        self.cue = None;
        self.morph = None;
        self.row_press = None;
        self.row_art = None;
        self.end_turn();
    }

    /// A page sliding over another comes to rest at once.
    pub(super) fn end_turn(&mut self) {
        if let Some(turn) = self.turn.take() {
            self.backdrop = Some(turn.left);
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use skyward_info::Charge;

    #[test]
    fn power_key_turns_the_panel_off_and_on() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        assert!(app.key(Key::Power, 0).is_empty());
        assert_eq!(app.panel(0), Some(SLIDER_START), "it waits for volume down first");
        assert_eq!(app.deadline(0), Some(CHORD_MS));
        assert_eq!(app.tick(CHORD_MS), vec![Effect::Ticks], "held on alone, it turns the panel off, ticking the motor");
        assert!(!app.display_on());
        assert!(app.key_up(Key::Power, CHORD_MS + 50).is_empty());
        assert!(app.key(Key::Power, CHORD_MS + 60).is_empty());
        assert_eq!(app.panel(CHORD_MS + 60), Some(SLIDER_START));
        assert!(app.key_up(Key::Power, CHORD_MS + 100).is_empty());
        assert!(app.key(Key::Power, 1000).is_empty());
        assert_eq!(app.key_up(Key::Power, 1050), vec![Effect::Ticks]);
        assert!(!app.display_on(), "let go sooner, it turns the panel off then");
    }

    #[test]
    fn the_power_key_turning_the_screen_off_ticks_the_motor_whatever_the_settings() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        assert_eq!(press(&mut app, Key::Power, 1000), vec![Effect::Ticks]);
        assert!(app.tick(1400).is_empty(), "once");
        app.state.vibrate = false;
        press(&mut app, Key::Power, 2000);
        assert_eq!(press(&mut app, Key::Power, 3000), vec![Effect::Ticks], "with vibration off too");
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        assert!(app.tick(OFF).is_empty(), "but not the screen going off by itself");
        assert!(!app.display_on());
    }

    #[test]
    fn the_power_key_and_volume_down_together_take_a_screenshot() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        app.state.vibrate = true;
        assert!(app.key(Key::Power, 0).is_empty());
        let shot = vec![Effect::Haptic, Effect::Screenshot { storage: false }];
        assert_eq!(app.key(Key::VolumeDown, CHORD_MS - 1), shot, "volume down follows in time");
        assert_eq!(app.chord, None, "nothing is left waiting");
        assert!(app.tick(1000).is_empty());
        assert!(app.key_up(Key::Power, 1000).is_empty());
        assert!(app.key_up(Key::VolumeDown, 1010).is_empty());
        assert!(app.display_on(), "the power key did not turn the panel off");
        assert!(!app.keys, "nor did volume down light a control");
        // Volume down first works the same, the other key coming up first.
        assert!(app.key(Key::VolumeDown, 2000).is_empty());
        assert_eq!(app.key(Key::Power, 2100), shot);
        assert!(app.key_up(Key::VolumeDown, 2150).is_empty());
        assert!(app.key_up(Key::Power, 2200).is_empty());
        assert!(app.display_on());
        assert!(!app.keys);
        // Without a tap of the motor where the settings turn it off, and into
        // the gallery once internal storage is open.
        app.state.vibrate = false;
        app.data_waiting(None);
        app.key(Key::Power, 3000);
        assert_eq!(app.key(Key::VolumeDown, 3010), vec![Effect::Screenshot { storage: true }]);
    }

    #[test]
    fn the_power_key_and_volume_down_too_far_apart_each_do_their_own() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert!(app.key(Key::VolumeDown, 0).is_empty());
        assert!(!app.keys, "volume down waits for the power key first");
        assert!(app.tick(CHORD_MS).is_empty());
        assert!(app.keys, "then lights a control on its own");
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.key(Key::Power, CHORD_MS + 10).is_empty(), "the power key came too late");
        assert!(app.key_up(Key::VolumeDown, CHORD_MS + 20).is_empty());
        assert!(app.tick(CHORD_MS * 2 + 10).is_empty());
        assert_eq!(app.screen(), Screen::Home, "the power key waits on the control lit until it comes up");
        app.key_up(Key::Power, CHORD_MS * 3);
        assert_eq!(app.screen(), Screen::Search, "and then works it");
    }

    #[test]
    fn a_screenshot_says_where_it_went() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.screenshot_saved(Ok(Shot::Album), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图已保存到相册"));
        app.screenshot_saved(Ok(Shot::Held), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图已暂存，解密后存入相册"));
        app.screenshot_saved(Err("No space left on device".into()), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("截图保存失败：No space left on device"));
        app.screenshots_not_kept("Read-only file system", 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("暂存的截图未能存入相册：Read-only file system"));
    }

    /// With the default two minutes, when the panel dims and when it goes
    /// off.
    const DIMS: u64 = 113_000;
    const OFF: u64 = 120_000;

    #[test]
    fn left_alone_the_panel_dims_then_goes_off() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        assert_eq!(app.deadline(0), Some(DIMS));
        app.tick(DIMS - 1);
        assert_eq!(app.panel(DIMS - 1), Some(SLIDER_START));
        app.tick(DIMS);
        assert_eq!(app.deadline(DIMS), Some(DIMS + BACKLIGHT_STEP_MS), "the backlight steps down a frame at a time");
        let midway = app.panel(DIMS + 250).unwrap();
        assert!(midway < SLIDER_START && midway > dim_level(SLIDER_START), "{midway}");
        assert_eq!(app.panel(DIMS + 500), Some(dim_level(SLIDER_START)));
        assert_eq!(app.deadline(DIMS + 500), Some(OFF), "dim until it goes off");
        app.tick(OFF);
        assert!(!app.display_on());
        assert_eq!(app.panel(OFF), Some(dim_level(SLIDER_START)), "the content goes dark under the dim backlight");
        app.tick(OFF + 365);
        assert_eq!(app.panel(OFF + 365), None);
        assert_eq!(app.deadline(OFF + 365), None, "nothing to wait for on a dark panel");
    }

    #[test]
    fn time_before_the_first_tick_does_not_count_as_idle() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        // The UI started long after boot; the idle clock begins at the
        // first tick, not at zero.
        app.tick(5_000_000);
        assert_eq!(app.panel(5_000_000), Some(SLIDER_START));
        assert_eq!(app.deadline(5_000_000), Some(5_000_000 + DIMS));
        app.tick(5_000_000 + OFF);
        assert!(!app.display_on());
    }

    #[test]
    fn the_dim_level_sits_below_the_brightness_set() {
        assert_eq!(dim_level(1.0), DIM_CEILING);
        assert_eq!(dim_level(0.06), MIN_BRIGHTNESS);
        assert_eq!(dim_level(0.03), MIN_BRIGHTNESS);
        assert_eq!(dim_level(0.01), 0.01, "never brighter than the level set");
        assert_eq!(dim_span(15_000), 3000);
        assert_eq!(dim_span(120_000), DIM_MS);
    }

    #[test]
    fn a_touch_while_dim_brightens_and_still_counts() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        app.tick(DIMS);
        assert_eq!(app.panel(DIMS + 500), Some(dim_level(SLIDER_START)));
        let (x, y) = spot(&mut app, &f, &i, Action::Open(Screen::Settings));
        app.touch(Touch::Down(x, y), &f, DIMS + 600);
        app.touch(Touch::Up, &f, DIMS + 650);
        assert_eq!(app.screen(), Screen::Settings, "the tap counts");
        assert_eq!(app.panel(DIMS + 800), Some(SLIDER_START), "back to full brightness");
        assert_eq!(app.deadline(DIMS + 800), Some(DIMS + 650 + DIMS), "the wait starts over");
    }

    #[test]
    fn a_touch_on_the_dark_panel_lights_nothing() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let (x, y) = spot(&mut app, &f, &i, Action::Open(Screen::Settings));
        press(&mut app, Key::Power, 0);
        assert!(app.touch(Touch::Down(x, y), &f, 100).is_empty());
        assert!(app.touch(Touch::Move(x, y - 300.0), &f, 120).is_empty());
        assert!(app.touch(Touch::Up, &f, 140).is_empty());
        assert!(!app.display_on(), "the panel stays dark");
        assert_eq!(app.screen(), Screen::Home, "the tap pressed nothing");
        assert_eq!(app.top().1, 0.0, "nor scrolled the page");
        press(&mut app, Key::Power, 200);
        assert!(app.display_on());
        tap_at(&mut app, &f, &i, Action::Open(Screen::Settings), 700);
        assert_eq!(app.screen(), Screen::Settings, "a tap on the lit panel does");
    }

    #[test]
    fn a_cable_coming_out_lights_the_dark_panel_and_keeps_a_lit_one_lit() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        app.tick(OFF);
        app.tick(OFF + 365);
        assert_eq!(app.panel(OFF + 365), None);
        let pulled = OFF + 1000;
        app.plugged(pulled, false);
        assert!(app.display_on(), "the cable coming out lights the panel");
        assert!(!app.charge_shown(), "and shows the page");
        assert_eq!(app.panel(pulled), Some(SLIDER_START));
        assert_eq!(app.deadline(pulled + 520), Some(pulled + DIMS), "for the screen-off time, once the content is up");
        app.tick(pulled + DIMS);
        assert_eq!(app.panel(pulled + DIMS + 500), Some(dim_level(SLIDER_START)));
        let again = pulled + DIMS + 600;
        app.plugged(again, false);
        assert_eq!(app.panel(again + 150), Some(SLIDER_START), "a cable coming out brightens a lit panel");
        assert_eq!(app.deadline(again + 150), Some(again + DIMS), "and the wait starts over");
        press(&mut app, Key::Power, again + 200);
        assert!(!app.display_on(), "the power key turns it off at once, as ever");
    }

    #[test]
    fn a_cable_going_in_on_the_dark_panel_shows_the_charging_screen_and_goes_dark_again() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        press(&mut app, Key::Power, 0);
        app.tick(365);
        assert_eq!(app.panel(365), None);
        let plugged = 1000;
        app.plugged(plugged, true);
        assert!(app.display_on() && app.charge_shown(), "the charging screen lights the panel");
        assert_eq!(app.deadline(plugged + 1000), Some(0), "its tide moves a frame at a time");
        app.tick(plugged + CHARGE_MS - 1);
        assert!(app.display_on());
        app.tick(plugged + CHARGE_MS);
        assert!(!app.display_on(), "then the panel goes dark again");
        assert!(app.panel(plugged + CHARGE_MS + 100).is_some(), "as the charging screen goes dark");
        app.tick(plugged + CHARGE_MS + 365);
        assert_eq!(app.panel(plugged + CHARGE_MS + 365), None);
        assert!(app.charging.is_none(), "and goes with it");
    }

    #[test]
    fn a_cable_going_in_on_a_lit_panel_brings_the_charging_screen_over_the_page_and_away() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        app.plugged(1000, true);
        assert!(app.display_on() && app.charge_shown());
        app.tick(1000 + CHARGE_MS);
        assert!(app.display_on(), "the panel stays lit");
        assert!(!app.charge_shown(), "as the charging screen goes for the page");
        app.tick(1000 + CHARGE_MS + 300);
        assert!(app.charging.is_none());
    }

    #[test]
    fn a_touch_or_a_volume_key_sends_the_charging_screen_away_and_keeps_the_panel_lit() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let (x, y) = spot(&mut app, &f, &i, Action::Open(Screen::Settings));
        app.tick(0);
        press(&mut app, Key::Power, 0);
        app.tick(365);
        app.plugged(1000, true);
        app.touch(Touch::Down(x, y), &f, 1500);
        app.touch(Touch::Up, &f, 1550);
        assert!(!app.charge_shown(), "the touch sent it away");
        assert_eq!(app.screen(), Screen::Home, "and pressed nothing");
        app.tick(1000 + CHARGE_MS);
        assert!(app.display_on(), "the panel stays lit for the page");
        app.plugged(10_000, true);
        press(&mut app, Key::VolumeDown, 10_500);
        assert!(!app.charge_shown(), "a volume key sends it away too");
        assert!(app.display_on());
    }

    #[test]
    fn the_charging_screen_shows_the_level_over_a_tide_of_light() {
        let (f, i) = (fonts(), DeviceInfo { battery: Some(battery_at(64, Charge::Charging)), ..info() });
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        press(&mut app, Key::Power, 0);
        app.tick(365);
        app.plugged(1000, true);
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 3000);
        let px = |x: u32, y: u32| {
            let k = ((y * W + x) * 4) as usize;
            [c.data[k], c.data[k + 1], c.data[k + 2]]
        };
        assert_eq!(px(0, 0), [0, 0, 0], "on black");
        let foot = px(W / 2, H - 4);
        assert!(foot[2] > 40 && foot[2] > foot[0] * 4 && foot[2] > foot[1] * 2, "the tide's light at the foot is blue: {foot:?}");
    }

    #[test]
    fn the_content_goes_dark_under_the_backlight_it_had_before_the_panel_goes_off() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        press(&mut app, Key::Power, 1000);
        assert!(!app.display_on(), "touches and keys take the screen as off at once");
        assert_eq!(app.panel(1000), Some(SLIDER_START), "while its content goes dark under the backlight it had");
        assert_eq!(app.deadline(1000), Some(0), "a frame at a time");
        assert_eq!(app.panel(1300), Some(SLIDER_START));
        app.tick(1365);
        assert_eq!(app.panel(1365), None, "then the panel goes off");
        assert_eq!(app.deadline(1365), None);
    }

    #[test]
    fn the_power_key_as_the_content_goes_dark_brings_it_back() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        press(&mut app, Key::Power, 1000);
        app.tick(1100);
        press(&mut app, Key::Power, 1100);
        assert!(app.display_on(), "the power key lit it again");
        let level = app.curtain.level(1100);
        assert!(level > 0.0 && level < 1.0, "from as dark as it had gone: {level}");
        assert_eq!(app.curtain.level(1100 + 520), 1.0);
    }

    #[test]
    fn the_content_comes_up_from_black_as_the_panel_lights() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.tick(0);
        press(&mut app, Key::Power, 0);
        app.tick(365);
        press(&mut app, Key::Power, 1000);
        let mut c = Canvas::new(W, H);
        let light = |c: &Canvas| c.data.as_chunks::<4>().0.iter().map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum::<u64>();
        app.compose(&mut c, &f, &i, "12:00", 1000);
        let first = light(&c);
        app.compose(&mut c, &f, &i, "12:00", 1260);
        let halfway = light(&c);
        app.compose(&mut c, &f, &i, "12:00", 1520);
        let full = light(&c);
        assert_eq!(first, 0, "black as the panel lights");
        assert!(halfway > full / 2 && halfway < full, "{halfway} of {full} halfway through");
    }

    #[test]
    fn only_the_power_key_lights_the_dark_panel_and_does_nothing_else() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        press(&mut app, Key::Power, 0);
        for (key, at) in [(Key::Back, 10), (Key::VolumeUp, 20), (Key::VolumeDown, 30)] {
            assert!(press(&mut app, key, at).is_empty());
            assert!(!app.display_on(), "{key:?} leaves the panel dark");
        }
        assert!(press(&mut app, Key::Power, 40).is_empty());
        assert!(app.display_on());
        assert_eq!(app.screen(), Screen::Settings, "back did not go back");
    }

    #[test]
    fn a_stray_press_just_after_the_panel_lights_only_keeps_it_awake() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        press(&mut app, Key::Power, 0);
        // A key wakes the panel; the next, within the guard, does not sleep
        // it again or act.
        app.key(Key::Power, 100);
        assert!(app.display_on());
        assert!(app.key(Key::Power, 300).is_empty());
        assert!(app.display_on(), "a second power press did not turn it off");
        assert!(app.key(Key::Back, 350).is_empty());
        assert_eq!(app.screen(), Screen::Reboot, "back did not go back");
        // Past the guard, power turns it off as usual.
        press(&mut app, Key::Power, 600);
        assert!(!app.display_on());
    }

    #[test]
    fn a_stray_touch_just_after_an_install_lights_the_panel_presses_nothing() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        press(&mut app, Key::Power, 0);
        app.install(Event::Finished(Outcome::Installed { slot: Slot::A, wipe: false }), 100);
        assert!(app.display_on(), "the finished install lit the panel");
        // A finger reaching for the phone lands on 重启到系统; within the
        // guard it only keeps the panel awake.
        assert!(tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 200).is_empty());
        assert_eq!(app.screen(), Screen::Installing, "the stray tap did not reboot");
        // Past the guard the same tap reboots.
        assert_eq!(
            tap_at(&mut app, &f, &i, Action::RebootTo(Target::System), 700),
            vec![Effect::Reboot(Target::System)]
        );
    }

    #[test]
    fn a_finger_held_as_the_panel_goes_off_lets_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Confirm(Confirm::Reboot)).unwrap().rect;
        let y = track.centre().1;
        app.touch(Touch::Down(track.x + track.h / 2.0, y), &f, 0);
        app.touch(Touch::Move(track.x + track.w * 0.6, y), &f, 10);
        assert!(app.state.confirm > 0.0);
        press(&mut app, Key::Power, 20);
        assert_eq!(app.state.confirm, 0.0, "the knob is back at the start");
        app.touch(Touch::Move(track.x + track.w, y), &f, 30);
        assert!(app.touch(Touch::Up, &f, 40).is_empty(), "no reboot from a dark panel");
        assert!(!app.display_on());
    }

    #[test]
    fn never_leaves_the_panel_on() {
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.state.screen_off = SCREEN_OFF.iter().position(Option::is_none).unwrap();
        app.tick(0);
        assert_eq!(app.deadline(0), None);
        let day = 24 * 3600 * 1000;
        app.tick(day);
        assert_eq!(app.panel(day), Some(SLIDER_START));
    }

    #[test]
    fn the_installer_lights_the_panel_when_it_needs_the_user() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        press(&mut app, Key::Power, 0);
        app.install(Event::Checking { size: 1 << 20 }, 10);
        assert!(!app.display_on(), "reading the package does not light it");
        app.install(Event::Image { kind: ImageKind::Dtbo, size: 1 << 20, partitions: vec!["dtbo"] }, 20);
        assert!(app.display_on(), "an image waiting for its partition does");
        app.tick(20 + 520);
        assert_eq!(app.deadline(20 + 520), Some(20 + DIMS));
        app.state.install = Install::Flashing { partition: "dtbo", stage: None };
        press(&mut app, Key::Power, 500);
        app.install(Event::Progress(Stage::Flashing { done: 1, total: 2 }), 510);
        assert!(!app.display_on(), "progress does not");
        app.install(Event::Finished(Outcome::Flashed { partition: "dtbo", slots: vec![Slot::A] }), 520);
        assert!(app.display_on(), "the end does");
    }

    #[test]
    fn an_install_ending_after_its_page_was_left_keeps_the_panel_dark() {
        let (f, i) = (fonts(), info());
        let mut app = sideloading(&f, &i);
        assert_eq!(app.key(Key::Back, 0), vec![Effect::Cancel]);
        press(&mut app, Key::Power, 10);
        app.install(Event::Finished(Outcome::Cancelled), 20);
        assert!(!app.display_on());
        assert_eq!(app.state.install, Install::Idle);
    }
}
