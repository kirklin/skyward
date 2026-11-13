//! Fingers and keys: what a touch on the glass and a key going down or
//! coming up do.

use super::*;

impl App {
    pub fn touch(&mut self, ev: Touch, f: &Fonts, now: u64) -> Vec<Effect> {
        // A finger on the dark panel lights nothing, as on the phone's own
        // system, where the power key lights it.
        if !self.display_on {
            return Vec::new();
        }
        // A finger on the charging screen sends it away for the page, and
        // does nothing else until it lifts.
        if self.charge_shown() {
            if matches!(ev, Touch::Down(..)) {
                self.dismiss_charging(now);
                self.drag = Drag::Waking;
            }
            return Vec::new();
        }
        // A finger landing just after the panel lit only keeps it awake.
        if matches!(ev, Touch::Down(..)) && self.just_woke(now) {
            self.stir(now);
            self.drag = Drag::Waking;
            return Vec::new();
        }
        // A finger on the glass puts the keys' light out, whatever a key
        // held down was doing comes to nothing, and a page sliding comes to
        // rest.
        if matches!(ev, Touch::Down(..)) {
            if self.keys {
                self.keys = false;
                self.adjusting = false;
                self.dirty = true;
            }
            self.drop_button();
            self.end_turn();
        }
        self.stir(now);
        let t = self.theme();
        let m = t.m;
        let (slop, edge) = (m.dp(8.0), m.dp(20.0));
        let scroll = self.top().1;
        let mut fx = Vec::new();
        let pressing = self.pressing();
        match ev {
            Touch::Down(x, y) => {
                self.tracker.clear();
                self.tracker.add(now, y);
                let side = if x < edge {
                    Some(true)
                } else if x > m.w as f32 - edge {
                    Some(false)
                } else {
                    None
                };
                // The first page has nothing to go back to: its edges are
                // like the rest of it.
                let at_edge = side.filter(|_| self.stack.len() > 1);
                // A swipe in from an edge goes back over whatever the page is
                // doing: a finger landing there leaves a gliding or springing
                // page alone until it drags the page, and presses nothing.
                if at_edge.is_some() && self.in_motion(now) {
                    self.drag = Drag::Press { x, y, action: None, edge: at_edge, at: now };
                    return fx;
                }
                // A terminal's screen or the editor's text gliding stops
                // under a finger landing on it, which holds it to scroll on.
                let glided = self.glide.take().filter(|(glide, ..)| !glide.at(now).1);
                let history = self.frame.as_ref().and_then(|fr| fr.scrollback).filter(|(r, _)| r.contains(x, y + scroll));
                if let (Some(_), Some((_, pitch))) = (glided, history) {
                    self.drag = Drag::History { last: y, carry: 0.0, pitch };
                    return fx;
                }
                // Anywhere else it stops such a page and holds it; the
                // control under the finger is not pressed.
                if self.catch(now) {
                    self.drag = Drag::Scroll { last: y };
                    return fx;
                }
                // The `<` stays put while the page scrolls; under the bar
                // only it answers. A pinned strip answers where it shows,
                // over the rows beneath it, and so does a docked one.
                let back = widgets::back_rect(&t);
                let strip = self.frame.as_ref().and_then(|fr| fr.pinned.as_ref()).map(|p| (p, pinned_top(&t, p, scroll)));
                let on_strip = strip.filter(|&(p, top)| y >= top && y < top + p.canvas.h as f32);
                let dock = self.frame.as_ref().and_then(|fr| fr.dock.as_ref()).map(|d| (d, self.h as f32 - d.canvas.h as f32));
                let on_dock = dock.filter(|&(_, top)| y >= top);
                let button = self.frame.as_ref().and_then(|fr| fr.button).map(|(_, a)| (a, widgets::button_rect(&t)));
                let hit = if self.frame.as_ref().is_some_and(|fr| fr.back) && back.contains(x, y) {
                    Some((Action::Back, back))
                } else if let Some(button) = button.filter(|(_, r)| r.contains(x, y)) {
                    Some(button)
                } else if let Some((d, top)) = on_dock {
                    d.hits.iter().find(|h| h.rect.contains(x, y - top)).map(|h| (h.action, Rect { y: h.rect.y + top, ..h.rect }))
                } else if let Some((p, top)) = on_strip {
                    p.hits.iter().find(|h| h.rect.contains(x, y - top)).map(|h| (h.action, Rect { y: h.rect.y + top, ..h.rect }))
                } else if self.bar.on && y < widgets::bar_bottom(&t) {
                    None
                } else {
                    // A search bar folding away answers only on what is
                    // left of it.
                    let fr = self.frame.as_ref();
                    fr.and_then(|fr| fr.hit(x, y + scroll).filter(|h| fr.shown(h.rect, scroll).is_some_and(|r| r.contains(x, y))))
                        .map(|h| (h.action, h.rect))
                };
                let action = hit.map(|h| h.0);
                // A row of the page darkens behind its words a moment after
                // the finger lands on it, if the finger stays.
                let row = hit
                    .filter(|&(a, r)| self.frame.as_ref().is_some_and(|fr| fr.hits.contains(&Hit { rect: r, action: a })))
                    .filter(|&(a, r)| row_wide(&t, r) && !matches!(a, Action::Open(Screen::Search) | Action::Confirm(_) | Action::TypeInto(_)))
                    .map(|(_, r)| r);
                self.drag = match hit {
                    Some((Action::Slider(which), track)) => {
                        self.set_slider(&t, which, track, x);
                        Drag::Slider(which, track)
                    }
                    Some((Action::Confirm(which), track)) if self.on_knob(track, x) => {
                        let knob_x = track.x + (track.w - track.h) * self.state.confirm;
                        Drag::Confirm { which, track, grab: x - knob_x }
                    }
                    Some((Action::Pattern, _)) => {
                        self.state.pattern.clear();
                        self.trace(&t, x, y + scroll);
                        Drag::Pattern
                    }
                    _ => {
                        if let Some(rect) = row {
                            self.row_press = Some(RowPress { rect, rise: now + ROW_PRESS_DELAY_MS, fall: None });
                            self.row_art = None;
                        }
                        Drag::Press { x, y, action, edge: at_edge, at: now }
                    }
                };
            }
            Touch::Move(x, y) => match std::mem::replace(&mut self.drag, Drag::Idle) {
                Drag::Press { x: x0, y: y0, action, edge: at_edge, at } => {
                    self.tracker.add(now, y);
                    let (dx, dy) = (x - x0, y - y0);
                    self.drag = match at_edge {
                        Some(from_left) if dx.abs() > slop && dx.abs() > dy.abs() => {
                            self.dirty = true;
                            Drag::Edge { start: x0, from_left, armed: false, x, cy: y0 - m.dp(CUE_LIFT_DP) }
                        }
                        _ if dy.abs() > slop => {
                            let history = self.frame.as_ref().and_then(|fr| fr.scrollback).filter(|(r, _)| r.contains(x0, y0 + scroll));
                            if let Some((_, pitch)) = history {
                                self.scroll_history(y0, y, 0.0, pitch)
                            } else {
                                // A page still gliding or springing back is
                                // caught where it has got to; one at rest
                                // follows the finger from where it came down.
                                if !self.catch(now) {
                                    self.drag_by(y0 - y);
                                }
                                Drag::Scroll { last: y }
                            }
                        }
                        _ => Drag::Press { x: x0, y: y0, action, edge: at_edge, at },
                    };
                    if !matches!(self.drag, Drag::Press { .. }) {
                        self.let_row_go(now);
                    }
                }
                Drag::Scroll { last } => {
                    self.tracker.add(now, y);
                    self.drag_by(last - y);
                    self.drag = Drag::Scroll { last: y };
                }
                Drag::Edge { start, from_left, armed, cy, .. } => {
                    let travel = if from_left { x - start } else { start - x };
                    let ready = travel > m.dp(BACK_SWIPE_DP);
                    // A tap of the motor as the `<` appears, as the system
                    // gives.
                    if ready && !armed && self.state.vibrate {
                        fx.push(Effect::Haptic);
                    }
                    self.drag = Drag::Edge { start, from_left, armed: ready, x, cy };
                    self.dirty = true;
                }
                Drag::Slider(which, track) => {
                    self.set_slider(&t, which, track, x);
                    self.drag = Drag::Slider(which, track);
                }
                Drag::Confirm { which, track, grab } => {
                    let travel = (track.w - track.h).max(1.0);
                    self.state.confirm = ((x - grab - track.x) / travel).clamp(0.0, 1.0);
                    self.repaint_confirm(&t, f, which, track);
                    self.drag = Drag::Confirm { which, track, grab };
                }
                Drag::Pattern => {
                    self.trace(&t, x, y + scroll);
                    self.drag = Drag::Pattern;
                }
                Drag::History { last, carry, pitch } => {
                    self.tracker.add(now, y);
                    self.drag = self.scroll_history(last, y, carry, pitch);
                }
                Drag::Waking => self.drag = Drag::Waking,
                Drag::Held => self.drag = Drag::Held,
                Drag::Idle => {}
            },
            Touch::Up => match std::mem::replace(&mut self.drag, Drag::Idle) {
                Drag::Press { action: Some(a), .. } => {
                    if let Some(p) = &mut self.row_press {
                        p.lift(now);
                    }
                    fx.extend(self.act(a, now));
                }
                Drag::Press { x, y, action: None, .. } if self.screen() == Screen::Editor => self.place_caret(f, x, y + scroll),
                Drag::History { pitch, .. } => {
                    // A finger lifted while still moving sends the rows
                    // gliding on.
                    let speed = self.tracker.velocity(now);
                    if speed.abs() >= m.dp(MIN_FLING_DP) {
                        let speed = speed.clamp(-m.dp(MAX_FLING_DP), m.dp(MAX_FLING_DP));
                        self.glide = Some((Fling::new(now, 0.0, speed, self.px_per_dp * 160.0), pitch, 0));
                    }
                }
                Drag::Scroll { .. } if self.pull != 0.0 => {
                    // A page let go past its end springs back.
                    self.bounce = Some(Spring::new(now, motion::rubber(self.pull, self.h as f32), 0.0, 0.4));
                    self.pull = 0.0;
                    self.dirty = true;
                }
                Drag::Scroll { .. } => {
                    // A finger lifted while still moving sends the page
                    // gliding on. One lifted slower leaves it where it is,
                    // but for a search bar part folded, which settles
                    // folded or unfolded; so does one lifted in the fold
                    // whose glide would end in it too.
                    let speed = -self.tracker.velocity(now);
                    let folding = |s: f32| self.frame.as_ref().is_some_and(|fr| fr.settle(s) != s);
                    let glide = (speed.abs() >= m.dp(MIN_FLING_DP)).then(|| {
                        let speed = speed.clamp(-m.dp(MAX_FLING_DP), m.dp(MAX_FLING_DP));
                        Fling::new(now, scroll, speed, self.px_per_dp * 160.0)
                    });
                    match glide {
                        Some(glide) if !(folding(scroll) && folding(glide.at(u64::MAX).0)) => self.fling = Some(glide),
                        _ => self.settle_fold(now),
                    }
                }
                Drag::Edge { armed: true, .. } => fx.extend(self.back(now)),
                Drag::Edge { start, from_left, x, cy, .. } => {
                    // Short of going back, the cue sinks back into the edge.
                    let travel = if from_left { x - start } else { start - x };
                    let depth = widgets::back_cue_depth(&t, travel);
                    self.cue = Some((from_left, cy, Spring::new(now, depth, 0.0, 0.15)));
                    self.dirty = true;
                }
                Drag::Confirm { which, track, .. } => {
                    let done = self.state.confirm > SLID;
                    self.state.confirm = 0.0;
                    self.repaint_confirm(&t, f, which, track);
                    if done {
                        fx.extend(self.confirmed(which, now));
                    }
                }
                Drag::Pattern => fx.extend(self.drawn(now)),
                // Let go, the strength slider taps the motor as strongly as
                // it was left at.
                Drag::Slider(Slider::Vibration, _) if self.state.vibrate => fx.push(Effect::Haptic),
                _ => {}
            },
        }
        if self.pressing() != pressing {
            self.dirty = true;
        }
        // A key of the on-screen keyboard, or a choice of a list open over
        // the page, shows held down while the finger rests on it.
        let held = match self.drag {
            Drag::Press { action: Some(a @ (Action::Type(_) | Action::PickSecurity(_))), .. } => Some(a),
            _ => None,
        };
        if held != self.state.held {
            let was = std::mem::replace(&mut self.state.held, held);
            // Only the rows of what is let go and what is held change.
            let rects: Option<Vec<Rect>> = self
                .frame
                .as_ref()
                .and_then(|fr| [was, held].into_iter().flatten().map(|a| fr.hits.iter().find(|h| h.action == a).map(|h| h.rect)).collect());
            match rects {
                Some(rects) => rects.into_iter().for_each(|r| self.refresh_rows(r.y.floor() as u32, (r.y + r.h).ceil() as u32)),
                None => self.refresh(),
            }
        }
        fx
    }

    /// The button a finger holds down, which shows held at once: `<`, the
    /// search page's round button, or the home page's search bar.
    pub(super) fn pressing(&self) -> Option<Action> {
        let button = self.frame.as_ref().and_then(|fr| fr.button).map(|(_, a)| a);
        match self.drag {
            Drag::Press { action: Some(a @ (Action::Back | Action::CloseSearch | Action::Open(Screen::Search))), .. } => Some(a),
            Drag::Press { action: Some(a), .. } if Some(a) == button => Some(a),
            _ => None,
        }
    }

    /// The finger left the row it held, scrolling the page or swiping: a
    /// row not darkening yet never does, and one darkening fades back.
    fn let_row_go(&mut self, now: u64) {
        let Some(p) = self.row_press else { return };
        if now < p.rise {
            self.row_press = None;
            self.row_art = None;
        } else if p.fall.is_none() {
            self.row_press = Some(RowPress { fall: Some(now), ..p });
        }
        self.dirty = true;
    }

    /// No touchscreen answers: the keys work the pages, their light shown
    /// from now on.
    pub fn touchscreen_missing(&mut self) {
        self.keys = true;
        self.dirty = true;
    }

    /// A button went down. With the keys working the page, the volume keys
    /// move the light from control to control and the power key works the
    /// one lit as it comes back up; otherwise the first volume press lights
    /// a control, and the power key turns the panel off. The power key and
    /// volume down together take a screenshot instead: whichever of the two
    /// goes down first waits [`CHORD_MS`] for the other, or until it is let
    /// go, and then does what it does alone, as pressed when it went down.
    pub fn key(&mut self, key: Key, now: u64) -> Vec<Effect> {
        // Any button stops a gliding page, and brings a sliding one to rest.
        self.fling = None;
        self.end_turn();
        if let Some((first, at)) = self.chord
            && matches!((first, key), (Key::Power, Key::VolumeDown) | (Key::VolumeDown, Key::Power))
            && now < at + CHORD_MS
        {
            return self.screenshot(key, now);
        }
        let mut fx = self.let_chord_go(now);
        if !self.display_on {
            // The power key lights the dark panel, and does only that, as on
            // the phone's own system; the other keys leave it dark.
            if key == Key::Power {
                self.wake(now);
            }
            self.button = Some((key, Press::Spent));
            return fx;
        }
        // A key other than power sends the charging screen away for the
        // page, and does nothing else.
        if key != Key::Power && self.charge_shown() {
            self.dismiss_charging(now);
            self.button = Some((key, Press::Spent));
            return fx;
        }
        // A button pressed just after the panel lit only keeps it awake, so
        // a power press meant to wake it does not turn it back off.
        if self.just_woke(now) {
            self.stir(now);
            self.button = Some((key, Press::Spent));
            return fx;
        }
        self.stir(now);
        if matches!(key, Key::Power | Key::VolumeDown) {
            self.chord = Some((key, now));
            return fx;
        }
        fx.extend(self.key_down(key, now, now));
        fx
    }

    /// The power key and volume down went down together: what the panel
    /// shows is saved as a picture, with a tap of the motor, and neither key
    /// does anything more until let go.
    fn screenshot(&mut self, key: Key, now: u64) -> Vec<Effect> {
        self.chord = None;
        self.stir(now);
        self.button = Some((key, Press::Spent));
        let mut fx = Vec::new();
        if self.state.vibrate {
            fx.push(Effect::Haptic);
        }
        fx.push(Effect::Screenshot { storage: self.state.place_open(Place::Storage) });
        fx
    }

    /// The key waiting for its partner in a screenshot does what it does
    /// alone `now`, as pressed when it went down.
    pub(super) fn let_chord_go(&mut self, now: u64) -> Vec<Effect> {
        match self.chord.take() {
            Some((key, at)) => self.key_down(key, at, now),
            None => Vec::new(),
        }
    }

    /// What a button that went down `at` does on its own, `now`: a key
    /// held is timed from when it went down, the screen goes off from now.
    fn key_down(&mut self, key: Key, at: u64, now: u64) -> Vec<Effect> {
        match key {
            Key::Back => {
                self.button = Some((key, Press::Spent));
                self.act(Action::Back, at)
            }
            Key::VolumeUp | Key::VolumeDown => {
                let down = key == Key::VolumeDown;
                self.button = Some((key, Press::Volume { down, next: at + REPEAT_DELAY_MS }));
                self.volume(down, at)
            }
            // A press before the page it lands on is drawn is lost, rather
            // than taken for one on nothing.
            Key::Power if self.keys && self.frame.is_none() => {
                self.button = Some((key, Press::Spent));
                Vec::new()
            }
            Key::Power => {
                self.dirty = true;
                match self.keys.then(|| self.lit()).flatten() {
                    // Held on a swipe-to-confirm control, the key slides
                    // its knob along as a finger would.
                    Some(Spot { stop: Stop::Hit(Action::Confirm(which)), rect, .. }) => {
                        self.button = Some((key, Press::Slide { which, track: rect, at }))
                    }
                    Some(spot) => self.button = Some((key, Press::Power { spot, at })),
                    // Turning the screen off, the key ticks the motor.
                    None => {
                        self.button = Some((key, Press::Spent));
                        self.sleep(now);
                        return vec![Effect::Ticks];
                    }
                }
                Vec::new()
            }
        }
    }

    /// A button came back up: the power key works the control it went down
    /// on, or confirms with the knob it slid far enough. Let go before its
    /// partner in a screenshot came, the key waiting for it first does what
    /// it does alone.
    pub fn key_up(&mut self, key: Key, now: u64) -> Vec<Effect> {
        let mut fx = match self.chord {
            Some((first, _)) if first == key => self.let_chord_go(now),
            _ => Vec::new(),
        };
        let Some((down, press)) = self.button else { return fx };
        if down != key {
            return fx;
        }
        self.button = None;
        self.dirty = true;
        match press {
            Press::Power { spot, .. } => fx.extend(self.work(spot, now)),
            Press::Slide { which, track, .. } => {
                let done = self.state.confirm > SLID;
                self.let_knob_go(which, track);
                if done {
                    fx.extend(self.confirmed(which, now));
                } else {
                    self.say(self.state.lang.pick(["请按住电源键", "請按住電源鍵", "Hold the power key"]), now);
                }
            }
            Press::Volume { .. } | Press::Spent => {}
        }
        fx
    }

    /// Whatever the button held down was doing comes to nothing until it is
    /// let go: a knob it slid goes back.
    pub(super) fn drop_button(&mut self) {
        let Some((key, press)) = self.button else { return };
        if let Press::Slide { which, track, .. } = press {
            self.let_knob_go(which, track);
        }
        self.button = Some((key, Press::Spent));
    }

    /// Let go of a swipe-to-confirm control's knob, which goes back to the
    /// start of its track.
    fn let_knob_go(&mut self, which: Confirm, track: Rect) {
        self.state.confirm = 0.0;
        self.knob = Some((which, track));
        self.dirty = true;
    }

    fn on_knob(&self, track: Rect, x: f32) -> bool {
        let knob_x = track.x + (track.w - track.h) * self.state.confirm;
        x >= knob_x && x <= knob_x + track.h
    }

    fn repaint_confirm(&mut self, t: &Theme, f: &Fonts, which: Confirm, track: Rect) {
        self.paint_knob(t, f, which, track);
        self.dirty = true;
    }

    /// Draw the swipe-to-confirm control at `track` on the page again, its
    /// knob where it has come to.
    pub(super) fn paint_knob(&mut self, t: &Theme, f: &Fonts, which: Confirm, track: Rect) {
        let (label, danger) = pages::confirm_label(which, &self.state);
        if let Some(fr) = self.frame.as_mut() {
            fr.canvas.fill_rect(track.x, track.y, track.w, track.h, t.pal.bg);
            widgets::confirm(&mut fr.canvas, t, f, track.y, label, danger, self.state.confirm);
        }
        self.layer = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use crate::color::over;

    #[test]
    fn a_row_held_darkens_behind_its_words_a_moment_after_the_finger_lands() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Home, W, H, PPD);
        let mut c = Canvas::new(W, H);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Backup)).expect("on the home page").rect;
        // The card between the row's words and its arrow.
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        app.compose(&mut c, &f, &i, "12:00", 0);
        let ink = darkest(&c, row);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r);

        app.touch(Touch::Down(x, y), &f, 1000);
        let rise = 1000 + ROW_PRESS_DELAY_MS;
        assert_eq!(app.deadline(1000), Some(rise), "nothing moves until the row starts darkening");
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert!(app.row_art.is_none(), "a finger that may yet scroll does not wait for the row drawn held down");
        app.compose(&mut c, &f, &i, "12:00", rise);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r, "it has not started yet");
        app.compose(&mut c, &f, &i, "12:00", rise + 30);
        let partway = grey_at(&c, x, y);
        assert!(partway < 255 && partway > 225, "darkening after 30 ms: {partway}");
        app.compose(&mut c, &f, &i, "12:00", 2000);
        assert_eq!(grey_at(&c, x, y), 225, "as dark as the stock settings' row held down");
        assert_eq!(darkest(&c, row), ink, "its words keep their ink");
        assert_ne!(app.deadline(2000), Some(0), "held still, nothing moves");

        // The finger moves on to scroll: the row fades back.
        app.touch(Touch::Move(x, y + t.m.dp(30.0)), &f, 2100);
        assert_eq!(app.deadline(2100), Some(0));
        app.touch(Touch::Up, &f, 2200);
        app.tick(3000);
        assert!(app.row_press.is_none() && app.row_art.is_none(), "faded, it is let go");
        assert_eq!(app.screen(), Screen::Home, "a scroll opens nothing");
    }

    #[test]
    fn a_row_scrolled_away_from_before_it_darkens_never_shows_held() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Home, W, H, PPD);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Backup)).expect("on the home page").rect;
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 1000);
        app.touch(Touch::Move(x, y + t.m.dp(30.0)), &f, 1050);
        assert!(app.row_press.is_none());
    }

    #[test]
    fn a_row_tapped_quickly_shows_held_from_the_lift_and_fades_back() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let mut c = Canvas::new(W, H);
        let row = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Toggle(Switch::Vibrate)).expect("on the settings page").rect;
        let (x, y) = (W as f32 * 0.6, row.y + row.h / 2.0);
        tap_at(&mut app, &f, &i, Action::Toggle(Switch::Vibrate), 1000);
        assert!(!app.state.vibrate, "the tap worked the switch");
        app.compose(&mut c, &f, &i, "12:00", 1040);
        let held = grey_at(&c, x, y);
        assert!(held < t.pal.card.r, "held from the lift: {held}");
        app.tick(1040 + QUICK_TAP_MS + 1000);
        assert!(app.row_press.is_none(), "and let go once faded");
        app.compose(&mut c, &f, &i, "12:00", 2200);
        assert_eq!(grey_at(&c, x, y), t.pal.card.r);
    }

    #[test]
    fn the_back_control_and_the_search_buttons_darken_at_once_under_a_finger() {
        let (f, i) = (fonts(), info());
        let t = Theme::light(W, H, PPD);
        let mut c = Canvas::new(W, H);

        // `<`: a disc behind it, the `<` lighter on it.
        let mut app = opened(Screen::Mount, &f, &i);
        let (cx, cy, r) = widgets::back_disc(&t);
        let (bx, by) = widgets::back_rect(&t).centre();
        let around = Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
        app.compose(&mut c, &f, &i, "12:00", 5000);
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), t.pal.bg.r);
        assert_eq!(darkest(&c, around), 0);
        app.touch(Touch::Down(bx, by), &f, 6000);
        app.compose(&mut c, &f, &i, "12:00", 6000);
        let disc = over(t.pal.button_press, t.pal.bg).r;
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), disc, "darker at once");
        let arrow = darkest(&c, around);
        assert!(arrow.abs_diff(85) <= 2, "the stock `<` held down is 85, not {arrow}");
        // A finger moving off lets it go.
        app.touch(Touch::Move(bx, by + t.m.dp(40.0)), &f, 6050);
        app.compose(&mut c, &f, &i, "12:00", 6050);
        assert_eq!(grey_at(&c, cx + r * 0.6, cy), t.pal.bg.r);
        app.touch(Touch::Up, &f, 6100);
        assert_eq!(app.screen(), Screen::Mount);

        // The search page's round button.
        let mut app = App::new(Screen::Search, W, H, PPD);
        app.frame(&f, &i);
        let (_, button) = widgets::search_layout(&t);
        let (x, y) = button.centre();
        app.touch(Touch::Down(x, y), &f, 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert_eq!(grey_at(&c, x, y + button.h * 0.35), over(t.pal.button_press, t.pal.search).r);
        let cross = darkest(&c, button);
        assert!(cross.abs_diff(70) <= 2, "the stock cross held down is 70, not {cross}");

        // The home page's search bar, clear of its words.
        let mut app = App::new(Screen::Home, W, H, PPD);
        let bar = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::Search)).expect("on the home page").rect;
        let (x, y) = (bar.x + bar.w * 0.8, bar.y + bar.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 1000);
        app.compose(&mut c, &f, &i, "12:00", 1000);
        assert_eq!(grey_at(&c, x, y), over(t.pal.button_press, t.pal.search).r);
    }

    #[test]
    fn swipe_from_the_left_edge_goes_back() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(90.0, 1605.0), &f, 0);
        app.touch(Touch::Move(320.0, 1610.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.screen(), Screen::Home);
    }

    #[test]
    fn a_short_edge_swipe_does_not_go_back() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.screen(), Screen::Reboot);
    }

    #[test]
    fn the_first_page_has_no_edge_swipe_back() {
        let f = fonts();
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 8);
        assert!(app.touch(Touch::Move(400.0, 1600.0), &f, 16).is_empty(), "no tap of the motor");
        assert!(!matches!(app.drag, Drag::Edge { .. }), "no cue");
        app.touch(Touch::Up, &f, 24);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(app.deadline(24), app.idle_due(), "nothing left on screen to settle");
    }

    #[test]
    fn the_back_cue_taps_once_as_it_arms_and_sinks_when_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        let ready = app.theme().m.dp(BACK_SWIPE_DP);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        assert!(app.touch(Touch::Move(80.0, 1600.0), &f, 8).is_empty());
        assert_eq!(app.touch(Touch::Move(20.0 + ready, 1600.0), &f, 16), vec![Effect::Haptic]);
        assert!(app.touch(Touch::Move(60.0 + ready, 1600.0), &f, 24).is_empty(), "one tap while armed");
        app.touch(Touch::Move(40.0, 1600.0), &f, 32);
        app.touch(Touch::Up, &f, 40);
        assert_eq!(app.screen(), Screen::Reboot, "pulled back in, it does not go back");
        assert_eq!(app.deadline(40), Some(0), "the cue is sinking back");
        app.tick(1000);
        assert_eq!(app.deadline(1000), app.idle_due());
    }

    #[test]
    fn the_back_cue_stays_quiet_with_touch_vibration_off() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        app.state.vibrate = false;
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1600.0), &f, 8);
        assert!(app.touch(Touch::Move(400.0, 1600.0), &f, 16).is_empty());
    }

    #[test]
    fn the_back_cue_keeps_to_the_height_the_swipe_began_at() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Reboot, &f, &i);
        let at = 1600.0 - app.theme().m.dp(CUE_LIFT_DP);
        app.touch(Touch::Down(10.0, 1600.0), &f, 0);
        app.touch(Touch::Move(80.0, 1610.0), &f, 8);
        app.touch(Touch::Move(200.0, 1300.0), &f, 16);
        app.touch(Touch::Move(150.0, 1900.0), &f, 24);
        let Drag::Edge { cy, .. } = app.drag else { panic!("still swiping in from the edge") };
        assert_eq!(cy, at, "the finger wandered up and down");
        app.touch(Touch::Move(40.0, 2100.0), &f, 32);
        app.touch(Touch::Up, &f, 40);
        assert_eq!(app.cue.as_ref().map(|c| c.1), Some(at), "it sinks back where it was");
    }
}
