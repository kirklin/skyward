use crate::canvas::Canvas;
use crate::color::{blend_over, mix, over, Rgba};
use crate::fmt;
use crate::font::{Face, Fonts};
use crate::geom::Rect;
use crate::icons::{self, Icon};
use crate::lang::Lang;
use crate::sprite::Sprite;
use crate::theme::Theme;
use std::f32::consts::FRAC_PI_4;

/// Where a drawn card ends and where each of its rows lies, for hit testing.
pub struct Block {
    pub bottom: f32,
    pub rows: Vec<Rect>,
}

/// The WLAN's fan centred on `(cx, cy)`, its widest arc reaching `r` from
/// the dot the arcs open up from: the first `level` arcs in `lit`, the
/// rest in `unlit`.
pub fn wlan_fan(c: &mut Canvas, cx: f32, cy: f32, r: f32, level: u8, lit: Rgba, unlit: Rgba) {
    let w = 0.17 * r;
    let dot = 0.13 * r;
    // From the top of the widest arc to the foot of the dot.
    let apex = cy + (r - dot) / 2.0;
    for (k, reach) in [0.42f32, 0.71, 1.0].into_iter().enumerate() {
        let color = if (k as u8) < level { lit } else { unlit };
        c.stroke_arc(cx, apex, r * reach - w / 2.0, -135f32.to_radians(), 90f32.to_radians(), w, color);
    }
    c.fill_circle(cx, apex, dot, lit);
}

/// A padlock `h` tall centred on `(cx, cy)`: its body, and the shackle
/// over it.
pub fn padlock(c: &mut Canvas, cx: f32, cy: f32, h: f32, color: Rgba) {
    let (bw, bh) = (0.84 * h, 0.6 * h);
    let sw = 0.13 * h;
    let shackle = 0.25 * h;
    let top = cy - h / 2.0;
    let body = top + h - bh;
    c.stroke_arc(cx, top + shackle + sw / 2.0, shackle, -180f32.to_radians(), 180f32.to_radians(), sw, color);
    c.line(cx - shackle, top + shackle + sw / 2.0, cx - shackle, body, sw, color);
    c.line(cx + shackle, top + shackle + sw / 2.0, cx + shackle, body, sw, color);
    c.fill_round_rect(cx - bw / 2.0, body, bw, bh, 0.12 * h, color);
}

/// The colours a card is drawn in: the page's own, or light on dark over the
/// About backdrop.
#[derive(Clone, Copy)]
pub struct Tone {
    pub card: Rgba,
    pub text: Rgba,
    pub sub: Rgba,
    pub value: Rgba,
    pub chevron: Rgba,
    pub caption: Rgba,
    /// The arcs of a WLAN fan its signal does not light.
    pub faint: Rgba,
    /// The round button at the end of a WLAN network's row, and its arrow.
    pub round: Rgba,
    pub round_mark: Rgba,
    /// The arrows after the value of a row that opens a list of choices.
    pub updown: Rgba,
}

impl Tone {
    pub fn page(t: &Theme) -> Tone {
        let p = &t.pal;
        Tone {
            card: p.card,
            text: p.text,
            sub: p.subtitle,
            value: p.value,
            chevron: p.chevron,
            caption: p.spec_label,
            faint: p.chevron,
            round: p.round,
            round_mark: p.round_mark,
            updown: p.updown,
        }
    }

    pub fn hero(t: &Theme) -> Tone {
        let h = &t.hero;
        Tone {
            card: h.card,
            text: h.title,
            sub: h.value,
            value: h.value,
            chevron: h.chevron,
            caption: h.value,
            faint: h.chevron,
            round: t.pal.round,
            round_mark: t.pal.round_mark,
            updown: t.pal.updown,
        }
    }

    /// White on the accent, as the network joined stands out on the WLAN
    /// page: what is not the title at 60%, the round button at 18%.
    pub fn lit(t: &Theme) -> Tone {
        let (accent, white) = (t.pal.accent, Rgba::rgb(255, 255, 255));
        let at = |k: f32| crate::color::mix(accent, white, k);
        Tone {
            card: accent,
            text: white,
            sub: at(0.6),
            value: at(0.6),
            chevron: white,
            caption: at(0.6),
            faint: at(0.35),
            round: at(0.18),
            round_mark: white,
            updown: at(0.6),
        }
    }
}

fn chevron(c: &mut Canvas, t: &Theme, right: f32, cy: f32, color: Rgba) {
    let m = &t.m;
    let (w, h, sw) = (m.chevron_w, m.chevron_h, m.chevron_stroke);
    let x0 = right - w + sw / 2.0;
    let x1 = right - sw / 2.0;
    c.line(x0, cy - h / 2.0 + sw / 2.0, x1, cy, sw, color);
    c.line(x1, cy, x0, cy + h / 2.0 - sw / 2.0, sw, color);
}

/// Punctuation that never starts a line; at the end of a full line it hangs
/// past the edge instead.
const NO_LINE_START: &str = "，。、；：！？）」』》%,.;:!?)]>”";

/// Opening punctuation that never ends a line; it moves down with what it
/// opens.
const NO_LINE_END: &str = "（「『《([<“";

pub(crate) fn wrap(c: &Canvas, face: &Face, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for ch in text.chars() {
        let mut probe = line.clone();
        probe.push(ch);
        if !line.is_empty() && c.text_width(face, &probe, px) > max_w {
            if NO_LINE_START.contains(ch) {
                line.push(ch);
                continue;
            }
            // A Latin word, file names, links and English words with an
            // apostrophe among them, moves to the next line whole, unless it
            // fills the line alone: then a path breaks after the last of its
            // slashes on the line, and any other word where the line ends.
            // An opening bracket in front of what moves goes with it.
            let word = |r: char| r.is_ascii_alphanumeric() || matches!(r, '_' | '-' | '.' | '/' | ':' | '\'');
            let mut split = line.len();
            if word(ch) {
                let start = line.char_indices().rev().take_while(|&(_, r)| word(r)).last().map(|(at, _)| at);
                match start {
                    Some(at) if at > 0 => split = at,
                    Some(_) => {
                        if let Some(slash) = line.rfind('/').filter(|&s| s > 0) {
                            split = slash + 1;
                        }
                    }
                    None => {}
                }
            }
            if let Some((at, before)) = line[..split].char_indices().last() {
                if at > 0 && NO_LINE_END.contains(before) {
                    split = at;
                }
            }
            let rest = line.split_off(split);
            lines.push(line.trim_end().to_string());
            line = rest;
            if !(line.is_empty() && ch == ' ') {
                line.push(ch);
            }
        } else {
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Status-bar contents: the clock and, when a fuel gauge is present, the
/// battery level and, while the cable is in, charging or full, the bolt
/// past it.
pub struct Bar<'a> {
    pub clock: &'a str,
    pub battery: Option<(u8, Option<Bolt>)>,
    /// The network joined, while it has an address.
    pub wlan: Option<WlanMark>,
    /// The data partition, open or not; nothing once it is wiped, when
    /// there is nothing in it to open.
    pub data: Option<bool>,
    /// The slot the system runs from, on an A/B phone.
    pub slot: Option<&'a str>,
    /// The battery's temperature, in tenths of a degree Celsius, which the
    /// bar gives to the tenth.
    pub temp: Option<i32>,
}

/// The bolt past the battery while the cable is in, as the stock bar draws
/// it: one step, or two while a charger charges quickly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bolt {
    Charging,
    Quick,
}

/// What the status bar shows of the network joined: how many of the fan's
/// three arcs its signal lights, whether data came in and went out in the
/// last second, and how fast in all, in figures over their unit.
pub struct WlanMark {
    pub level: u8,
    pub receiving: bool,
    pub sending: bool,
    pub speed: String,
    pub unit: &'static str,
}

/// Below this level the battery fills red while the cable is out.
pub const LOW_BATTERY: u8 = 20;

/// Clock on the left; on the right the battery as the stock status bar draws
/// it, with a bolt past it while the cable is in, and before it the network
/// joined and how fast data goes over it, the data partition's lock, the
/// slot in a tag as the stock bar tags a VPN, and the battery's temperature
/// in figures over their unit as the stock bar gives a speed. `ink` colours
/// the clock and every mark but the battery.
pub fn status_bar(c: &mut Canvas, t: &Theme, f: &Fonts, bar: &Bar, ink: Rgba) {
    let m = &t.m;
    let cy = m.status_cy;
    // Digits carry no descender, so centre on half their height (≈0.72 em).
    c.text(&f.status, bar.clock, m.status_x, cy + m.status_px * 0.36, m.status_px, ink, None);
    let mut right = m.w as f32 - m.status_right;
    if let Some((level, charging)) = bar.battery {
        if let Some(kind) = charging {
            let x = m.w as f32 - m.status_bolt_right - m.dp(BOLT_W_DP);
            let (art, ox, oy) = bolt(t, kind, x, cy);
            c.lay(&art, ox, oy, 1.0);
            right = x - m.dp(2.42);
        }
        let (art, ox, oy) = battery(t, f, level, charging.is_some(), right, cy);
        c.lay(&art, ox, oy, 1.0);
        right -= m.dp(BATTERY_W_DP + BATTERY_CAP_DP);
    }
    // From the battery leftwards: the network's fan, the data partition's
    // lock, the slot's badge, the temperature, and leftmost, as the stock
    // bar keeps it, how fast data goes.
    let gap = m.dp(MARK_GAP_DP);
    let mut left = right;
    if let Some(w) = &bar.wlan {
        left = wlan_mark(c, t, w, right, cy, ink);
    }
    if let Some(open) = bar.data {
        let (art, ox, oy, w) = lock_mark(t, open, left - gap, cy, ink);
        c.lay(&art, ox, oy, 1.0);
        left -= gap + w;
    }
    if let Some(slot) = bar.slot {
        let (art, ox, oy, w) = badge(t, f, slot, left - gap, cy, ink);
        c.lay(&art, ox, oy, 1.0);
        left -= gap + w;
    }
    if let Some(tenths) = bar.temp {
        left = speed(c, t, f, &fmt::degrees(tenths), "°C", left - gap, cy, ink);
    }
    if let Some(w) = &bar.wlan {
        speed(c, t, f, &w.speed, w.unit, left - gap, cy, ink);
    }
}

/// Room between the marks on the bar's right, as the stock bar leaves it
/// after its network speed and before its battery, in dp.
const MARK_GAP_DP: f32 = 4.33;

/// The stock status bar icons' corners, which ease into their sides: from
/// where a side starts to bend to where the next one is straight again,
/// three cubic pieces, in shares of how far along each side the bend
/// reaches, in the corner's own frame, the side coming in along x and the
/// side going out along y. Read off the stock NFC icon.
const SMOOTH_CORNER: [[(f32, f32); 4]; 3] = [
    [(-1.0, 0.0), (-0.65, 0.0), (-0.475, 0.0), (-0.341, 0.068)],
    [(-0.341, 0.068), (-0.2236, 0.128), (-0.128, 0.2236), (-0.068, 0.341)],
    [(-0.068, 0.341), (0.0, 0.475), (0.0, 0.65), (0.0, 1.0)],
];

/// Points taken along each cubic piece of a corner.
const SMOOTH_STEPS: usize = 4;

/// Where the cubic `piece` stands at `s` of the way along it.
fn cubic([p0, p1, p2, p3]: [(f32, f32); 4], s: f32) -> (f32, f32) {
    let r = 1.0 - s;
    let (a, b, c, d) = (r * r * r, 3.0 * r * r * s, 3.0 * r * s * s, s * s * s);
    (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
}

/// The outline of a rectangle whose corners ease into its sides as the
/// stock status bar's icons' do, bending `bend` along each side from each
/// corner: points round it, clockwise, for a convex fill.
fn smooth_rect(x: f32, y: f32, w: f32, h: f32, bend: f32) -> Vec<(f32, f32)> {
    // Each corner, the way the outline comes into it and the way it leaves.
    let corners = [
        ((x + w, y), (1.0, 0.0), (0.0, 1.0)),
        ((x + w, y + h), (0.0, 1.0), (-1.0, 0.0)),
        ((x, y + h), (-1.0, 0.0), (0.0, -1.0)),
        ((x, y), (0.0, -1.0), (1.0, 0.0)),
    ];
    let mut pts = Vec::with_capacity(4 * (3 * SMOOTH_STEPS + 1));
    for ((kx, ky), (ax, ay), (bx, by)) in corners {
        let at = |(u, v): (f32, f32)| (kx + (u * ax + v * bx) * bend, ky + (u * ay + v * by) * bend);
        for piece in SMOOTH_CORNER {
            pts.extend((0..SMOOTH_STEPS).map(|k| at(cubic(piece, k as f32 / SMOOTH_STEPS as f32))));
        }
        pts.push(at(SMOOTH_CORNER[2][3]));
    }
    pts
}

/// `shape`'s coverage less `cut`'s, in `ink`: a mark filled, its details
/// cut out of it, as the stock bar draws its icons.
fn knock_out(shape: &Canvas, cut: &Canvas, ink: Rgba) -> Canvas {
    let mut art = Canvas::new(shape.w, shape.h);
    for (i, px) in art.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (a, hole) = (shape.data[i * 4 + 3] as u32, cut.data[i * 4 + 3] as u32);
        *px = [ink.r, ink.g, ink.b, (a * (255 - hole) / 255 * ink.a as u32 / 255) as u8];
    }
    art
}

// The slot's badge, as the stock bar's NFC badge, in dp: its side, how far
// its corners bend, and how tall the capital cut out of it stands.
const BADGE_DP: f32 = 12.67;
const BADGE_BEND_DP: f32 = 3.27;
const BADGE_CAP_DP: f32 = 6.6;

/// `letter` cut out of a filled badge in `ink`, ending at `right` on the
/// bar's middle `cy`: its own canvas, where it goes, and how wide it is.
fn badge(t: &Theme, f: &Fonts, letter: &str, right: f32, cy: f32, ink: Rgba) -> (Canvas, i32, i32, f32) {
    let m = &t.m;
    let side = m.dp(BADGE_DP);
    // The size whose capitals stand that tall, from a size large enough to
    // measure them to a hundredth.
    let px = m.dp(BADGE_CAP_DP) * 100.0 / f.speed.cap(100.0);
    let (ox, oy) = ((right - side - 1.0).floor(), (cy - side / 2.0 - 1.0).floor());
    let n = ((right - ox + 1.0).ceil() as u32).max((cy + side / 2.0 - oy + 1.0).ceil() as u32);
    let (x, top) = (right - side - ox, cy - side / 2.0 - oy);
    let white = Rgba::rgb(255, 255, 255);
    let mut shape = Canvas::new(n, n);
    shape.fill_convex(&smooth_rect(x, top, side, side, m.dp(BADGE_BEND_DP)), white);
    // The letter centred in the badge by its ink.
    let mut cut = Canvas::new(n, n);
    let (l, r) = f.speed.ink_span(letter, px);
    cut.text(&f.speed, letter, x + side / 2.0 - (l + r) / 2.0, top + (side + m.dp(BADGE_CAP_DP)) / 2.0, px, white, None);
    (knock_out(&shape, &cut, ink), ox as i32, oy as i32, side)
}

// The data partition's lock, drawn as the stock bar draws its icons, in
// dp: how tall it stands shut, its body's width and height and how far the
// body's corners bend, its shackle's stroke and outer width, how far an
// open one rises and how far its right leg reaches below the arc, and the
// keyhole cut out of the body: its round, and the slot under it.
const LOCK_MARK_H_DP: f32 = 12.67;
const LOCK_BODY_W_DP: f32 = 10.0;
const LOCK_BODY_H_DP: f32 = 7.0;
const LOCK_BEND_DP: f32 = 2.0;
const LOCK_STROKE_DP: f32 = 1.5;
const LOCK_SHACKLE_W_DP: f32 = 7.0;
const LOCK_RISE_DP: f32 = 2.3;
const LOCK_STUB_DP: f32 = 0.9;
const KEYHOLE_R_DP: f32 = 1.0;
const KEYHOLE_SLOT_DP: (f32, f32) = (0.9, 1.9);

/// The data partition's lock in `ink`, shut, or `open` with its shackle
/// risen and its right leg out of the body, ending at `right` with the
/// shut lock on the bar's middle `cy`: its own canvas, where it goes, and
/// how wide it is.
fn lock_mark(t: &Theme, open: bool, right: f32, cy: f32, ink: Rgba) -> (Canvas, i32, i32, f32) {
    let m = &t.m;
    let (w, h, bh, sw) = (m.dp(LOCK_BODY_W_DP), m.dp(LOCK_MARK_H_DP), m.dp(LOCK_BODY_H_DP), m.dp(LOCK_STROKE_DP));
    // The canvas has room for the shackle risen, shut or not.
    let top = cy - h / 2.0;
    let (ox, oy) = ((right - w - 1.0).floor(), (top - m.dp(LOCK_RISE_DP) - 1.0).floor());
    let (cw, ch) = ((right - ox + 1.0).ceil() as u32, (top + h - oy + 1.0).ceil() as u32);
    let (x, top) = (right - w - ox, top - oy);
    let (mid, body) = (x + w / 2.0, top + h - bh);
    // The arc through the shackle's middle, and where it centres.
    let r = (m.dp(LOCK_SHACKLE_W_DP) - sw) / 2.0;
    let crown = top + sw / 2.0 + r - if open { m.dp(LOCK_RISE_DP) } else { 0.0 };
    let white = Rgba::rgb(255, 255, 255);
    let mut shape = Canvas::new(cw, ch);
    shape.fill_convex(&smooth_rect(x, body, w, bh, m.dp(LOCK_BEND_DP)), white);
    shape.stroke_arc(mid, crown, r, std::f32::consts::PI, std::f32::consts::PI, sw, white);
    shape.line(mid - r, crown, mid - r, body + sw, sw, white);
    let foot = if open { crown + m.dp(LOCK_STUB_DP) } else { body + sw };
    shape.line(mid + r, crown, mid + r, foot, sw, white);
    let mut hole = Canvas::new(cw, ch);
    let (round, (slot_w, slot_h)) = (body + bh * 0.42, (m.dp(KEYHOLE_SLOT_DP.0), m.dp(KEYHOLE_SLOT_DP.1)));
    hole.fill_circle(mid, round, m.dp(KEYHOLE_R_DP), white);
    hole.fill_round_rect(mid - slot_w / 2.0, round, slot_w, slot_h, slot_w / 2.0, white);
    (knock_out(&shape, &hole, ink), ox as i32, oy as i32, w)
}

// The network's marks in the status bar, as the stock one draws them, in
// dp: the arrows' middle in from what is right of them, and the fan's point
// left of the arrows and below the bar's middle; the fan's slice and its
// inner and outer arc's edges from its point, either side of straight up;
// and the arrows' tips and bases from the fan's point, either half of their
// width, and how round their tips and other corners are.
const ARROWS_IN_DP: f32 = 5.57;
const FAN_LEFT_OF_ARROWS_DP: f32 = 7.0;
const FAN_POINT_DP: f32 = 6.06;
const FAN_SLICE_DP: f32 = 3.81;
const FAN_INNER_DP: (f32, f32) = (5.49, 7.83);
const FAN_OUTER_DP: (f32, f32) = (9.51, 11.83);
const FAN_HALF: f32 = std::f32::consts::FRAC_PI_4;
const ARROW_UP_DP: (f32, f32) = (-5.46, -2.84);
const ARROW_DOWN_DP: (f32, f32) = (-2.09, 0.58);
const ARROW_HALF_DP: f32 = 1.83;
const ARROW_ROUND_DP: (f32, f32) = (0.63, 0.33);

/// The network joined, left of `right` and on the bar's middle `cy`: the
/// fan, its arcs lit as far as the signal reaches, and the arrows out and
/// in, each lit while data goes that way. Returns where it starts.
fn wlan_mark(c: &mut Canvas, t: &Theme, w: &WlanMark, right: f32, cy: f32, ink: Rgba) -> f32 {
    let m = &t.m;
    let dim = t.pal.battery.empty;
    let ax = right - m.dp(ARROWS_IN_DP);
    let (fx, point) = (ax - m.dp(FAN_LEFT_OF_ARROWS_DP), cy + m.dp(FAN_POINT_DP));
    // The slice is lit whenever a network is joined.
    c.fill_sector(fx, point, 0.0, m.dp(FAN_SLICE_DP), FAN_HALF, ink);
    for (arc, from) in [(FAN_INNER_DP, 2), (FAN_OUTER_DP, 3)] {
        c.fill_sector(fx, point, m.dp(arc.0), m.dp(arc.1), FAN_HALF, if w.level >= from { ink } else { dim });
    }
    let half = m.dp(ARROW_HALF_DP);
    let (tip, corner) = (m.dp(ARROW_ROUND_DP.0), m.dp(ARROW_ROUND_DP.1));
    let (up, up_base) = (point + m.dp(ARROW_UP_DP.0), point + m.dp(ARROW_UP_DP.1));
    let (down_base, down) = (point + m.dp(ARROW_DOWN_DP.0), point + m.dp(ARROW_DOWN_DP.1));
    let out = [(ax, up), (ax + half, up_base), (ax - half, up_base)];
    round_triangle(c, out, [tip, corner, corner], if w.sending { ink } else { dim });
    let into = [(ax, down), (ax - half, down_base), (ax + half, down_base)];
    round_triangle(c, into, [tip, corner, corner], if w.receiving { ink } else { dim });
    fx - m.dp(FAN_OUTER_DP.1) * FAN_HALF.sin()
}

/// A triangle with each corner rounded to its own radius in `radii`.
fn round_triangle(c: &mut Canvas, corners: [(f32, f32); 3], radii: [f32; 3], color: Rgba) {
    let centre = ((corners[0].0 + corners[1].0 + corners[2].0) / 3.0, (corners[0].1 + corners[1].1 + corners[2].1) / 3.0);
    // The normal of the edge from `p` to `q` that points away from the
    // middle.
    let outward = |p: (f32, f32), q: (f32, f32)| {
        let (ex, ey) = (q.0 - p.0, q.1 - p.1);
        let len = ex.hypot(ey);
        let n = (ey / len, -ex / len);
        let mid = ((p.0 + q.0) / 2.0 - centre.0, (p.1 + q.1) / 2.0 - centre.1);
        if n.0 * mid.0 + n.1 * mid.1 < 0.0 {
            (-n.0, -n.1)
        } else {
            n
        }
    };
    let mut outline = Vec::new();
    for (i, r) in radii.into_iter().enumerate() {
        let (p, v, q) = (corners[(i + 2) % 3], corners[i], corners[(i + 1) % 3]);
        let (a, b) = (outward(p, v), outward(v, q));
        // The corner's circle sits `r` in from both its edges.
        let (sx, sy) = (a.0 + b.0, a.1 + b.1);
        let k = 2.0 * r / (sx * sx + sy * sy);
        let (ox, oy) = (v.0 - sx * k, v.1 - sy * k);
        let (from, to) = (a.1.atan2(a.0), b.1.atan2(b.0));
        let turn = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        for s in 0..=4 {
            let at = from + turn * s as f32 / 4.0;
            outline.push((ox + r * at.cos(), oy + r * at.sin()));
        }
    }
    c.fill_convex(&outline, color);
}

// How fast data goes, as the stock status bar writes it, in dp: the size
// of its figures and of their unit, and their baselines from the bar's
// middle.
const SPEED_PX_DP: f32 = 6.9;
const SPEED_UNIT_PX_DP: f32 = 6.23;
const SPEED_BASES_DP: (f32, f32) = (-2.03, 5.97);

/// Figures over their unit, as the stock bar tells how fast data goes: the
/// two lines centred on one another and ending at `right`. Returns where
/// they start.
#[allow(clippy::too_many_arguments)]
fn speed(c: &mut Canvas, t: &Theme, f: &Fonts, figures: &str, unit: &str, right: f32, cy: f32, ink: Rgba) -> f32 {
    let m = &t.m;
    let (px, unit_px) = (m.dp(SPEED_PX_DP), m.dp(SPEED_UNIT_PX_DP));
    // Each line placed by its ink, the room between marks being the room
    // between their ink.
    let (a, b) = (f.speed.ink_span(figures, px), f.speed.ink_span(unit, unit_px));
    let wide = (a.1 - a.0).max(b.1 - b.0);
    let mid = right - wide / 2.0;
    c.text(&f.speed, figures, mid - (a.0 + a.1) / 2.0, cy + m.dp(SPEED_BASES_DP.0), px, ink, None);
    c.text(&f.speed, unit, mid - (b.0 + b.1) / 2.0, cy + m.dp(SPEED_BASES_DP.1), unit_px, ink, None);
    right - wide
}

/// The stock bar's box for the bolt, as wide as the bolt's drawing and as
/// tall as the bar's icons' boxes.
const BOLT_W_DP: f32 = 8.0;
const BOLT_H_DP: f32 = 20.0;
const BATTERY_W_DP: f32 = 21.75;
const BATTERY_CAP_DP: f32 = 1.3;

/// A piece of an outline from where the last one ended: a line to a point,
/// or a cubic through two control points to one.
#[derive(Clone, Copy)]
enum Piece {
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
}

/// A closed contour: where it starts, and its pieces round back to there.
type Contour = ((f32, f32), &'static [Piece]);

/// The stock bar's bolts as its SystemUI draws them past the battery, in
/// their box a unit a dp: one step while charging, and two while a charger
/// charges quickly.
const BOLT: Contour = (
    (4.923, 4.722),
    &[
        Piece::Cubic(5.033, 4.722, 5.113, 4.827, 5.083, 4.933),
        Piece::Line(3.902, 9.162),
        Piece::Line(5.655, 9.162),
        Piece::Cubic(5.902, 9.162, 6.026, 9.162, 6.091, 9.213),
        Piece::Cubic(6.148, 9.258, 6.181, 9.325, 6.183, 9.397),
        Piece::Cubic(6.185, 9.48, 6.11, 9.579, 5.962, 9.777),
        Piece::Line(1.882, 15.217),
        Piece::Cubic(1.853, 15.256, 1.808, 15.278, 1.761, 15.278),
        Piece::Cubic(1.661, 15.278, 1.588, 15.182, 1.615, 15.086),
        Piece::Line(2.81, 10.838),
        Piece::Line(0.794, 10.838),
        Piece::Cubic(0.54, 10.838, 0.413, 10.838, 0.347, 10.785),
        Piece::Cubic(0.29, 10.74, 0.256, 10.671, 0.256, 10.598),
        Piece::Cubic(0.256, 10.514, 0.335, 10.415, 0.493, 10.215),
        Piece::Line(4.792, 4.785),
        Piece::Cubic(4.823, 4.745, 4.872, 4.722, 4.923, 4.722),
    ],
);
const QUICK_BOLT: Contour = (
    (4.557, 5.191),
    &[
        Piece::Cubic(4.75, 4.68, 4.847, 4.425, 4.792, 4.348),
        Piece::Cubic(4.763, 4.307, 4.718, 4.281, 4.668, 4.275),
        Piece::Cubic(4.575, 4.265, 4.399, 4.474, 4.048, 4.893),
        Piece::Line(0.652, 8.942),
        Piece::Cubic(0.423, 9.215, 0.308, 9.352, 0.32, 9.468),
        Piece::Cubic(0.326, 9.531, 0.354, 9.591, 0.398, 9.636),
        Piece::Cubic(0.48, 9.719, 0.658, 9.719, 1.015, 9.719),
        Piece::Line(2.404, 9.719),
        Piece::Line(0.8, 12.061),
        Piece::Cubic(0.619, 12.326, 0.529, 12.458, 0.546, 12.568),
        Piece::Cubic(0.556, 12.628, 0.585, 12.683, 0.629, 12.725),
        Piece::Cubic(0.71, 12.802, 0.87, 12.802, 1.191, 12.802),
        Piece::Line(2.361, 12.802),
        Piece::Line(1.604, 14.811),
        Piece::Cubic(1.412, 15.321, 1.315, 15.576, 1.37, 15.653),
        Piece::Cubic(1.399, 15.693, 1.444, 15.72, 1.494, 15.725),
        Piece::Cubic(1.588, 15.735, 1.763, 15.526, 2.113, 15.108),
        Piece::Line(5.509, 11.057),
        Piece::Cubic(5.737, 10.784, 5.852, 10.648, 5.841, 10.532),
        Piece::Cubic(5.834, 10.468, 5.807, 10.409, 5.762, 10.363),
        Piece::Cubic(5.68, 10.28, 5.502, 10.28, 5.146, 10.28),
        Piece::Line(3.761, 10.28),
        Piece::Line(5.361, 7.938),
        Piece::Cubic(5.542, 7.673, 5.632, 7.541, 5.614, 7.432),
        Piece::Cubic(5.605, 7.371, 5.576, 7.316, 5.531, 7.274),
        Piece::Cubic(5.451, 7.198, 5.291, 7.198, 4.971, 7.198),
        Piece::Line(3.799, 7.198),
        Piece::Line(4.557, 5.191),
    ],
);

/// The stock bar's bolt of `kind` in its box from `x`, on the bar's middle
/// `cy`: as its own canvas in the bolt's ink, and where it goes.
fn bolt(t: &Theme, kind: Bolt, x: f32, cy: f32) -> (Canvas, i32, i32) {
    let m = &t.m;
    let (start, pieces) = match kind {
        Bolt::Charging => BOLT,
        Bolt::Quick => QUICK_BOLT,
    };
    // The box's corner on whole pixels, as the stock bar lays its views
    // out, and the canvas over the box.
    let (ox, oy) = (x.round(), (cy - m.dp(BOLT_H_DP) / 2.0).round());
    let (w, h) = (m.dp(BOLT_W_DP).ceil() as u32, m.dp(BOLT_H_DP).ceil() as u32);
    let at = |(u, v): (f32, f32)| ab_glyph_rasterizer::point(m.dp(u), m.dp(v));
    let mut fill = ab_glyph_rasterizer::Rasterizer::new(w as usize, h as usize);
    let mut from = at(start);
    for piece in pieces {
        from = match *piece {
            Piece::Line(u, v) => {
                fill.draw_line(from, at((u, v)));
                at((u, v))
            }
            Piece::Cubic(a, b, c, d, u, v) => {
                fill.draw_cubic(from, at((a, b)), at((c, d)), at((u, v)));
                at((u, v))
            }
        };
    }
    fill.draw_line(from, at(start));
    let ink = t.pal.battery.bolt;
    let mut art = Canvas::new(w, h);
    fill.for_each_pixel_2d(|px, py, cover| {
        let i = ((py * w + px) * 4) as usize;
        let a = (cover.clamp(0.0, 1.0) * ink.a as f32).round() as u8;
        art.data[i..i + 4].copy_from_slice(&[ink.r, ink.g, ink.b, a]);
    });
    (art, ox as i32, oy as i32)
}

/// The battery ending at `right`, centred on `cy`, as its own canvas and
/// where it goes: body and cap, the level's share filled from the left, the
/// digits in white over a charging or low fill and cut out of any other.
fn battery(t: &Theme, f: &Fonts, level: u8, plugged: bool, right: f32, cy: f32) -> (Canvas, i32, i32) {
    let m = &t.m;
    let ink = t.pal.battery;
    let (bw, bh, cap_w, cap_h) = (m.dp(BATTERY_W_DP), m.dp(12.1), m.dp(BATTERY_CAP_DP), m.dp(4.25));
    let left = right - bw - cap_w;
    let (ox, oy) = ((left - 1.0).floor(), (cy - bh / 2.0 - 1.0).floor());
    let (w, h) = ((right - ox + 1.0).ceil() as u32, (cy + bh / 2.0 - oy + 1.0).ceil() as u32);
    let (x, top, mid) = (left - ox, cy - bh / 2.0 - oy, cy - oy);
    // Coverage of the body with its cap, and of the digits.
    let white = Rgba::rgb(255, 255, 255);
    let mut shape = Canvas::new(w, h);
    shape.fill_round_rect(x, top, bw, bh, m.dp(3.7), white);
    shape.fill_round_rect(x + bw - cap_w, mid - cap_h / 2.0, cap_w * 2.0, cap_h, cap_w, white);
    let mut digits = Canvas::new(w, h);
    digits.text_center(&f.strong, &level.to_string(), x + bw / 2.0, mid, m.dp(9.7), white);
    // The level fills its share of the whole width, the cap's included, so
    // a full battery fills the cap too.
    let edge = x + (bw + cap_w) * level as f32 / 100.0;
    let low = !plugged && level < LOW_BATTERY;
    let fill = if plugged {
        ink.charging
    } else if low {
        ink.low
    } else {
        ink.level
    };
    let mut art = Canvas::new(w, h);
    for (i, px) in art.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let a = shape.data[i * 4 + 3] as f32 / 255.0;
        if a == 0.0 {
            continue;
        }
        let k = (edge - (i as u32 % w) as f32).clamp(0.0, 1.0);
        let base = mix(ink.empty, fill, k);
        *px = [base.r, base.g, base.b, base.a];
        let d = digits.data[i * 4 + 3] as f32 / 255.0;
        if plugged || low {
            blend_over(px, white, d);
        } else {
            px[3] = (px[3] as f32 * (1.0 - d)).round() as u8;
        }
        px[3] = (px[3] as f32 * a).round() as u8;
    }
    (art, ox as i32, oy as i32)
}

/// The `<` back control.
pub fn back_arrow(c: &mut Canvas, t: &Theme, color: Rgba) {
    arrow_at(c, t, 0.0, 0.0, color);
}

/// The `<` moved by (`dx`, `dy`).
fn arrow_at(c: &mut Canvas, t: &Theme, dx: f32, dy: f32, color: Rgba) {
    let m = &t.m;
    let (x, cy, w, h, sw) = (m.back_x + dx, m.back_cy + dy, m.back_w, m.back_h, m.back_stroke);
    let (tip, end) = (x + sw / 2.0, x + w - sw / 2.0);
    c.line(tip, cy, end, cy - h / 2.0 + sw / 2.0, sw, color);
    c.line(tip, cy, end, cy + h / 2.0 - sw / 2.0, sw, color);
}

/// The area that answers a tap on the `<` control.
pub fn back_rect(t: &Theme) -> Rect {
    let m = &t.m;
    Rect::new(0.0, m.back_cy - m.dp(28.0), m.back_x + m.back_w + m.dp(24.0), m.dp(56.0))
}

/// How deep the back cue bulges for a finger `travel` pixels in from the
/// edge: about 0.31 of the travel at first, flattening out at 42 dp.
pub fn back_cue_depth(t: &Theme, travel: f32) -> f32 {
    let d = (travel / t.m.px_per_dp).clamp(0.0, 272.6);
    t.m.dp(0.3096 * d - 5.679e-4 * d * d)
}

/// The back cue swelling from a screen edge while a finger swipes in from
/// it: a dark wave `depth` pixels deep at its crest around `cy`, edged in
/// light blue, holding a white `<` once lifting there would go back. The
/// `<` points back on either edge.
pub fn back_cue(c: &mut Canvas, t: &Theme, from_left: bool, cy: f32, depth: f32, arrow: bool) {
    const FILL: Rgba = Rgba::rgb(48, 48, 48);
    const RIM: Rgba = Rgba::rgb(117, 170, 246);
    let m = &t.m;
    if depth <= 0.0 {
        return;
    }
    // A Gaussian bell narrowed to nothing past 93 dp from its crest; the
    // rim hugs it and runs on along the edge as a thin line a little
    // further.
    let sigma = m.dp(60.7);
    let rim = m.dp(0.77);
    let taper = |dy: f32, from: f32| {
        let k = ((dy.abs() - from) / m.dp(24.0)).clamp(0.0, 1.0);
        1.0 - k * k * (3.0 - 2.0 * k)
    };
    let (fill_from, rim_from) = (m.dp(93.0), m.dp(111.0));
    let reach = rim_from + m.dp(24.0);
    let span = |c: &mut Canvas, y: i32, w: f32, color: Rgba| {
        let full = w.floor() as i32;
        for i in 0..=full {
            let cov = if i < full { 1.0 } else { w - full as f32 };
            let x = if from_left { i } else { m.w as i32 - 1 - i };
            c.blend(x, y, color, cov);
        }
    };
    for y in (cy - reach).floor() as i32..(cy + reach).ceil() as i32 {
        let dy = y as f32 + 0.5 - cy;
        let fill = depth * (-(dy / sigma).powi(2)).exp() * taper(dy, fill_from);
        span(c, y, fill + rim * taper(dy, rim_from), RIM);
        span(c, y, fill, FILL);
    }
    if arrow {
        let (w, h, sw) = (m.dp(6.0), m.dp(12.7), m.dp(2.1));
        let cx = if from_left { 0.53 * depth } else { m.w as f32 - 0.53 * depth };
        let (tip, end) = (cx - w / 2.0 + sw / 2.0, cx + w / 2.0 - sw / 2.0);
        let white = Rgba::rgb(255, 255, 255);
        c.line(tip, cy, end, cy - h / 2.0 + sw / 2.0, sw, white);
        c.line(tip, cy, end, cy + h / 2.0 - sw / 2.0, sw, white);
    }
}

/// Room left around title ink for it to blur into.
fn blur_room(t: &Theme) -> f32 {
    t.m.dp(14.0)
}

/// The large title a page opens with, as ink that fades and blurs away
/// while the page scrolls; it sits where the page starts and scrolls with
/// it.
pub fn big_title(t: &Theme, f: &Fonts, title: &str) -> Sprite {
    let m = &t.m;
    let pad = blur_room(t);
    let w = Canvas::new(0, 0).text_width(&f.title, title, m.title_px);
    let (x, y) = (m.title_x - pad, m.title_top - pad);
    let size = ((w + pad * 2.0).ceil() as u32, (m.title_px * 1.4 + pad * 2.0).ceil() as u32);
    Sprite::capture(x, y, size.0, size.1, |c, dx, dy| {
        c.text_top(&f.title, title, m.title_x + dx, m.title_top + dy, m.title_px, Rgba::rgb(0, 0, 0), None);
    })
}

/// The page title centred in the bar over a scrolled page, as ink that
/// fades and sharpens in.
pub fn small_title(t: &Theme, f: &Fonts, title: &str) -> Sprite {
    let m = &t.m;
    let (px, pad) = (m.account_title_px, blur_room(t));
    let w = Canvas::new(0, 0).text_width(&f.heading, title, px);
    let h = px * 1.5;
    let (x, y) = ((m.w as f32 - w) / 2.0 - pad, m.back_cy - h / 2.0 - pad);
    Sprite::capture(x, y, (w + pad * 2.0).ceil() as u32, (h + pad * 2.0).ceil() as u32, |c, dx, dy| {
        c.text_center(&f.heading, title, m.w as f32 / 2.0 + dx, m.back_cy + dy, px, Rgba::rgb(0, 0, 0));
    })
}

/// Where the bar over a scrolled page ends: the status bar and the 56 dp
/// title bar under it.
pub fn bar_bottom(t: &Theme) -> f32 {
    t.m.back_cy + t.m.dp(28.0)
}

/// Strength of the bar's mask from its top (0) to 20 dp below the bar (1).
pub const BAR_MASK: [(f32, f32); 17] = [
    (0.0, 0.98),
    (0.5, 0.98),
    (0.555, 0.97),
    (0.605, 0.94),
    (0.65, 0.91),
    (0.69, 0.86),
    (0.725, 0.8),
    (0.76, 0.74),
    (0.785, 0.67),
    (0.815, 0.59),
    (0.84, 0.51),
    (0.865, 0.42),
    (0.89, 0.34),
    (0.915, 0.25),
    (0.94, 0.16),
    (0.97, 0.08),
    (1.0, 0.0),
];

/// The About page's mask, solid at its top.
pub const ABOUT_BAR_MASK: [(f32, f32); 5] = [(0.0, 1.0), (0.52, 0.98), (0.74, 0.8), (0.85, 0.5), (1.0, 0.0)];

/// The mask over the top of a scrolled page: `color`, nearly solid behind
/// the status bar and title, thinning out by 20 dp below them, with an
/// ordered dither against banding. `level` brings it in and out.
pub fn bar_mask(c: &mut Canvas, t: &Theme, stops: &[(f32, f32)], color: Rgba, level: f32) {
    mask(c, bar_bottom(t) + t.m.dp(20.0), stops, color, level, 0..0);
}

/// How far below a pinned strip the list thins out from under the mask, as
/// the stock file manager's does.
const STRIP_FADE_DP: f32 = 40.0;

/// The mask over a scrolled page with an opaque strip from `top` to
/// `bottom` held under the bar: as solid as the bar's own down to the strip,
/// then thinning out below it the way the bar's mask does. The strip's own
/// rows are left to it.
pub fn strip_mask(c: &mut Canvas, t: &Theme, color: Rgba, level: f32, top: f32, bottom: f32) {
    let h = bottom + t.m.dp(STRIP_FADE_DP);
    let s = bottom / h;
    // The bar mask from where it starts to thin, stretched over the fade.
    let (solid, _) = BAR_MASK.iter().rev().find(|&&(_, a)| a == BAR_MASK[0].1).expect("the bar mask starts solid");
    let mut stops = vec![(0.0, BAR_MASK[0].1)];
    stops.extend(BAR_MASK.iter().filter(|&&(p, _)| p >= *solid).map(|&(p, a)| (s + (p - solid) / (1.0 - solid) * (1.0 - s), a)));
    mask(c, h, &stops, color, level, top.max(0.0) as u32..bottom.max(0.0) as u32);
}

/// `color` over the top `h` pixels at the strength `stops` give from top (0)
/// to bottom (1), with an ordered dither against banding; the rows in `skip`
/// are left as they are.
fn mask(c: &mut Canvas, h: f32, stops: &[(f32, f32)], color: Rgba, level: f32, skip: std::ops::Range<u32>) {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    assert!(c.whole(), "the mask lies over a whole screen");
    let ink = [color.r as u32, color.g as u32, color.b as u32];
    let row_len = c.w as usize * 4;
    for y in (0..(h.ceil() as u32).min(c.h)).filter(|y| !skip.contains(y)) {
        let f = (y as f32 + 0.5) / h;
        let a = ramp(stops, f) * level * 255.0;
        if a <= 0.0 {
            continue;
        }
        // The dither shifts the opacity by under half a step, in a 4×4
        // pattern. Opacities run 0 to 256 so a shift divides by them.
        let k: [u32; 4] = std::array::from_fn(|i| {
            let k = (a + BAYER[y as usize % 4][i] as f32 / 16.0 - 0.5).clamp(0.0, 255.0).round() as u32;
            k + (k >> 7)
        });
        // Each pixel as a word, its red and blue mixed towards the ink in
        // one multiply, then its green; its alpha kept.
        let lit: [(u32, u32, u32); 4] = k.map(|k| ((ink[0] | ink[2] << 16) * k + 0x0080_0080, ink[1] * k + 0x80, 256 - k));
        let start = y as usize * row_len;
        for quad in c.data[start..start + row_len].as_chunks_mut::<16>().0 {
            for (px, &(lit_rb, lit_g, keep)) in quad.as_chunks_mut::<4>().0.iter_mut().zip(&lit) {
                let v = u32::from_le_bytes(*px);
                let rb = ((lit_rb + (v & 0x00ff_00ff) * keep) >> 8) & 0x00ff_00ff;
                let g = ((lit_g + ((v >> 8) & 0xff) * keep) >> 8) & 0xff;
                *px = (rb | (g << 8) | (v & 0xff00_0000)).to_le_bytes();
            }
        }
    }
}

/// The piecewise-linear value of `stops` at `f`.
fn ramp(stops: &[(f32, f32)], f: f32) -> f32 {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if f <= first.0 {
        return first.1;
    }
    if f >= last.0 {
        return last.1;
    }
    let i = stops.iter().position(|s| s.0 > f).expect("f lies inside the stops");
    let ((p0, a0), (p1, a1)) = (stops[i - 1], stops[i]);
    a0 + (a1 - a0) * (f - p0) / (p1 - p0)
}

/// The centre and radius of the round chip behind `<`.
pub fn back_disc(t: &Theme) -> (f32, f32, f32) {
    let m = &t.m;
    (m.back_x + m.back_w / 2.0 + m.dp(1.55), m.back_cy, m.dp(21.7))
}

/// The round chip behind `<` while the bar shows: a pale disc with a
/// bright rim and a faint shadow. Drawn once per theme on its own canvas,
/// returned with where that canvas's top-left corner goes.
pub fn back_chip(t: &Theme) -> (Canvas, i32, i32) {
    chip(t, back_disc(t))
}

/// Such a chip about the disc centred at (`cx`, `cy`) of radius `r`.
fn chip(t: &Theme, (cx, cy, r): (f32, f32, f32)) -> (Canvas, i32, i32) {
    let m = &t.m;
    let (fill, rim) = if t.dark { (Rgba::rgb(38, 38, 38), Rgba::rgb(62, 62, 62)) } else { (Rgba::rgb(248, 248, 248), Rgba::rgb(255, 255, 255)) };
    let spread = m.dp(7.0);
    let reach = r + spread;
    let (x0, y0) = ((cx - reach).floor() as i32 - 1, (cy - reach).floor() as i32 - 1);
    let size = (reach * 2.0).ceil() as u32 + 3;
    let mut c = Canvas::new(size, size);
    let (cx, cy) = (cx - x0 as f32, cy - y0 as f32);
    for py in 0..size as i32 {
        for px in 0..size as i32 {
            let d = (px as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy) - r;
            if d > 0.0 && d < spread {
                let k = 1.0 - d / spread;
                c.blend(px, py, Rgba::rgb(0, 0, 0), 0.04 * k * k);
            }
        }
    }
    c.fill_circle(cx, cy, r, fill);
    let rim_w = m.dp(0.8);
    c.stroke_circle(cx, cy, r - rim_w / 2.0, rim_w, rim);
    (c, x0, y0)
}

/// A short message in a dark capsule above the bottom edge.
pub fn toast(c: &mut Canvas, t: &Theme, f: &Fonts, text: &str) {
    let m = &t.m;
    let px = m.sub_px;
    let w = c.text_width(&f.heading, text, px) + m.dp(40.0);
    let h = m.dp(44.0);
    let (x, y) = ((m.w as f32 - w) / 2.0, m.h as f32 - m.dp(120.0));
    c.fill_round_rect(x, y, w, h, h / 2.0, Rgba::rgb(30, 30, 30).with_alpha(235));
    c.text_center(&f.heading, text, m.w as f32 / 2.0, y + h / 2.0, px, Rgba::rgb(255, 255, 255));
}

/// The home page's search bar in `r`, at `level` of its strength, without
/// its icon and words: those are laid over it from [`search_words`].
pub fn search_bar(c: &mut Canvas, t: &Theme, r: Rect, level: f32) {
    let ink = t.pal.search;
    c.fill_round_rect(r.x, r.y, r.w, r.h, r.h / 2.0, ink.with_alpha((ink.a as f32 * level).round() as u8));
}

/// The icon and words of the home page's search bar standing at `bar`, on
/// their own to lay over it, the layer's top-left corner at `bar`'s taken
/// down to whole pixels.
pub fn search_words(t: &Theme, f: &Fonts, bar: Rect, placeholder: &str) -> Canvas {
    let m = &t.m;
    let (left, top) = (bar.x.floor(), bar.y.floor());
    let mut c = Canvas::new((bar.x + bar.w - left).ceil() as u32, (bar.y + bar.h - top).ceil() as u32);
    let (x, cy) = (bar.x - left, bar.y - top + bar.h / 2.0);
    let (icx, icy, sw) = (x + m.dp(22.3), cy - m.dp(2.0), m.dp(1.45));
    let ring = m.dp(6.6);
    c.stroke_circle(icx, icy, ring, sw, t.pal.search_icon);
    let k = ring * std::f32::consts::FRAC_1_SQRT_2;
    c.line(icx + k, icy + k, icx + m.dp(9.9), icy + m.dp(10.0), sw, t.pal.search_icon);
    // The placeholder is set at the row-title size and weight, in grey.
    c.text_vc(&f.heading, placeholder, x + m.search_text_inset, cy, m.row_title_px, t.pal.placeholder, None);
    c
}

/// What a search field shows and where its parts landed.
pub struct SearchField {
    pub field: Rect,
    /// The clear button, while there is text to clear.
    pub clear: Option<Rect>,
    pub caret: Caret,
}

/// A text field's cursor: the stretch of the field it stands in, drawn
/// again with it, and where that stretch goes. The field itself is drawn
/// without it, so the cursor blinks as this is laid over the field or not.
pub struct Caret {
    pub art: Canvas,
    pub x: i32,
    pub y: i32,
}

/// How wide a text cursor is and how round its ends are, in dp.
const CARET_W_DP: f32 = 2.0;
const CARET_RADIUS_DP: f32 = 0.35;

/// The search page, as the stock one lays it out: how far down its field
/// stands, and the round button beside it that leaves the search, its
/// distance from the screen's edge and from the field, and its cross.
const SEARCH_TOP_DP: f32 = 51.7;
const CANCEL_RIGHT_DP: f32 = 14.0;
const CANCEL_GAP_DP: f32 = 10.0;
const CANCEL_ARM_DP: f32 = 8.0;
const CANCEL_STROKE_DP: f32 = 1.8;

/// Where the search page puts its field and the button that leaves the
/// search.
pub fn search_layout(t: &Theme) -> (Rect, Rect) {
    let m = &t.m;
    let (top, d) = (m.dp(SEARCH_TOP_DP), m.search_h);
    let cancel = Rect::new(m.w as f32 - m.dp(CANCEL_RIGHT_DP) - d, top, d, d);
    (Rect::new(m.margin, top, cancel.x - m.dp(CANCEL_GAP_DP) - m.margin, d), cancel)
}

/// The round button that leaves the search, in `r`.
pub fn cancel_button(c: &mut Canvas, t: &Theme, r: Rect) {
    draw_disc(c, r, t.pal.search);
    draw_cross(c, t, r, t.pal.text);
}

/// How strongly `<` and the cross of the search page's round button show
/// while held down, sampled from the stock settings: black at 61% and at
/// 66% in the light theme.
const BACK_HELD_ALPHA: u8 = 156;
const CANCEL_HELD_ALPHA: u8 = 168;

/// The `<` held down: a disc darkening what is behind it, and the `<`
/// lighter on it. The see-through `<` is inked once where its strokes
/// meet, as one shape.
pub fn back_held(c: &mut Canvas, t: &Theme) {
    let (cx, cy, r) = back_disc(t);
    c.fill_circle(cx, cy, r, t.pal.button_press);
    let side = (r * 2.0).ceil() as u32 + 2;
    let arrow = Sprite::capture(cx - r - 1.0, cy - r - 1.0, side, side, |c, dx, dy| arrow_at(c, t, dx, dy, Rgba::rgb(0, 0, 0)));
    arrow.draw(c, 0.0, 0.0, t.pal.text, BACK_HELD_ALPHA as f32 / 255.0);
}

/// What the round button at the right end of the bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// A prompt and a cursor: a terminal.
    Terminal,
    /// A tick: keep what was done.
    Done,
}

/// The centre and radius of the round button at the right end of the bar,
/// the `<` chip mirrored.
pub fn button_disc(t: &Theme) -> (f32, f32, f32) {
    let (cx, cy, r) = back_disc(t);
    (t.m.w as f32 - cx, cy, r)
}

/// The area that answers a tap on that button, the `<` control's mirrored.
pub fn button_rect(t: &Theme) -> Rect {
    let b = back_rect(t);
    Rect::new(t.m.w as f32 - b.x - b.w, b.y, b.w, b.h)
}

/// The chip behind that button, as the one behind `<`.
pub fn button_chip(t: &Theme) -> (Canvas, i32, i32) {
    chip(t, button_disc(t))
}

/// The button's glyph, in strokes as heavy as the `<`'s.
pub fn button_glyph(c: &mut Canvas, t: &Theme, glyph: Glyph, color: Rgba) {
    glyph_at(c, t, glyph, 0.0, 0.0, color);
}

/// The glyph moved by (`dx`, `dy`), its strokes spanning the same box about
/// the disc's centre either way.
fn glyph_at(c: &mut Canvas, t: &Theme, glyph: Glyph, dx: f32, dy: f32, color: Rgba) {
    let m = &t.m;
    let (cx, cy, _) = button_disc(t);
    let (cx, cy, sw) = (cx + dx, cy + dy, m.back_stroke);
    let at = |x: f32, y: f32| (cx + m.dp(x), cy + m.dp(y));
    let mut stroke = |(x0, y0): (f32, f32), (x1, y1): (f32, f32)| c.line(x0, y0, x1, y1, sw, color);
    match glyph {
        Glyph::Terminal => {
            stroke(at(-8.0, -6.0), at(-2.0, 0.0));
            stroke(at(-2.0, 0.0), at(-8.0, 6.0));
            stroke(at(1.0, 6.0), at(8.0, 6.0));
        }
        Glyph::Done => {
            stroke(at(-8.0, 0.0), at(-2.5, 5.5));
            stroke(at(-2.5, 5.5), at(8.0, -5.5));
        }
    }
}

/// That button held down, as `<` is: its disc darkening what is behind it,
/// and its glyph lighter on it, inked once where its strokes meet.
pub fn button_held(c: &mut Canvas, t: &Theme, glyph: Glyph) {
    let (cx, cy, r) = button_disc(t);
    c.fill_circle(cx, cy, r, t.pal.button_press);
    let side = (r * 2.0).ceil() as u32 + 2;
    let art = Sprite::capture(cx - r - 1.0, cy - r - 1.0, side, side, |c, dx, dy| glyph_at(c, t, glyph, dx, dy, Rgba::rgb(0, 0, 0)));
    art.draw(c, 0.0, 0.0, t.pal.text, BACK_HELD_ALPHA as f32 / 255.0);
}

/// The round button that leaves the search held down, in `r`: its disc
/// darker and its cross lighter, inked once where its strokes cross.
pub fn cancel_held(c: &mut Canvas, t: &Theme, r: Rect) {
    draw_disc(c, r, over(t.pal.button_press, t.pal.search));
    let cross = Sprite::capture(r.x, r.y, r.w.ceil() as u32 + 1, r.h.ceil() as u32 + 1, |c, dx, dy| {
        draw_cross(c, t, Rect::new(r.x + dx, r.y + dy, r.w, r.h), Rgba::rgb(0, 0, 0))
    });
    cross.draw(c, 0.0, 0.0, t.pal.text, CANCEL_HELD_ALPHA as f32 / 255.0);
}

/// The home page's search bar held down, in `r`: darker, its words and
/// icon with it.
pub fn search_bar_held(c: &mut Canvas, t: &Theme, r: Rect) {
    c.fill_round_rect(r.x, r.y, r.w, r.h, r.h / 2.0, t.pal.button_press);
}

fn draw_disc(c: &mut Canvas, r: Rect, color: Rgba) {
    let (cx, cy) = r.centre();
    c.fill_circle(cx, cy, r.w / 2.0, color);
}

fn draw_cross(c: &mut Canvas, t: &Theme, r: Rect, color: Rgba) {
    let m = &t.m;
    let ((cx, cy), arm, stroke) = (r.centre(), m.dp(CANCEL_ARM_DP), m.dp(CANCEL_STROKE_DP));
    c.line(cx - arm, cy - arm, cx + arm, cy + arm, stroke, color);
    c.line(cx - arm, cy + arm, cx + arm, cy - arm, stroke, color);
}

/// The button that leaves the search, where the search page puts it, as
/// ink that fades and blurs in and out: its disc and its cross, to lay in
/// their colours.
pub fn cancel_art(t: &Theme) -> (Sprite, Sprite) {
    let (_, r) = search_layout(t);
    let pad = blur_room(t);
    let (x, y, side) = (r.x - pad, r.y - pad, (r.w + pad * 2.0).ceil() as u32);
    let ink = Rgba::rgb(0, 0, 0);
    let disc = Sprite::capture(x, y, side, side, |c, dx, dy| draw_disc(c, Rect::new(r.x + dx, r.y + dy, r.w, r.h), ink));
    let cross = Sprite::capture(x, y, side, side, |c, dx, dy| draw_cross(c, t, Rect::new(r.x + dx, r.y + dy, r.w, r.h), ink));
    (disc, cross)
}

/// The search field with nothing typed in it, filling `r`: the home page's
/// bar as it moves to where the search page has its field.
pub fn search_capsule(c: &mut Canvas, t: &Theme, f: &Fonts, r: Rect, placeholder: &str) {
    draw_search_field(c, t, f, r.x, r.w, r.y + r.h / 2.0, "", placeholder, false);
}

/// The search bar opened for typing, from `x` across `w` and centred on
/// `cy`: the text typed so far, or the placeholder, a button that clears
/// the text, and the cursor where the typing goes.
#[allow(clippy::too_many_arguments)]
pub fn search_field(c: &mut Canvas, t: &Theme, f: &Fonts, x: f32, w: f32, cy: f32, text: &str, placeholder: &str) -> SearchField {
    let (field, clear, at) = draw_search_field(c, t, f, x, w, cy, text, placeholder, false);
    let caret = caret_over(at, |art, dx, dy| {
        draw_search_field(art, t, f, x - dx, w, cy - dy, text, placeholder, true);
    });
    SearchField { field, clear, caret }
}

/// Draw the search field, with its cursor if `caret`; returns where the
/// field, its clear button and the cursor are.
#[allow(clippy::too_many_arguments)]
fn draw_search_field(
    c: &mut Canvas,
    t: &Theme,
    f: &Fonts,
    x: f32,
    w: f32,
    cy: f32,
    text: &str,
    placeholder: &str,
    caret: bool,
) -> (Rect, Option<Rect>, Rect) {
    let m = &t.m;
    let h = m.search_h;
    let y = cy - h / 2.0;
    c.fill_round_rect(x, y, w, h, h / 2.0, t.pal.search);
    let (icx, icy, sw) = (x + m.dp(22.3), cy - m.dp(2.0), m.dp(1.45));
    let ring = m.dp(6.6);
    c.stroke_circle(icx, icy, ring, sw, t.pal.search_icon);
    let k = ring * std::f32::consts::FRAC_1_SQRT_2;
    c.line(icx + k, icy + k, icx + m.dp(9.9), icy + m.dp(10.0), sw, t.pal.search_icon);
    let tx = x + m.search_text_inset;
    let clear_d = m.dp(18.0);
    let room = w - m.search_text_inset - clear_d - m.dp(24.0);
    let px = m.row_title_px;
    // A long query shows its end, where the typing is.
    let mut shown = text;
    while c.text_width(&f.heading, shown, px) > room {
        let mut chars = shown.chars();
        chars.next();
        shown = chars.as_str();
    }
    // The text goes over the cursor, as the stock field draws them.
    let at = cursor(t, f, tx + c.text_width(&f.heading, shown, px), cy, px);
    if caret {
        c.fill_round_rect(at.x, at.y, at.w, at.h, m.dp(CARET_RADIUS_DP), t.pal.accent);
    }
    if text.is_empty() {
        c.text_vc(&f.heading, placeholder, tx, cy, px, t.pal.placeholder, None);
    } else {
        c.text_vc(&f.heading, shown, tx, cy, px, t.pal.text, None);
    }
    let clear = (!text.is_empty()).then(|| {
        let (ccx, r) = (x + w - m.dp(12.0) - clear_d / 2.0, clear_d / 2.0);
        c.fill_circle(ccx, cy, r, t.pal.search_icon);
        let s = r * 0.42;
        c.line(ccx - s, cy - s, ccx + s, cy + s, m.dp(1.6), t.pal.search);
        c.line(ccx - s, cy + s, ccx + s, cy - s, m.dp(1.6), t.pal.search);
        Rect::new(ccx - h / 2.0, y, h, h)
    });
    (Rect::new(x, y, w, h), clear, at)
}

/// Where a cursor stands, with its left edge on `x`, in text of `px`
/// whose ink is centred on `cy`: as tall as the face's line, from its
/// ascent to its descent, as the stock text fields draw it.
fn cursor(t: &Theme, f: &Fonts, x: f32, cy: f32, px: f32) -> Rect {
    let (ink_top, ink_bottom) = f.heading.ink(px);
    let base = cy - (ink_top + ink_bottom) / 2.0;
    let (top, bottom) = f.heading.line(px);
    Rect::new(x, base + top, t.m.dp(CARET_W_DP), bottom - top)
}

/// A text cursor with its left edge on `x`, from `top` to `bottom`.
pub fn caret_rect(t: &Theme, x: f32, top: f32, bottom: f32) -> Rect {
    Rect::new(x, top, t.m.dp(CARET_W_DP), bottom - top)
}

/// Draw a text cursor in `at`.
pub fn draw_caret(c: &mut Canvas, t: &Theme, at: Rect) {
    c.fill_round_rect(at.x, at.y, at.w, at.h, t.m.dp(CARET_RADIUS_DP), t.pal.accent);
}

/// The stretch of a field round the cursor at `at`, drawn by `draw` into a
/// canvas of its own; `draw` gets how far that canvas's corner is from
/// the field's.
pub fn caret_over(at: Rect, draw: impl FnOnce(&mut Canvas, f32, f32)) -> Caret {
    let (x, y) = (at.x.floor() as i32 - 1, at.y.floor() as i32 - 1);
    let mut art = Canvas::new(at.w.ceil() as u32 + 3, at.h.ceil() as u32 + 3);
    draw(&mut art, x as f32, y as f32);
    Caret { art, x, y }
}

/// Section caption between two cards. `prev_bottom` is the bottom of the card
/// above (or the content top for the first section). Returns the next card top.
pub fn section(c: &mut Canvas, t: &Theme, f: &Fonts, prev_bottom: f32, label: &str) -> f32 {
    let m = &t.m;
    let ink_top = prev_bottom + m.section_label_top;
    c.text_top(&f.label, label, m.margin + m.inner, ink_top, m.label_px, t.pal.section, None);
    prev_bottom + m.section_block
}

/// First section on a page, directly under the content top.
pub fn section_first(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, label: &str) -> f32 {
    let m = &t.m;
    section(c, t, f, top - m.section_label_top + m.dp(2.0), label)
}

/// Grey explanatory text below a card, wrapped to the content width.
pub fn footnote(c: &mut Canvas, t: &Theme, f: &Fonts, y: f32, text: &str) -> f32 {
    let m = &t.m;
    let x = m.title_x + m.dp(0.9);
    let max_w = m.w as f32 - x - m.title_x;
    let lines = wrap(c, &f.value, text, m.sub_px, max_w);
    let mut top = y + m.dp(10.0);
    for line in &lines {
        c.text_top(&f.value, line, x, top, m.sub_px, t.pal.value, None);
        top += m.sub_pitch;
    }
    top
}

/// Body text set on the page itself, as a licence or notice reads,
/// wrapped to the content width; a blank line starts a new paragraph.
/// Returns where the text ends.
pub fn paragraph(c: &mut Canvas, t: &Theme, f: &Fonts, y: f32, text: &str) -> f32 {
    let m = &t.m;
    let x = m.title_x + m.dp(0.9);
    let max_w = m.w as f32 - x - m.title_x;
    let mut top = y + m.dp(12.0);
    for (k, para) in text.split("\n\n").enumerate() {
        if k > 0 {
            top += m.dp(12.0);
        }
        let lines: Vec<String> = para.lines().flat_map(|l| wrap(c, &f.body, l, m.sub_px, max_w)).collect();
        for line in &lines {
            c.text_top(&f.body, line, x, top, m.sub_px, t.pal.info, None);
            top += m.sub_pitch;
        }
    }
    top
}

/// A bar held at the bottom of the screen on the page colour `bg`, with
/// buttons side by side, the last in the accent colour, and room under
/// them for the gesture area. Returns the bar and where each button is on
/// it.
pub fn dock_bar(t: &Theme, f: &Fonts, bg: Rgba, labels: &[&str]) -> (Canvas, Vec<Rect>) {
    let m = &t.m;
    let (h, pad, gap) = (m.dp(48.0), m.dp(12.0), m.dp(12.0));
    let mut c = Canvas::new(m.w, (pad + h + m.dp(KEYBOARD_BOTTOM_DP)).ceil() as u32);
    c.clear(bg);
    let n = labels.len() as f32;
    let w = (m.w as f32 - m.margin * 2.0 - gap * (n - 1.0)) / n;
    let mut rects = Vec::new();
    for (k, label) in labels.iter().enumerate() {
        let x = m.margin + k as f32 * (w + gap);
        let (fill, ink) = if k + 1 == labels.len() { (t.pal.accent, Rgba::rgb(255, 255, 255)) } else { (t.pal.search, t.pal.text) };
        c.fill_round_rect(x, pad, w, h, h / 2.0, fill);
        c.text_center(&f.heading, label, x + w / 2.0, pad + h / 2.0, m.row_title_px, ink);
        rects.push(Rect::new(x, pad, w, h));
    }
    (c, rects)
}

/// A card holding text being typed, with the cursor after it; text too long
/// for the card shows its end, where the cursor is. Returns where the card
/// ends, and the cursor.
pub fn text_field(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, text: &str) -> (f32, Caret) {
    let at = draw_text_field(c, t, f, t.m.margin, top, text, false);
    let caret = caret_over(at, |art, dx, dy| {
        draw_text_field(art, t, f, t.m.margin - dx, top - dy, text, true);
    });
    (top + t.m.dp(56.0), caret)
}

/// Draw the text field's card from `x0`, `top`, with its cursor if
/// `caret`; returns where the cursor is.
fn draw_text_field(c: &mut Canvas, t: &Theme, f: &Fonts, x0: f32, top: f32, text: &str, caret: bool) -> Rect {
    let m = &t.m;
    let (w, h) = (m.w as f32 - m.margin * 2.0, m.dp(56.0));
    c.fill_round_rect(x0, top, w, h, m.card_radius, t.pal.card);
    let (x, room) = (x0 + m.inner, w - m.inner * 2.0 - m.dp(4.0));
    let px = m.row_title_px;
    let mut shown = text;
    while c.text_width(&f.heading, shown, px) > room {
        let mut rest = shown.chars();
        rest.next();
        shown = rest.as_str();
    }
    let at = cursor(t, f, x + c.text_width(&f.heading, shown, px), top + h / 2.0, px);
    if caret {
        c.fill_round_rect(at.x, at.y, at.w, at.h, m.dp(CARET_RADIUS_DP), t.pal.accent);
    }
    c.text_vc(&f.heading, shown, x, top + h / 2.0, px, t.pal.text, None);
    at
}

/// A full-width button in the accent colour, for a page's one action that
/// destroys nothing. Returns its rect.
pub fn button(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, label: &str) -> Rect {
    let m = &t.m;
    let (x, w, h) = (m.margin, m.w as f32 - m.margin * 2.0, m.dp(48.0));
    c.fill_round_rect(x, top, w, h, h / 2.0, t.pal.accent);
    c.text_center(&f.heading, label, x + w / 2.0, top + h / 2.0, m.row_title_px, Rgba::rgb(255, 255, 255));
    Rect::new(x, top, w, h)
}

#[derive(Clone, Copy)]
pub enum Trail<'a> {
    None,
    Arrow,
    Value(&'a str),
    /// Right-aligned grey text with no chevron (read-only information).
    Text(&'a str),
    Switch(bool),
    Badge(&'a str),
    Check(bool),
    /// A WLAN network's padlock, where joining it takes a password, and the
    /// round button that opens its details.
    Network { locked: bool },
    /// The value chosen, and the arrows up and down that say the row opens
    /// a list of choices.
    Choice(&'a str),
}

#[derive(Clone, Copy)]
pub struct Row<'a> {
    pub icon: Option<(Rgba, Icon)>,
    /// A WLAN fan before the title in place of an icon, lit to this many
    /// arcs of three.
    pub fan: Option<u8>,
    pub title: &'a str,
    /// A word in an outline after the title, such as a network's bands.
    pub tag: Option<&'a str>,
    pub sub: Option<&'a str>,
    pub trail: Trail<'a>,
    pub tint: Option<Rgba>,
}

impl<'a> Row<'a> {
    pub fn new(title: &'a str) -> Self {
        Row { icon: None, fan: None, title, tag: None, sub: None, trail: Trail::Arrow, tint: None }
    }
    pub fn icon(mut self, color: Rgba, icon: Icon) -> Self {
        self.icon = Some((color, icon));
        self
    }
    pub fn fan(mut self, level: u8) -> Self {
        self.fan = Some(level);
        self
    }
    pub fn tag(mut self, tag: &'a str) -> Self {
        self.tag = Some(tag);
        self
    }
    pub fn subtitle(mut self, sub: &'a str) -> Self {
        self.sub = Some(sub);
        self
    }
    pub fn trail(mut self, trail: Trail<'a>) -> Self {
        self.trail = trail;
        self
    }
    pub fn tint(mut self, color: Rgba) -> Self {
        self.tint = Some(color);
        self
    }
}

struct Layout {
    text_x: f32,
    text_max: f32,
    sub_lines: Vec<String>,
    h: f32,
}

fn layout(c: &Canvas, t: &Theme, f: &Fonts, r: &Row) -> Layout {
    let m = &t.m;
    let card_w = m.w as f32 - m.margin * 2.0;
    let text_x = match (r.fan, r.icon) {
        (Some(_), _) => m.dp(FAN_TEXT_X),
        (None, Some(_)) => m.inner + m.icon + m.icon_gap,
        (None, None) => m.inner,
    };
    let trail_w = match r.trail {
        Trail::None => m.inner,
        Trail::Arrow => m.chevron_inset + m.chevron_w + m.value_gap,
        Trail::Value(s) => m.chevron_inset + m.chevron_w + m.value_gap * 2.0 + c.text_width(&f.value, s, m.sub_px),
        Trail::Text(s) => m.inner + m.value_gap + c.text_width(&f.value, s, m.sub_px),
        Trail::Switch(_) => m.inner + m.switch_w + m.value_gap,
        Trail::Badge(_) => m.inner + m.badge_w + m.value_gap,
        Trail::Check(_) => m.inner + m.dp(24.0) + m.value_gap,
        Trail::Network { locked } => {
            m.inner + m.dp(ROUND_D) + m.value_gap + if locked { m.dp(LOCK_GAP + LOCK_W) } else { 0.0 }
        }
        Trail::Choice(s) => m.dp(UPDOWN_RIGHT + UPDOWN_W + UPDOWN_VALUE_GAP) + m.value_gap + c.text_width(&f.value, s, m.sub_px),
    };
    let text_max = card_w - text_x - trail_w;
    let sub_lines = r.sub.map(|s| wrap(c, &f.body, s, m.sub_px, text_max)).unwrap_or_default();
    let h = if sub_lines.is_empty() {
        if r.fan.is_some() {
            m.dp(FAN_ROW_H)
        } else {
            m.row_h
        }
    } else {
        let (tt, tb) = f.heading.ink(m.row_title_px);
        let (st, sb) = f.body.ink(m.sub_px);
        let block = (tb - tt) + m.sub_gap + (sub_lines.len() as f32 - 1.0) * m.sub_pitch + (sb - st);
        (block + m.row_pad_v * 2.0).max(m.row_h)
    };
    Layout { text_x, text_max, sub_lines, h }
}

fn draw_row(c: &mut Canvas, t: &Theme, f: &Fonts, r: &Row, lay: &Layout, top: f32, tone: &Tone) {
    let m = &t.m;
    let card_x = m.margin;
    let right = m.w as f32 - m.margin;
    let cy = top + lay.h / 2.0;
    if let Some((color, icon)) = r.icon {
        icons::tile(c, t, card_x + m.inner, cy - m.icon / 2.0, color, icon);
    }
    if let Some(level) = r.fan {
        wlan_fan(c, card_x + m.dp(FAN_CX), cy, m.dp(FAN_R), level, tone.text, tone.faint);
    }
    let tx = card_x + lay.text_x;
    let color = r.tint.unwrap_or(tone.text);
    // A tag takes its room from the title's.
    let tag_w = r.tag.map_or(0.0, |tag| c.text_width(&f.body, tag, m.dp(TAG_PX)) + m.dp(TAG_PAD * 2.0 + TAG_GAP));
    let title_room = (lay.text_max - tag_w).max(0.0);
    let title_max = Some(title_room);
    let (tt, tb) = f.heading.ink(m.row_title_px);
    let title_cy = if lay.sub_lines.is_empty() {
        c.text_vc(&f.heading, r.title, tx, cy, m.row_title_px, color, title_max);
        cy
    } else {
        let (st, sb) = f.body.ink(m.sub_px);
        let block = (tb - tt) + m.sub_gap + (lay.sub_lines.len() as f32 - 1.0) * m.sub_pitch + (sb - st);
        let ink_top = cy - block / 2.0;
        c.text_top(&f.heading, r.title, tx, ink_top, m.row_title_px, color, title_max);
        let mut st_top = ink_top + (tb - tt) + m.sub_gap;
        for line in &lay.sub_lines {
            c.text_top(&f.body, line, tx, st_top, m.sub_px, tone.sub, None);
            st_top += m.sub_pitch;
        }
        ink_top + (tb - tt) / 2.0
    };
    if let Some(tag) = r.tag {
        let title_w = c.text_width(&f.heading, r.title, m.row_title_px).min(title_room);
        let x = tx + title_w + m.dp(TAG_GAP);
        let w = c.text_width(&f.body, tag, m.dp(TAG_PX)) + m.dp(TAG_PAD * 2.0);
        let h = m.dp(TAG_H);
        let (stroke, radius) = (m.dp(TAG_STROKE), m.dp(TAG_RADIUS));
        c.fill_round_rect(x, title_cy - h / 2.0, w, h, radius, tone.value);
        c.fill_round_rect(x + stroke, title_cy - h / 2.0 + stroke, w - stroke * 2.0, h - stroke * 2.0, radius - stroke, tone.card);
        c.text_center(&f.body, tag, x + w / 2.0, title_cy, m.dp(TAG_PX), tone.value);
    }
    match r.trail {
        Trail::None => {}
        Trail::Arrow => chevron(c, t, right - m.chevron_inset, cy, tone.chevron),
        Trail::Value(s) => {
            let chev_left = right - m.chevron_inset - m.chevron_w;
            chevron(c, t, right - m.chevron_inset, cy, tone.chevron);
            let w = c.text_width(&f.value, s, m.sub_px);
            c.text_vc(&f.value, s, chev_left - m.value_gap - w, cy, m.sub_px, tone.value, None);
        }
        Trail::Text(s) => {
            let w = c.text_width(&f.value, s, m.sub_px);
            c.text_vc(&f.value, s, right - m.inner - w, cy, m.sub_px, tone.value, None);
        }
        Trail::Switch(on) => switch(c, t, right - m.inner, cy, on),
        Trail::Badge(s) => {
            let (bw, bh) = (m.badge_w, m.badge_h);
            let bx = right - m.inner - bw;
            c.fill_round_rect(bx, cy - bh / 2.0, bw, bh, bh / 2.0, t.pal.badge);
            c.text_center(&f.badge, s, bx + bw / 2.0, cy, m.badge_px, t.pal.accent);
        }
        Trail::Check(on) => check_circle(c, t, right - m.inner - m.dp(12.0), cy, on),
        Trail::Network { locked } => {
            let round = round_button(t, Rect::new(card_x, top, right - card_x, lay.h));
            let (rcx, rcy, rr) = (round.x + round.w / 2.0, round.y + round.h / 2.0, round.w / 2.0);
            c.fill_circle(rcx, rcy, rr, tone.round);
            let (w, h, sw) = (m.dp(ROUND_MARK_W), m.dp(ROUND_MARK_H), m.dp(ROUND_MARK_STROKE));
            let x0 = rcx - w / 2.0 + sw / 2.0;
            let x1 = rcx + w / 2.0 - sw / 2.0;
            c.line(x0, rcy - h / 2.0 + sw / 2.0, x1, rcy, sw, tone.round_mark);
            c.line(x1, rcy, x0, rcy + h / 2.0 - sw / 2.0, sw, tone.round_mark);
            if locked {
                padlock(c, round.x - m.dp(LOCK_GAP + LOCK_W / 2.0), cy, m.dp(LOCK_H), tone.value);
            }
        }
        Trail::Choice(s) => {
            let arrows = right - m.dp(UPDOWN_RIGHT);
            updown(c, t, arrows, cy, tone.updown);
            let w = c.text_width(&f.value, s, m.sub_px);
            c.text_vc(&f.value, s, arrows - m.dp(UPDOWN_W + UPDOWN_VALUE_GAP) - w, cy, m.sub_px, tone.value, None);
        }
    }
}

// The arrows after the value of a row that opens a list of choices, as the
// stock rows draw them, in dp: their right end from the card's, how wide
// and tall each is, how far apart, their stroke, and the room before them.
const UPDOWN_RIGHT: f32 = 16.0;
const UPDOWN_W: f32 = 10.0;
const UPDOWN_H: f32 = 5.67;
const UPDOWN_GAP: f32 = 4.0;
const UPDOWN_STROKE: f32 = 1.6;
const UPDOWN_VALUE_GAP: f32 = 9.0;

/// The arrow up over the arrow down, their right end at `right`, the pair
/// centred on `cy`.
fn updown(c: &mut Canvas, t: &Theme, right: f32, cy: f32, color: Rgba) {
    let m = &t.m;
    let (w, h, gap, sw) = (m.dp(UPDOWN_W), m.dp(UPDOWN_H), m.dp(UPDOWN_GAP), m.dp(UPDOWN_STROKE));
    let (x0, mid, x1) = (right - w + sw / 2.0, right - w / 2.0, right - sw / 2.0);
    let (up, down) = (cy - gap / 2.0 - h, cy + gap / 2.0);
    let (top, foot) = (sw / 2.0, h - sw / 2.0);
    c.line(x0, up + foot, mid, up + top, sw, color);
    c.line(mid, up + top, x1, up + foot, sw, color);
    c.line(x0, down + top, mid, down + foot, sw, color);
    c.line(mid, down + foot, x1, down + top, sw, color);
}

// The stock list of choices opened from a row, in dp: how wide it is, how
// far in from the row's right end and down from its top it opens, its
// corners, the room above and below its choices, each choice's height,
// where the words start and how big they are, the tick's centre from the
// right edge and its stroke, and the deviation of the blur that makes its
// shadow.
const CHOICES_W: f32 = 286.0;
const CHOICES_RIGHT: f32 = 16.7;
const CHOICES_DROP: f32 = 15.0;
const CHOICES_RADIUS: f32 = 24.0;
const CHOICES_PAD: f32 = 8.0;
const CHOICE_H: f32 = 43.67;
const CHOICE_TEXT_X: f32 = 20.3;
const CHOICE_PX: f32 = 16.0;
const TICK_RIGHT: f32 = 31.0;
const TICK_STROKE: f32 = 2.0;
const CHOICES_SHADOW: f32 = 10.7;

/// A list of choices open over the page from `row`, as the stock one
/// opens: a card lifted off the page, its choices one under another, the
/// one `chosen` in the accent with a tick, and the one `held` darkened
/// while a finger holds it. Returns each choice's rect.
#[allow(clippy::too_many_arguments)]
pub fn choices(c: &mut Canvas, t: &Theme, f: &Fonts, row: Rect, names: &[&str], chosen: Option<usize>, held: Option<usize>) -> Vec<Rect> {
    let m = &t.m;
    let (w, item, pad, r) = (m.dp(CHOICES_W), m.dp(CHOICE_H), m.dp(CHOICES_PAD), m.dp(CHOICES_RADIUS));
    let (x, y) = (row.x + row.w - m.dp(CHOICES_RIGHT) - w, row.y + m.dp(CHOICES_DROP));
    let h = pad * 2.0 + item * names.len() as f32;
    let rects: Vec<Rect> = (0..names.len()).map(|i| Rect::new(x, y + pad + item * i as f32, w, item)).collect();
    c.shadow_round_rect(x, y, w, h, r, m.dp(CHOICES_SHADOW), t.pal.shadow);
    let (popup, darker) = (t.pal.popup, over(t.pal.row_press, t.pal.popup));
    let held = held.map(|i| rects[i]);
    c.fill_round_rect_graded(x, y, w, h, r, &|fy| match held {
        Some(b) if fy >= b.y && fy < b.y + b.h => darker,
        _ => popup,
    });
    for (i, (name, at)) in names.iter().zip(&rects).enumerate() {
        let cy = at.y + at.h / 2.0;
        let ink = if chosen == Some(i) { t.pal.accent } else { t.pal.text };
        c.text_vc(&f.heading, name, x + m.dp(CHOICE_TEXT_X), cy, m.dp(CHOICE_PX), ink, None);
        if chosen == Some(i) {
            tick(c, t, x + w - m.dp(TICK_RIGHT), cy, t.pal.accent);
        }
    }
    rects
}

/// The tick beside the choice taken, its ink centred on (`cx`, `cy`).
fn tick(c: &mut Canvas, t: &Theme, cx: f32, cy: f32, color: Rgba) {
    let m = &t.m;
    let at = |dx: f32, dy: f32| (cx + m.dp(dx), cy + m.dp(dy));
    let (a, b, z) = (at(-5.0, 0.0), at(-1.17, 4.1), at(5.27, -3.83));
    let sw = m.dp(TICK_STROKE);
    c.line(a.0, a.1, b.0, b.1, sw, color);
    c.line(b.0, b.1, z.0, z.1, sw, color);
}

/// The round button at the end of a WLAN network's row `row`, which opens
/// the network's details apart from the row.
pub fn round_button(t: &Theme, row: Rect) -> Rect {
    let d = t.m.dp(ROUND_D);
    Rect::new(row.x + row.w - t.m.inner - d, row.y + row.h / 2.0 - d / 2.0, d, d)
}

// What a WLAN network's row holds, as the stock WLAN page lays it out, in
// dp: the fan's centre from the card's edge and its reach, where the name
// starts, the tag after the name, the padlock and the round button.
const FAN_CX: f32 = 27.3;
const FAN_R: f32 = 13.5;
const FAN_TEXT_X: f32 = 54.7;
const FAN_ROW_H: f32 = 52.2;
const TAG_PX: f32 = 11.0;
const TAG_H: f32 = 13.6;
const TAG_PAD: f32 = 4.0;
const TAG_GAP: f32 = 5.0;
const TAG_STROKE: f32 = 0.7;
const TAG_RADIUS: f32 = 3.5;
const LOCK_W: f32 = 10.0;
const LOCK_H: f32 = 11.5;
const LOCK_GAP: f32 = 16.0;
const ROUND_D: f32 = 25.3;
const ROUND_MARK_W: f32 = 4.0;
const ROUND_MARK_H: f32 = 7.5;
const ROUND_MARK_STROKE: f32 = 1.7;

/// A flat settings card holding `rows`.
pub fn list(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, rows: &[Row]) -> Block {
    list_toned(c, t, f, top, rows, &Tone::page(t))
}

/// [`list`] in the given colours.
pub fn list_toned(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, rows: &[Row], tone: &Tone) -> Block {
    let m = &t.m;
    let lays: Vec<Layout> = rows.iter().map(|r| layout(c, t, f, r)).collect();
    let h: f32 = lays.iter().map(|l| l.h).sum();
    let w = m.w as f32 - m.margin * 2.0;
    c.fill_round_rect(m.margin, top, w, h, m.card_radius, tone.card);
    let mut y = top;
    let mut rects = Vec::new();
    for (r, l) in rows.iter().zip(&lays) {
        draw_row(c, t, f, r, l, y, tone);
        rects.push(Rect::new(m.margin, y, w, l.h));
        y += l.h;
    }
    Block { bottom: top + h, rows: rects }
}

/// Account-style card: avatar, name and detail, then optional rows. The
/// first rect returned covers the name row.
pub fn device_card(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, title: &str, sub: &str, rows: &[Row]) -> Block {
    let m = &t.m;
    let lays: Vec<Layout> = rows.iter().map(|r| layout(c, t, f, r)).collect();
    let h = m.account_row_h + lays.iter().map(|l| l.h).sum::<f32>();
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    c.fill_round_rect(card_x, top, right - card_x, h, m.card_radius, t.pal.card);

    let cy = top + m.account_row_h / 2.0;
    let av = m.avatar;
    let acx = card_x + m.avatar_inset + av / 2.0;
    c.fill_circle(acx, cy, av / 2.0, t.pal.avatar);
    icons::glyph(c, Icon::Phone, acx, cy, av * 0.86, t.pal.avatar_glyph);

    let tx = card_x + m.inner + m.icon + m.icon_gap;
    let (tt, tb) = f.heading.ink(m.account_title_px);
    let (st, sb) = f.body.ink(m.sub_px);
    let block = (tb - tt) + m.sub_gap + (sb - st);
    let ink_top = cy - block / 2.0;
    c.text_top(&f.heading, title, tx, ink_top, m.account_title_px, t.pal.text, None);
    c.text_top(&f.body, sub, tx, ink_top + (tb - tt) + m.sub_gap, m.sub_px, t.pal.subtitle, None);
    chevron(c, t, right - m.chevron_inset, cy, t.pal.chevron);

    let tone = Tone::page(t);
    let mut rects = vec![Rect::new(card_x, top, right - card_x, m.account_row_h)];
    let mut y = top + m.account_row_h;
    for (r, l) in rows.iter().zip(&lays) {
        draw_row(c, t, f, r, l, y, &tone);
        rects.push(Rect::new(card_x, y, right - card_x, l.h));
        y += l.h;
    }
    Block { bottom: top + h, rows: rects }
}

pub fn switch(c: &mut Canvas, t: &Theme, right: f32, cy: f32, on: bool) {
    let m = &t.m;
    let (w, h, k) = (m.switch_w, m.switch_h, m.knob);
    let x = right - w;
    c.fill_round_rect(x, cy - h / 2.0, w, h, h / 2.0, if on { t.pal.accent } else { t.pal.switch_off });
    let kx = if on { right - h / 2.0 } else { x + h / 2.0 };
    c.fill_circle(kx, cy, k / 2.0, Rgba::rgb(255, 255, 255));
}

/// The stock thick slider: a capsule track with the fill and a white knob
/// that sits inside the fill's rounded end.
pub fn slider(c: &mut Canvas, t: &Theme, x: f32, top: f32, w: f32, value: f32) {
    let m = &t.m;
    let h = m.slider_h;
    c.fill_round_rect(x, top, w, h, h / 2.0, t.pal.track);
    let fw = (w * value.clamp(0.0, 1.0)).max(h);
    c.fill_round_rect(x, top, fw, h, h / 2.0, t.pal.accent);
    c.fill_circle(x + fw - h / 2.0, top + h / 2.0, m.slider_knob / 2.0, Rgba::rgb(255, 255, 255));
}

/// Where a slider card ends, its track, and the row under the slider.
pub struct SliderBlock {
    pub bottom: f32,
    pub track: Rect,
    pub below: Option<Rect>,
}

/// The monochrome icon beside a slider card's caption.
#[derive(Clone, Copy)]
pub enum Caption {
    Sun,
    /// A phone between the lines of its shaking.
    Vibrate,
}

/// Card with a monochrome icon caption, a slider, and an optional row below.
#[allow(clippy::too_many_arguments)]
pub fn slider_card(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, caption: Caption, label: &str, value: f32, below: Option<Row>) -> SliderBlock {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let below_lay = below.as_ref().map(|r| layout(c, t, f, r));
    let slider_top = top + m.dp(55.0);
    let slider_bottom = slider_top + m.slider_h;
    let below_gap = m.dp(16.3);
    let h = match &below_lay {
        Some(l) => slider_bottom - top + below_gap + l.h,
        None => slider_bottom - top + m.dp(18.0),
    };
    c.fill_round_rect(card_x, top, right - card_x, h, m.card_radius, t.pal.card);

    let lcy = top + m.dp(31.2);
    let scx = card_x + m.dp(28.0);
    match caption {
        Caption::Sun => {
            c.fill_circle(scx, lcy, m.dp(5.7), t.pal.mono_icon);
            for k in 0..8 {
                let a = k as f32 * FRAC_PI_4;
                let (ux, uy) = (a.cos(), a.sin());
                c.line(scx + ux * m.dp(8.3), lcy + uy * m.dp(8.3), scx + ux * m.dp(8.9), lcy + uy * m.dp(8.9), m.dp(2.0), t.pal.mono_icon);
            }
        }
        Caption::Vibrate => {
            c.stroke_round_rect(scx - m.dp(4.3), lcy - m.dp(7.7), m.dp(8.6), m.dp(15.4), m.dp(2.2), m.dp(2.0), t.pal.mono_icon);
            for side in [-1.0f32, 1.0] {
                let x = scx + side * m.dp(8.0);
                let (out, step) = (side * m.dp(1.7), m.dp(3.2));
                let ys = [lcy - step * 1.5, lcy - step * 0.5, lcy + step * 0.5, lcy + step * 1.5];
                for (k, pair) in ys.windows(2).enumerate() {
                    let (a, b) = if k % 2 == 0 { (0.0, out) } else { (out, 0.0) };
                    c.line(x + a, pair[0], x + b, pair[1], m.dp(1.8), t.pal.mono_icon);
                }
            }
        }
    }
    // The caption is set like the account name, in the softer text colour.
    c.text_vc(&f.heading, label, card_x + m.dp(53.7), lcy, m.account_title_px, t.pal.info, None);
    let track = Rect::new(card_x + m.inner, slider_top, right - card_x - m.inner * 2.0, m.slider_h);
    slider(c, t, track.x, track.y, track.w, value);
    let mut below_rect = None;
    if let (Some(r), Some(l)) = (below, below_lay) {
        let y = slider_bottom + below_gap;
        draw_row(c, t, f, &r, &l, y, &Tone::page(t));
        below_rect = Some(Rect::new(card_x, y, right - card_x, l.h));
    }
    SliderBlock { bottom: top + h, track, below: below_rect }
}

/// Swipe-to-confirm control in the thick-slider style; `progress` (0 to 1)
/// is how far the knob has been dragged. Returns the track.
pub fn confirm(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, label: &str, danger: bool, progress: f32) -> Rect {
    let m = &t.m;
    let accent = if danger { t.pal.danger } else { t.pal.accent };
    let (x, w, h) = (m.margin, m.w as f32 - m.margin * 2.0, m.dp(56.0));
    c.fill_round_rect(x, top, w, h, h / 2.0, if danger { t.pal.danger_soft } else { t.pal.badge });
    c.text_center(&f.heading, label, x + w / 2.0 + h / 4.0, top + h / 2.0, m.row_title_px, accent);
    let k = h - m.dp(8.0);
    let kcx = x + h / 2.0 + (w - h) * progress.clamp(0.0, 1.0);
    let kcy = top + h / 2.0;
    if progress > 0.0 {
        c.fill_round_rect(x, top, kcx + h / 2.0 - x, h, h / 2.0, accent.with_alpha(60));
    }
    c.fill_circle(kcx, kcy, k / 2.0, accent);
    let s = k * 0.3;
    c.line(kcx - s * 0.5, kcy, kcx + s * 0.5, kcy, m.stroke, Rgba::rgb(255, 255, 255));
    c.line(kcx + s * 0.5, kcy, kcx + s * 0.05, kcy - s * 0.45, m.stroke, Rgba::rgb(255, 255, 255));
    c.line(kcx + s * 0.5, kcy, kcx + s * 0.05, kcy + s * 0.45, m.stroke, Rgba::rgb(255, 255, 255));
    Rect::new(x, top, w, h)
}

pub fn progress_bar(c: &mut Canvas, t: &Theme, x: f32, y: f32, w: f32, value: f32) {
    let m = &t.m;
    let h = m.dp(6.0);
    c.fill_round_rect(x, y, w, h, h / 2.0, t.pal.track);
    c.fill_round_rect(x, y, (w * value.clamp(0.0, 1.0)).max(h), h, h / 2.0, t.pal.accent);
}

/// A card for a long job: what is under way, a line on where it stands,
/// and a bar when the share done is known. Returns the card's bottom.
pub fn progress_card(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, title: &str, detail: &str, done: Option<f32>) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let x = card_x + m.inner;
    let width = right - m.inner - x;
    let (ink_top, _) = f.heading.ink(m.row_title_px);
    let title_base = top + m.dp(23.5) - ink_top;
    let detail_base = title_base + m.dp(20.2);
    let bar_top = detail_base + m.dp(18.0);
    let bottom = match done {
        Some(_) => bar_top + m.dp(6.0) + m.dp(22.0),
        None => detail_base + m.dp(24.0),
    };
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, t.pal.card);
    c.text(&f.heading, title, x, title_base, m.row_title_px, t.pal.text, Some(width));
    c.text(&f.body, detail, x, detail_base, m.sub_px, t.pal.subtitle, Some(width));
    if let Some(v) = done {
        progress_bar(c, t, x, bar_top, width, v);
    }
    bottom
}

/// How near a finger comes to a pattern dot to join it, in dp.
pub const PATTERN_REACH_DP: f32 = 32.0;

/// Centres of the nine pattern dots, row by row.
pub fn pattern_dots(cx: f32, cy: f32, span: f32) -> [(f32, f32); 9] {
    let step = span / 2.0;
    std::array::from_fn(|i| (cx - step + (i % 3) as f32 * step, cy - step + (i / 3) as f32 * step))
}

pub fn pattern_lock(c: &mut Canvas, t: &Theme, cx: f32, cy: f32, span: f32, path: &[usize]) {
    let m = &t.m;
    let dots = pattern_dots(cx, cy, span);
    let pos = |idx: usize| dots[idx];
    for w in path.windows(2) {
        let (ax, ay) = pos(w[0]);
        let (bx, by) = pos(w[1]);
        c.line(ax, ay, bx, by, m.dp(3.0), t.pal.accent.with_alpha(200));
    }
    for i in 0..9 {
        let (px, py) = pos(i);
        if path.contains(&i) {
            c.fill_circle(px, py, m.dp(14.0), t.pal.accent.with_alpha(60));
            c.fill_circle(px, py, m.dp(6.0), t.pal.accent);
        } else {
            c.fill_circle(px, py, m.dp(5.0), t.pal.value);
        }
    }
}

/// The round keys a screen lock's code is typed on.
const PAD_KEY_DP: f32 = 76.0;
const PAD_GAP_X_DP: f32 = 30.0;
const PAD_GAP_Y_DP: f32 = 18.0;
const PAD_TOP_DP: f32 = 10.0;
/// Room under the last row for the gesture area of the screen's edge.
const PAD_BOTTOM_DP: f32 = 26.0;

/// What each key of the keypad types, read as rows of three.
const PAD_KEYS: [Cap; 12] = [
    Cap::Char('1'),
    Cap::Char('2'),
    Cap::Char('3'),
    Cap::Char('4'),
    Cap::Char('5'),
    Cap::Char('6'),
    Cap::Char('7'),
    Cap::Char('8'),
    Cap::Char('9'),
    Cap::Backspace,
    Cap::Char('0'),
    Cap::Enter,
];

/// How tall the keypad is on this theme's screen.
pub fn keypad_height(t: &Theme) -> f32 {
    t.m.dp(PAD_TOP_DP + 4.0 * PAD_KEY_DP + 3.0 * PAD_GAP_Y_DP + PAD_BOTTOM_DP)
}

/// The keypad a screen lock's code is typed on, its top at `top`: four rows
/// of three round keys, `pressed` drawn held down, and the last row erasing
/// and confirming, in `l`. Returns where each key answers a touch.
pub fn keypad(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, top: f32, pressed: Option<Cap>) -> Vec<(Rect, Cap)> {
    let m = &t.m;
    let (size, gx, gy) = (m.dp(PAD_KEY_DP), m.dp(PAD_GAP_X_DP), m.dp(PAD_GAP_Y_DP));
    let left = (m.w as f32 - (3.0 * size + 2.0 * gx)) / 2.0;
    let mut keys = Vec::with_capacity(PAD_KEYS.len());
    for (i, &cap) in PAD_KEYS.iter().enumerate() {
        let (row, col) = (i / 3, i % 3);
        let r = Rect::new(left + col as f32 * (size + gx), top + m.dp(PAD_TOP_DP) + row as f32 * (size + gy), size, size);
        keys.push((r, cap));
    }
    for &(r, cap) in &keys {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        // Erasing and confirming carry their own words, and sit on the
        // page rather than on a key of their own.
        let bare = matches!(cap, Cap::Backspace | Cap::Enter);
        let base = match cap {
            Cap::Enter => t.pal.accent,
            _ if bare => t.pal.bg,
            _ => t.pal.card,
        };
        let fill = if pressed == Some(cap) { mix(base, t.pal.text, 0.16) } else { base };
        if !bare || cap == Cap::Enter {
            c.fill_circle(cx, cy, r.w / 2.0, fill);
        }
        match cap {
            Cap::Char(ch) => c.text_center(&f.heading, &ch.to_string(), cx, cy, m.dp(34.0), t.pal.text),
            Cap::Backspace => c.text_center(&f.body, l.pick(["删除", "刪除", "Delete"]), cx, cy, m.row_title_px, t.pal.accent),
            _ => c.text_center(&f.heading, l.pick(["解密", "解密", "Decrypt"]), cx, cy, m.row_title_px, Rgba::rgb(255, 255, 255)),
        }
    }
    keys
}

/// The code typed so far, as a dot for each character. Nothing is drawn
/// for a code not started.
pub fn code_dots(c: &mut Canvas, t: &Theme, cx: f32, cy: f32, typed: usize) {
    let m = &t.m;
    let pitch = m.dp(26.0);
    let left = cx - (typed.saturating_sub(1) as f32) * pitch / 2.0;
    for i in 0..typed {
        c.fill_circle(left + i as f32 * pitch, cy, m.dp(7.0), t.pal.accent);
    }
}

/// Grey line icons of the parameters page, drawn on a 24 dp grid.
#[derive(Clone, Copy, Debug)]
pub enum Outline {
    Phone,
    Battery,
    Chip,
    Memory,
    Resolution,
    Screen,
    Storage,
    Tag,
    /// An i in a ring: how a network stands.
    Info,
    /// A WLAN fan in lines: which generation of Wi-Fi.
    Wifi,
    /// A dial: how fast a link is.
    Speed,
    /// Bars rising: how strong a signal is.
    Signal,
    /// A shield with a keyhole: how a network is protected.
    Shield,
    /// A pin: an address.
    Address,
    /// A box over two: a network's mask.
    Mask,
    /// A router with its antennas.
    Router,
}

pub fn outline(c: &mut Canvas, t: &Theme, icon: Outline, cx: f32, cy: f32) {
    outline_in(c, t, icon, cx, cy, t.pal.outline);
}

/// [`outline`] in `col`.
pub fn outline_in(c: &mut Canvas, t: &Theme, icon: Outline, cx: f32, cy: f32, col: Rgba) {
    let m = &t.m;
    let d = |v: f32| m.dp(v);
    let sw = d(1.7);
    // Rounded box whose outer edge spans (x, y, w, h) in dp from the centre.
    let boxed = |c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32| {
        c.stroke_round_rect(cx + d(x) + sw / 2.0, cy + d(y) + sw / 2.0, d(w) - sw, d(h) - sw, d(r) - sw / 2.0, sw, col);
    };
    let seg = |c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32| c.line(cx + d(x0), cy + d(y0), cx + d(x1), cy + d(y1), sw, col);
    let ring = |c: &mut Canvas, pts: &[(f32, f32)]| {
        for (i, &(x0, y0)) in pts.iter().enumerate() {
            let (x1, y1) = pts[(i + 1) % pts.len()];
            seg(c, x0, y0, x1, y1);
        }
    };
    match icon {
        Outline::Phone => {
            boxed(c, -7.7, -10.0, 15.4, 20.0, 3.4);
            seg(c, -3.1, 5.9, 3.1, 5.9);
        }
        Outline::Battery => {
            boxed(c, -10.9, -7.15, 19.0, 14.3, 3.4);
            c.fill_round_rect(cx + d(9.2), cy - d(2.85), d(1.7), d(5.7), d(0.85), col);
        }
        Outline::Chip => {
            boxed(c, -7.0, -7.0, 14.0, 14.0, 3.0);
            for k in [-2.6f32, 2.6] {
                seg(c, k, -9.6, k, -7.4);
                seg(c, k, 7.4, k, 9.6);
                seg(c, -9.6, k, -7.4, k);
                seg(c, 7.4, k, 9.6, k);
            }
        }
        Outline::Memory => {
            boxed(c, -8.0, -8.0, 16.0, 16.0, 4.0);
            for k in [-3.4f32, 0.0, 3.4] {
                seg(c, k, -10.3, k, -8.2);
                seg(c, k, 8.2, k, 10.3);
            }
        }
        Outline::Resolution => {
            boxed(c, -9.5, -9.5, 19.0, 19.0, 4.5);
            for k in 0..4 {
                let s = -4.4 + k as f32 * 2.95;
                seg(c, s, s, s + 0.9, s + 0.9);
            }
        }
        Outline::Screen => {
            boxed(c, -9.5, -9.5, 19.0, 19.0, 4.5);
            seg(c, -4.6, -1.4, -4.6, -4.6);
            seg(c, -4.6, -4.6, -1.4, -4.6);
            seg(c, 4.6, 1.4, 4.6, 4.6);
            seg(c, 4.6, 4.6, 1.4, 4.6);
        }
        Outline::Storage => {
            ring(c, &[(-2.6, -8.65), (5.95, -8.65), (5.95, 8.65), (-5.95, 8.65), (-5.95, -5.3)]);
            for x in [-0.6f32, 1.6, 3.8] {
                seg(c, x, -5.6, x, -3.4);
            }
        }
        Outline::Tag => {
            ring(c, &[(-8.6, 0.0), (-4.3, -5.95), (8.65, -5.95), (8.65, 5.95), (-4.3, 5.95)]);
            c.fill_circle(cx - d(3.3), cy, d(1.6), col);
        }
        Outline::Info => {
            c.stroke_circle(cx, cy, d(8.6), sw, col);
            c.fill_circle(cx, cy - d(3.7), d(1.15), col);
            seg(c, 0.0, -0.7, 0.0, 4.3);
        }
        Outline::Wifi => {
            let apex = cy + d(5.6);
            for r in [3.6f32, 6.8, 10.0] {
                c.stroke_arc(cx, apex, d(r), -135f32.to_radians(), 90f32.to_radians(), sw, col);
            }
            c.fill_circle(cx, apex, d(1.25), col);
        }
        Outline::Speed => {
            c.stroke_circle(cx, cy, d(8.6), sw, col);
            seg(c, 0.0, 1.2, 3.8, -3.4);
            c.fill_circle(cx, cy + d(1.2), d(1.5), col);
        }
        Outline::Signal => {
            for (k, h) in [3.0f32, 6.4, 9.8, 13.2].into_iter().enumerate() {
                let x = -6.3 + k as f32 * 4.2;
                seg(c, x, 6.6, x, 6.6 - h);
            }
        }
        Outline::Shield => {
            ring(c, &[(0.0, -9.2), (7.6, -6.1), (7.4, 0.6), (5.1, 5.4), (0.0, 9.2), (-5.1, 5.4), (-7.4, 0.6), (-7.6, -6.1)]);
            c.fill_circle(cx, cy - d(1.3), d(1.5), col);
            seg(c, 0.0, -1.3, 0.0, 3.0);
        }
        Outline::Address => {
            let (ccy, r) = (cy - d(2.4), d(7.0));
            let (left, right) = (150f32.to_radians(), 30f32.to_radians());
            c.stroke_arc(cx, ccy, r, left, 240f32.to_radians(), sw, col);
            c.line(cx + r * left.cos(), ccy + r * left.sin(), cx, cy + d(9.4), sw, col);
            c.line(cx + r * right.cos(), ccy + r * right.sin(), cx, cy + d(9.4), sw, col);
            c.stroke_circle(cx, ccy, d(2.4), sw, col);
        }
        Outline::Mask => {
            boxed(c, -3.6, -9.3, 7.2, 6.2, 1.6);
            boxed(c, -9.3, 3.1, 7.2, 6.2, 1.6);
            boxed(c, 2.1, 3.1, 7.2, 6.2, 1.6);
            seg(c, 0.0, -3.1, 0.0, 0.0);
            seg(c, -5.7, 0.0, 5.7, 0.0);
            seg(c, -5.7, 0.0, -5.7, 3.1);
            seg(c, 5.7, 0.0, 5.7, 3.1);
        }
        Outline::Router => {
            boxed(c, -9.6, -0.6, 19.2, 9.6, 2.6);
            for x in [-4.2f32, 0.0, 4.2] {
                seg(c, x, -0.6, x, -8.2);
            }
            for x in [-5.0f32, -1.8] {
                c.fill_circle(cx + d(x), cy + d(4.2), d(1.0), col);
            }
        }
    }
}

/// A box of [`detail_grid`].
pub struct Detail<'a> {
    pub icon: Outline,
    pub title: &'a str,
    pub value: String,
}

/// Two-column grid of icon, title and the value under it, as the stock
/// network details lay them out; values keep their own line breaks and
/// wrap to their column.
pub fn detail_grid(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, items: &[Detail]) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let cols = [card_x + m.inner, card_x + (right - card_x) * 0.5];
    let widths = [cols[1] - cols[0] - m.inner, right - m.inner - cols[1]];
    let (to_title, to_value, after, pitch) = (m.dp(30.0), m.dp(23.3), m.dp(44.0), m.dp(18.7));
    let cells: Vec<Vec<String>> = items
        .iter()
        .enumerate()
        .map(|(i, d)| d.value.split('\n').flat_map(|l| wrap(c, &f.body, l, m.sub_px, widths[i % 2])).collect())
        .collect();
    let mut centres = Vec::new();
    let mut icy = top + m.dp(28.0);
    let mut last = icy;
    for pair in cells.chunks(2) {
        centres.push(icy);
        let lines = pair.iter().map(Vec::len).max().unwrap_or(1).max(1);
        last = icy + to_title + to_value + pitch * (lines - 1) as f32;
        icy = last + after;
    }
    let bottom = last + to_value;
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, t.pal.card);
    for (i, (d, lines)) in items.iter().zip(&cells).enumerate() {
        let (x, icy) = (cols[i % 2], centres[i / 2]);
        outline_in(c, t, d.icon, x + m.dp(10.0), icy, t.pal.spec_label);
        let title_cy = icy + to_title;
        c.text_vc(&f.heading, d.title, x, title_cy, m.row_title_px, t.pal.text, Some(widths[i % 2]));
        for (k, l) in lines.iter().enumerate() {
            c.text_vc(&f.body, l, x, title_cy + to_value + pitch * k as f32, m.sub_px, t.pal.spec_label, None);
        }
    }
    bottom
}

/// A card of things to know: its heading, then each point after a dot,
/// its lines kept clear of the dot, as the stock system update page ends.
pub fn notes(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, heading: &str, points: &[&str]) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let x = card_x + m.inner;
    let px = m.dp(NOTES_PX);
    let indent = m.dp(NOTES_INDENT);
    let width = right - m.inner - x - indent;
    let lines: Vec<Vec<String>> = points.iter().map(|p| wrap(c, &f.body, p, px, width)).collect();
    // Centre to centre: the heading to the first point and one point to the
    // next, and a point's lines.
    let (pad, step, pitch) = (m.dp(NOTES_PAD), m.dp(NOTES_STEP), m.dp(NOTES_PITCH));
    let mut cy = top + pad;
    let mut centres = Vec::new();
    for point in &lines {
        cy += step;
        let first = cy;
        cy += pitch * (point.len().max(1) - 1) as f32;
        centres.push(first);
    }
    let bottom = cy + pad;
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, t.pal.card);
    c.text_vc(&f.body, heading, x, top + pad, px, t.pal.text, None);
    for (point, first) in lines.iter().zip(centres) {
        c.text_vc(&f.body, "·", x, first, px, t.pal.text, None);
        for (k, l) in point.iter().enumerate() {
            c.text_vc(&f.body, l, x + indent, first + pitch * k as f32, px, t.pal.text, None);
        }
    }
    bottom
}

// The stock system update page's notes, in dp: their text, the room above
// the heading and below the last line, and the steps between.
const NOTES_PX: f32 = 15.0;
const NOTES_INDENT: f32 = 6.7;
const NOTES_PAD: f32 = 25.0;
const NOTES_STEP: f32 = 38.0;
const NOTES_PITCH: f32 = 18.0;

/// [`section`] with a word at its right end that does something, such as
/// the WLAN page's「刷新」: where the word can be touched.
pub fn section_action(c: &mut Canvas, t: &Theme, f: &Fonts, prev_bottom: f32, label: &str, action: &str) -> (f32, Rect) {
    let m = &t.m;
    let y = section(c, t, f, prev_bottom, label);
    let ink_top = prev_bottom + m.section_label_top;
    let w = c.text_width(&f.label, action, m.label_px);
    let x = m.w as f32 - m.margin - m.inner - w;
    c.text_top(&f.label, action, x, ink_top, m.label_px, t.pal.accent, None);
    let reach = m.dp(12.0);
    (y, Rect::new(x - reach, ink_top - reach, w + reach + m.margin + m.inner, m.label_px + reach * 2.0))
}

/// Centred lettering with its cap line at `cap_top`, as ink whose colour
/// comes from the field under it.
pub fn wordmark(t: &Theme, f: &Fonts, text: &str, cap_top: f32, px: f32) -> Sprite {
    let face = &f.display;
    let w = Canvas::new(0, 0).text_width(face, text, px);
    let x = (t.m.w as f32 - w) / 2.0;
    let pad = t.m.dp(4.0);
    let (left, top) = (x - pad, cap_top - px * 0.4);
    Sprite::capture(left, top, (w + pad * 2.0).ceil() as u32, (px * 1.6).ceil() as u32, |c, dx, dy| {
        c.text(face, text, x + dx, cap_top + face.cap(px) + dy, px, Rgba::rgb(0, 0, 0), None);
    })
}

/// The version line centred under the About wordmark.
pub fn version_line(t: &Theme, f: &Fonts, text: &str) -> Sprite {
    hero_line(t, f, text, 0.0)
}

/// How far below the version, in dp, the author's line stands.
const AUTHOR_BELOW_DP: f32 = 19.0;

/// Who made the recovery, a line under the version in its type.
pub fn author_line(t: &Theme, f: &Fonts, text: &str) -> Sprite {
    hero_line(t, f, text, t.m.dp(AUTHOR_BELOW_DP))
}

/// A line centred under the About wordmark in the version's type, `below`
/// the version's place.
fn hero_line(t: &Theme, f: &Fonts, text: &str, below: f32) -> Sprite {
    let m = &t.m;
    let px = m.dp(13.3);
    let w = Canvas::new(0, 0).text_width(&f.body, text, px);
    let base = m.dp(296.3) + f.body.cap(px) + below;
    let (left, top) = ((m.w as f32 - w) / 2.0 - m.dp(2.0), base - px * 1.2);
    Sprite::capture(left, top, (w + m.dp(4.0)).ceil() as u32, (px * 1.6).ceil() as u32, |c, dx, dy| {
        c.text(&f.body, text, (m.w as f32 - w) / 2.0 + dx, base + dy, px, Rgba::rgb(0, 0, 0), None);
    })
}

/// A row of the About summary card: title on the left, value on the right,
/// and a chevron when the row opens a page.
pub struct Fact<'a> {
    pub title: &'a str,
    pub value: String,
    pub link: bool,
}

/// The About summary card: rows a little shorter than settings rows.
pub fn facts(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, rows: &[Fact], tone: &Tone) -> Block {
    let m = &t.m;
    let row_h = m.dp(51.0);
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let h = row_h * rows.len() as f32;
    c.fill_round_rect(card_x, top, right - card_x, h, m.card_radius, tone.card);
    let mut rects = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        rects.push(Rect::new(card_x, top + row_h * i as f32, right - card_x, row_h));
        let cy = top + row_h * (i as f32 + 0.5);
        let tx = card_x + m.inner;
        let tw = c.text_vc(&f.heading, r.title, tx, cy, m.row_title_px, tone.text, None);
        let value_right = if r.link {
            chevron(c, t, right - m.chevron_inset, cy, tone.chevron);
            right - m.chevron_inset - m.chevron_w - m.value_gap
        } else {
            right - m.inner
        };
        let room = value_right - (tx + tw + m.dp(16.0));
        let w = c.text_width(&f.value, &r.value, m.sub_px).min(room);
        c.text_vc(&f.value, &r.value, value_right - w, cy, m.sub_px, tone.value, Some(room));
    }
    Block { bottom: top + h, rows: rects }
}

/// The About page hardware card: the model in large type (`title` may hold
/// several lines separated by `\n`), then values with small captions.
pub fn spec_card(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, title: &str, items: &[(&str, String)], tone: &Tone) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let x = card_x + m.inner;
    let room = right - m.inner - x;
    let title_px = m.dp(24.0);
    let lines: Vec<String> = title.split('\n').flat_map(|l| wrap(c, &f.heading, l, title_px, room)).collect();
    let first_title = top + m.dp(23.0) + f.heading.cap(title_px);
    let title_pitch = m.dp(33.0);
    let first = first_title + title_pitch * lines.len().saturating_sub(1) as f32 + m.dp(39.4);
    let (pitch, drop) = (m.dp(45.7), m.dp(15.5));
    let last_caption = first + pitch * items.len().saturating_sub(1) as f32 + drop;
    let bottom = last_caption + m.dp(19.7);
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, tone.card);
    for (k, line) in lines.iter().enumerate() {
        c.text(&f.heading, line, x, first_title + title_pitch * k as f32, title_px, tone.text, Some(room));
    }
    for (i, (caption, value)) in items.iter().enumerate() {
        let base = first + pitch * i as f32;
        c.text(&f.heading, value, x, base, m.dp(14.1), tone.text, Some(room));
        c.text(&f.body, caption, x, base + drop, m.dp(11.0), tone.caption, None);
    }
    bottom
}

/// One cell of the parameters grid. `value` may hold several lines separated
/// by `\n`; each is wrapped to the column.
pub struct Spec<'a> {
    pub icon: Outline,
    pub label: &'a str,
    pub value: String,
}

/// Two-column grid of icon, caption and value, as on the stock「全部参数」.
pub fn spec_grid(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, items: &[Spec]) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let cols = [card_x + m.inner, card_x + (right - card_x) * 0.507];
    let widths = [cols[1] - cols[0] - m.inner, right - m.inner - cols[1]];
    let (value_px, pitch) = (m.dp(16.0), m.dp(21.5));
    let (to_label, to_value) = (m.dp(30.8), m.dp(23.0));
    let cells: Vec<Vec<String>> = items
        .iter()
        .enumerate()
        .map(|(i, s)| s.value.split('\n').flat_map(|l| wrap(c, &f.heading, l, value_px, widths[i % 2])).collect())
        .collect();
    let mut centres = Vec::new();
    let mut cy = top + m.dp(38.2);
    let mut last = cy;
    for pair in cells.chunks(2) {
        centres.push(cy);
        let lines = pair.iter().map(Vec::len).max().unwrap_or(1).max(1);
        last = cy + to_label + to_value + pitch * (lines - 1) as f32;
        cy = last + m.dp(28.3);
    }
    let bottom = last + m.dp(19.7);
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, t.pal.card);
    for (i, (s, lines)) in items.iter().zip(&cells).enumerate() {
        let (x, icy) = (cols[i % 2], centres[i / 2]);
        outline(c, t, s.icon, x + m.dp(11.5), icy);
        let label_base = icy + to_label;
        c.text(&f.body, s.label, x, label_base, m.sub_px, t.pal.subtitle, Some(widths[i % 2]));
        for (k, l) in lines.iter().enumerate() {
            c.text(&f.heading, l, x, label_base + to_value + pitch * k as f32, value_px, t.pal.info, None);
        }
    }
    bottom
}

/// Parameters-page card of title-over-value entries; values keep their own
/// line breaks and wrap to the card.
pub fn info_card(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, items: &[(&str, String)]) -> f32 {
    let m = &t.m;
    let (card_x, right) = (m.margin, m.w as f32 - m.margin);
    let x = card_x + m.inner;
    let width = right - m.inner - x;
    let (ink_top, _) = f.heading.ink(m.row_title_px);
    let (to_value, pitch) = (m.dp(20.2), m.dp(19.0));
    let blocks: Vec<Vec<String>> =
        items.iter().map(|(_, v)| v.lines().flat_map(|l| wrap(c, &f.body, l, m.sub_px, width)).collect()).collect();
    let mut bases = Vec::new();
    let mut title_top = top + m.dp(23.5);
    let mut last = title_top;
    for lines in &blocks {
        let base = title_top - ink_top;
        bases.push(base);
        last = base + to_value + pitch * lines.len().saturating_sub(1) as f32;
        title_top = last + m.dp(33.0);
    }
    let bottom = last + m.dp(24.0);
    c.fill_round_rect(card_x, top, right - card_x, bottom - top, m.card_radius, t.pal.card);
    for ((title, _), (lines, base)) in items.iter().zip(blocks.iter().zip(&bases)) {
        c.text(&f.heading, title, x, *base, m.row_title_px, t.pal.info, Some(width));
        for (k, l) in lines.iter().enumerate() {
            c.text(&f.body, l, x, base + to_value + pitch * k as f32, m.sub_px, t.pal.subtitle, None);
        }
    }
    bottom
}

/// The height of each line on a card of what a program showed.
const OUTPUT_PITCH_DP: f32 = 19.0;
/// From the card's top to the ink of its first line.
const OUTPUT_TOP_DP: f32 = 18.0;
/// From the baseline of the card's last line to its foot.
const OUTPUT_FOOT_DP: f32 = 20.0;

/// One line a program showed, as its card lays it out: in the monospaced
/// face where it has every character of the line, so that what is laid out
/// in columns stays so, and in the body face otherwise; wrapped to the
/// card, on one line at least.
fn output_line<'f>(c: &Canvas, t: &Theme, f: &'f Fonts, line: &str) -> (&'f Face, Vec<String>) {
    let m = &t.m;
    let face = if line.chars().all(|ch| f.mono.has(ch)) { &f.mono } else { &f.body };
    let parts = wrap(c, face, line, m.sub_px, m.w as f32 - 2.0 * (m.margin + m.inner));
    (face, if parts.is_empty() { vec![String::new()] } else { parts })
}

/// Where a line's baseline sits in a band as tall as the line, the ink of
/// 「中」 centred in it; then the padding the card's first row has above its
/// bands, and its last row below them.
fn output_layout(t: &Theme, f: &Fonts) -> (f32, f32, f32) {
    let m = &t.m;
    let pitch = m.dp(OUTPUT_PITCH_DP);
    let (ink_top, ink_bottom) = f.body.ink(m.sub_px);
    let base = (pitch - ink_top - ink_bottom) / 2.0;
    (base, m.dp(OUTPUT_TOP_DP) - (base + ink_top), m.dp(OUTPUT_FOOT_DP) - (pitch - base))
}

/// The rows of a card of the lines a program showed, one to each line and
/// as tall as it wraps to, the first starting at `top` and the first and
/// the last holding the card's padding; whole pixels, so the rows meet
/// without seams. Each is drawn as it comes into view, by [`output_row`].
pub fn output_edges(c: &Canvas, t: &Theme, f: &Fonts, top: f32, lines: &[String]) -> Vec<f32> {
    let pitch = t.m.dp(OUTPUT_PITCH_DP);
    let (_, above, below) = output_layout(t, f);
    let mut y = top.round();
    let mut edges = Vec::with_capacity(lines.len() + 1);
    edges.push(y);
    for (k, line) in lines.iter().enumerate() {
        let (_, parts) = output_line(c, t, f, line);
        let pad = if k == 0 { above } else { 0.0 } + if k + 1 == lines.len() { below } else { 0.0 };
        y += (pitch * parts.len() as f32 + pad).round();
        edges.push(y);
    }
    edges
}

/// Line `i` of `lines` over the whole of `c`, as its row of the card
/// [`output_edges`] lays out. The card runs on past each end of the row
/// that it goes on from, so only its first and last rows round it.
pub fn output_row(c: &mut Canvas, t: &Theme, f: &Fonts, lines: &[String], i: usize) {
    let m = &t.m;
    let pitch = m.dp(OUTPUT_PITCH_DP);
    let (base, above, _) = output_layout(t, f);
    let y0 = if i == 0 { 0.0 } else { -m.card_radius };
    let y1 = c.h as f32 + if i + 1 == lines.len() { 0.0 } else { m.card_radius };
    c.clear(t.pal.bg);
    c.fill_round_rect(m.margin, y0, m.w as f32 - 2.0 * m.margin, y1 - y0, m.card_radius, t.pal.card);
    let (face, parts) = output_line(c, t, f, &lines[i]);
    let first = base + if i == 0 { above } else { 0.0 };
    for (k, part) in parts.iter().enumerate() {
        c.text(face, part, m.margin + m.inner, first + pitch * k as f32, m.sub_px, t.pal.info, None);
    }
}

/// Light and dark previews side by side; the active one carries an accent
/// ring and an accent caption. The rows are the light and the dark choice.
pub fn theme_picker(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, top: f32) -> Block {
    let m = &t.m;
    let gap = m.dp(4.0);
    let slot_w = (m.w as f32 - m.margin * 2.0 - gap) / 2.0;
    let (ring, inset) = (m.dp(3.0), m.dp(6.0));
    let tile = slot_w - inset * 2.0;
    let label_px = m.dp(15.9);
    let (ink_top, ink_bottom) = f.heading.ink(label_px);
    let caption_base = top + slot_w + m.dp(11.0) - ink_top;
    let bottom = caption_base + ink_bottom;
    let mut rects = Vec::new();
    let modes = [(["浅色模式", "淺色模式", "Light mode"], false), (["深色模式", "深色模式", "Dark mode"], true)];
    for (i, (label, dark)) in modes.map(|(label, dark)| (l.pick(label), dark)).into_iter().enumerate() {
        let sx = m.margin + (slot_w + gap) * i as f32;
        let on = t.dark == dark;
        if on {
            let r = m.dp(26.0) - ring / 2.0;
            c.stroke_round_rect(sx + ring / 2.0, top + ring / 2.0, slot_w - ring, slot_w - ring, r, ring, t.pal.accent);
        }
        theme_tile(c, t, sx + inset, top + inset, tile, tile, dark);
        let w = c.text_width(&f.heading, label, label_px);
        let color = if on { t.pal.accent } else { t.pal.info };
        c.text(&f.heading, label, sx + (slot_w - w) / 2.0, caption_base, label_px, color, None);
        rects.push(Rect::new(sx, top, slot_w, bottom - top));
    }
    Block { bottom, rows: rects }
}

/// What a card on the systems page shows: what the system is to the phone,
/// in small type, its name in large, and its version or what the card
/// offers under that.
pub struct SystemCard<'a> {
    pub role: &'a str,
    pub name: &'a str,
    pub detail: &'a str,
    pub mark: SystemMark,
}

/// What a system card shows at its right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemMark {
    /// The check of a choice, filled on the system picked.
    Check(bool),
    /// The place a second system would take: the card opens a page, and
    /// its name is in the accent colour, as an action.
    Opens,
}

/// A system card's height, and the size of its name.
const SYSTEM_CARD_DP: f32 = 132.0;
const SYSTEM_NAME_DP: f32 = 30.0;

/// The systems as big cards one over the other: each a choice of a list
/// grown large, in the same type and colours, its name in the type of a
/// page's title. The rows are the cards.
pub fn system_cards(c: &mut Canvas, t: &Theme, f: &Fonts, top: f32, cards: &[SystemCard]) -> Block {
    let m = &t.m;
    let (x, w, h) = (m.margin, m.w as f32 - m.margin * 2.0, m.dp(SYSTEM_CARD_DP));
    let mut rows = Vec::new();
    let mut y = top;
    for card in cards {
        let r = Rect::new(x, y, w, h);
        system_card(c, t, f, r, card);
        rows.push(r);
        y += h + m.card_gap;
    }
    Block { bottom: y - m.card_gap, rows }
}

/// One system's card at `r`: the role, the name and the line under it as
/// one block in the middle of the card's height, and its mark at the right.
fn system_card(c: &mut Canvas, t: &Theme, f: &Fonts, r: Rect, card: &SystemCard) {
    let m = &t.m;
    c.fill_round_rect(r.x, r.y, r.w, r.h, m.card_radius, t.pal.card);
    let (x, cy) = (r.x + m.inner, r.y + r.h / 2.0);
    let room = r.w - m.inner * 2.0 - m.dp(36.0);
    let px = fitted_px(c, &f.title, card.name, m.dp(SYSTEM_NAME_DP), m.dp(20.0), room);
    let ink = |face: &Face, px: f32| {
        let (top, bottom) = face.ink(px);
        bottom - top
    };
    let (role_h, name_h, detail_h) = (ink(&f.label, m.label_px), ink(&f.title, px), ink(&f.body, m.sub_px));
    let (above, below) = (m.dp(12.0), m.dp(12.0));
    let mut y = cy - (role_h + above + name_h + below + detail_h) / 2.0;
    c.text_top(&f.label, card.role, x, y, m.label_px, t.pal.section, Some(room));
    y += role_h + above;
    let name_ink = if card.mark == SystemMark::Opens { t.pal.accent } else { t.pal.text };
    // Only a name too long even at the least size is cut short; one sized
    // to fit is drawn whole, its last pixel's rounding aside.
    let cut = (c.text_width(&f.title, card.name, px) > room + 1.0).then_some(room);
    c.text_top(&f.title, card.name, x, y, px, name_ink, cut);
    y += name_h + below;
    c.text_top(&f.body, card.detail, x, y, m.sub_px, t.pal.subtitle, Some(room));
    let right = r.x + r.w;
    match card.mark {
        SystemMark::Check(on) => check_circle(c, t, right - m.inner - m.dp(12.0), cy, on),
        SystemMark::Opens => chevron(c, t, right - m.chevron_inset, cy, t.pal.chevron),
    }
}

/// The check of a choice centred on (`cx`, `cy`): an accent disc with a
/// white tick on the one chosen, an empty ring on the others.
pub fn check_circle(c: &mut Canvas, t: &Theme, cx: f32, cy: f32, on: bool) {
    let m = &t.m;
    let d = m.dp(24.0);
    if on {
        c.fill_circle(cx, cy, d / 2.0, t.pal.accent);
        let s = d * 0.5;
        c.line(cx - s * 0.42, cy + s * 0.02, cx - s * 0.1, cy + s * 0.32, m.stroke, Rgba::rgb(255, 255, 255));
        c.line(cx - s * 0.1, cy + s * 0.32, cx + s * 0.44, cy - s * 0.3, m.stroke, Rgba::rgb(255, 255, 255));
    } else {
        c.stroke_circle(cx, cy, d / 2.0 - m.dp(1.0), m.dp(2.0), t.pal.switch_off);
    }
}

/// The largest size, from `most` down to `least`, at which `text` fits in
/// `room`.
fn fitted_px(c: &Canvas, face: &Face, text: &str, most: f32, least: f32, room: f32) -> f32 {
    let w = c.text_width(face, text, most);
    if w <= room {
        most
    } else {
        (most * room / w).max(least)
    }
}

/// Which keys the on-screen keyboard shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Letters,
    /// Capitals, for the next letter typed.
    Shifted,
    /// Digits and the common symbols.
    Digits,
    /// The rest of the ASCII symbols.
    Symbols,
}

/// A key of the on-screen keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Char(char),
    Shift,
    Backspace,
    Space,
    Enter,
    /// Show another layer.
    Show(Layer),
}

/// The keys between the side keys of each row, per layer. Together the
/// digit and symbol layers hold every printable ASCII character.
fn rows(layer: Layer) -> [&'static str; 3] {
    match layer {
        Layer::Letters | Layer::Shifted => ["qwertyuiop", "asdfghjkl", "zxcvbnm"],
        Layer::Digits => ["1234567890", "-/:;()$&@\"", ".,?!'"],
        Layer::Symbols => ["[]{}#%^*+=", "_\\|~<>`", ".,?!'"],
    }
}

const KEY_H_DP: f32 = 44.0;
/// How round a key's corners are.
pub const KEY_RADIUS_DP: f32 = 6.0;
const KEY_GAP_X_DP: f32 = 5.5;
const KEY_GAP_Y_DP: f32 = 11.0;
const KEYBOARD_PAD_X_DP: f32 = 3.5;
const KEYBOARD_TOP_DP: f32 = 9.0;
/// Room under the last row for the gesture area of the screen's edge.
const KEYBOARD_BOTTOM_DP: f32 = 26.0;

/// How tall the keyboard is on this theme's screen.
pub fn keyboard_height(t: &Theme) -> f32 {
    let m = &t.m;
    m.dp(KEYBOARD_TOP_DP + 4.0 * KEY_H_DP + 3.0 * KEY_GAP_Y_DP + KEYBOARD_BOTTOM_DP)
}

/// The on-screen keyboard across the bottom of the screen, its top at
/// `top`: four rows of keys in `layer`, `pressed` drawn held down, and the
/// return key saying what it does; its words in `l`. Returns where each
/// key answers a touch, the gaps between keys shared out among their
/// neighbours.
#[allow(clippy::too_many_arguments)]
pub fn keyboard(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, top: f32, layer: Layer, pressed: Option<Cap>, enter: &str) -> Vec<(Rect, Cap)> {
    let m = &t.m;
    let w = m.w as f32;
    let (panel, key, side, ink, shadow) = if t.dark {
        (Rgba::rgb(18, 18, 18), Rgba::rgb(58, 58, 58), Rgba::rgb(38, 38, 38), Rgba::rgb(237, 237, 237), Rgba::rgb(0, 0, 0))
    } else {
        (Rgba::rgb(224, 226, 231), Rgba::rgb(255, 255, 255), Rgba::rgb(193, 198, 207), Rgba::rgb(0, 0, 0), Rgba::rgb(138, 141, 147))
    };
    c.fill_rect(0.0, top, w, keyboard_height(t), panel);
    let (pad, gx, gy, kh) = (m.dp(KEYBOARD_PAD_X_DP), m.dp(KEY_GAP_X_DP), m.dp(KEY_GAP_Y_DP), m.dp(KEY_H_DP));
    let kw = (w - pad * 2.0 - gx * 9.0) / 10.0;
    let side_w = kw * 1.35;
    let row_top = |r: usize| top + m.dp(KEYBOARD_TOP_DP) + r as f32 * (kh + gy);
    let mut keys: Vec<(Rect, Cap)> = Vec::new();
    let [first, second, third] = rows(layer);
    // Rows of plain keys are centred: the letters' second row sits half a
    // key in, as on any phone keyboard.
    let centred = |n: usize, width: f32| (w - (n as f32 * width + (n as f32 - 1.0) * gx)) / 2.0;
    for (r, row) in [first, second].iter().enumerate() {
        let n = row.chars().count();
        let x0 = centred(n, kw);
        for (k, ch) in row.chars().enumerate() {
            keys.push((Rect::new(x0 + k as f32 * (kw + gx), row_top(r), kw, kh), Cap::Char(ch)));
        }
    }
    let (left, right) = (pad, w - pad - side_w);
    let shift = match layer {
        Layer::Letters | Layer::Shifted => Cap::Shift,
        Layer::Digits => Cap::Show(Layer::Symbols),
        Layer::Symbols => Cap::Show(Layer::Digits),
    };
    keys.push((Rect::new(left, row_top(2), side_w, kh), shift));
    let n = third.chars().count();
    let room = right - gx * 2.0 - (left + side_w + gx * 2.0);
    let mid_w = ((room - (n as f32 - 1.0) * gx) / n as f32).min(kw * 1.4);
    let x0 = centred(n, mid_w);
    for (k, ch) in third.chars().enumerate() {
        keys.push((Rect::new(x0 + k as f32 * (mid_w + gx), row_top(2), mid_w, kh), Cap::Char(ch)));
    }
    keys.push((Rect::new(right, row_top(2), side_w, kh), Cap::Backspace));
    let switch = match layer {
        Layer::Letters | Layer::Shifted => Cap::Show(Layer::Digits),
        Layer::Digits | Layer::Symbols => Cap::Show(Layer::Letters),
    };
    let enter_w = kw * 2.2;
    let space_x = left + side_w + gx + kw + gx;
    let space_w = w - pad - enter_w - gx - kw - gx - space_x;
    keys.push((Rect::new(left, row_top(3), side_w, kh), switch));
    keys.push((Rect::new(left + side_w + gx, row_top(3), kw, kh), Cap::Char(',')));
    keys.push((Rect::new(space_x, row_top(3), space_w, kh), Cap::Space));
    keys.push((Rect::new(space_x + space_w + gx, row_top(3), kw, kh), Cap::Char('.')));
    keys.push((Rect::new(w - pad - enter_w, row_top(3), enter_w, kh), Cap::Enter));

    let radius = m.dp(KEY_RADIUS_DP);
    let glyph_px = m.dp(21.0);
    for &(r, cap) in &keys {
        let plain = matches!(cap, Cap::Char(_) | Cap::Space);
        let base = match cap {
            Cap::Enter => t.pal.accent,
            Cap::Shift if layer == Layer::Shifted => key,
            _ if plain => key,
            _ => side,
        };
        let fill = if pressed == Some(cap) { mix(base, ink, 0.16) } else { base };
        if !t.dark {
            c.fill_round_rect(r.x, r.y + m.dp(1.0), r.w, r.h, radius, shadow.with_alpha(110));
        }
        c.fill_round_rect(r.x, r.y, r.w, r.h, radius, fill);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        match cap {
            Cap::Char(ch) => {
                let label = if layer == Layer::Shifted { ch.to_ascii_uppercase() } else { ch };
                c.text_center(&f.heading, &label.to_string(), cx, cy, glyph_px, ink);
            }
            Cap::Space => c.text_center(&f.body, l.pick(["空格", "空格", "Space"]), cx, cy, m.sub_px, ink),
            Cap::Enter => c.text_center(&f.heading, enter, cx, cy, m.sub_px, Rgba::rgb(255, 255, 255)),
            Cap::Show(Layer::Digits) => c.text_center(&f.heading, "123", cx, cy, m.sub_px, ink),
            Cap::Show(Layer::Symbols) => c.text_center(&f.heading, "#+=", cx, cy, m.sub_px, ink),
            Cap::Show(_) => c.text_center(&f.heading, "abc", cx, cy, m.sub_px, ink),
            Cap::Shift => {
                // An upward arrow, solid while the next letter is a capital.
                let s = m.dp(9.0);
                let pts = [
                    (cx, cy - s),
                    (cx + s, cy),
                    (cx + s * 0.45, cy),
                    (cx + s * 0.45, cy + s * 0.9),
                    (cx - s * 0.45, cy + s * 0.9),
                    (cx - s * 0.45, cy),
                    (cx - s, cy),
                ];
                if layer == Layer::Shifted {
                    c.fill_convex(&pts[..3], ink);
                    c.fill_convex(&[pts[0], pts[6], pts[5]], ink);
                    c.fill_rect(cx - s * 0.45, cy - m.dp(0.5), s * 0.9, s * 0.9 + m.dp(0.5), ink);
                } else {
                    for k in 0..pts.len() {
                        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                        c.line(a.0, a.1, b.0, b.1, m.dp(1.6), ink);
                    }
                }
            }
            Cap::Backspace => {
                // A key cap pointing left, with a cross in it.
                let (hw, hh) = (m.dp(11.0), m.dp(8.0));
                let pts = [(cx - hw, cy), (cx - hw * 0.45, cy - hh), (cx + hw, cy - hh), (cx + hw, cy + hh), (cx - hw * 0.45, cy + hh)];
                for k in 0..pts.len() {
                    let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                    c.line(a.0, a.1, b.0, b.1, m.dp(1.6), ink);
                }
                let (xc, s) = (cx + hw * 0.22, m.dp(3.6));
                c.line(xc - s, cy - s, xc + s, cy + s, m.dp(1.6), ink);
                c.line(xc - s, cy + s, xc + s, cy - s, m.dp(1.6), ink);
            }
        }
    }
    // A touch between keys belongs to the nearest one.
    keys.into_iter().map(|(r, cap)| (Rect::new(r.x - gx / 2.0, r.y - gy / 2.0, r.w + gx, r.h + gy), cap)).collect()
}

/// Miniature settings page: a blue backdrop with two cards of placeholder
/// lines, light cards for the light theme and dark ones for the dark theme.
fn theme_tile(c: &mut Canvas, t: &Theme, x: f32, y: f32, w: f32, h: f32, dark: bool) {
    let (top, bottom) = (Rgba::rgb(77, 113, 246), Rgba::rgb(115, 184, 249));
    let grad = |py: f32| mix(top, bottom, (py - y) / h);
    c.fill_round_rect_graded(x, y, w, h, t.m.dp(20.0), &grad);
    let card = if dark { Rgba::rgb(0, 0, 0).with_alpha(51) } else { Rgba::rgb(255, 255, 255).with_alpha(51) };
    let lw = h * 0.027;
    for (c0, c1) in [(0.218f32, 0.472f32), (0.523, 0.779)] {
        c.fill_round_rect(x + w * 0.069, y + h * c0, w * 0.86, h * (c1 - c0), w * 0.045, card);
        for (at, end) in [(0.37f32, 0.841f32), (0.64, 0.622)] {
            let ly = y + h * (c0 + (c1 - c0) * at);
            let base = grad(ly);
            let ink = if dark { mix(base, Rgba::rgb(180, 190, 190), 0.45) } else { mix(base, Rgba::rgb(0, 0, 0), 0.14) };
            c.line(x + w * 0.162 + lw / 2.0, ly, x + w * end - lw / 2.0, ly, lw, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The battery's pixel at (`x`, `y`) on screen.
    fn pixel(art: &(Canvas, i32, i32), x: f32, y: f32) -> [u8; 4] {
        let (c, ox, oy) = art;
        let i = ((y as i32 - oy) as usize * c.w as usize + (x as i32 - ox) as usize) * 4;
        c.data[i..i + 4].try_into().unwrap()
    }

    /// The box round the pixels of `c` in `area` that `keep` picks: its
    /// left, top, right and bottom edges.
    fn ink_box(c: &Canvas, area: Rect, keep: impl Fn(&[u8]) -> bool) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for y in area.y as u32..(area.y + area.h) as u32 {
            for x in area.x as u32..(area.x + area.w) as u32 {
                let i = ((y * c.w + x) * 4) as usize;
                if keep(&c.data[i..i + 4]) {
                    b = (b.0.min(x as f32), b.1.min(y as f32), b.2.max(x as f32 + 1.0), b.3.max(y as f32 + 1.0));
                }
            }
        }
        b
    }

    #[test]
    fn the_wlan_fan_lights_as_many_arcs_as_the_signal_and_the_padlock_stands_as_asked() {
        let (black, white, grey) = (Rgba::rgb(0, 0, 0), Rgba::rgb(255, 255, 255), Rgba::rgb(0, 0, 255));
        let mut c = Canvas::new(400, 400);
        c.clear(black);
        wlan_fan(&mut c, 200.0, 200.0, 150.0, 1, white, grey);
        let lit = ink_box(&c, Rect::new(0.0, 0.0, 400.0, 400.0), |p| p[0] > 0);
        let unlit = ink_box(&c, Rect::new(0.0, 0.0, 400.0, 400.0), |p| p[2] > 0 && p[0] == 0);
        assert!(lit.1 > unlit.1 + 60.0, "only the inner arc is lit: lit from {}, unlit from {}", lit.1, unlit.1);
        let mut c = Canvas::new(400, 400);
        c.clear(black);
        padlock(&mut c, 200.0, 200.0, 300.0, white);
        let (l, top, right, bottom) = ink_box(&c, Rect::new(0.0, 0.0, 400.0, 400.0), |p| p[0] > 0);
        assert!(((l + right) / 2.0 - 200.0).abs() <= 2.0 && ((top + bottom) / 2.0 - 200.0).abs() <= 2.0, "the padlock is centred");
        assert!((bottom - top - 300.0).abs() <= 3.0, "and as tall as asked: {}", bottom - top);
    }

    #[test]
    fn the_network_details_icons_stand_in_the_middle_of_their_place() {
        let t = Theme::light(1200, 2608, 3.0);
        for icon in [Outline::Info, Outline::Wifi, Outline::Speed, Outline::Signal, Outline::Shield, Outline::Address, Outline::Mask, Outline::Router] {
            let mut c = Canvas::new(200, 200);
            c.clear(Rgba::rgb(255, 255, 255));
            outline_in(&mut c, &t, icon, 100.0, 100.0, Rgba::rgb(0, 0, 0));
            let (l, top, right, bottom) = ink_box(&c, Rect::new(0.0, 0.0, 200.0, 200.0), |p| p[0] < 200);
            let (x, y) = ((l + right) / 2.0, (top + bottom) / 2.0);
            // Within a dp of the place's centre, three pixels a dp.
            assert!((x - 100.0).abs() <= 3.0 && (y - 100.0).abs() <= 3.0, "{icon:?} centred: ({x}, {y})");
            assert!(right - l <= 66.0 && bottom - top <= 66.0, "{icon:?} within 22 dp: {}x{}", right - l, bottom - top);
        }
    }

    #[test]
    fn the_network_joined_shows_in_the_status_bar_where_the_stock_one_does() {
        // The 17 Ultra's bar over its grey page, three pixels a dp, charging:
        // its fan's ink from 930 to 968 and 59 to 94, its arrows' rows across
        // 970..979, and the speed's lines from 54 to 71 and 80 to 95, ending
        // 13 pixels short of the fan.
        let t = Theme::light(1200, 2608, 3.0);
        let f = crate::font::bundled();
        let draw = |mark: WlanMark| {
            let mut c = Canvas::filled(1200, 160, t.pal.bg);
            let bar = Bar { clock: "01:27", battery: Some((40, Some(Bolt::Charging))), wlan: Some(mark), data: None, slot: None, temp: None };
            status_bar(&mut c, &t, &f, &bar, t.pal.status);
            c
        };
        let full = WlanMark { level: 3, receiving: true, sending: true, speed: "7.02".into(), unit: "KB/s" };
        let c = draw(full);
        let inked = |p: &[u8]| p[0] < 220;
        let fan = ink_box(&c, Rect::new(920.0, 40.0, 49.0, 70.0), inked);
        assert!((fan.0 - 930.0).abs() <= 1.0 && (fan.1 - 59.0).abs() <= 1.0 && (fan.3 - 95.0).abs() <= 1.0, "fan {fan:?}");
        for row in [85.0, 90.0] {
            let arrow = ink_box(&c, Rect::new(965.0, row, 20.0, 1.0), |p| p[0] < 200);
            assert!((arrow.0 - 970.0).abs() <= 1.0 && (arrow.2 - 980.0).abs() <= 1.0, "arrow row {row}: {arrow:?}");
        }
        let figures = ink_box(&c, Rect::new(840.0, 45.0, 85.0, 30.0), inked);
        let unit = ink_box(&c, Rect::new(840.0, 76.0, 85.0, 24.0), inked);
        assert!((figures.1 - 54.0).abs() <= 1.0 && (figures.3 - 72.0).abs() <= 1.0, "figures {figures:?}");
        assert!((unit.1 - 80.0).abs() <= 1.0 && (unit.3 - 96.0).abs() <= 1.0, "unit {unit:?}");
        assert!((fan.0 - figures.2.max(unit.2) - 13.0).abs() <= 1.5, "the speed ends short of the fan: {figures:?} {unit:?}");
        // A weak signal lights the slice alone, and data going neither way
        // leaves both arrows dim, in the battery's empty grey.
        let c = draw(WlanMark { level: 1, receiving: false, sending: false, speed: "0.00".into(), unit: "KB/s" });
        let dim = over(t.pal.battery.empty, t.pal.bg);
        let at = |x: u32, y: u32| Rgba::rgb(c.data[((y * 1200 + x) * 4) as usize], c.data[((y * 1200 + x) * 4 + 1) as usize], c.data[((y * 1200 + x) * 4 + 2) as usize]);
        assert_eq!(at(954, 62), dim, "the outer arc");
        assert_eq!(at(954, 75), dim, "the inner arc");
        assert_eq!(at(954, 88), t.pal.status, "the slice");
        assert_eq!((at(975, 84), at(975, 91)), (dim, dim), "the arrows");
    }

    #[test]
    fn the_marks_stand_in_the_stock_order_and_spacing_left_of_the_battery() {
        // On the 17 Ultra's bar, three pixels a dp, the stock network speed
        // stands 13 pixels short of the next mark, and so does each mark of
        // its own the next.
        let t = Theme::dark(1200, 2608, 3.0);
        let f = crate::font::bundled();
        let mut c = Canvas::filled(1200, 160, t.pal.bg);
        let wlan = Some(WlanMark { level: 3, receiving: true, sending: true, speed: "0.00".into(), unit: "KB/s" });
        let bar = Bar { clock: "01:27", battery: Some((40, None)), wlan, data: Some(false), slot: Some("B"), temp: Some(362) };
        status_bar(&mut c, &t, &f, &bar, t.pal.status);
        // Runs of inked columns on the bar's right half, those a few pixels
        // apart taken as one mark.
        let inked = |x: u32| (40..120).any(|y| c.data[((y * 1200 + x) * 4) as usize] > 60);
        let mut marks: Vec<(u32, u32)> = Vec::new();
        for x in (600..1200).filter(|&x| inked(x)) {
            match marks.last_mut() {
                Some(m) if x <= m.1 + 5 => m.1 = x + 1,
                _ => marks.push((x, x + 1)),
            }
        }
        assert_eq!(marks.len(), 6, "speed, temperature, badge, lock, fan and battery: {marks:?}");
        for k in 0..4 {
            let gap = marks[k + 1].0 - marks[k].1;
            assert!((gap as i32 - 13).abs() <= 1, "mark {k} stands 13 pixels short of the next: {marks:?}");
        }
    }

    #[test]
    fn the_slot_is_cut_out_of_a_badge_as_the_stock_bar_draws_nfc() {
        // The stock NFC badge, three pixels a dp: 38 pixels square over the
        // bar's middle, here with a capital 20 tall cut out of it.
        let t = Theme::dark(1200, 2608, 3.0);
        let f = crate::font::bundled();
        let (art, _, oy, side) = badge(&t, &f, "B", 1000.0, t.m.status_cy, Rgba::rgb(255, 255, 255));
        assert!((side - 38.0).abs() < 0.1);
        let alpha = |x: u32, y: u32| art.data[((y * art.w + x) * 4 + 3) as usize];
        let rows: Vec<u32> = (0..art.h).filter(|&y| (0..art.w).any(|x| alpha(x, y) > 128)).collect();
        let cols = (0..art.w).filter(|&x| (0..art.h).any(|y| alpha(x, y) > 128)).count();
        assert!((rows.len() as i32 - 38).abs() <= 1 && (cols as i32 - 38).abs() <= 1, "{} by {cols}", rows.len());
        let middle = oy as f32 + (rows[0] + rows[rows.len() - 1] + 1) as f32 / 2.0;
        assert!((middle - t.m.status_cy).abs() <= 1.0, "on the bar's middle: {middle}");
        // Rows of the badge, clear of its corners, where the letter lets what
        // is behind show through.
        let inner = rows[0] + 6..rows[rows.len() - 1] - 5;
        let cut = inner.filter(|&y| (6..art.w - 6).any(|x| alpha(x, y) < 64)).count();
        assert!((cut as i32 - 20).abs() <= 1, "the capital cut out: {cut} rows");
    }

    #[test]
    fn the_lock_opens_its_shackle_up_with_its_right_leg_out_and_its_keyhole_cut() {
        let t = Theme::dark(1200, 2608, 3.0);
        let (m, white, cy) = (&t.m, Rgba::rgb(255, 255, 255), t.m.status_cy);
        let (shut, ox, oy, w) = lock_mark(&t, false, 1000.0, cy, white);
        let (open, _, _, _) = lock_mark(&t, true, 1000.0, cy, white);
        let alpha = |art: &Canvas, x: f32, y: f32| art.data[(((y - oy as f32) as u32 * art.w + (x - ox as f32) as u32) * 4 + 3) as usize];
        let rows = |art: &Canvas| -> Vec<u32> { (0..art.h).filter(|&y| (0..art.w).any(|x| art.data[((y * art.w + x) * 4 + 3) as usize] > 128)).collect() };
        let (shut_rows, open_rows) = (rows(&shut), rows(&open));
        assert!((shut_rows.len() as i32 - 38).abs() <= 1, "12.67 dp tall shut: {}", shut_rows.len());
        let middle = oy as f32 + (shut_rows[0] + shut_rows[shut_rows.len() - 1] + 1) as f32 / 2.0;
        assert!((middle - cy).abs() <= 1.0, "on the bar's middle: {middle}");
        assert!(((shut_rows[0] - open_rows[0]) as f32 - m.dp(LOCK_RISE_DP)).abs() <= 1.0, "open, the shackle rises");
        assert_eq!(shut_rows.last(), open_rows.last(), "on the same body");
        // Just above the body, down the right leg: inked shut, clear open.
        let (mid, body) = (1000.0 - w / 2.0, cy + m.dp(LOCK_MARK_H_DP) / 2.0 - m.dp(LOCK_BODY_H_DP));
        let leg = mid + (m.dp(LOCK_SHACKLE_W_DP) - m.dp(LOCK_STROKE_DP)) / 2.0;
        assert!(alpha(&shut, leg, body - 2.0) > 200 && alpha(&open, leg, body - 2.0) < 30);
        // The keyhole lets what is behind show through the body.
        let keyhole = body + m.dp(LOCK_BODY_H_DP) * 0.42;
        assert!(alpha(&shut, mid, keyhole) < 30 && alpha(&shut, mid - m.dp(3.0), keyhole) > 200);
    }

    #[test]
    fn the_bolts_stand_past_the_battery_as_the_stock_bar_draws_them() {
        // The 17 Ultra's bar, three pixels a dp, charging quickly: the stock
        // bolt covers a quarter of a pixel or more across 1069..1086 and
        // down 60..94, and its left edge steps right at rows 76 and 85.
        let t = Theme::dark(1200, 2608, 3.0);
        let f = crate::font::bundled();
        let draw = |kind| {
            let mut c = Canvas::filled(1200, 160, Rgba::rgb(0, 0, 0));
            let bar = Bar { clock: "01:27", battery: Some((64, Some(kind))), wlan: None, data: None, slot: None, temp: None };
            status_bar(&mut c, &t, &f, &bar, t.pal.status);
            c
        };
        // How much of a pixel the bolt covers, and the rows where its left
        // edge steps right, each the top of a step.
        let cover = |p: &[u8]| p[0] as f32 / t.pal.battery.bolt.a as f32;
        let steps = |c: &Canvas| -> Vec<u32> {
            let lefts: Vec<(u32, u32)> = (40..120)
                .filter_map(|y| (1064..1100).find(|&x| cover(&c.data[((y * 1200 + x) * 4) as usize..]) > 0.6).map(|x| (y, x)))
                .collect();
            lefts.windows(2).filter(|w| w[1].1 >= w[0].1 + 3).map(|w| w[1].0).collect()
        };
        let quick = draw(Bolt::Quick);
        let ink = ink_box(&quick, Rect::new(1064.0, 40.0, 36.0, 80.0), |p| cover(p) > 0.25);
        assert_eq!(ink, (1069.0, 60.0, 1086.0, 94.0), "where the stock bolt is");
        assert_eq!(steps(&quick), [76, 85], "two steps where the stock bolt's are");
        let plain = draw(Bolt::Charging);
        let ink = ink_box(&plain, Rect::new(1064.0, 40.0, 36.0, 80.0), |p| cover(p) > 0.25);
        assert!(ink.0 >= 1068.0 && ink.2 <= 1092.0 && ((ink.1 + ink.3) / 2.0 - t.m.status_cy).abs() <= 1.0, "in the same box: {ink:?}");
        assert_eq!(steps(&plain).len(), 1, "one step while the charger charges as it may");
    }

    #[test]
    fn a_list_of_choices_opens_where_the_stock_one_does_and_ticks_the_one_taken() {
        // The 17 Ultra, three pixels a dp: its row of protections spans
        // 621 to 797, the arrows after its value 1086..1116 by 685..731,
        // and its list 256..1114 from 666, the tick 1003..1040 by 741..771.
        let t = Theme::light(1200, 2608, 3.0);
        let f = crate::font::bundled();
        let mut c = Canvas::filled(1200, 2608, t.pal.bg);
        let rows = list(&mut c, &t, &f, 621.0, &[Row::new("安全性").trail(Trail::Choice("无"))]);
        let row = rows.rows[0];
        let arrows = ink_box(&c, Rect::new(1070.0, row.y + 55.0, 55.0, row.h - 110.0), |p| p[0] < 252);
        let (cy, w, h) = ((arrows.1 + arrows.3) / 2.0, arrows.2 - arrows.0, arrows.3 - arrows.1);
        assert!((arrows.2 - 1116.0).abs() <= 1.0 && (w - 30.0).abs() <= 1.5 && (h - 46.0).abs() <= 1.5, "arrows {arrows:?}");
        assert!((cy - (row.y + row.h / 2.0)).abs() <= 1.0, "the arrows centred on the row: {cy}");
        let stock = Rect::new(36.0, 621.0, 1128.0, 176.0);
        let picks = choices(&mut c, &t, &f, stock, &["无", "WPA/WPA2-Personal", "WPA3-Personal"], Some(0), None);
        let (first, last) = (picks[0], picks[2]);
        assert!((first.x - 256.0).abs() <= 0.5 && (first.x + first.w - 1114.0).abs() <= 0.5, "across {}..{}", first.x, first.x + first.w);
        assert!((first.y - 690.0).abs() <= 0.5 && (last.y + last.h - 1083.0).abs() <= 0.5, "down {}..{}", first.y, last.y + last.h);
        let tick = ink_box(&c, Rect::new(990.0, first.y, 60.0, first.h), |p| p[2] > 200 && p[0] < 150);
        let centre = ((tick.0 + tick.2) / 2.0, (tick.1 + tick.3) / 2.0);
        assert!((centre.0 - 1021.4).abs() <= 1.5 && (centre.1 - 755.9).abs() <= 1.5, "the tick centred where the stock one is: {centre:?}");
        assert!((tick.2 - tick.0 - 37.0).abs() <= 2.0 && (tick.3 - tick.1 - 30.0).abs() <= 2.0, "and as big: {tick:?}");
    }

    #[test]
    fn the_bars_round_button_mirrors_the_back_control_and_centres_its_glyph() {
        let t = Theme::light(1440, 3200, 3.5);
        let (back, button) = (back_rect(&t), button_rect(&t));
        assert_eq!((button.x + button.w, button.y, button.w, button.h), (1440.0 - back.x, back.y, back.w, back.h));
        let (cx, cy, r) = button_disc(&t);
        let disc = Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
        for glyph in [Glyph::Terminal, Glyph::Done] {
            let mut c = Canvas::new(1440, 400);
            button_glyph(&mut c, &t, glyph, Rgba::rgb(0, 0, 0));
            let (l, top, right, bottom) = ink_box(&c, disc, |p| p[3] > 0);
            let ((x, y), size) = (((l + right) / 2.0, (top + bottom) / 2.0), (right - l, bottom - top));
            assert!((x - cx).abs() <= 1.0 && (y - cy).abs() <= 1.0, "{glyph:?} centred on the disc: ({x}, {y}) for ({cx}, {cy})");
            assert!(size.0 < r * 1.2 && size.1 < r, "{glyph:?} well inside it: {size:?}");
        }
    }

    #[test]
    fn the_search_cursor_stands_where_the_stock_field_puts_it() {
        let f = crate::font::bundled();
        // A Xiaomi 13 Ultra panel.
        let t = Theme::light(1440, 3200, 3.5);
        let (x, w, cy) = (t.m.dp(48.0), t.m.dp(340.0), t.m.dp(60.0));
        let px = t.m.row_title_px;
        let (ink_top, ink_bottom) = f.heading.ink(px);
        let base = cy - (ink_top + ink_bottom) / 2.0;
        let (line_top, line_bottom) = f.heading.line(px);
        // The cursor's pixels are blue, darker where the placeholder crosses
        // it; the field, its text and the placeholder are grey.
        let blue = |p: &[u8]| p[2] as i32 - p[0] as i32 > 60;
        for text in ["", "bf"] {
            let mut c = Canvas::new(t.m.w, t.m.dp(120.0) as u32);
            let field = search_field(&mut c, &t, &f, x, w, cy, text, "搜索功能与分区");
            assert_eq!(ink_box(&c, field.field, blue).0, f32::MAX, "{text:?}: the field is drawn without its cursor");
            // The cursor laid over the field, as the page does while it shows.
            c.lay(&field.caret.art, field.caret.x, field.caret.y, 1.0);
            let (cl, ct, cr, cb) = ink_box(&c, field.field, blue);
            let left = x + t.m.search_text_inset + c.text_width(&f.heading, text, px);
            let across = format!("{cl}..{cr} × {ct}..{cb}");
            assert!((cl - left).abs() <= 1.0 && (cr - left - t.m.dp(2.0)).abs() <= 1.0, "{text:?}: 2 dp wide from where the next character goes: {across}");
            assert!((ct - (base + line_top)).abs() <= 1.0 && (cb - (base + line_bottom)).abs() <= 1.0, "{text:?}: as tall as the line: {across}");
        }
    }

    #[test]
    fn the_battery_fills_its_level_in_the_colour_of_its_state() {
        let f = crate::font::bundled();
        let t = Theme::light(1200, 2608, 3.0);
        let ink = t.pal.battery;
        let (right, cy) = (1000.0, 77.0);
        // 69 px wide from 930.85; the row 15 px above the middle is clear
        // of the digits.
        let rgba = |c: Rgba| [c.r, c.g, c.b, c.a];
        let half = battery(&t, &f, 50, false, right, cy);
        assert_eq!(pixel(&half, 948.0, cy - 15.0), rgba(ink.level), "a quarter in is filled");
        assert_eq!(pixel(&half, 983.0, cy - 15.0), rgba(ink.empty), "three quarters in is empty");
        let charging = battery(&t, &f, 50, true, right, cy);
        assert_eq!(pixel(&charging, 948.0, cy - 15.0), rgba(ink.charging), "the cable in fills green");
        let low = battery(&t, &f, 15, false, right, cy);
        assert_eq!(pixel(&low, 935.0, cy), rgba(ink.low), "low and unplugged fills red");
        assert_eq!(pixel(&low, 948.0, cy - 15.0), rgba(ink.empty), "past 15% it is empty");
        let low_charging = battery(&t, &f, 15, true, right, cy);
        assert_eq!(pixel(&low_charging, 935.0, cy), rgba(ink.charging), "low but charging stays green");
        let full = battery(&t, &f, 100, false, right, cy);
        assert_eq!(pixel(&full, right - 1.5, cy), rgba(ink.level), "a full battery fills its cap");
    }

    #[test]
    fn digits_are_cut_out_of_a_plain_fill_and_white_over_a_coloured_one() {
        let f = crate::font::bundled();
        let t = Theme::dark(1200, 2608, 3.0);
        let (right, cy) = (1000.0, 77.0);
        // Inside the body, clear of its rounded ends and edges.
        let holes = |art: &(Canvas, i32, i32)| -> usize {
            let mut n = 0;
            for y in (cy - 15.0) as i32..(cy + 15.0) as i32 {
                for x in 945..985 {
                    n += usize::from(pixel(art, x as f32, y as f32)[3] < 40);
                }
            }
            n
        };
        assert!(holes(&battery(&t, &f, 100, false, right, cy)) > 60, "the digits show the page through");
        assert_eq!(holes(&battery(&t, &f, 100, true, right, cy)), 0, "white digits over green");
        assert_eq!(holes(&battery(&t, &f, 15, false, right, cy)), 0, "white digits over red");
    }

    #[test]
    fn wrapping_keeps_brackets_punctuation_and_words_with_their_text() {
        let f = crate::font::bundled();
        let c = Canvas::new(1, 1);
        let px = 40.0;
        let texts = [
            "Skyward 是自由软件：你可以按照自由软件基金会发布的 GNU 通用公共许可证第 3 版，或者（由你选择）任何更新的版本，重新发布和修改它。",
            "You should have received a copy of the GNU General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.",
            "Skyward 的界面文字使用小米的 MiSans 字体，按《MiSans 字体知识产权许可协议》免费使用。",
            "boot、init_boot、vendor_boot、dtbo、vbmeta 和 recovery",
            "Copyright 2026 The Kirk Mono Project Authors (https://github.com/kirklin/kirk-mono)",
            "This phone's system doesn't make an emptied data partition anew when it starts, so it can't be wiped.",
            "This zip isn't signed. Turn off “Verify zip signatures”, then try again.",
            "這個刷機包未簽署，請關閉「驗證刷機包簽章」後重試。",
        ];
        let latin = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '\'');
        for text in texts {
            for ems in 6..30 {
                let max_w = ems as f32 * px;
                let lines = wrap(&c, &f.body, text, px, max_w);
                for l in &lines {
                    assert!(!(l.chars().count() > 1 && l.ends_with(|ch| NO_LINE_END.contains(ch))), "{l:?} ends on an opening bracket: {lines:?}");
                    assert!(!l.starts_with(|ch| NO_LINE_START.contains(ch)), "{l:?} starts with closing punctuation: {lines:?}");
                }
                let squeezed = |s: &str| s.chars().filter(|ch| *ch != ' ').collect::<String>();
                assert_eq!(squeezed(&lines.concat()), squeezed(text), "nothing lost at {ems} em");
                for word in text.split(|ch: char| !latin(ch)).filter(|w| !w.is_empty()) {
                    if c.text_width(&f.body, word, px) <= max_w {
                        assert!(lines.iter().any(|l| l.contains(word)), "{word:?} broken at {ems} em: {lines:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_path_too_long_for_a_line_breaks_after_a_slash() {
        let f = crate::font::bundled();
        let c = Canvas::new(1, 1);
        let px = 40.0;
        let text = "/data/dalvik-cache、/data/misc/apexdata/com.android.art/dalvik-cache、/data/ota 里预编译的代码";
        let path = "/data/misc/apexdata/com.android.art/dalvik-cache";
        let max_w = c.text_width(&f.body, path, px) * 0.8;
        let lines = wrap(&c, &f.body, text, px, max_w);
        assert_eq!(lines.concat(), text, "nothing lost: {lines:?}");
        let broken = lines.iter().position(|l| l.ends_with("com.android.art/")).expect("the long path breaks after a slash");
        assert!(lines[broken].starts_with("/data/misc/"), "the path starts its own line: {lines:?}");
        assert!(lines[broken + 1].starts_with("dalvik-cache"), "and goes on with what follows the slash: {lines:?}");
    }
}
