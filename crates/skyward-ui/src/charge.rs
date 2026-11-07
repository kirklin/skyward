//! The charging screen, shown as a cable goes in: the battery's level
//! large on black, what the charger does under it, and a tide of light at
//! the foot of the screen that rises as high as the battery is full and
//! breathes.

use crate::canvas::Canvas;
use crate::color::Rgba;
use crate::fmt;
use crate::font::Fonts;
use crate::glow::{self, Dither, Field};
use crate::lang::Lang;
use crate::motion;
use crate::sprite::Sprite;
use crate::theme::Metrics;
use skyward_info::{Battery, Charge};

/// The tide's colours: a deep navy where its light is faint, an electric
/// blue where it glows.
const NAVY: [f32; 3] = [0.03, 0.07, 0.32];
const ELECTRIC: [f32; 3] = [0.14, 0.30, 0.94];

/// How long, in seconds, the tide takes to rise into place, and to
/// breathe once.
const RISE: f32 = 0.9;
const BREATH: f32 = 2.6;

/// The tide's crest, as a share of the screen's height above its foot: how
/// high it stands on an empty battery, how much higher on a full one, and
/// how far it swells as it breathes. The crest is soft over this share.
const CREST_EMPTY: f32 = 0.14;
const CREST_FULL: f32 = 0.20;
const SWELL: f32 = 0.012;
const SOFT: f32 = 0.11;

/// The tide is worked out every this many pixels and filled in between.
const CELL: usize = 16;

/// The level: the weight of the screens' face its digits are set in, their
/// size and the percent sign's in dp, and where their middle sits, as a
/// share of the screen's height from its top. Under them, what the charger
/// does, at this size, this far below the digits.
const DIGITS_WEIGHT: f32 = 300.0;
const DIGITS_DP: f32 = 88.0;
const PERCENT_DP: f32 = 30.0;
const PERCENT_GAP_DP: f32 = 4.0;
const DIGITS_AT: f32 = 0.36;
const LINE_DP: f32 = 15.0;
const LINE_GAP_DP: f32 = 30.0;

const BLACK: Rgba = Rgba::rgb(0, 0, 0);
const INK: Rgba = Rgba::rgb(255, 255, 255);
const LINE_INK: Rgba = Rgba { r: 255, g: 255, b: 255, a: 160 };

/// What the charger does to the battery, in the language.
fn doing(l: Lang, b: &Battery) -> &'static str {
    match b.charge {
        Charge::Charging if b.quick => l.pick(["快速充电中", "快速充電中", "Charging quickly"]),
        other => fmt::charge(l, other),
    }
}

/// The level and what the charger does, drawn once for as long as they
/// hold.
pub struct Art {
    shows: (u8, &'static str),
    digits: Sprite,
    percent: Sprite,
    line: Sprite,
}

impl Art {
    pub fn new(f: &Fonts, m: &Metrics, l: Lang, b: &Battery) -> anyhow::Result<Art> {
        let face = f.face(DIGITS_WEIGHT)?;
        let (w, h) = (m.w as f32, m.h as f32);
        let (dpx, ppx, lpx) = (m.dp(DIGITS_DP), m.dp(PERCENT_DP), m.dp(LINE_DP));
        let (digits, line) = (b.level.to_string(), doing(l, b));
        let probe = Canvas::new(1, 1);
        let (dw, pw, gap) = (probe.text_width(&face, &digits, dpx), probe.text_width(&face, "%", ppx), m.dp(PERCENT_GAP_DP));
        let x = (w - dw - gap - pw) / 2.0;
        // The digits and the percent sign share a baseline, the digits'
        // middle on the line the screen puts them.
        let baseline = h * DIGITS_AT + face.cap(dpx) / 2.0;
        let pad = m.dp(8.0);
        let box_h = (dpx * 1.6) as u32;
        let top = baseline - dpx * 1.2;
        let digits_art = Sprite::capture(x - pad, top, (dw + 2.0 * pad) as u32, box_h, |c, ox, oy| {
            c.text(&face, &digits, x + ox, baseline + oy, dpx, INK, None);
        });
        let px = x + dw + gap;
        let percent = Sprite::capture(px - pad, top, (pw + 2.0 * pad) as u32, box_h, |c, ox, oy| {
            c.text(&face, "%", px + ox, baseline + oy, ppx, INK, None);
        });
        let lw = probe.text_width(&f.body, line, lpx);
        let (lx, cy) = ((w - lw) / 2.0, baseline + m.dp(LINE_GAP_DP) + lpx / 2.0);
        let line_art = Sprite::capture(lx - pad, cy - lpx, (lw + 2.0 * pad) as u32, (lpx * 2.0) as u32, |c, ox, oy| {
            c.text_vc(&f.body, line, lx + ox, cy + oy, lpx, INK, None);
        });
        Ok(Art { shows: (b.level, line), digits: digits_art, percent, line: line_art })
    }

    /// Whether the art shows `b` as it stands, in `l`.
    pub fn shows(&self, l: Lang, b: &Battery) -> bool {
        self.shows == (b.level, doing(l, b))
    }
}

/// Draw the charging screen over the whole of `out`, `ms` after the cable
/// went in, the battery `level` full.
pub fn draw(out: &mut Canvas, art: &Art, m: &Metrics, level: u8, ms: u64, dither: &Dither) {
    let (top, field) = tide(m, level, ms);
    out.fill_rect(0.0, 0.0, out.w as f32, top as f32, BLACK);
    glow::paint_band(out, &field, top, dither);
    art.digits.draw(out, 0.0, 0.0, INK, 1.0);
    art.percent.draw(out, 0.0, 0.0, INK, 1.0);
    art.line.draw(out, 0.0, 0.0, LINE_INK, 1.0);
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The tide `ms` after the cable went in, the battery `level` full: the
/// row its band starts on, and the band, sampled from there to the foot of
/// the screen.
fn tide(m: &Metrics, level: u8, ms: u64) -> (usize, Field) {
    let (w, h) = (m.w as usize, m.h as usize);
    let t = ms as f32 / 1000.0;
    let rise = motion::settle(ms, RISE);
    let breath = (std::f32::consts::TAU * t / BREATH).sin();
    let crest = rise * (CREST_EMPTY + CREST_FULL * level.min(100) as f32 / 100.0) + SWELL * breath * rise;
    let reach = CREST_EMPTY + CREST_FULL + SWELL + SOFT;
    let top = (h as f32 * (1.0 - reach)) as usize / CELL * CELL;
    let (cols, rows) = (w / CELL + 2, (h - top) / CELL + 2);
    let mut rgb = vec![[0.0f32; 3]; cols * rows];
    for (j, line) in rgb.chunks_exact_mut(cols).enumerate() {
        // Height above the foot of the screen, as a share of its height.
        let v = 1.0 - (top + j * CELL) as f32 / h as f32;
        for (i, px) in line.iter_mut().enumerate() {
            let u = (i * CELL) as f32 / w as f32;
            // Two swells roll along the crest, each its own way.
            let roll = 0.016 * ((u * 5.0 + t * 0.9).sin() + 0.6 * (u * 9.0 - t * 1.3).sin()) * rise;
            let edge = crest + roll;
            let fill = smooth((edge + SOFT / 2.0 - v) / SOFT);
            if fill <= 0.0 {
                continue;
            }
            // A broad glow just under the crest, electric blue, going to
            // navy toward the foot.
            let glow = (-((edge - 0.06 - v) / 0.16).powi(2)).exp();
            let noise = glow::perlin(u * 2.5 + t * 0.15, v * 4.0 - t * 0.1);
            let light = fill * (0.30 + 0.70 * glow) * (0.86 + 0.14 * breath) * (0.85 + 0.15 * noise);
            *px = std::array::from_fn(|c| (NAVY[c] + (ELECTRIC[c] - NAVY[c]) * glow) * light * 255.0);
        }
    }
    (top, Field { cell: CELL, cols, rows, rgb })
}
