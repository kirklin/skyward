//! The keys working the pages: the light they move from control to control.

use super::*;

impl App {
    /// Which pane currently receives hardware-key navigation.
    pub(super) fn focus_pane(&self) -> Pane {
        match &self.tablet {
            Some(tb) if self.split_active() => tb.key_pane,
            _ => Pane::Detail,
        }
    }

    pub(super) fn pane_frame(&self, pane: Pane) -> Option<&Frame> {
        match pane {
            Pane::Sidebar => self.tablet.as_ref().and_then(|tb| tb.frame.as_ref()),
            Pane::Detail => self.frame.as_ref(),
        }
    }

    fn pane_theme(&self, pane: Pane) -> Theme {
        match pane {
            Pane::Sidebar => self.sidebar_theme(),
            Pane::Detail => self.theme(),
        }
    }

    fn pane_max_scroll(&self, pane: Pane) -> f32 {
        match pane {
            Pane::Sidebar => self.tablet.as_ref().map_or(0.0, |tb| tb.max_scroll(self.h)),
            Pane::Detail => self.max_scroll(),
        }
    }

    fn pane_resting(&self, pane: Pane) -> f32 {
        match pane {
            Pane::Sidebar => self.tablet.as_ref().map_or(0.0, |tb| tb.resting()),
            Pane::Detail => self.resting(),
        }
    }

    fn pane_seek_to(&mut self, pane: Pane, target: f32, now: u64) {
        match pane {
            Pane::Sidebar => {
                if let Some(tb) = &mut self.tablet
                    && tb.seek_to(target, now)
                {
                    self.dirty = true;
                }
            }
            Pane::Detail => self.seek_to(target, now),
        }
    }

    fn pane_focus(&self, pane: Pane) -> Focus {
        match pane {
            Pane::Sidebar => self.stack[0].2,
            Pane::Detail => self.top().2,
        }
    }

    fn pane_focus_mut(&mut self, pane: Pane) -> &mut Focus {
        match pane {
            Pane::Sidebar => &mut self.stack[0].2,
            Pane::Detail => &mut self.top_mut().2,
        }
    }

    fn set_focus_pane(&mut self, pane: Pane) {
        if let Some(tb) = &mut self.tablet {
            tb.key_pane = pane;
        }
    }

    fn cached_spots(&self, pane: Pane) -> Option<&[Spot]> {
        match pane {
            Pane::Sidebar => self.tablet.as_ref().and_then(|tb| tb.spots.as_deref()),
            Pane::Detail => self.spots.as_deref(),
        }
    }

    fn pane_spots(&mut self, pane: Pane) -> Option<&[Spot]> {
        match pane {
            Pane::Sidebar => {
                let t = self.sidebar_theme();
                let tb = self.tablet.as_mut()?;
                if tb.spots.is_none() {
                    tb.spots = Some(focus::spots(&t, tb.frame.as_ref()?));
                }
                tb.spots.as_deref()
            }
            Pane::Detail => {
                if self.spots.is_none() {
                    let t = self.theme();
                    self.spots = Some(focus::spots(&t, self.frame.as_ref()?));
                }
                self.spots.as_deref()
            }
        }
    }

    /// Work the control the keys are on in `pane`, as touching it does. The
    /// brightness slider takes the volume keys until the power key lets it
    /// go, and a pattern's dots are joined one press at a time, a second
    /// press on the last one joined ending the pattern.
    pub(super) fn work(&mut self, pane: Pane, spot: Spot, now: u64) -> Vec<Effect> {
        if pane == Pane::Sidebar {
            if let Stop::Hit(Action::Open(target)) = spot.stop {
                let fx = self.open_from_sidebar(target, now);
                if self.screen() == target {
                    self.set_focus_pane(Pane::Detail);
                }
                return fx;
            }
            return Vec::new();
        }
        let l = self.state.lang;
        match spot.stop {
            Stop::Back => self.act(Action::Back, now),
            Stop::Button(action) => self.act(action, now),
            Stop::Hit(Action::Slider(which)) => {
                self.adjusting.toggle();
                if self.adjusting {
                    let said = match which {
                        Slider::Brightness => texts::toast::BRIGHTNESS_KEYS,
                        Slider::Vibration => texts::toast::VIBRATION_KEYS,
                    };
                    self.say(l.pick(said), now);
                }
                Vec::new()
            }
            Stop::Hit(action) => self.act(action, now),
            Stop::Dot(i) if self.state.pattern.last() == Some(&i) => self.drawn(now),
            Stop::Dot(i) => {
                if self.state.pattern.is_empty() {
                    self.say(l.pick(texts::toast::PATTERN_KEYS), now);
                }
                self.join(&self.theme(), i);
                Vec::new()
            }
        }
    }

    /// A pattern drawn to its end: four dots or more are sent to be
    /// checked, read as the number of each dot counted from one across the
    /// rows; fewer are wiped and asked for again.
    pub(super) fn drawn(&mut self, now: u64) -> Vec<Effect> {
        let drawn = std::mem::take(&mut self.state.pattern);
        self.repaint_pattern(&self.theme());
        if drawn.len() < 4 {
            self.say(self.state.lang.pick(texts::toast::PATTERN_SHORT), now);
            return Vec::new();
        }
        self.unlock(drawn.iter().map(|i| (b'1' + *i as u8) as char).collect())
    }

    /// A volume key: the first lights the control the keys were last on
    /// if it shows, or else the first in view; each after moves the light
    /// down the page or up it, or moves a slider that takes the keys, the
    /// strength slider tapping the motor as strongly as it was moved to.
    pub(super) fn volume(&mut self, down: bool, now: u64) -> Vec<Effect> {
        let pane = self.focus_pane();
        if self.pane_frame(pane).is_none() {
            return Vec::new();
        }
        if self.adjusting {
            if let Some(Spot { stop: Stop::Hit(Action::Slider(which)), rect, .. }) = self.lit_on(Pane::Detail) {
                let level = self.level(which) + if down { -SLIDER_STEP } else { SLIDER_STEP };
                self.set_level(&self.theme(), which, rect, level);
                if which == Slider::Vibration && self.state.vibrate {
                    return vec![Effect::Haptic];
                }
            }
            return Vec::new();
        }
        if self.keys {
            self.step(pane, down, now);
            return Vec::new();
        }
        self.keys = true;
        self.dirty = true;
        let scroll = self.pane_resting(pane);
        let shows = self.lit_on(pane).is_some_and(|spot| self.shows_on(pane, &spot, scroll));
        if !shows {
            *self.pane_focus_mut(pane) = Focus::Unset;
            if self.lit_on(pane).is_none()
                && self.split_active()
                && self.stack.len() == 2
                && pane == Pane::Detail
                && self.pane_spots(Pane::Detail).is_some_and(<[Spot]>::is_empty)
            {
                self.set_focus_pane(Pane::Sidebar);
                self.lit_on(Pane::Sidebar);
            }
        }
        Vec::new()
    }

    /// Move the light to the next control down `pane`, or up it. A control
    /// far off is scrolled to half a screen at a time, the light staying
    /// on its control while that shows; past the last control the light
    /// crosses to the sibling pane at root split level or goes round to the
    /// first, and past the first to the last.
    fn step(&mut self, pane: Pane, down: bool, now: u64) {
        let scroll = self.pane_resting(pane);
        let current = self.lit_on(pane);
        if self.pane_spots(pane).is_none() {
            return;
        }
        let spots = self.cached_spots(pane).expect("made above").to_vec();
        let cur = current.and_then(|c| focus::find(&spots, &c));
        let (top, bottom) = self.band_on(pane, scroll);
        let (max, n) = (self.pane_max_scroll(pane), spots.len());
        let page = |k: usize| spots[k].group == Group::Page;
        let held = |k: usize| matches!(spots[k].group, Group::Fixed | Group::Strip);
        let docked = |k: usize| spots[k].group == Group::Dock;
        // The page's own controls not wholly above the band, or below it.
        let from_top = |k: usize| page(k) && self.place_on(pane, &spots[k], scroll).is_some_and(|r| r.y + r.h > top);
        let from_bottom = |k: usize| page(k) && self.place_on(pane, &spots[k], scroll).is_some_and(|r| r.y < bottom);
        // A page padded only for its search bar's fold holds no unseen content
        // past its last control, so stepping past it wraps or crosses at once.
        let fold_only = self.pane_frame(pane).and_then(|fr| fr.fold).is_some_and(|f| max <= f.span.ceil());
        enum To {
            Spot(usize),
            Scroll,
            Round,
        }
        let to = if down {
            // Past the page's own: the rest of the page, then the bottom
            // strip, then round.
            let past = || {
                if scroll < max && !fold_only {
                    To::Scroll
                } else {
                    (0..n).find(|&k| docked(k)).map_or(To::Round, To::Spot)
                }
            };
            match cur {
                Some(i) if page(i) => (i + 1..n).find(|&k| page(k)).map_or_else(past, To::Spot),
                Some(i) if docked(i) => (i + 1..n).find(|&k| docked(k)).map_or(if scroll < max { To::Scroll } else { To::Round }, To::Spot),
                Some(i) => (i + 1..n).find(|&k| held(k)).or_else(|| (0..n).find(|&k| from_top(k))).map_or_else(past, To::Spot),
                None => (0..n).find(|&k| from_top(k)).map_or_else(past, To::Spot),
            }
        } else {
            // Before the page's own: the rest of the page, then the `<` and
            // the strip at the top, then round.
            let before = || {
                if scroll > 0.0 {
                    To::Scroll
                } else {
                    (0..n).rev().find(|&k| held(k)).map_or(To::Round, To::Spot)
                }
            };
            match cur {
                Some(i) if page(i) => (0..i).rev().find(|&k| page(k)).map_or_else(before, To::Spot),
                Some(i) if docked(i) => (0..i).rev().find(|&k| docked(k)).or_else(|| (0..n).rev().find(|&k| from_bottom(k))).map_or_else(before, To::Spot),
                Some(i) => (0..i).rev().find(|&k| held(k)).map_or(if scroll > 0.0 { To::Scroll } else { To::Round }, To::Spot),
                None => (0..n).rev().find(|&k| from_bottom(k)).map_or_else(before, To::Spot),
            }
        };
        match to {
            To::Spot(k) => {
                let s = spots[k];
                if s.group == Group::Page {
                    let target = self.reveal_on(pane, &s, scroll);
                    if (target - scroll).abs() > (bottom - top) / 2.0 {
                        return self.scroll_on(pane, down, &spots, cur, now);
                    }
                    self.pane_seek_to(pane, target, now);
                }
                *self.pane_focus_mut(pane) = Focus::On(s);
            }
            To::Scroll => return self.scroll_on(pane, down, &spots, cur, now),
            To::Round => {
                let sibling = match pane {
                    Pane::Sidebar => Pane::Detail,
                    Pane::Detail => Pane::Sidebar,
                };
                let crossed = self.split_active() && self.stack.len() == 2 && self.pane_spots(sibling).is_some_and(|sp| !sp.is_empty());
                let (next_pane, next_spots) = if crossed {
                    let sp = self.cached_spots(sibling).expect("checked above").to_vec();
                    self.set_focus_pane(sibling);
                    (sibling, sp)
                } else {
                    if n == 0 {
                        return;
                    }
                    (pane, spots)
                };
                // Going round or crossing panes, go to the top or end, as far as
                // its control there shows.
                let s = next_spots[if down { 0 } else { next_spots.len() - 1 }];
                let end = if down { 0.0 } else { self.pane_max_scroll(next_pane) };
                let target = if self.shows_on(next_pane, &s, end) { end } else { self.reveal_on(next_pane, &s, end) };
                self.pane_seek_to(next_pane, target, now);
                *self.pane_focus_mut(next_pane) = Focus::On(s);
            }
        }
        self.dirty = true;
    }

    /// Scroll `pane` half a screen down or up for the keys. The light stays
    /// on its control while that shows, and goes to the first in view the
    /// way the pane went when it does not.
    fn scroll_on(&mut self, pane: Pane, down: bool, spots: &[Spot], cur: Option<usize>, now: u64) {
        let scroll = self.pane_resting(pane);
        let (top, bottom) = self.band_on(pane, scroll);
        let half = (bottom - top) / 2.0;
        let max = self.pane_max_scroll(pane);
        let target = if down { (scroll + half).min(max) } else { (scroll - half).max(0.0) };
        // A page coming to rest with its search bar part folded folds it
        // whole or not at all, as one let go there does.
        let target = self.pane_frame(pane).map_or(target, |fr| fr.settle(target).min(max));
        self.pane_seek_to(pane, target, now);
        if cur.is_some_and(|i| self.shows_on(pane, &spots[i], target)) {
            return;
        }
        let band = self.band_on(pane, target);
        let in_view = |k: &usize| spots[*k].group == Group::Page && self.place_on(pane, &spots[*k], target).is_some_and(|r| within(r, band));
        let next = if down { (0..spots.len()).find(in_view) } else { (0..spots.len()).rev().find(in_view) };
        *self.pane_focus_mut(pane) = next.map_or(Focus::Off, |k| Focus::On(spots[k]));
    }

    /// The page's controls in the order the keys go through them, once the
    /// page is drawn.
    #[cfg(test)]
    pub(super) fn spots(&mut self) -> Option<&[Spot]> {
        self.pane_spots(Pane::Detail)
    }

    /// The control the keys are on in `pane`, once drawn: the one they were
    /// on, found again, or the first in view when they have not been
    /// anywhere on `pane` yet or theirs has gone from it.
    pub(super) fn lit_on(&mut self, pane: Pane) -> Option<Spot> {
        let scroll = self.pane_resting(pane);
        self.pane_spots(pane)?;
        let spots = self.cached_spots(pane).expect("made above");
        let found = match self.pane_focus(pane) {
            Focus::Off => return None,
            Focus::On(at) => focus::find(spots, &at),
            Focus::Unset => None,
        };
        let spot = spots[found.or_else(|| self.first_in_view_on(pane, spots, scroll))?];
        *self.pane_focus_mut(pane) = Focus::On(spot);
        Some(spot)
    }

    /// The control the keys are currently on, in the active focus pane.
    #[cfg(test)]
    pub(super) fn lit(&mut self) -> Option<Spot> {
        self.lit_on(self.focus_pane())
    }

    /// The first of `pane`'s own controls in view at `scroll`, or else
    /// the bottom strip's first, or else the `<` or the top strip's first.
    fn first_in_view_on(&self, pane: Pane, spots: &[Spot], scroll: f32) -> Option<usize> {
        let band = self.band_on(pane, scroll);
        let k = spots.iter().position(|s| s.group == Group::Page && self.place_on(pane, s, scroll).is_some_and(|r| within(r, band)));
        k.or_else(|| spots.iter().position(|s| s.group == Group::Dock))
            .or_else(|| spots.iter().position(|s| matches!(s.group, Group::Fixed | Group::Strip)))
    }

    /// Where on screen `s` in `pane` is with `pane` `offset` pixels down;
    /// nowhere for a search bar folded away.
    fn place_on(&self, pane: Pane, s: &Spot, offset: f32) -> Option<Rect> {
        let fr = self.pane_frame(pane).expect("a control is placed on the pane drawn");
        let dy = match s.group {
            Group::Fixed => 0.0,
            Group::Strip => pinned_top(&self.pane_theme(pane), fr.pinned.as_ref().expect("the strip holds its controls"), offset),
            Group::Page => return fr.shown(s.rect, offset),
            Group::Dock => self.h as f32 - fr.dock.as_ref().expect("the dock holds its controls").canvas.h as f32,
        };
        Some(Rect { y: s.rect.y + dy, ..s.rect })
    }

    /// The band of the screen `pane`'s own controls show in with `pane` at
    /// `scroll`: under the bar, or under the strip held there, down to the
    /// strip held at the bottom.
    fn band_on(&self, pane: Pane, scroll: f32) -> (f32, f32) {
        let t = self.pane_theme(pane);
        let fr = self.pane_frame(pane).expect("the band is the pane drawn's");
        let top = match &fr.pinned {
            Some(p) => pinned_top(&t, p, scroll) + p.canvas.h as f32,
            None => widgets::bar_bottom(&t),
        };
        (top, self.h as f32 - fr.dock.as_ref().map_or(0.0, |d| d.canvas.h as f32))
    }

    #[cfg(test)]
    fn band(&self, scroll: f32) -> (f32, f32) {
        self.band_on(Pane::Detail, scroll)
    }

    /// Whether `s` in `pane` shows, in part at least, at `scroll`.
    pub(super) fn shows_on(&self, pane: Pane, s: &Spot, scroll: f32) -> bool {
        s.group != Group::Page || self.place_on(pane, s, scroll).is_some_and(|r| within(r, self.band_on(pane, scroll)))
    }

    #[cfg(test)]
    pub(super) fn shows(&self, s: &Spot, scroll: f32) -> bool {
        self.shows_on(Pane::Detail, s, scroll)
    }

    /// How far down to scroll `pane` from `scroll` to show `s` whole, a
    /// margin clear of the band's edges, or as much of it from its top as
    /// fits. A search bar under the large title shows only unfolded, at
    /// the page's top, and a page scrolled down to show any other control
    /// goes on to fold the bar away whole.
    fn reveal_on(&self, pane: Pane, s: &Spot, scroll: f32) -> f32 {
        let fr = self.pane_frame(pane).expect("a control is revealed on the pane drawn");
        let fold = fr.fold.filter(|_| s.group == Group::Page);
        if fold.is_some_and(|f| f.bar == s.rect) {
            return 0.0;
        }
        let margin = self.pane_theme(pane).m.margin;
        let (top, bottom) = self.band_on(pane, scroll);
        let (top, bottom) = (top + margin, bottom - margin);
        let r = self.place_on(pane, s, scroll).expect("only a search bar folded away has no place");
        let d = if r.y < top {
            r.y - top
        } else if r.y + r.h > bottom {
            (r.y + r.h - bottom).min(r.y - top)
        } else {
            0.0
        };
        let target = scroll + d;
        let target = fold.map_or(target, |f| if target > 0.0 && target < f.span { f.span } else { target });
        target.clamp(0.0, self.pane_max_scroll(pane))
    }

    /// The light on the control the keys are on while they work `pane`:
    /// the part of the screen its control is drawn in, where the light
    /// falls with the pane `offset` pixels down, how round it is, and its
    /// tint, stronger while the power key is held down on it.
    pub(super) fn light_on(&mut self, pane: Pane, offset: f32) -> Option<(Group, Rect, f32, Rgba)> {
        if !self.keys || self.focus_pane() != pane {
            return None;
        }
        let spot = self.lit_on(pane)?;
        let t = self.pane_theme(pane);
        let keypad = pane == Pane::Detail
            && self.screen() == Screen::Decrypt
            && matches!(self.state.data, Data::Locked { keys: Keys::Digits, .. });
        let (r, radius) = focus::lit(&t, spot.stop, self.place_on(pane, &spot, offset)?, keypad);
        let held = matches!(self.button, Some((_, Press::Power { .. } | Press::Slide { .. })));
        let alpha = if held || self.adjusting { LIGHT_HELD_ALPHA } else { LIGHT_ALPHA };
        Some((spot.group, r, radius, t.pal.accent.with_alpha(alpha)))
    }

    pub(super) fn light(&mut self, offset: f32) -> Option<(Group, Rect, f32, Rgba)> {
        self.light_on(Pane::Detail, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn the_keys_unfold_the_search_bar_to_light_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        let span = app.frame(&f, &i).fold.expect("the home page's search bar folds").span;
        scroll_to(&mut app, &f, span, 0);
        app.tick(3000);
        press(&mut app, Key::VolumeDown, 3000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))), "a bar folded away is not in view");
        press(&mut app, Key::VolumeUp, 3000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        for ms in (3000..5000).step_by(8) {
            app.tick(ms);
        }
        assert_eq!(app.top().1, 0.0, "the page went back up to unfold it");
        press(&mut app, Key::Power, 5000);
        assert_eq!(app.screen(), Screen::Search);
    }

    #[test]
    fn the_keys_reach_the_last_row_of_a_home_page_grown_by_an_update_notice() {
        let f = fonts();
        let i = DeviceInfo { snapshot: SnapshotState::Snapshotted, ..info() };
        let mut app = App::new(Screen::Home, W, SNUG, PPD);
        let (_, y) = spot(&mut app, &f, &i, Action::Open(Screen::Reboot));
        assert!(y > SNUG as f32, "below the screen to start with");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Reboot)));
        for ms in (0..3000).step_by(8) {
            app.tick(ms);
        }
        let (_, y) = spot(&mut app, &f, &i, Action::Open(Screen::Reboot));
        assert!(y < SNUG as f32, "the page scrolled the lit row into view");
        press(&mut app, Key::Power, 3000);
        assert_eq!(app.screen(), Screen::Reboot);
    }

    #[test]
    fn without_a_touchscreen_the_first_control_is_lit_at_once() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert_eq!(app.light(0.0), None, "a page opens to the touch");
        app.touchscreen_missing();
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.light(0.0).is_some(), "lit before any key is pressed");
        assert!(app.take_dirty(), "and shown");
    }

    #[test]
    fn the_volume_keys_light_the_controls_down_the_page_and_up_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        assert!(!app.keys, "a page opens to the touch");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "the first press lights the first control in view");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))));
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Reboot))), "up from the first goes round to the last");
        app.tick(5000);
        let reboot = app.lit().expect("lit");
        assert!(app.shows(&reboot, app.top().1), "scrolled into view");
        press(&mut app, Key::VolumeDown, 6000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "down from the last goes round to the first");
        app.tick(10_000);
        assert_eq!(app.top().1, 0.0, "at the top of the page again");
    }

    #[test]
    fn the_power_key_works_the_control_lit_as_it_comes_up() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Settings)));
        assert!(app.key(Key::Power, 0).is_empty());
        assert_eq!(app.screen(), Screen::Home, "nothing happens as the key goes down");
        app.key_up(Key::Power, 0);
        assert_eq!(app.screen(), Screen::Settings, "the row opens its page as the key comes up");
        let first = lit(&mut app, &f, &i);
        assert!(first.is_some() && first != Some(Stop::Back), "the page's first control is lit, not the `<`: {first:?}");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Settings))), "back on the page, the keys are where they left it");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::Settings)));
        press(&mut app, Key::Power, 0);
        assert_eq!(lit(&mut app, &f, &i), first);
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Back), "up from the page's first control comes to the `<`");
        press(&mut app, Key::Power, 0);
        assert_eq!(app.screen(), Screen::Home, "the `<` goes back");
    }

    #[test]
    fn past_the_last_control_the_keys_scroll_on_half_a_screen_a_press() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Installing, W, H, PPD);
        app.state.install = Install::Finished { outcome: Outcome::Ran { wipe_cache: false }, damage: None };
        app.state.script_output = (0..300).map(|k| format!("- line {k}")).collect();
        app.frame(&f, &i);
        let (max, (top, bottom)) = (app.max_scroll(), app.band(0.0));
        let half = (bottom - top) / 2.0;
        assert!(max > half * 4.0, "the script's output runs on below the controls: {max}");
        let last = *app.spots().expect("drawn").last().expect("controls on the page");
        light_on(&mut app, &f, &i, last.stop);
        let mut at = app.resting();
        let mut presses = 0;
        while at < max {
            press(&mut app, Key::VolumeDown, 0);
            assert_eq!(app.resting(), (at + half).min(max), "half a screen a press");
            at = app.resting();
            presses += 1;
            assert!(presses < 40);
            if !app.shows(&last, at) {
                assert_eq!(lit(&mut app, &f, &i), None, "with no control in view, nothing is lit");
            }
        }
        press(&mut app, Key::VolumeDown, 0);
        let first = app.spots().expect("drawn")[0].stop;
        assert_eq!((lit(&mut app, &f, &i), app.resting()), (Some(first), 0.0), "past the end the light goes round to the top");
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(last.stop), "and up from it round to the last control");
        assert!(app.shows(&last, app.resting()), "which shows");
        let resting = app.resting();
        app.tick(5000);
        assert_eq!(app.top().1, resting, "where the page eased to");
    }

    #[test]
    fn the_power_key_held_slides_a_swipe_control_and_confirms_it_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Reboot, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Confirm(Confirm::Reboot)));
        assert!(press(&mut app, Key::Power, 0).is_empty());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("请按住电源键"), "a short press says how");
        app.key(Key::Power, 1000);
        assert_eq!(app.deadline(1000), Some(1000 + CHORD_MS), "it waits for volume down first");
        app.tick(1000 + CHORD_MS);
        assert_eq!(app.deadline(1000 + CHORD_MS), Some(0), "then the knob moves while the key is held");
        app.tick(1000 + SLIDE_MS / 2);
        assert!((app.state.confirm - 0.5).abs() < 0.01, "half way after half the time: {}", app.state.confirm);
        assert!(app.key_up(Key::Power, 1000 + SLIDE_MS / 2).is_empty(), "let go short of the end, nothing happens");
        assert_eq!(app.state.confirm, 0.0, "and the knob goes back");
        app.key(Key::Power, 3000);
        app.tick(3000 + SLIDE_MS * 2);
        assert_eq!(app.state.confirm, 1.0, "the knob stops at the end");
        assert_eq!(app.key_up(Key::Power, 3000 + SLIDE_MS * 2), vec![Effect::Reboot(TARGETS[0])], "let go there, it confirms");
        assert_eq!(app.state.confirm, 0.0);
        app.key(Key::Power, 6000);
        app.tick(6000 + SLIDE_MS);
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 6000 + SLIDE_MS);
        assert_eq!(app.state.confirm, 0.0, "a touch lets the knob go");
        assert!(app.key_up(Key::Power, 6000 + SLIDE_MS).is_empty(), "and the key comes up to nothing");
    }

    #[test]
    fn the_brightness_slider_takes_the_volume_keys_until_the_power_key_lets_it_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Slider(Slider::Brightness)));
        assert_eq!(app.state.brightness, SLIDER_START, "it starts a step from its left end");
        press(&mut app, Key::Power, 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("按音量键调节亮度，按电源键完成"));
        assert!(press(&mut app, Key::VolumeUp, 0).is_empty(), "the panel shows the brightness");
        assert!((app.state.brightness - (SLIDER_START + SLIDER_STEP)).abs() < 1e-5, "up turns it up");
        press(&mut app, Key::VolumeDown, 0);
        assert!((app.state.brightness - SLIDER_START).abs() < 1e-5, "down turns it down");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(app.state.brightness, MIN_BRIGHTNESS, "no lower than the slider's floor");
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Slider(Slider::Brightness))), "the light stays on the slider");
        press(&mut app, Key::Power, 0);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(app.state.brightness, MIN_BRIGHTNESS, "let go, the slider keeps its level");
        assert_ne!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Slider(Slider::Brightness))), "and the keys move the light again");
    }

    #[test]
    fn the_strength_slider_taps_the_motor_at_each_step_of_the_keys() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Slider(Slider::Vibration)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("按音量键调节振动强度，按电源键完成"));
        assert_eq!(press(&mut app, Key::VolumeDown, 0), vec![Effect::Haptic], "each step taps at the strength reached");
        assert_eq!(app.state.vibration, 0.0, "it starts a step from its left end, the lightest");
        for _ in 0..20 {
            press(&mut app, Key::VolumeUp, 0);
        }
        assert_eq!(app.state.vibration, 1.0, "up to its right end, the strongest");
        press(&mut app, Key::Power, 0);
        assert!(press(&mut app, Key::VolumeDown, 0).is_empty(), "let go, the keys move the light");
        assert_eq!(app.state.vibration, 1.0);
    }

    #[test]
    fn the_keys_join_a_patterns_dots_and_end_it_on_the_last_one_joined() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Pattern)));
        app.frame(&f, &i);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(0)), "the grid's first dot is lit first");
        assert!(press(&mut app, Key::Power, 0).is_empty());
        assert_eq!(app.state.pattern, [0]);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("连接完成后，在最后一个点上再按一次电源键"));
        press(&mut app, Key::Power, 0);
        assert!(app.state.pattern.is_empty(), "ended at one dot, the pattern is wiped");
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("请至少连接 4 个点"));
        press(&mut app, Key::Power, 0);
        press(&mut app, Key::VolumeDown, 0);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(2)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2], "the dot passed straight over is joined on the way");
        for _ in 0..6 {
            press(&mut app, Key::VolumeDown, 0);
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Dot(8)));
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2, 5, 8]);
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::VolumeUp, 0);
        press(&mut app, Key::Power, 0);
        assert_eq!(app.state.pattern, [0, 1, 2, 5, 8, 7, 6], "two along the row joins the one between first");
        press(&mut app, Key::VolumeDown, 0);
        assert!(press(&mut app, Key::Power, 0).is_empty(), "a dot joined already is not joined again");
        press(&mut app, Key::VolumeUp, 0);
        assert_eq!(press(&mut app, Key::Power, 0), vec![Effect::Unlock { user: 0, code: "1236987".into() }], "the last one joined ends it");
    }

    #[test]
    fn the_keys_type_a_screen_locks_code_on_its_keypad() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, W, H, PPD);
        app.data_waiting(Some((0, Keys::Digits)));
        app.frame(&f, &i);
        for ch in ['2', '5', '8', '0'] {
            light_on(&mut app, &f, &i, Stop::Hit(Action::Type(Cap::Char(ch))));
            press(&mut app, Key::Power, 0);
        }
        assert_eq!(app.state.code, "2580");
        light_on(&mut app, &f, &i, Stop::Hit(Action::Type(Cap::Enter)));
        assert_eq!(press(&mut app, Key::Power, 0), vec![Effect::Unlock { user: 0, code: "2580".into() }]);
    }

    #[test]
    fn the_power_key_held_on_an_entry_opens_what_can_be_done_with_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        app.set_files("/".into(), Ok(vec![entry("vendor", folder(12)), entry("init.rc", FileKind::File)]), LISTED);
        app.frame(&f, &i);
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))));
        assert!(app.key(Key::Power, 1000).is_empty());
        assert_eq!(app.deadline(1000), Some(1000 + CHORD_MS), "it waits for volume down first");
        assert!(app.tick(1000 + CHORD_MS).is_empty());
        assert_eq!(app.deadline(1000 + CHORD_MS), Some(1000 + LONG_PRESS_MS), "the press is timed from when it went down");
        assert!(app.tick(1000 + LONG_PRESS_MS - 1).is_empty());
        assert_eq!(app.tick(1000 + LONG_PRESS_MS), vec![Effect::Haptic], "held long enough, the folder's page opens");
        assert_eq!(app.screen(), Screen::FileInfo);
        assert_eq!(app.state.file.as_ref().map(|e| e.name.as_str()), Some("vendor"));
        assert!(app.key_up(Key::Power, 1000 + LONG_PRESS_MS + 100).is_empty(), "and the key comes up to nothing");
        app.key(Key::Back, 2000);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::OpenEntry(0))));
        assert_eq!(press(&mut app, Key::Power, 3000), vec![Effect::ReadDir("/vendor".into())], "a short press goes into the folder");
    }

    #[test]
    fn a_touch_puts_the_light_out_and_a_volume_key_brings_it_back() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        light_on(&mut app, &f, &i, Stop::Hit(Action::Open(Screen::About)));
        let mut c = Canvas::new(W, H);
        app.compose(&mut c, &f, &i, "12:00", 0);
        assert!(app.light(0.0).is_some(), "lit while the keys work the page");
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert!(app.light(0.0).is_none(), "a touch puts the light out");
        press(&mut app, Key::VolumeDown, 0);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))), "the keys come back where they were");
        app.touch(Touch::Down(W as f32 / 2.0, 200.0), &f, 0);
        app.touch(Touch::Up, &f, 0);
        press(&mut app, Key::Power, 0);
        assert!(!app.display_on(), "with the light out, power turns the panel off");
        press(&mut app, Key::VolumeDown, 1000);
        assert!(!app.display_on(), "a volume key leaves the panel dark");
        press(&mut app, Key::Power, 1100);
        assert!(app.display_on(), "the power key lights it");
        assert!(!app.keys, "and does only that");
    }

    #[test]
    fn a_volume_key_held_moves_the_light_on_and_on() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.frame(&f, &i);
        app.key(Key::VolumeDown, 0);
        assert_eq!(app.deadline(0), Some(CHORD_MS), "it waits for the power key first");
        app.tick(CHORD_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert_eq!(app.deadline(CHORD_MS), Some(REPEAT_DELAY_MS), "and repeats as from when it went down");
        app.tick(REPEAT_DELAY_MS - 1);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))), "not held long enough yet");
        app.tick(REPEAT_DELAY_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::About))));
        app.tick(REPEAT_DELAY_MS + REPEAT_MS);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))));
        app.key_up(Key::VolumeDown, REPEAT_DELAY_MS + REPEAT_MS + 10);
        app.tick(REPEAT_DELAY_MS + REPEAT_MS * 5);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Install))), "let go, it stops");
    }
}
