use crate::app::{Action, Frame, Screen};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widgets;

/// A control the keys can reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stop {
    /// The `<`.
    Back,
    /// The round button at the right end of the bar, and what it does.
    Button(Action),
    /// A control of the page, of a strip laid over it, or a row of its long
    /// list.
    Hit(Action),
    /// A dot of the pattern grid, by its number counted from zero across
    /// the rows.
    Dot(usize),
}

/// Where a stop lies, which is how it moves as the page scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// The `<` and the bar's round button, held at the top of the screen.
    Fixed,
    /// On the strip that scrolls with the page until it reaches the bar.
    Strip,
    /// On the page, scrolling with it.
    Page,
    /// On the strip held at the bottom of the screen.
    Dock,
}

/// A stop and where it is: in screen coordinates for the `<`, and in its
/// strip's or the page's own coordinates for the others.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub stop: Stop,
    pub group: Group,
    pub rect: Rect,
}

/// The stops of `frame` in the order the keys go through them: the `<` and
/// the bar's round button, the strip's from left to right, the page's own
/// from the top down and left to right, then the bottom strip's.
pub fn spots(t: &Theme, frame: &Frame) -> Vec<Spot> {
    let mut out = Vec::new();
    if frame.back {
        out.push(Spot { stop: Stop::Back, group: Group::Fixed, rect: widgets::back_rect(t) });
    }
    if let Some((_, action)) = frame.button {
        out.push(Spot { stop: Stop::Button(action), group: Group::Fixed, rect: widgets::button_rect(t) });
    }
    let across = |a: &Spot, b: &Spot| a.rect.x.total_cmp(&b.rect.x);
    if let Some(p) = &frame.pinned {
        let mut strip: Vec<Spot> = p.hits.iter().map(|h| Spot { stop: Stop::Hit(h.action), group: Group::Strip, rect: h.rect }).collect();
        strip.sort_by(across);
        out.extend(strip);
    }
    // The pattern grid is drawn on with a finger; the keys join its dots
    // one by one. Off a list open over the page a touch closes it, which
    // the keys do going back.
    let mut page: Vec<Spot> = frame
        .hits
        .iter()
        .filter(|h| !matches!(h.action, Action::Pattern | Action::CloseChoices))
        .map(|h| Spot { stop: Stop::Hit(h.action), group: Group::Page, rect: h.rect })
        .collect();
    if let Some((cx, cy, span)) = frame.pattern {
        let reach = t.m.dp(widgets::PATTERN_REACH_DP);
        let dots = widgets::pattern_dots(cx, cy, span).into_iter().enumerate();
        page.extend(dots.map(|(i, (x, y))| Spot { stop: Stop::Dot(i), group: Group::Page, rect: Rect::new(x - reach, y - reach, reach * 2.0, reach * 2.0) }));
    }
    if let Some(rows) = &frame.rows
        && let Some(action) = rows.action
    {
        let w = frame.canvas.w as f32;
        page.extend((0..rows.count()).map(|i| Spot { stop: Stop::Hit(action(i)), group: Group::Page, rect: rows.rect(i, w) }));
    }
    page.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y).then(across(a, b)));
    out.extend(page);
    if let Some(d) = &frame.dock {
        let mut dock: Vec<Spot> = d.hits.iter().map(|h| Spot { stop: Stop::Hit(h.action), group: Group::Dock, rect: h.rect }).collect();
        dock.sort_by(across);
        out.extend(dock);
    }
    out
}

/// Which of `spots` is the control `at` was on a page drawn before: the
/// same control in the same part of the screen, the one nearest to where
/// it was when there are more.
pub fn find(spots: &[Spot], at: &Spot) -> Option<usize> {
    let (x, y) = at.rect.centre();
    let away = |s: &Spot| {
        let (sx, sy) = s.rect.centre();
        (sx - x).hypot(sy - y)
    };
    let same = spots.iter().enumerate().filter(|(_, s)| s.stop == at.stop && s.group == at.group);
    same.min_by(|(_, a), (_, b)| away(a).total_cmp(&away(b))).map(|(k, _)| k)
}

/// Where the light falls on the stop at `r` on screen, and how round its
/// corners are: within a row of a list, as a list worked with the keys
/// lights its rows, and over the whole of any other control, in its shape.
/// `keypad` when the page types with the round keys of a screen lock's code.
pub fn lit(t: &Theme, stop: Stop, r: Rect, keypad: bool) -> (Rect, f32) {
    let m = &t.m;
    match stop {
        Stop::Back => {
            let (cx, cy, radius) = widgets::back_disc(t);
            (Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0), radius)
        }
        Stop::Button(_) => {
            let (cx, cy, radius) = widgets::button_disc(t);
            (Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0), radius)
        }
        Stop::Dot(_) => {
            let ((cx, cy), radius) = (r.centre(), m.dp(DOT_LIGHT_DP));
            (Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0), radius)
        }
        Stop::Hit(Action::Type(_)) if keypad => (r, r.w.min(r.h) / 2.0),
        Stop::Hit(Action::Type(_)) => (r, m.dp(widgets::KEY_RADIUS_DP)),
        Stop::Hit(Action::TermKey(_)) => (r, m.dp(crate::term::KEY_RADIUS_DP)),
        // The swipe and slider tracks and the buttons are tinted already,
        // and lit as a halo round them.
        Stop::Hit(
            Action::Confirm(_) | Action::Slider(_) | Action::OpenData | Action::ExportLogs | Action::Unpick | Action::Paste | Action::CloseSearch,
        ) => {
            let halo = m.dp(HALO_DP);
            (Rect::new(r.x - halo, r.y - halo, r.w + halo * 2.0, r.h + halo * 2.0), r.h / 2.0 + halo)
        }
        // The other capsules: the path's folders, the text that leaves a
        // page, and the search bar.
        Stop::Hit(Action::OpenAncestor(_) | Action::Skip | Action::Open(Screen::Search)) => (r, r.h / 2.0),
        Stop::Hit(_) => {
            let (dx, dy) = (m.dp(ROW_INSET_X_DP), m.dp(ROW_INSET_Y_DP));
            (Rect::new(r.x + dx, r.y + dy, r.w - dx * 2.0, r.h - dy * 2.0), m.card_radius - dx)
        }
    }
}

/// How far the light sits within a row, across and down.
const ROW_INSET_X_DP: f32 = 6.0;
const ROW_INSET_Y_DP: f32 = 4.0;
/// The radius of the light on a pattern dot, round the ring a dot joined
/// shows.
const DOT_LIGHT_DP: f32 = 22.0;
/// How far the light reaches out round a control tinted already.
const HALO_DP: f32 = 6.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Dock, Hit, Pinned, Rows};
    use crate::canvas::Canvas;
    use crate::widgets::Cap;

    fn theme() -> Theme {
        Theme::light(1440, 3200, 3.5)
    }

    fn hit(x: f32, y: f32, action: Action) -> Hit {
        Hit { rect: Rect::new(x, y, 100.0, 50.0), action }
    }

    #[test]
    fn the_keys_go_from_the_back_control_across_the_strip_down_the_page_then_across_the_dock() {
        let t = theme();
        let page = vec![
            hit(500.0, 900.0, Action::Open(Screen::Mount)),
            hit(100.0, 900.0, Action::Open(Screen::Wipe)),
            hit(100.0, 400.0, Action::Pattern),
            hit(100.0, 500.0, Action::Open(Screen::Install)),
        ];
        let mut frame = Frame::new(&t, Canvas::new(1440, 3200), page, "页面", 300.0);
        frame.pinned = Some(Pinned { canvas: Canvas::new(1440, 100), top: 300.0, hits: vec![hit(400.0, 10.0, Action::OpenAncestor(1)), hit(50.0, 10.0, Action::OpenAncestor(0))] });
        frame.rows = Some(Rows { edges: vec![1000.0, 1100.0, 1200.0], action: Some(Action::OpenEntry) });
        frame.dock = Some(Dock { canvas: Canvas::new(1440, 200), hits: vec![hit(800.0, 20.0, Action::Paste), hit(100.0, 20.0, Action::Unpick)] });
        let order: Vec<(Stop, Group)> = spots(&t, &frame).iter().map(|s| (s.stop, s.group)).collect();
        assert_eq!(order, [
            (Stop::Back, Group::Fixed),
            (Stop::Hit(Action::OpenAncestor(0)), Group::Strip),
            (Stop::Hit(Action::OpenAncestor(1)), Group::Strip),
            (Stop::Hit(Action::Open(Screen::Install)), Group::Page),
            (Stop::Hit(Action::Open(Screen::Wipe)), Group::Page),
            (Stop::Hit(Action::Open(Screen::Mount)), Group::Page),
            (Stop::Hit(Action::OpenEntry(0)), Group::Page),
            (Stop::Hit(Action::OpenEntry(1)), Group::Page),
            (Stop::Hit(Action::Unpick), Group::Dock),
            (Stop::Hit(Action::Paste), Group::Dock),
        ], "the pattern grid takes no key");
        frame.back = false;
        assert_eq!(spots(&t, &frame)[0].stop, Stop::Hit(Action::OpenAncestor(0)), "a page without the `<` starts at its strip");
        frame.back = true;
        frame.button = Some((widgets::Glyph::Terminal, Action::TerminalHere));
        let order: Vec<(Stop, Group)> = spots(&t, &frame).iter().take(3).map(|s| (s.stop, s.group)).collect();
        assert_eq!(order, [
            (Stop::Back, Group::Fixed),
            (Stop::Button(Action::TerminalHere), Group::Fixed),
            (Stop::Hit(Action::OpenAncestor(0)), Group::Strip),
        ], "the bar's round button comes after the `<`");
    }

    #[test]
    fn a_control_is_found_again_where_it_was_on_the_page_drawn_anew() {
        let t = theme();
        let twice = vec![hit(100.0, 400.0, Action::Unavailable("暂不支持")), hit(100.0, 900.0, Action::Unavailable("暂不支持"))];
        let spots = spots(&t, &Frame::new(&t, Canvas::new(1440, 3200), twice, "页面", 300.0));
        let lower = Spot { stop: Stop::Hit(Action::Unavailable("暂不支持")), group: Group::Page, rect: Rect::new(100.0, 880.0, 100.0, 50.0) };
        assert_eq!(find(&spots, &lower).map(|k| spots[k].rect.y), Some(900.0), "the nearer of two doing the same");
        assert_eq!(find(&spots, &Spot { group: Group::Dock, ..lower }), None, "on the page, not in the dock");
        assert_eq!(find(&spots, &Spot { stop: Stop::Back, ..lower }), None);
    }

    #[test]
    fn the_light_takes_the_shape_of_its_control() {
        let t = theme();
        let m = &t.m;
        let row = Rect::new(m.margin, 1000.0, 1440.0 - m.margin * 2.0, 200.0);
        let (r, radius) = lit(&t, Stop::Hit(Action::Open(Screen::Wipe)), row, false);
        assert!(r.x > row.x && r.y > row.y && r.x + r.w < row.x + row.w && r.y + r.h < row.y + row.h, "a row is lit within it");
        assert_eq!(radius, m.card_radius - m.dp(ROW_INSET_X_DP), "its corners following the card's");
        let track = Rect::new(m.margin, 2000.0, 1300.0, 196.0);
        let (halo, round) = lit(&t, Stop::Hit(Action::Confirm(crate::app::Confirm::Wipe)), track, false);
        assert!(halo.x < track.x && halo.y < track.y && halo.w > track.w && halo.h > track.h, "a track is lit as a halo round it");
        assert_eq!(round, halo.h / 2.0, "in its own capsule shape");
        let key = Rect::new(400.0, 2600.0, 266.0, 266.0);
        assert_eq!(lit(&t, Stop::Hit(Action::Type(Cap::Char('5'))), key, true), (key, 133.0), "a round key of the code's keypad");
        assert_eq!(lit(&t, Stop::Hit(Action::Type(Cap::Char('q'))), key, false), (key, m.dp(widgets::KEY_RADIUS_DP)), "a keyboard's key");
        let (cx, cy, rr) = widgets::back_disc(&t);
        let (disc, round) = lit(&t, Stop::Back, widgets::back_rect(&t), false);
        assert_eq!((disc.centre(), round), ((cx, cy), rr), "the `<` lit as the disc it sits on");
        let (bx, by, br) = widgets::button_disc(&t);
        let (disc, round) = lit(&t, Stop::Button(Action::TerminalHere), widgets::button_rect(&t), false);
        assert_eq!((disc.centre(), round), ((bx, by), br), "the bar's round button too");
        assert_eq!((bx + cx, by), (1440.0, cy), "across the screen from the `<`");
    }
}
