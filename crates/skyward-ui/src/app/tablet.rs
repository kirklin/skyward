//! Tablet split-pane support: left navigation sidebar (`Home`) paired with a
//! right detail pane, keeping phones and standalone full-screen pages unchanged.

use super::*;

/// Which sidebar root entry owns `screen`, for highlighting the active item.
pub(super) fn sidebar_section(screen: Screen, restore_job: bool) -> Option<Screen> {
    match screen {
        Screen::About | Screen::Specs | Screen::License | Screen::FontLicense => Some(Screen::About),
        Screen::Update => Some(Screen::Update),
        Screen::Slot => Some(Screen::Slot),
        Screen::Systems => Some(Screen::Systems),
        Screen::Wlan | Screen::WlanPassword | Screen::WlanNetwork | Screen::WlanAdd => Some(Screen::Wlan),
        Screen::Install | Screen::Choose(_) | Screen::Installing => Some(Screen::Install),
        Screen::Backup => Some(Screen::Backup),
        Screen::BackupJob => Some(if restore_job { Screen::Restore } else { Screen::Backup }),
        Screen::Restore | Screen::RestoreBackup => Some(Screen::Restore),
        Screen::Mount => Some(Screen::Mount),
        Screen::Decrypt => Some(Screen::Decrypt),
        Screen::Wipe | Screen::Erase => Some(Screen::Wipe),
        Screen::Advanced
        | Screen::Files
        | Screen::FileInfo
        | Screen::Permissions
        | Screen::Editor
        | Screen::Rename
        | Screen::Clash
        | Screen::FileJob
        | Screen::Terminal
        | Screen::Relabel
        | Screen::Filesystems
        | Screen::Filesystem
        | Screen::PartWork
        | Screen::Logs => Some(Screen::Advanced),
        Screen::Settings
        | Screen::ScreenOff
        | Screen::Language
        | Screen::TimeZone
        | Screen::TimeZones(_)
        | Screen::SkywardUpdate => Some(Screen::Settings),
        Screen::Reboot => Some(Screen::Reboot),
        Screen::Home | Screen::Search | Screen::Fastboot | Screen::CantStart | Screen::Unhandled => None,
    }
}

pub(super) enum SidebarDrag {
    Idle,
    Press { x: f32, y: f32, action: Option<Action>, edge: bool, at: u64 },
    Scroll { last: f32 },
}

pub(super) struct Tablet {
    pub(super) sidebar_w: u32,
    pub(super) detail_w: u32,
    pub(super) frame: Option<Frame>,
    pub(super) spots: Option<Vec<Spot>>,
    pub(super) key_pane: Pane,
    pub(super) canvas: Canvas,
    pub(super) detail_canvas: Canvas,
    pub(super) active: Option<Screen>,
    pub(super) before_search: Option<Screen>,
    pub(super) scroll: f32,
    pub(super) height: f32,
    pub(super) pull: f32,
    pub(super) bounce: Option<Spring>,
    pub(super) fling: Option<Fling>,
    pub(super) seek: Option<(f32, Spring)>,
    pub(super) title: Fade,
    pub(super) bar: Fade,
    pub(super) snap: bool,
    pub(super) titles: Option<((&'static str, bool), Blurs, Blurs)>,
    pub(super) search_words: Option<((&'static str, bool), Canvas)>,
    pub(super) row_press: Option<RowPress>,
    pub(super) row_art: Option<(Canvas, Canvas)>,
    pub(super) drag: SidebarDrag,
    pub(super) tracker: Tracker,
    pub(super) touch_sidebar: bool,
    pub(super) charge_dither: Option<glow::Dither>,
    pub(super) split_turn: Option<Turn>,
    pub(super) split_behind: Option<Canvas>,
}

impl Tablet {
    pub(super) fn new(w: u32, h: u32, px_per_dp: f32) -> Tablet {
        let sidebar_w = crate::theme::sidebar_width(w, px_per_dp);
        let detail_w = w - sidebar_w;
        Tablet {
            sidebar_w,
            detail_w,
            frame: None,
            spots: None,
            key_pane: Pane::Sidebar,
            canvas: Canvas::new(sidebar_w, h),
            detail_canvas: Canvas::new(detail_w, h),
            active: Some(Screen::About),
            before_search: None,
            scroll: 0.0,
            height: 0.0,
            pull: 0.0,
            bounce: None,
            fling: None,
            seek: None,
            title: title_fade(false),
            bar: bar_fade(false),
            snap: true,
            titles: None,
            search_words: None,
            row_press: None,
            row_art: None,
            drag: SidebarDrag::Idle,
            tracker: Tracker::default(),
            touch_sidebar: false,
            charge_dither: None,
            split_turn: None,
            split_behind: None,
        }
    }

    pub(super) fn max_scroll(&self, h: u32) -> f32 {
        (self.height - h as f32).max(0.0)
    }

    pub(super) fn resting(&self) -> f32 {
        self.seek.map_or(self.scroll, |(target, _)| target)
    }

    pub(super) fn seek_to(&mut self, target: f32, now: u64) -> bool {
        let at = self.scroll;
        self.fling = None;
        let next = (target != at).then(|| (target, Spring::new(now, at - target, 0.0, SEEK_PERIOD)));
        let changed = next.is_some() || self.seek.is_some();
        self.seek = next;
        changed
    }

    pub(super) fn invalidate_frame(&mut self) {
        self.frame = None;
        self.spots = None;
        self.row_art = None;
    }

    pub(super) fn stop_motion(&mut self) {
        self.drag = SidebarDrag::Idle;
        self.fling = None;
        self.bounce = None;
        self.seek = None;
        self.pull = 0.0;
        self.row_press = None;
        self.row_art = None;
        self.split_turn = None;
        self.split_behind = None;
    }

    pub(super) fn compose_split(&mut self, out: &mut Canvas, detail: &Canvas, corners: f32, now: u64) {
        if let Some(turn) = self.split_turn.as_mut() {
            let spring = *turn.spring.get_or_insert_with(|| Spring::new(now, -1000.0, 0.0, BACK_PERIOD));
            let (x, done) = spring.at(now);
            if !done {
                let (w, h) = (out.w, out.h);
                let mut behind = self.split_behind.take().filter(|c| c.w == w && c.h == h).unwrap_or_else(|| Canvas::new(w, h));
                behind.blit_at(&self.canvas, 0, 0);
                behind.blit_at(detail, self.sidebar_w, 0);
                let along = 1.0 + x / 1000.0;
                let wf = w as f32;
                let (at, shift, dim) = ((along * wf).round() as i32, (BEHIND_SHIFT * wf * (1.0 - along)).round() as i32, BEHIND_DIM * (1.0 - along));
                out.slide(&turn.left, &behind, at, shift, dim, corners);
                self.split_behind = Some(behind);
                return;
            }
            self.split_turn = None;
            self.split_behind = None;
        }
        out.blit_at(&self.canvas, 0, 0);
        out.blit_at(detail, self.sidebar_w, 0);
    }

    fn clamp_scroll(&mut self, h: u32) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(h));
    }

    fn overscroll(&self, h: u32, now: u64) -> f32 {
        if self.pull != 0.0 {
            motion::rubber(self.pull, h as f32)
        } else {
            self.bounce.map_or(0.0, |b| b.at(now).0)
        }
    }

    fn in_motion(&self, now: u64) -> bool {
        self.fling.is_some_and(|fl| !fl.at(now).1)
            || self.bounce.is_some_and(|b| !b.at(now).1)
            || self.seek.is_some_and(|(_, s)| !s.at(now).1)
    }

    fn catch(&mut self, h: u32, now: u64) -> bool {
        let glide = self.fling.take().is_some_and(|fl| !fl.at(now).1);
        let seeking = self.seek.take().is_some_and(|(_, s)| !s.at(now).1);
        let spring = self.bounce.take().map(|b| b.at(now)).filter(|&(_, done)| !done);
        if let Some((offset, _)) = spring {
            self.pull = motion::unrubber(offset, h as f32);
        }
        glide || seeking || spring.is_some()
    }

    fn drag_by(&mut self, h: u32, d: f32) -> bool {
        let mut d = d;
        if self.pull != 0.0 {
            let pull = self.pull + d;
            if pull * self.pull > 0.0 {
                self.pull = pull;
                return true;
            }
            self.pull = 0.0;
            d = pull;
        }
        let max = self.max_scroll(h);
        let next = self.scroll + d;
        self.scroll = next.clamp(0.0, max);
        self.pull = next - self.scroll;
        true
    }

    fn settle_fold(&mut self, h: u32, now: u64) -> bool {
        let Some(fr) = self.frame.as_ref() else { return false };
        let at = self.scroll;
        let rest = fr.settle(at).min(self.max_scroll(h));
        if rest != at {
            self.seek = Some((rest, Spring::new(now, at - rest, 0.0, FOLD_PERIOD)));
            return true;
        }
        false
    }

    fn pressing(&self) -> Option<Action> {
        match self.drag {
            SidebarDrag::Press { action: Some(a @ Action::Open(Screen::Search)), .. } => Some(a),
            _ => None,
        }
    }

    fn let_row_go(&mut self, now: u64) -> bool {
        let Some(p) = self.row_press else { return false };
        if now < p.rise {
            self.row_press = None;
            self.row_art = None;
        } else if p.fall.is_none() {
            self.row_press = Some(RowPress { fall: Some(now), ..p });
        }
        true
    }

    fn bar_wanted(&self, t: &Theme) -> Option<(bool, bool)> {
        let fr = self.frame.as_ref()?;
        let scroll = self.scroll;
        let under = (fr.content_top - widgets::bar_bottom(t) - t.m.dp(20.0)).max(0.0);
        Some((fr.header && fr.title_lift(scroll) >= t.m.dp(TITLE_FADE_DP), scroll > under))
    }

    fn follow_scroll(&mut self, t: &Theme, now: u64) -> bool {
        let Some((title, bar)) = self.bar_wanted(t) else { return false };
        self.title.set(title, now);
        self.bar.set(bar, now);
        self.title.moving(now) || self.bar.moving(now)
    }

    pub(super) fn has_deadline(&self, now: u64) -> Option<u64> {
        let moving = self.fling.is_some()
            || self.bounce.is_some()
            || self.seek.is_some()
            || self.title.moving(now)
            || self.bar.moving(now)
            || self.row_press.is_some_and(|p| p.changing(now) || p.gone(now))
            || self.split_turn.is_some();
        if moving {
            return Some(0);
        }
        self.row_press.map(|p| p.rise).filter(|&rise| rise > now)
    }
}

impl App {
    /// Theme formatted for the left navigation sidebar width.
    pub(super) fn sidebar_theme(&self) -> Theme {
        let sw = self.tablet.as_ref().map_or(self.w, |tb| tb.sidebar_w);
        if self.state.dark {
            Theme::dark(sw, self.h, self.px_per_dp)
        } else {
            Theme::light(sw, self.h, self.px_per_dp)
        }
    }

    /// Theme formatted for the full display width (status bar, toast, charging).
    pub(super) fn full_theme(&self) -> Theme {
        if self.state.dark {
            Theme::dark(self.w, self.h, self.px_per_dp)
        } else {
            Theme::light(self.w, self.h, self.px_per_dp)
        }
    }

    /// Which home sidebar item corresponds to the currently active detail tree.
    pub(super) fn sidebar_active(&self) -> Option<Screen> {
        let restore = self.state.restore.is_some();
        if let Some((Screen::Search, ..)) = self.stack.get(1) {
            return tablet::sidebar_section(self.screen(), restore);
        }
        self.stack
            .get(1)
            .and_then(|(s, _, _)| tablet::sidebar_section(*s, restore).or(Some(*s)))
    }

    /// Compose the left navigation sidebar onto `tb.canvas`.
    pub(super) fn compose_sidebar(&mut self, f: &Fonts, info: &DeviceInfo, now: u64) {
        let desired_active = self.sidebar_active();
        let t = self.sidebar_theme();
        let m = t.m;
        let dark = self.state.dark;
        let h = self.h;
        {
            let term = self.term.as_ref();
            let tb = self.tablet.as_mut().expect("sidebar composed only in split view");
            if tb.active != desired_active {
                tb.active = desired_active;
                tb.invalidate_frame();
                tb.row_press = None;
            }
            if tb.frame.is_none() {
                let cx = Ctx { t: &t, f, info, state: &self.state, term, band: None };
                let frame = pages::home_with_active(&cx, desired_active);
                tb.height = frame.height(&t);
                tb.frame = Some(frame);
                tb.clamp_scroll(h);
                if std::mem::take(&mut tb.snap) {
                    let (title, bar) = tb.bar_wanted(&t).expect("rendered above");
                    tb.title.jump(title);
                    tb.bar.jump(bar);
                }
            }
        }
        let offset = {
            let tb = self.tablet.as_ref().expect("sidebar composed only in split view");
            (tb.scroll + tb.overscroll(h, now)).round()
        };
        let light = self.light_on(Pane::Sidebar, offset);
        let state = &self.state;
        let term = self.term.as_ref();
        let tb = self.tablet.as_mut().expect("sidebar composed only in split view");
        let scroll = tb.scroll;
        let title = tb.frame.as_ref().expect("rendered above").title;
        let key = (title, dark);
        if tb.titles.as_ref().is_none_or(|(k, _, _)| *k != key) {
            tb.titles = Some((key, Blurs::new(widgets::big_title(&t, f, title)), Blurs::new(widgets::small_title(&t, f, title))));
        }
        if let Some(fold) = tb.frame.as_ref().and_then(|fr| fr.fold) {
            let key = (fold.hint, dark);
            if tb.search_words.as_ref().is_none_or(|(k, _)| *k != key) {
                tb.search_words = Some((key, widgets::search_words(&t, f, fold.bar, fold.hint)));
            }
        }
        if let (Some(press), None) = (tb.row_press.filter(|p| now >= p.rise), &tb.row_art) {
            let page = &tb.frame.as_ref().expect("rendered above").canvas;
            let (x, y, rw, rh) = row_part(press.rect, page);
            let pressed = t.pressed();
            let band = Band { rows: (y, y + rh), cols: (x, x + rw) };
            let cx = Ctx { t: &pressed, f, info, state, term, band: Some(band) };
            let down = pages::home_with_active(&cx, desired_active).canvas;
            assert_eq!((down.w, down.h), (page.w, page.h), "a sidebar row held down lays out identically");
            tb.row_art = Some((page.crop(x, y, rw, rh), down.crop(x, y, rw, rh)));
        }

        let bar_level = tb.bar.level(now);
        let title_level = tb.title.level(now);
        let row_level = tb.row_press.map_or(0.0, |p| p.level(now));
        let pressing = tb.pressing();
        let frame = tb.frame.as_ref().expect("rendered above");
        let out = &mut tb.canvas;
        let ch = out.h as f32;
        let first = (-offset).clamp(0.0, ch) as usize;
        let last = (frame.canvas.h as f32 - offset).clamp(first as f32, ch) as usize;
        if first < last {
            let from = offset as i32 + first as i32;
            out.put_rows(&frame.canvas, from, from + (last - first) as i32, first as i32);
        }
        out.fill_rect(0.0, 0.0, out.w as f32, first as f32, frame.bg);
        out.fill_rect(0.0, last as f32, out.w as f32, ch - last as f32, frame.bg);

        if let (Some(press), Some((was, is))) = (tb.row_press, &tb.row_art) {
            let (x, y, _, _) = row_part(press.rect, &frame.canvas);
            out.relay(was, is, x as i32, y as i32 - offset as i32, row_level);
        }
        if let Some(fold) = &frame.fold {
            let (_, words) = tb.search_words.as_ref().expect("made above");
            fold.draw(out, &t, words, offset);
        }
        let (_, big, small) = tb.titles.as_mut().expect("made above");
        if frame.header {
            let level = (1.0 - frame.title_lift(scroll) / m.dp(TITLE_FADE_DP)).clamp(0.0, 1.0);
            draw_faded(out, big, 0.0, -frame.title_lift(offset), t.pal.text, level, (1.0 - level) * m.dp(TITLE_BLUR_DP));
        }
        if let Some(a) = pressing {
            let r = frame.hits.iter().find(|h| h.action == a).expect("search bar held").rect;
            if let Some(r) = frame.shown(r, offset) {
                widgets::search_bar_held(out, &t, r);
            }
        }
        if let Some((Group::Page, r, radius, tint)) = light {
            out.fill_round_rect(r.x, r.y, r.w, r.h, radius, tint);
        }
        if bar_level > 0.0 {
            widgets::bar_mask(out, &t, &widgets::BAR_MASK[..], frame.bg, bar_level);
        }
        draw_faded(out, small, 0.0, 0.0, t.pal.text, title_level, (1.0 - title_level) * m.dp(TITLE_BLUR_DP));
    }

    /// Advance animations and physics on the tablet sidebar.
    pub(super) fn tick_sidebar(&mut self, now: u64) {
        let t = self.sidebar_theme();
        let h = self.h;
        let mut dirty = false;
        let tb = self.tablet.as_mut().expect("sidebar tick only in split view");
        if let Some(p) = tb.row_press {
            if p.gone(now) {
                tb.row_press = None;
                tb.row_art = None;
                dirty = true;
            } else if p.changing(now) {
                dirty = true;
            }
        }
        if let Some(fling) = tb.fling {
            let (y, done) = fling.at(now);
            let next = y.clamp(0.0, tb.max_scroll(h));
            if next != tb.scroll {
                tb.scroll = next;
                dirty = true;
            }
            if next != y {
                let speed = fling.velocity(now);
                let period = if speed.abs() <= t.m.dp(HARD_BOUNCE_DP) { 0.4 } else { 0.55 };
                tb.bounce = Some(Spring::new(now, 0.0, speed, period));
                tb.fling = None;
                dirty = true;
            } else if done {
                tb.fling = None;
                dirty |= tb.settle_fold(h, now);
            }
        }
        if let Some(b) = tb.bounce {
            dirty = true;
            if b.at(now).1 {
                tb.bounce = None;
            }
        }
        if let Some((target, spring)) = tb.seek {
            let (off, done) = spring.at(now);
            let at = if done { target } else { (target + off).clamp(0.0, tb.max_scroll(h)) };
            if tb.scroll != at {
                tb.scroll = at;
                dirty = true;
            }
            if done {
                tb.seek = None;
            }
        }
        dirty |= tb.follow_scroll(&t, now);
        dirty |= tb.split_turn.is_some();
        if dirty {
            self.dirty = true;
        }
    }

    /// End any active full-screen slide transition into tablet split view.
    pub(super) fn end_tablet_transitions(&mut self) {
        if let Some(tb) = &mut self.tablet
            && tb.split_turn.take().is_some()
        {
            tb.split_behind = None;
            self.dirty = true;
        }
    }

    /// Handle touch events inside the tablet navigation sidebar.
    pub(super) fn touch_sidebar(&mut self, ev: Touch, now: u64) -> Vec<Effect> {
        let t = self.sidebar_theme();
        let m = t.m;
        let (slop, edge_w) = (m.dp(8.0), m.dp(20.0));
        let h = self.h;
        let can_back = self.can_edge_back();
        let mut fx = Vec::new();
        let mut tap_action = None;
        let mut promoted_edge = None;
        {
            let tb = self.tablet.as_mut().expect("sidebar touch only in split view");
            let pressing_before = tb.pressing();
            let scroll = tb.scroll;
            match ev {
                Touch::Down(x, y) => {
                    tb.tracker.clear();
                    tb.tracker.add(now, y);
                    let at_left_edge = can_back && x < edge_w;
                    if at_left_edge && tb.in_motion(now) {
                        tb.drag = SidebarDrag::Press { x, y, action: None, edge: true, at: now };
                        return fx;
                    }
                    if tb.catch(h, now) {
                        tb.drag = SidebarDrag::Scroll { last: y };
                        self.dirty = true;
                        return fx;
                    }
                    let hit = if tb.bar.on && y < widgets::bar_bottom(&t) {
                        None
                    } else {
                        tb.frame
                            .as_ref()
                            .and_then(|fr| fr.hit(x, y + scroll).filter(|hit| fr.shown(hit.rect, scroll).is_some_and(|r| r.contains(x, y))))
                    };
                    let action = hit.map(|h| h.action);
                    if let Some(hit) = hit.filter(|hit| row_wide(&t, hit.rect) && !matches!(hit.action, Action::Open(Screen::Search))) {
                        tb.row_press = Some(RowPress { rect: hit.rect, rise: now + ROW_PRESS_DELAY_MS, fall: None });
                        tb.row_art = None;
                    }
                    tb.drag = SidebarDrag::Press { x, y, action, edge: at_left_edge, at: now };
                }
                Touch::Move(x, y) => match std::mem::replace(&mut tb.drag, SidebarDrag::Idle) {
                    SidebarDrag::Press { x: x0, y: y0, action, edge, at } => {
                        tb.tracker.add(now, y);
                        let (dx, dy) = (x - x0, y - y0);
                        if edge && dx.abs() > slop && dx.abs() > dy.abs() {
                            tb.let_row_go(now);
                            tb.touch_sidebar = false;
                            promoted_edge = Some(Drag::Edge { start: x0, from_left: true, armed: false, x, cy: y0 - m.dp(CUE_LIFT_DP) });
                        } else if dy.abs() > slop {
                            if !tb.catch(h, now) {
                                tb.drag_by(h, y0 - y);
                            }
                            tb.drag = SidebarDrag::Scroll { last: y };
                            if tb.let_row_go(now) {
                                self.dirty = true;
                            }
                            self.dirty = true;
                        } else {
                            tb.drag = SidebarDrag::Press { x: x0, y: y0, action, edge, at };
                        }
                    }
                    SidebarDrag::Scroll { last } => {
                        tb.tracker.add(now, y);
                        if tb.drag_by(h, last - y) {
                            self.dirty = true;
                        }
                        tb.drag = SidebarDrag::Scroll { last: y };
                    }
                    SidebarDrag::Idle => {}
                },
                Touch::Up => match std::mem::replace(&mut tb.drag, SidebarDrag::Idle) {
                    SidebarDrag::Press { action: Some(a), .. } => {
                        if let Some(p) = &mut tb.row_press {
                            p.lift(now);
                            self.dirty = true;
                        }
                        tap_action = Some(a);
                    }
                    SidebarDrag::Press { .. } => {}
                    SidebarDrag::Scroll { .. } if tb.pull != 0.0 => {
                        tb.bounce = Some(Spring::new(now, motion::rubber(tb.pull, h as f32), 0.0, 0.4));
                        tb.pull = 0.0;
                        self.dirty = true;
                    }
                    SidebarDrag::Scroll { .. } => {
                        let speed = -tb.tracker.velocity(now);
                        let folding = |s: f32| tb.frame.as_ref().is_some_and(|fr| fr.settle(s) != s);
                        let glide = (speed.abs() >= m.dp(MIN_FLING_DP)).then(|| {
                            let speed = speed.clamp(-m.dp(MAX_FLING_DP), m.dp(MAX_FLING_DP));
                            Fling::new(now, scroll, speed, self.px_per_dp * 160.0)
                        });
                        match glide {
                            Some(glide) if !(folding(scroll) && folding(glide.at(u64::MAX).0)) => {
                                tb.fling = Some(glide);
                                self.dirty = true;
                            }
                            _ => {
                                if tb.settle_fold(h, now) {
                                    self.dirty = true;
                                }
                            }
                        }
                    }
                    SidebarDrag::Idle => {}
                },
            }
            if tb.pressing() != pressing_before {
                self.dirty = true;
            }
        }
        if let Some(edge) = promoted_edge {
            self.drag = edge;
            self.dirty = true;
            return fx;
        }
        if let Some(Action::Open(target)) = tap_action {
            fx.extend(self.open_from_sidebar(target, now));
        }
        fx
    }

    /// Switch the right detail pane to a category picked in the tablet sidebar.
    pub(super) fn open_from_sidebar(&mut self, target: Screen, now: u64) -> Vec<Effect> {
        let l = self.state.lang;
        let mut fx = Vec::new();
        if self.stack.len() == 2 && self.screen() == target {
            if target != Screen::Search {
                self.seek_to(0.0, now);
            }
            return fx;
        }
        // Check active non-interruptible work anywhere in the current detail branch.
        for &(s, _, _) in &self.stack[1..] {
            match s {
                Screen::Systems if self.state.systems_work.is_some() => {
                    self.say(l.pick(texts::toast::back::WRITING), now);
                    return fx;
                }
                Screen::Editor if self.state.editor.as_ref().is_some_and(|e| e.changed) => {
                    if self.unsaved_warned.is_none_or(|at| now >= at + TOAST_MS) {
                        self.unsaved_warned = Some(now);
                        self.say(l.pick(texts::toast::back::UNSAVED), now);
                        return fx;
                    }
                }
                Screen::FileJob if self.state.job.as_ref().is_some_and(|j| j.outcome.is_none()) => {
                    self.say(l.pick(texts::toast::back::WAIT_OR_STOP), now);
                    return fx;
                }
                Screen::Relabel if self.state.relabel.as_ref().is_some_and(|j| j.outcome.is_none()) => {
                    self.say(l.pick(texts::toast::back::WAIT_OR_STOP), now);
                    return fx;
                }
                Screen::PartWork if self.state.part_work.as_ref().is_some_and(|w| w.outcome.is_none()) => {
                    self.say(l.pick(texts::toast::back::WAIT), now);
                    return fx;
                }
                Screen::SkywardUpdate if self.state.skyward_job.as_ref().is_some_and(|j| j.outcome.is_none()) => {
                    match self.state.skyward_job.as_ref().and_then(|j| j.stage) {
                        Some(SkywardStage::Write { .. }) => {
                            self.say(l.pick(texts::toast::back::WRITING), now);
                        }
                        _ => self.say(l.pick(texts::toast::back::WAIT_OR_STOP), now),
                    }
                    return fx;
                }
                Screen::BackupJob if self.state.backup_job.as_ref().is_some_and(|j| j.outcome.is_none()) => {
                    let job = self.state.backup_job.as_ref().expect("checked");
                    let writing = job.restore && job.at.is_some_and(|a| !a.checking);
                    let said = if writing { texts::toast::back::RESTORING } else { texts::toast::back::WAIT_OR_STOP };
                    self.say(l.pick(said), now);
                    return fx;
                }
                Screen::Installing => match self.state.install {
                    Install::Checking { stage: Some(Stage::Signature { .. }), .. } => {
                        self.state.install = Install::Ending;
                        fx.push(Effect::Cancel);
                    }
                    Install::Checking { .. } => {
                        self.say(l.pick(texts::toast::back::READING), now);
                        return fx;
                    }
                    Install::Updating { .. } | Install::Scripting { .. } | Install::Systeming { .. } => {
                        self.say(l.pick(texts::toast::back::INSTALLING), now);
                        return fx;
                    }
                    Install::Flashing { .. } => {
                        self.say(l.pick(texts::toast::back::FLASHING), now);
                        return fx;
                    }
                    Install::Opening => {
                        self.say(l.pick(DATA_OPENING), now);
                        return fx;
                    }
                    Install::Wiping => {
                        self.say(l.pick(texts::toast::back::WIPING), now);
                        return fx;
                    }
                    Install::Starting
                    | Install::Locked(_)
                    | Install::Waiting
                    | Install::Choosing(_)
                    | Install::SystemChoosing(_)
                    | Install::Confirming { .. }
                    | Install::ConfirmingScript => {
                        self.state.install = Install::Ending;
                        fx.push(Effect::Cancel);
                    }
                    Install::Finished { .. } | Install::Wiped(_) if self.state.source == Source::Rescue => {
                        self.state.install = Install::Ending;
                        fx.push(Effect::Cancel);
                    }
                    Install::Finished { .. } | Install::Wiped(_) => self.state.install = Install::Idle,
                    Install::Idle | Install::Ending => {}
                },
                _ => {}
            }
        }
        // Clean up transient state of the left-behind detail branch.
        self.state.mount_failure = None;
        self.state.next_slot = None;
        self.state.system_pick = None;
        self.state.asking_delete = false;
        self.state.clip = None;
        self.state.file = None;
        self.state.editor = None;
        self.unsaved_warned = None;
        self.state.clash = None;
        self.state.joining = None;
        self.state.details = None;
        self.state.adding = None;
        self.state.password.clear();
        self.state.job = None;
        self.state.relabel = None;
        self.state.part_work = None;
        self.state.fs_action = FsAction::Repair;
        self.state.restore = None;
        self.state.backup_job = None;
        self.state.confirm = 0.0;

        let prev_root = self.stack[1].0;
        if target == Screen::Search {
            if let Some(tb) = &mut self.tablet
                && prev_root != Screen::Search
            {
                tb.before_search = Some(prev_root);
            }
        } else if let Some(tb) = &mut self.tablet {
            tb.before_search = None;
        }
        self.stack = vec![self.stack[0], (target, 0.0, Focus::Unset)];
        self.reset_split_caches();
        self.settle_page();
        self.refresh();
        fx.extend(self.arrive_at(target));
        fx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    const TW: u32 = 2800;
    const TH: u32 = 1980;
    const TPPD: f32 = 2.25;

    #[test]
    fn tablet_boots_into_home_and_about_split_view() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        assert!(app.split_active());
        assert_eq!(app.screen(), Screen::About);
        assert_eq!(app.sidebar_active(), Some(Screen::About));
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 100);
        // Right pane starts at sidebar_w and hides Back button on top-level About.
        let fr = app.frame(&f, &i);
        assert!(!fr.back);
        assert_eq!(fr.canvas.w, TW - crate::theme::sidebar_width(TW, TPPD));
    }

    #[test]
    fn tapping_sidebar_switches_detail_pane_and_nests_with_back() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);

        // Tap Settings row inside the sidebar.
        let hit = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Settings))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(hit.0, hit.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.sidebar_active(), Some(Screen::Settings));
        app.compose(&mut out, &f, &i, "12:00", 200);
        assert!(!app.frame(&f, &i).back, "top-level detail pane omits Back");

        // Open nested ScreenOff inside Settings: now Back is shown and pops to Settings.
        let sw = crate::theme::sidebar_width(TW, TPPD) as f32;
        let off_rect = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::ScreenOff)).unwrap().rect;
        app.touch(Touch::Down(sw + off_rect.centre().0, off_rect.centre().1), &f, 300);
        app.touch(Touch::Up, &f, 320);
        assert_eq!(app.screen(), Screen::ScreenOff);
        assert_eq!(app.sidebar_active(), Some(Screen::Settings));
        assert!(app.frame(&f, &i).back);

        let back = widgets::back_rect(&app.theme()).centre();
        app.touch(Touch::Down(sw + back.0, back.1), &f, 400);
        app.touch(Touch::Up, &f, 420);
        assert_eq!(app.screen(), Screen::Settings);

        // Back at top level of right pane stays put instead of popping Home.
        app.key(Key::Back, 500);
        assert_eq!(app.screen(), Screen::Settings);
    }

    #[test]
    fn initial_decrypt_and_fastboot_run_fullscreen_and_enter_split_view_when_dismissed() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, TW, TH, TPPD);
        assert!(!app.split_active());
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);
        assert_eq!(app.frame(&f, &i).canvas.w, TW);

        // Skip transitions to tablet split view [Home, About].
        let skip = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Skip).unwrap().rect.centre();
        app.touch(Touch::Down(skip.0, skip.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert!(app.split_active());
        assert_eq!(app.screen(), Screen::About);

        let mut fb = App::new(Screen::Fastboot, TW, TH, TPPD);
        assert!(!fb.split_active());
        fb.enter_recovery();
        assert!(fb.split_active());
        assert_eq!(fb.screen(), Screen::About);
    }

    #[test]
    fn active_install_blocks_sidebar_switch() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);
        app.start_sideload();
        app.install(skyward_install::Event::Checking { size: 1000 }, 50);
        assert_eq!(app.screen(), Screen::Installing);
        app.compose(&mut out, &f, &i, "12:00", 60);

        let reboot = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Reboot))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(reboot.0, reboot.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert_eq!(app.screen(), Screen::Installing, "active install stays put when sidebar tapped");
    }

    #[test]
    fn search_in_sidebar_opens_in_detail_and_close_restores_previous_section() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);
        let wipe = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Wipe))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(wipe.0, wipe.1), &f, 50);
        app.touch(Touch::Up, &f, 70);
        assert_eq!(app.screen(), Screen::Wipe);
        app.compose(&mut out, &f, &i, "12:00", 80);

        let search = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Search))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(search.0, search.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert_eq!(app.screen(), Screen::Search);

        let sw = crate::theme::sidebar_width(TW, TPPD) as f32;
        app.compose(&mut out, &f, &i, "12:00", 200);
        let close = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::CloseSearch).unwrap().rect.centre();
        app.touch(Touch::Down(sw + close.0, close.1), &f, 300);
        app.touch(Touch::Up, &f, 320);
        assert_eq!(app.screen(), Screen::Wipe);
    }

    #[test]
    fn tablet_split_slide_uses_square_leading_corners() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        app.set_screen_corners(65.0);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);

        let settings = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Settings))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(settings.0, settings.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert_eq!(app.screen(), Screen::Settings);
        app.compose(&mut out, &f, &i, "12:00", 200);

        let sw = crate::theme::sidebar_width(TW, TPPD);
        let dw = TW - sw;
        let off = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::ScreenOff)).unwrap().rect.centre();
        app.touch(Touch::Down(sw as f32 + off.0, off.1), &f, 1000);
        app.touch(Touch::Up, &f, 1020);
        app.compose(&mut out, &f, &i, "12:00", 1000);
        app.compose(&mut out, &f, &i, "12:00", 1050);
        let mid = shot(&out);
        app.compose(&mut out, &f, &i, "12:00", 3000);
        let settled = shot(&out);

        let (x, _) = Spring::new(0, 1000.0, -OPEN_SPEED * 1000.0, OPEN_PERIOD).at(50);
        let front = ((x / 1000.0) * dw as f32).round() as u32;
        let at = |c: &Canvas, x: u32, y: u32| c.data[((y * TW + x) * 4) as usize];
        let y = TH - 3;
        assert!(at(&mid, sw + front - 5, y) < at(&settled, sw + 5, y), "behind pane is dimmed left of front edge");
        assert_eq!(at(&mid, sw + front + 5, y), at(&settled, sw + 5, y), "square leading corner does not clip behind divider");
    }

    #[test]
    fn tablet_split_edge_swipe_uses_physical_edges_not_center_divider() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);

        let settings = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Settings))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(settings.0, settings.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        app.compose(&mut out, &f, &i, "12:00", 200);

        let sw = crate::theme::sidebar_width(TW, TPPD) as f32;
        let open_screen_off = |app: &mut App, out: &mut Canvas, at: u64| {
            let off = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Open(Screen::ScreenOff)).unwrap().rect.centre();
            app.touch(Touch::Down(sw + off.0, off.1), &f, at);
            app.touch(Touch::Up, &f, at + 20);
            app.compose(out, &f, &i, "12:00", at + 3000);
            assert_eq!(app.screen(), Screen::ScreenOff);
        };
        open_screen_off(&mut app, &mut out, 300);

        // Swiping right from the center divider must not trigger edge-back.
        app.touch(Touch::Down(sw + 10.0, 1000.0), &f, 4000);
        app.touch(Touch::Move(sw + 90.0, 1005.0), &f, 4010);
        app.touch(Touch::Move(sw + 320.0, 1010.0), &f, 4020);
        assert!(!matches!(app.drag, Drag::Edge { .. }));
        app.touch(Touch::Up, &f, 4030);
        assert_eq!(app.screen(), Screen::ScreenOff);

        // Swiping right from the physical left edge arms back and draws the cue on the physical left edge only.
        app.touch(Touch::Down(10.0, 1000.0), &f, 4100);
        app.touch(Touch::Move(90.0, 1005.0), &f, 4110);
        app.touch(Touch::Move(320.0, 1010.0), &f, 4120);
        let Drag::Edge { from_left: true, armed: true, cy, .. } = app.drag else {
            panic!("expected armed left-edge drag");
        };
        app.compose(&mut out, &f, &i, "12:00", 4120);
        let bg = app.full_theme().pal.bg.r;
        assert!(grey_at(&out, 8.0, cy) < bg - 100, "cue painted at physical left edge");
        assert!(grey_at(&out, sw + 8.0, cy).abs_diff(bg) <= 1, "no cue at center divider");
        app.touch(Touch::Up, &f, 4130);
        app.compose(&mut out, &f, &i, "12:00", 7000);
        assert_eq!(app.screen(), Screen::Settings);

        // Swiping left from the physical right edge arms back and draws the cue on the physical right edge.
        open_screen_off(&mut app, &mut out, 8000);
        let right_x = TW as f32 - 10.0;
        app.touch(Touch::Down(right_x, 1000.0), &f, 12000);
        app.touch(Touch::Move(right_x - 80.0, 1005.0), &f, 12010);
        app.touch(Touch::Move(right_x - 310.0, 1010.0), &f, 12020);
        let Drag::Edge { from_left: false, armed: true, cy, .. } = app.drag else {
            panic!("expected armed right-edge drag");
        };
        app.compose(&mut out, &f, &i, "12:00", 12020);
        assert!(grey_at(&out, right_x, cy) < bg - 100, "cue painted at physical right edge");
        assert!(grey_at(&out, 8.0, cy).abs_diff(bg) <= 1, "no cue at physical left edge");
        app.touch(Touch::Up, &f, 12030);
        app.compose(&mut out, &f, &i, "12:00", 15000);
        assert_eq!(app.screen(), Screen::Settings);

        // Top-level detail page stays put when swiping from the physical left edge.
        app.touch(Touch::Down(10.0, 1600.0), &f, 16000);
        app.touch(Touch::Move(90.0, 1605.0), &f, 16010);
        app.touch(Touch::Move(320.0, 1610.0), &f, 16020);
        assert!(!matches!(app.drag, Drag::Edge { .. }));
        app.touch(Touch::Up, &f, 16030);
        assert_eq!(app.screen(), Screen::Settings);
    }

    #[test]
    fn tablet_sidebar_switch_cuts_cleanly_and_clears_row_press() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);

        let sw = crate::theme::sidebar_width(TW, TPPD);
        let settings = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Settings))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(settings.0, settings.1), &f, 100);
        app.touch(Touch::Up, &f, 120);
        assert_eq!(app.screen(), Screen::Settings);
        assert!(app.turn.is_none());
        app.compose(&mut out, &f, &i, "12:00", 120);
        assert!(app.tablet.as_ref().unwrap().row_press.is_none());

        // Switching to Search and closing it restores Settings cleanly without a page slide.
        let search = app
            .tablet
            .as_ref()
            .unwrap()
            .frame
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == Action::Open(Screen::Search))
            .unwrap()
            .rect
            .centre();
        app.touch(Touch::Down(search.0, search.1), &f, 2100);
        app.touch(Touch::Up, &f, 2120);
        assert_eq!(app.screen(), Screen::Search);
        app.compose(&mut out, &f, &i, "12:00", 4000);

        let close = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::CloseSearch).unwrap().rect.centre();
        app.touch(Touch::Down(sw as f32 + close.0, close.1), &f, 4100);
        app.touch(Touch::Up, &f, 4120);
        assert_eq!(app.screen(), Screen::Settings);
        assert!(app.turn.is_none());
        app.compose(&mut out, &f, &i, "12:00", 4120);
    }

    #[test]
    fn tablet_volume_and_power_keys_navigate_sidebar_and_enter_detail() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);
        assert!(!app.keys);

        // First volume press lights Search in Sidebar without lighting Detail.
        press(&mut app, Key::VolumeDown, 100);
        assert!(app.keys);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.light_on(Pane::Sidebar, 0.0).is_some());
        assert!(app.light(0.0).is_none(), "detail pane stays unlit while focus is on sidebar");
        app.compose(&mut out, &f, &i, "12:00", 120);

        // Step down Sidebar to its last item (Reboot); verify off-screen scroll behavior as well.
        for ms in 1..=30u64 {
            press(&mut app, Key::VolumeDown, 120 + ms * 10);
            if app.focus_pane() == Pane::Sidebar && app.lit().map(|s| s.stop) == Some(Stop::Hit(Action::Open(Screen::Reboot))) {
                break;
            }
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Reboot))));
        app.tick(3000);
        let tb_scroll = app.tablet.as_ref().unwrap().scroll;

        // Stepping past the bottom of Sidebar crosses cleanly into Detail, and VolumeUp crosses back.
        press(&mut app, Key::VolumeDown, 3100);
        assert_eq!(app.focus_pane(), Pane::Detail);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Specs))));
        assert!(app.light_on(Pane::Sidebar, tb_scroll).is_none());
        assert!(app.light(app.top().1).is_some());

        press(&mut app, Key::VolumeUp, 3200);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Reboot))));

        // Navigate back up to Settings in Sidebar and activate it with Power.
        for ms in 1..=20u64 {
            press(&mut app, Key::VolumeUp, 3200 + ms * 10);
            if lit(&mut app, &f, &i) == Some(Stop::Hit(Action::Open(Screen::Settings))) {
                break;
            }
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Settings))));
        assert!(app.key(Key::Power, 4000).is_empty());
        assert_eq!(app.screen(), Screen::About, "power triggers on key_up");
        app.key_up(Key::Power, 4020);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.sidebar_active(), Some(Screen::Settings));
        assert_eq!(app.focus_pane(), Pane::Detail);

        // Compose settled Settings, verify first detail control is lit (no Back at root), then enter nested ScreenOff.
        app.compose(&mut out, &f, &i, "12:00", 6000);
        let first_detail = lit(&mut app, &f, &i);
        assert!(first_detail.is_some() && first_detail != Some(Stop::Back));
        for ms in 1..=20u64 {
            press(&mut app, Key::VolumeDown, 6000 + ms * 10);
            if lit(&mut app, &f, &i) == Some(Stop::Hit(Action::Open(Screen::ScreenOff))) {
                break;
            }
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::ScreenOff))));
        press(&mut app, Key::Power, 6300);
        assert_eq!(app.screen(), Screen::ScreenOff);
        assert_eq!(app.focus_pane(), Pane::Detail);
        app.compose(&mut out, &f, &i, "12:00", 8000);
        assert!(app.frame(&f, &i).back);

        // Nested detail page keeps focus scoped inside Detail, including Stop::Back.
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::ScreenOff(0))));
        press(&mut app, Key::VolumeUp, 8100);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Back));
        assert_eq!(app.focus_pane(), Pane::Detail);

        // Key::Back from nested page pops to Settings in Detail; a second Key::Back returns focus to Sidebar.
        app.key(Key::Back, 8200);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.focus_pane(), Pane::Detail);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::ScreenOff))));
        app.compose(&mut out, &f, &i, "12:00", 8250);

        app.key(Key::Back, 8300);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Settings))));

        // Opening Search from Sidebar with Power and closing via Key::Back returns focus to Sidebar.
        for ms in 1..=20u64 {
            press(&mut app, Key::VolumeUp, 8300 + ms * 10);
            if lit(&mut app, &f, &i) == Some(Stop::Hit(Action::Open(Screen::Search))) {
                break;
            }
        }
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        press(&mut app, Key::Power, 8600);
        assert_eq!(app.screen(), Screen::Search);
        assert_eq!(app.focus_pane(), Pane::Detail);
        app.compose(&mut out, &f, &i, "12:00", 8650);
        app.key(Key::Back, 8700);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        app.compose(&mut out, &f, &i, "12:00", 8750);
        assert!(app.light_on(Pane::Sidebar, 0.0).is_some());

        // On a shorter landscape panel where Reboot falls below the viewport,
        // stepping down Sidebar scrolls Reboot cleanly into view.
        let mut short = App::new(Screen::Home, TW, 1400, TPPD);
        let mut short_out = Canvas::new(TW, 1400);
        short.compose(&mut short_out, &f, &i, "12:00", 0);
        for ms in 1..=30u64 {
            press(&mut short, Key::VolumeDown, ms * 10);
            if short.focus_pane() == Pane::Sidebar && short.lit().map(|s| s.stop) == Some(Stop::Hit(Action::Open(Screen::Reboot))) {
                break;
            }
        }
        short.tick(3000);
        assert_eq!(lit(&mut short, &f, &i), Some(Stop::Hit(Action::Open(Screen::Reboot))));
        let reboot = short.lit().unwrap();
        let scroll = short.tablet.as_ref().unwrap().scroll;
        assert!(scroll > 0.0, "short tablet sidebar scrolled to reveal Reboot");
        assert!(short.shows_on(Pane::Sidebar, &reboot, scroll), "Reboot shown in sidebar viewport");
    }

    #[test]
    fn tablet_touchscreen_missing_and_empty_detail_pane_keys() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, TW, TH, TPPD);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);

        // Without a touchscreen, Sidebar starts lit immediately.
        app.touchscreen_missing();
        assert!(app.keys);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        assert_eq!(lit(&mut app, &f, &i), Some(Stop::Hit(Action::Open(Screen::Search))));
        assert!(app.light_on(Pane::Sidebar, 0.0).is_some());

        // Holding Power on a Sidebar row strengthens the focus light and acts on release.
        press(&mut app, Key::VolumeDown, 100);
        let about_spot = lit(&mut app, &f, &i);
        assert_eq!(about_spot, Some(Stop::Hit(Action::Open(Screen::About))));
        app.key(Key::Power, 200);
        app.tick(200 + CHORD_MS);
        assert_eq!(app.light_on(Pane::Sidebar, 0.0).map(|l| l.3.a), Some(LIGHT_HELD_ALPHA));
        app.key_up(Key::Power, 400);
        assert_eq!(app.screen(), Screen::About);
        assert_eq!(app.focus_pane(), Pane::Detail);

        // Switching to Restore (with 0 backups -> 0 focusable detail stops) never traps keys.
        app.key(Key::Back, 450);
        for ms in 1..=20u64 {
            press(&mut app, Key::VolumeDown, 450 + ms * 10);
            if lit(&mut app, &f, &i) == Some(Stop::Hit(Action::Open(Screen::Restore))) {
                break;
            }
        }
        press(&mut app, Key::Power, 700);
        assert_eq!(app.screen(), Screen::Restore);
        app.compose(&mut out, &f, &i, "12:00", 2000);
        assert!(app.spots().expect("rendered").is_empty());

        // VolumeDown crosses directly from empty Restore back to Sidebar.
        press(&mut app, Key::VolumeDown, 2100);
        assert_eq!(app.focus_pane(), Pane::Sidebar);

        // Touching the empty Detail pane clears keys and sets focus_pane = Detail;
        // pressing VolumeDown afterward falls back immediately to Sidebar rather than lighting nothing.
        let sw = crate::theme::sidebar_width(TW, TPPD) as f32;
        app.touch(Touch::Down(sw + 100.0, 200.0), &f, 2200);
        app.touch(Touch::Up, &f, 2210);
        assert!(!app.keys);
        assert_eq!(app.focus_pane(), Pane::Detail);
        press(&mut app, Key::VolumeDown, 2300);
        assert!(app.keys);
        assert_eq!(app.focus_pane(), Pane::Sidebar);
        assert!(app.lit().is_some());
    }

    #[test]
    fn tablet_entering_split_view_from_decrypt_slides_fullscreen_page_off() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Decrypt, TW, TH, TPPD);
        app.set_screen_corners(65.0);
        let mut out = Canvas::new(TW, TH);
        app.compose(&mut out, &f, &i, "12:00", 0);
        let decrypt = shot(&out);

        let skip = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Skip).unwrap().rect.centre();
        app.touch(Touch::Down(skip.0, skip.1), &f, 1000);
        app.touch(Touch::Up, &f, 1020);
        assert!(app.split_active());
        assert!(app.tablet.as_ref().unwrap().split_turn.is_some());
        assert_eq!(app.deadline(1020), Some(0));

        app.compose(&mut out, &f, &i, "12:00", 1000);
        app.compose(&mut out, &f, &i, "12:00", 1060);
        let mid = shot(&out);
        app.compose(&mut out, &f, &i, "12:00", 3000);
        let settled = shot(&out);
        {
            let tb = app.tablet.as_ref().unwrap();
            assert!(tb.split_turn.is_none());
            assert!(tb.split_behind.is_none());
        }

        let (x, _) = Spring::new(0, -1000.0, 0.0, BACK_PERIOD).at(60);
        let front = ((1.0 + x / 1000.0) * TW as f32).round() as u32;
        let at = |c: &Canvas, x: u32, y: u32| c.data[((y * TW + x) * 4) as usize];
        assert!(at(&mid, 5, TH / 2) < at(&settled, 5, TH / 2), "split view is dimmed under sliding decrypt page");
        assert_eq!(at(&mid, front + 5, TH / 2), at(&decrypt, 5, TH / 2), "outgoing decrypt edge slides cleanly");
        assert!(at(&mid, front + 5, TH - 3) < at(&decrypt, 5, TH - 3), "rounded fullscreen corner reveals dimmed split view behind");
    }
}
