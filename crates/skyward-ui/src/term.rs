//! The terminal page: what a shell has written, kept as a screen of cells
//! by a terminal emulator, and the bytes each key sends the shell.

use crate::canvas::Canvas;
use crate::color::{mix, Rgba};
use crate::font::Fonts;
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widgets::{self, Cap};
use std::num::NonZeroU16;

/// Lines kept above the screen to scroll back to.
const SCROLLBACK: usize = 2000;

/// Type size, line pitch and inner padding of the screen, in dp.
const TEXT_DP: f32 = 13.0;
const PITCH_DP: f32 = 19.0;
const PAD_DP: f32 = 14.0;

/// The row of keys a phone keyboard lacks, between the screen and the
/// keyboard: its height, and the gap above and below it.
const KEYS_H_DP: f32 = 38.0;
const KEYS_GAP_DP: f32 = 10.0;
/// How round the row's keys' corners are.
pub const KEY_RADIUS_DP: f32 = 8.0;

/// A key of the row above the keyboard, the terminal's or the editor's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermKey {
    Esc,
    Tab,
    /// Held for the next key typed.
    Ctrl,
    Left,
    Up,
    Down,
    Right,
}

/// The terminal's own keys, in the order the row shows them.
pub const TERM_KEYS: [TermKey; 7] = [TermKey::Esc, TermKey::Tab, TermKey::Ctrl, TermKey::Left, TermKey::Up, TermKey::Down, TermKey::Right];

/// A shell's screen as it has written it, and whether the shell still runs.
pub struct Term {
    parser: vt100::Parser,
    /// Once the shell has ended, its screen stays up until Enter starts
    /// another.
    pub running: bool,
    /// Ctrl is held for the next key typed.
    pub ctrl: bool,
}

impl Term {
    /// A blank screen of `rows` by `cols` for a shell that has just started.
    pub fn new(rows: u16, cols: u16) -> Term {
        let (rows, cols) = (NonZeroU16::new(rows).expect("a screen has rows"), NonZeroU16::new(cols).expect("a screen has columns"));
        Term { parser: vt100::Parser::new(rows, cols, SCROLLBACK), running: true, ctrl: false }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (NonZeroU16::new(rows).expect("a screen has rows"), NonZeroU16::new(cols).expect("a screen has columns"));
        self.parser.set_size(rows, cols);
    }

    /// Take what the shell wrote; the view returns to the live screen.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.parser.screen_mut().set_scrollback(0);
    }

    /// Scroll back through what went off the top by `rows`, or forward
    /// with a negative count; the view stays between the oldest line kept
    /// and the live screen.
    pub fn scroll(&mut self, rows: isize) {
        let screen = self.parser.screen_mut();
        let at = screen.scrollback().saturating_add_signed(rows);
        screen.set_scrollback(at);
    }

    /// What a key of the keyboard sends; `shifted` makes a letter a
    /// capital. Ctrl held turns a character into its control code.
    pub fn typed(&mut self, cap: Cap, shifted: bool) -> Vec<u8> {
        let ctrl = std::mem::take(&mut self.ctrl);
        match cap {
            Cap::Char(ch) => {
                let ch = if shifted { ch.to_ascii_uppercase() } else { ch };
                match control(ch).filter(|_| ctrl) {
                    Some(code) => vec![code],
                    None => ch.to_string().into_bytes(),
                }
            }
            Cap::Space if ctrl => vec![0],
            Cap::Space => vec![b' '],
            // What a terminal's backspace sends, which the line discipline
            // and the shell both take as erasing.
            Cap::Backspace => vec![0x7f],
            Cap::Enter => vec![b'\r'],
            Cap::Shift | Cap::Show(_) => {
                self.ctrl = ctrl;
                Vec::new()
            }
        }
    }

    /// What a key of the terminal's own row sends. The arrows send what the
    /// program reading them has asked for: cursor keys, or the keys of an
    /// application.
    pub fn key(&mut self, key: TermKey) -> Vec<u8> {
        if key == TermKey::Ctrl {
            self.ctrl = !self.ctrl;
            return Vec::new();
        }
        self.ctrl = false;
        let arrow = |final_byte: u8| {
            let lead = if self.parser.screen().application_cursor() { b'O' } else { b'[' };
            vec![0x1b, lead, final_byte]
        };
        match key {
            TermKey::Esc => vec![0x1b],
            TermKey::Tab => vec![b'\t'],
            TermKey::Up => arrow(b'A'),
            TermKey::Down => arrow(b'B'),
            TermKey::Right => arrow(b'C'),
            TermKey::Left => arrow(b'D'),
            TermKey::Ctrl => unreachable!("handled above"),
        }
    }
}

/// The control code Ctrl makes of a character, where it makes one.
fn control(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' | 'A'..='Z' => Some(ch.to_ascii_uppercase() as u8 - b'@'),
        '@' | '[' | '\\' | ']' | '^' | '_' => Some(ch as u8 - b'@'),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// Where the screen and the terminal's own keys sit on the page, and how
/// many rows and columns the screen holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The card the screen is drawn on.
    pub card: Rect,
    /// The top left of the first cell.
    pub x0: f32,
    pub y0: f32,
    /// The type size, a cell's width, and a row's height.
    pub px: f32,
    pub cell: f32,
    pub pitch: f32,
    pub rows: u16,
    pub cols: u16,
    /// The top of the terminal's own keys.
    pub keys_top: f32,
}

/// Lay out the terminal page's screen between `top` and the keyboard.
pub fn layout(t: &Theme, f: &Fonts, top: f32) -> Layout {
    let m = &t.m;
    let px = m.dp(TEXT_DP);
    let keys_top = m.h as f32 - widgets::keyboard_height(t) - m.dp(KEYS_GAP_DP + KEYS_H_DP);
    let card = Rect::new(m.margin, top, m.w as f32 - m.margin * 2.0, keys_top - m.dp(KEYS_GAP_DP) - top);
    let pad = m.dp(PAD_DP);
    let cell = cell_width(f, px);
    let pitch = m.dp(PITCH_DP);
    let cols = ((card.w - pad * 2.0) / cell).floor().max(1.0) as u16;
    let rows = ((card.h - pad * 2.0) / pitch).floor().max(1.0) as u16;
    Layout { card, x0: card.x + pad, y0: card.y + pad, px, cell, pitch, rows, cols, keys_top }
}

/// The advance of the monospaced face, one cell.
fn cell_width(f: &Fonts, px: f32) -> f32 {
    // Measured on a probe canvas: the width does not depend on the canvas.
    Canvas::new(1, 1).text_width(&f.mono, "0", px)
}

/// The sixteen colours a program names by number, for the light and the
/// dark card: the bright half brighter, and none lost against the card.
const LIGHT: [Rgba; 16] = [
    Rgba::rgb(29, 29, 31),
    Rgba::rgb(201, 40, 33),
    Rgba::rgb(31, 136, 61),
    Rgba::rgb(158, 116, 0),
    Rgba::rgb(31, 92, 214),
    Rgba::rgb(165, 50, 170),
    Rgba::rgb(0, 128, 145),
    Rgba::rgb(118, 118, 124),
    Rgba::rgb(96, 96, 102),
    Rgba::rgb(224, 66, 58),
    Rgba::rgb(52, 163, 82),
    Rgba::rgb(184, 140, 0),
    Rgba::rgb(74, 123, 247),
    Rgba::rgb(196, 86, 199),
    Rgba::rgb(23, 156, 173),
    Rgba::rgb(60, 60, 66),
];
const DARK: [Rgba; 16] = [
    Rgba::rgb(80, 80, 86),
    Rgba::rgb(242, 90, 82),
    Rgba::rgb(98, 197, 84),
    Rgba::rgb(229, 195, 74),
    Rgba::rgb(91, 140, 250),
    Rgba::rgb(198, 111, 224),
    Rgba::rgb(63, 199, 214),
    Rgba::rgb(208, 208, 212),
    Rgba::rgb(128, 128, 134),
    Rgba::rgb(255, 120, 112),
    Rgba::rgb(130, 222, 116),
    Rgba::rgb(245, 215, 110),
    Rgba::rgb(126, 165, 255),
    Rgba::rgb(218, 140, 240),
    Rgba::rgb(103, 218, 230),
    Rgba::rgb(255, 255, 255),
];

/// The colour a cell names, `fallback` for the default; bold makes the
/// first eight colours their bright halves, as terminals do.
fn color(t: &Theme, named: vt100::Color, bold: bool, fallback: Rgba) -> Rgba {
    let table = if t.dark { &DARK } else { &LIGHT };
    match named {
        vt100::Color::Default => fallback,
        vt100::Color::Idx(i @ 0..=7) if bold => table[i as usize + 8],
        vt100::Color::Idx(i @ 0..=15) => table[i as usize],
        // The 6×6×6 cube, then 24 greys.
        vt100::Color::Idx(i @ 16..=231) => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let i = i - 16;
            Rgba::rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + (i - 232) * 10;
            Rgba::rgb(v, v, v)
        }
        vt100::Color::Rgb(r, g, b) => Rgba::rgb(r, g, b),
    }
}

/// Draw the shell's screen on its card, with the cursor where the shell
/// left it while the live screen is in view.
pub fn draw(c: &mut Canvas, t: &Theme, f: &Fonts, l: &Layout, term: Option<&Term>) {
    let m = &t.m;
    c.fill_round_rect(l.card.x, l.card.y, l.card.w, l.card.h, m.card_radius, t.pal.card);
    let Some(term) = term else { return };
    let screen = term.screen();
    let px = l.px;
    // Every row sits on the baseline the UI face would give it.
    let ascent = -f.body.ink(px).0;
    let (cursor_row, cursor_col) = screen.cursor_position();
    let cursor = term.running && !screen.hide_cursor() && screen.scrollback() == 0;
    for row in 0..l.rows {
        let y = l.y0 + row as f32 * l.pitch;
        for col in 0..l.cols {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let span = if cell.is_wide() { 2.0 } else { 1.0 };
            let x = l.x0 + col as f32 * l.cell;
            let mut fg = color(t, cell.fgcolor(), cell.bold(), t.pal.text);
            let mut bg = color(t, cell.bgcolor(), false, t.pal.card);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            let on_cursor = cursor && (row, col) == (cursor_row, cursor_col);
            if on_cursor {
                (fg, bg) = (t.pal.card, t.pal.accent);
            }
            if cell.dim() {
                fg = mix(fg, bg, 0.45);
            }
            if bg != t.pal.card {
                c.fill_rect(x, y - m.dp(2.0), l.cell * span, l.pitch, bg);
            }
            if let Some(ch) = cell.contents().chars().next() {
                let face = if f.mono.has(ch) { &f.mono } else { &f.body };
                let glyph = cell.contents();
                let gw = c.text_width(face, glyph, px);
                c.text(face, glyph, x + (l.cell * span - gw) / 2.0, y + ascent, px, fg, None);
            }
            if cell.underline() {
                c.fill_rect(x, y + l.pitch - m.dp(4.0), l.cell * span, m.dp(1.0), fg);
            }
        }
    }
}

/// The row of `keys` above the keyboard, the terminal's or the editor's;
/// Ctrl shows held while it is.
pub fn keys(c: &mut Canvas, t: &Theme, f: &Fonts, l: &Layout, keys: &[TermKey], ctrl: bool) -> Vec<(Rect, TermKey)> {
    let m = &t.m;
    let n = keys.len() as f32;
    let gap = m.dp(6.0);
    let w = (m.w as f32 - m.margin * 2.0 - gap * (n - 1.0)) / n;
    let h = m.dp(KEYS_H_DP);
    let px = m.dp(14.0);
    let mut out = Vec::new();
    for (k, &key) in keys.iter().enumerate() {
        let r = Rect::new(m.margin + k as f32 * (w + gap), l.keys_top, w, h);
        let held = key == TermKey::Ctrl && ctrl;
        let (fill, ink) = if held { (t.pal.accent, t.pal.card) } else { (t.pal.card, t.pal.text) };
        c.fill_round_rect(r.x, r.y, r.w, r.h, m.dp(KEY_RADIUS_DP), fill);
        let label = match key {
            TermKey::Esc => "Esc",
            TermKey::Tab => "Tab",
            TermKey::Ctrl => "Ctrl",
            TermKey::Left => "←",
            TermKey::Up => "↑",
            TermKey::Down => "↓",
            TermKey::Right => "→",
        };
        let face = &f.value;
        let tw = c.text_width(face, label, px);
        let (top, bottom) = face.ink(px);
        c.text(face, label, r.x + (r.w - tw) / 2.0, r.y + (r.h - (bottom - top)) / 2.0 - top, px, ink, None);
        out.push((r, key));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::Layer;

    #[test]
    fn keys_send_what_a_terminal_sends() {
        let mut term = Term::new(24, 80);
        assert_eq!(term.typed(Cap::Char('l'), false), b"l");
        assert_eq!(term.typed(Cap::Char('l'), true), b"L");
        assert_eq!(term.typed(Cap::Char('/'), false), b"/");
        assert_eq!(term.typed(Cap::Space, false), b" ");
        assert_eq!(term.typed(Cap::Backspace, false), [0x7f]);
        assert_eq!(term.typed(Cap::Enter, false), b"\r");
        assert!(term.typed(Cap::Show(Layer::Digits), false).is_empty());
        assert_eq!(term.key(TermKey::Esc), [0x1b]);
        assert_eq!(term.key(TermKey::Tab), b"\t");
    }

    #[test]
    fn ctrl_held_makes_the_next_character_a_control_code() {
        let mut term = Term::new(24, 80);
        assert!(term.key(TermKey::Ctrl).is_empty());
        assert!(term.ctrl);
        assert!(term.typed(Cap::Show(Layer::Digits), false).is_empty(), "changing the layer keeps it held");
        assert_eq!(term.typed(Cap::Char('c'), false), [0x03]);
        assert!(!term.ctrl, "it lasts one key");
        assert_eq!(term.typed(Cap::Char('c'), false), b"c");
        term.key(TermKey::Ctrl);
        assert_eq!(term.typed(Cap::Char('['), false), [0x1b]);
        term.key(TermKey::Ctrl);
        assert_eq!(term.typed(Cap::Char('1'), false), b"1", "a digit has no control code");
        term.key(TermKey::Ctrl);
        term.key(TermKey::Ctrl);
        assert!(!term.ctrl, "a second press lets it go");
    }

    #[test]
    fn arrows_follow_the_mode_the_program_asked_for() {
        let mut term = Term::new(24, 80);
        assert_eq!(term.key(TermKey::Up), b"\x1b[A");
        assert_eq!(term.key(TermKey::Left), b"\x1b[D");
        // What a full-screen program sends to have the keys of an
        // application.
        term.feed(b"\x1b[?1h");
        assert_eq!(term.key(TermKey::Down), b"\x1bOB");
        assert_eq!(term.key(TermKey::Right), b"\x1bOC");
    }

    #[test]
    fn what_the_shell_writes_shows_and_scrolls_back() {
        let mut term = Term::new(3, 20);
        term.feed(b"ishtar:/ # ls\r\nbin\r\netc\r\nvendor\r\nishtar:/ # ");
        assert_eq!(term.screen().contents(), "etc\nvendor\nishtar:/ # ");
        assert_eq!(term.screen().cursor_position(), (2, 11));
        term.scroll(2);
        assert_eq!(term.screen().contents(), "ishtar:/ # ls\nbin\netc");
        term.scroll(10);
        assert_eq!(term.screen().scrollback(), 2, "no further back than the oldest line kept");
        term.scroll(-1);
        assert_eq!(term.screen().scrollback(), 1);
        term.feed(b"x");
        assert_eq!(term.screen().scrollback(), 0, "new output brings the live screen back");
    }

    #[test]
    fn named_colours_come_from_the_page_and_the_cube() {
        let t = Theme::light(1440, 3200, 3.5);
        assert_eq!(color(&t, vt100::Color::Default, false, t.pal.text), t.pal.text);
        assert_eq!(color(&t, vt100::Color::Idx(1), false, t.pal.text), LIGHT[1]);
        assert_eq!(color(&t, vt100::Color::Idx(1), true, t.pal.text), LIGHT[9], "bold red is bright red");
        assert_eq!(color(&t, vt100::Color::Idx(196), false, t.pal.text), Rgba::rgb(255, 0, 0));
        assert_eq!(color(&t, vt100::Color::Idx(232), false, t.pal.text), Rgba::rgb(8, 8, 8));
        assert_eq!(color(&t, vt100::Color::Rgb(1, 2, 3), false, t.pal.text), Rgba::rgb(1, 2, 3));
    }
}
