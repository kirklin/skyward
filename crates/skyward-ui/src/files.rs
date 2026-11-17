use crate::app::{Action, FileKind, FileRow, Hit, Pinned, Rows};
use crate::canvas::Canvas;
use crate::color::{mix, Rgba};
use crate::fmt;
use crate::font::{Face, Fonts};
use crate::geom::Rect;
use crate::lang::Lang;
use crate::theme::Theme;
use crate::widgets;
use std::f32::consts::FRAC_PI_2;
use std::path::{Component, Path};

// The file manager's list as the stock file manager draws a folder: rows
// laid straight on the page, under a path of capsules. Sizes are dp,
// measured on a Xiaomi 17 Ultra.

/// Where the path strip starts on an unscrolled page; scrolled up to the
/// bar, it stays right under it.
pub const STRIP_TOP_DP: f32 = 164.0;
const STRIP_DP: f32 = 51.0;
const PILL_TOP_DP: f32 = 13.0;
const PILL_DP: f32 = 33.0;
const PILL_PAD_DP: f32 = 14.0;
const PILL_TEXT_DP: f32 = 10.9;
/// Space either side of the arrow between two capsules.
const ARROW_GAP_DP: f32 = 6.0;
const ARROW_DP: f32 = 14.0;
/// Where the first row starts on an unscrolled page.
pub const ROWS_TOP_DP: f32 = 223.7;
const ROW_DP: f32 = 72.0;
/// What a second line of name adds to a row.
const NAME_LINE_DP: f32 = 22.33;
const SIDE_DP: f32 = 28.0;
const ICON_DP: f32 = 40.0;
const TEXT_X_DP: f32 = 84.0;
/// Room the text leaves at the right, for the chevron's column.
const TEXT_RIGHT_DP: f32 = 65.7;
const NAME_DP: f32 = 16.9;
/// The top of a name's ink in its row.
const NAME_TOP_DP: f32 = 19.0;
const LINE_DP: f32 = 11.7;
/// The baseline of the line under a one-line name.
const LINE_BASE_DP: f32 = 53.33;
/// Where the message in place of an empty or unreadable list sits, below
/// where the rows would start.
const NOTE_DP: f32 = 140.0;
const NOTE_TEXT_DP: f32 = 17.0;

/// The list's colours: the page, the text, and the path's capsules, the
/// folder shown in amber.
struct Palette {
    page: Rgba,
    name: Rgba,
    line: Rgba,
    note: Rgba,
    chevron: Rgba,
    pill: Rgba,
    pill_text: Rgba,
    here: Rgba,
    here_text: Rgba,
    arrow: Rgba,
}

fn palette(t: &Theme) -> Palette {
    if t.wallpaper.is_some() {
        return Palette {
            page: t.pal.bg,
            name: t.outside_ink(),
            line: t.outside_note(),
            note: t.outside_note(),
            chevron: t.outside_note(),
            pill: Rgba::rgb(43, 51, 59),
            pill_text: t.outside_ink(),
            here: Rgba::rgb(69, 52, 27),
            here_text: Rgba::rgb(255, 211, 135),
            arrow: t.outside_note(),
        };
    }
    if t.dark {
        Palette {
            page: Rgba::rgb(0, 0, 0),
            name: Rgba::rgb(242, 242, 242),
            line: Rgba::rgb(128, 128, 128),
            note: Rgba::rgb(128, 128, 128),
            chevron: Rgba::rgb(128, 128, 128),
            pill: Rgba::rgb(38, 38, 38),
            pill_text: Rgba::rgb(136, 140, 154),
            here: Rgba::rgb(51, 37, 0),
            here_text: Rgba::rgb(204, 150, 0),
            arrow: Rgba::rgb(51, 51, 51),
        }
    } else {
        Palette {
            page: Rgba::rgb(255, 255, 255),
            name: Rgba::rgb(0, 0, 0),
            line: Rgba::rgb(153, 153, 153),
            note: Rgba::rgb(102, 102, 102),
            chevron: Rgba::rgb(153, 153, 153),
            pill: Rgba::rgb(243, 244, 247),
            pill_text: Rgba::rgb(136, 140, 154),
            here: Rgba::rgb(255, 245, 217),
            here_text: Rgba::rgb(255, 178, 29),
            arrow: Rgba::rgb(217, 217, 217),
        }
    }
}

/// The colour of the page under the list.
pub fn page(t: &Theme) -> Rgba {
    palette(t).page
}

/// What a file holds, as its icon shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Archive,
    /// A partition's image, as a recovery installs it.
    DiskImage,
    Picture,
    Video,
    Audio,
    /// An Android app to install.
    Package,
    Text,
    /// Scripts, markup and settings.
    Code,
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Other,
}

/// The extensions of each type, in lower case: those the stock file
/// manager gives an icon of their own, grouped as it colours them, and the
/// compressed files and images a recovery handles besides.
const TYPES: [(FileType, &[&str]); 12] = [
    (FileType::Archive, &["zip", "rar", "7z", "tar", "gz", "tgz", "xz", "bz2", "lz4", "zst"]),
    (FileType::DiskImage, &["img", "iso"]),
    (FileType::Picture, &["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "dng", "svg", "tif", "tiff", "ico", "avif", "arw", "cr2", "nef", "psd"]),
    (FileType::Video, &["mp4", "mkv", "mov", "avi", "3gp", "3gpp", "webm", "m4v", "ts", "flv", "wmv", "mpeg", "mpg", "rm", "rmvb", "vob", "mts", "m2ts", "f4v"]),
    (FileType::Audio, &["mp3", "flac", "aac", "m4a", "wav", "ogg", "oga", "opus", "amr", "ape", "wma", "mid", "midi", "mka", "aif", "aiff", "ac3", "dsf", "dff", "awb"]),
    (FileType::Package, &["apk", "apks", "xapk"]),
    (FileType::Text, &["txt", "log", "md", "ini", "lrc"]),
    (FileType::Code, &["xml", "html", "htm", "json", "sh", "py", "js", "css", "cfg", "conf", "prop", "rc", "yaml", "yml", "toml"]),
    (FileType::Document, &["doc", "docx", "wps", "wpt", "odt", "rtf", "pages"]),
    (FileType::Spreadsheet, &["xls", "xlsx", "et", "ett", "ods", "csv", "numbers"]),
    (FileType::Presentation, &["ppt", "pptx", "pps", "dps", "dpt", "odp"]),
    (FileType::Pdf, &["pdf"]),
];

/// What the file called `name` holds, by its extension in any case.
pub fn file_type(name: &str) -> FileType {
    let Some(ext) = Path::new(name).extension() else { return FileType::Other };
    let ext = ext.to_string_lossy().to_ascii_lowercase();
    TYPES.iter().find(|(_, exts)| exts.contains(&ext.as_str())).map_or(FileType::Other, |&(kind, _)| kind)
}

/// The colour of a type's sheet, as the stock list shows it on screen.
fn sheet(kind: FileType) -> Rgba {
    match kind {
        FileType::Archive | FileType::DiskImage | FileType::Code | FileType::Document => Rgba::rgb(74, 128, 247),
        FileType::Picture | FileType::Video => Rgba::rgb(116, 104, 240),
        FileType::Audio | FileType::Pdf => Rgba::rgb(230, 75, 59),
        FileType::Package | FileType::Spreadsheet => Rgba::rgb(97, 202, 81),
        FileType::Presentation => Rgba::rgb(237, 114, 46),
        FileType::Text | FileType::Other => Rgba::rgb(143, 156, 174),
    }
}

fn name_width(t: &Theme) -> f32 {
    t.m.w as f32 - t.m.dp(TEXT_X_DP) - t.m.dp(TEXT_RIGHT_DP)
}

/// A name as the stock file manager sets it in `w` pixels: on one line if
/// it fits, on two if they hold it; longer, the first line full and the
/// second keeping the extension, with the middle taken out.
pub fn name_lines(face: &Face, px: f32, name: &str, w: f32) -> Vec<String> {
    let c = Canvas::new(0, 0);
    let width = |s: &str| c.text_width(face, s, px);
    if width(name) <= w {
        return vec![name.to_string()];
    }
    let lines = widgets::wrap(&c, face, name, px, w);
    if lines.len() <= 2 {
        return lines;
    }
    let cut = name.char_indices().map(|(at, ch)| at + ch.len_utf8()).take_while(|&end| width(&name[..end]) <= w).last().unwrap_or(0);
    let (first, rest) = name.split_at(cut);
    let second = match rest.rfind('.') {
        Some(dot) if width(&rest[dot..]) < w => {
            let ext = &rest[dot..];
            format!("{}{ext}", middle(&c, face, px, &rest[..dot], w - width(ext)))
        }
        _ => middle(&c, face, px, rest, w),
    };
    vec![first.to_string(), second]
}

/// `text` cut down to `w` pixels by taking characters out of its middle:
/// the end keeps up to half the room, the start the rest.
fn middle(c: &Canvas, face: &Face, px: f32, text: &str, w: f32) -> String {
    let width = |s: &[char]| c.text_width(face, &s.iter().collect::<String>(), px);
    let chars: Vec<char> = text.chars().collect();
    if width(&chars) <= w {
        return text.to_string();
    }
    let room = w - c.text_width(face, "…", px);
    let mut right = chars.len();
    while right > 0 && width(&chars[right - 1..]) <= room / 2.0 {
        right -= 1;
    }
    let left_room = room - width(&chars[right..]);
    let mut left = 0;
    while left < right && width(&chars[..=left]) <= left_room {
        left += 1;
    }
    format!("{}…{}", chars[..left].iter().collect::<String>(), chars[right..].iter().collect::<String>())
}

/// The list's rows for `entries`, the first starting where an unscrolled
/// page puts it, each touched for `action`; a row grows by a line for a
/// name on two.
pub fn rows(t: &Theme, f: &Fonts, entries: &[FileRow], action: fn(usize) -> Action) -> Rows {
    let m = &t.m;
    let w = name_width(t);
    let mut y = m.dp(ROWS_TOP_DP).round();
    let mut edges = Vec::with_capacity(entries.len() + 1);
    edges.push(y);
    for e in entries {
        let lines = name_lines(&f.heading, m.dp(NAME_DP), &e.name, w).len();
        y += (m.dp(ROW_DP) + m.dp(NAME_LINE_DP) * (lines - 1) as f32).round();
        edges.push(y);
    }
    Rows { edges, action: Some(action) }
}

/// Entry `e`'s row over the whole of `c`, sized for it: its icon, its name
/// on up to two lines, the line under it in `l` with dates told against
/// `now` in the zone at `zone` in [`crate::app::TIME_ZONES`], and a chevron
/// for what opens as a folder.
#[allow(clippy::too_many_arguments)]
pub fn row(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, e: &FileRow, now: i64, zone: usize) {
    let m = &t.m;
    let p = palette(t);
    c.clear(if t.wallpaper.is_some() { p.page.with_alpha(0) } else { p.page });
    let cy = c.h as f32 / 2.0;
    let size = m.dp(ICON_DP);
    let folder = matches!(e.kind, FileKind::Folder { .. } | FileKind::Link { folder: true, .. });
    let (ix, iy) = (m.dp(SIDE_DP), cy - size / 2.0);
    if folder {
        folder_icon(c, ix, iy, size);
    } else {
        let kind = match e.kind {
            FileKind::Special => FileType::Other,
            _ => file_type(&e.name),
        };
        file_icon(c, f, ix, iy, size, p.page, kind);
    }
    let (x, w) = (m.dp(TEXT_X_DP), name_width(t));
    let lines = name_lines(&f.heading, m.dp(NAME_DP), &e.name, w);
    for (k, line) in lines.iter().enumerate() {
        c.text_top(&f.heading, line, x, m.dp(NAME_TOP_DP + NAME_LINE_DP * k as f32), m.dp(NAME_DP), p.name, None);
    }
    let base = m.dp(LINE_BASE_DP + NAME_LINE_DP * (lines.len() - 1) as f32);
    c.text(&f.value, &fmt::file_line(l, e, now, zone), x, base, m.dp(LINE_DP), p.line, Some(w));
    if folder {
        chevron(c, t, cy, p.chevron);
    }
}

/// A folder 40 units square scaled to `size`: an amber back with its tab,
/// a sheet of paper, and a yellow front shading darker towards its foot,
/// in the colours the stock list shows its folders in.
fn folder_icon(c: &mut Canvas, x: f32, y: f32, size: f32) {
    let u = size / 40.0;
    let at = |v: f32, w: f32| (x + v * u, y + w * u);
    let (tab, under) = (Rgba::rgb(243, 175, 64), Rgba::rgb(238, 164, 59));
    let (b0, b1) = (y + 6.34 * u, y + 12.92 * u);
    let back = |py: f32| mix(tab, under, (py - b0) / (b1 - b0));
    c.fill_round_rect_graded(x + 3.0 * u, b0, 12.6 * u, 6.0 * u, 1.9 * u, &back);
    c.fill_convex(&[at(13.0, 6.34), at(14.6, 6.34), at(17.9, 8.8), at(13.0, 8.8)], tab);
    c.fill_round_rect_graded(x + 3.0 * u, y + 8.8 * u, 34.0 * u, 24.86 * u, 4.7 * u, &back);
    c.fill_round_rect(x + 5.4 * u, y + 11.1 * u, 29.4 * u, 20.6 * u, 1.2 * u, Rgba::rgb(255, 255, 255));
    let (top, foot) = (Rgba::rgb(248, 215, 83), Rgba::rgb(241, 176, 67));
    let (y0, y1) = (y + 12.92 * u, y + 33.66 * u);
    let front = |py: f32| mix(top, foot, (py - y0) / (y1 - y0));
    c.fill_round_rect_graded(x + 3.0 * u, y0, 34.0 * u, y1 - y0, 4.7 * u, &front);
    c.fill_round_rect_graded(x + 3.0 * u, y0, 34.0 * u, 10.0 * u, 1.45 * u, &front);
}

/// A file 40 units square scaled to `size`: a sheet with its top right
/// corner folded down, in the colour of what the file holds, and a sign of
/// that in white; `page` is the colour around it.
fn file_icon(c: &mut Canvas, f: &Fonts, x: f32, y: f32, size: f32, page: Rgba, kind: FileType) {
    let u = size / 40.0;
    let at = |v: f32, w: f32| (x + v * u, y + w * u);
    let color = sheet(kind);
    c.fill_round_rect(x + 4.0 * u, y, 32.0 * u, 40.0 * u, 8.0 * u, color);
    // The corner cut away along the fold, and the flap folded down over the
    // sheet, its inner corner rounded.
    c.fill_convex(&[at(25.5, -0.5), at(36.5, -0.5), at(36.5, 10.5)], page);
    let mut flap = vec![at(25.0, 0.0), at(26.0, 0.0), at(36.0, 10.0), at(36.0, 11.0)];
    flap.extend((0..=8).map(|k| {
        let a = FRAC_PI_2 * (1.0 + k as f32 / 8.0);
        at(31.4 + 6.4 * a.cos(), 4.6 + 6.4 * a.sin())
    }));
    c.fill_convex(&flap, mix(color, WHITE, 0.3));
    sign(c, f, kind, x, y, u, color);
}

const WHITE: Rgba = Rgba::rgb(255, 255, 255);

/// The white sign of what a file holds, on its sheet at (`x`, `y`) in
/// units of `u` pixels; `sheet` is the sheet's colour, for what is cut out
/// of the sign.
fn sign(c: &mut Canvas, f: &Fonts, kind: FileType, x: f32, y: f32, u: f32, sheet: Rgba) {
    let at = |v: f32, w: f32| (x + v * u, y + w * u);
    let line = |c: &mut Canvas, a: (f32, f32), b: (f32, f32), t: f32, color: Rgba| {
        let (p, q) = (at(a.0, a.1), at(b.0, b.1));
        c.line(p.0, p.1, q.0, q.1, t * u, color);
    };
    // A sector of a circle, from straight up to straight right.
    let quarter = |cx: f32, cy: f32, r: f32| -> Vec<(f32, f32)> {
        let arc = (0..=8).map(|k| {
            let a = FRAC_PI_2 * (k as f32 / 8.0 - 1.0);
            at(cx + r * a.cos(), cy + r * a.sin())
        });
        std::iter::once(at(cx, cy)).chain(arc).collect()
    };
    match kind {
        // The zip's teeth down from the top edge, and its pull.
        FileType::Archive => {
            for k in 0..7 {
                let left = if k % 2 == 0 { 20.0 } else { 18.3 };
                c.fill_rect(x + left * u, y + (1.6 + 2.5 * k as f32) * u, 1.7 * u, 1.5 * u, WHITE);
            }
            c.fill_round_rect(x + 17.3 * u, y + 19.6 * u, 5.4 * u, 10.2 * u, 1.1 * u, WHITE);
            c.fill_round_rect(x + 18.7 * u, y + 23.4 * u, 2.6 * u, 4.6 * u, 0.6 * u, sheet);
        }
        // A drive, with its slot and its light.
        FileType::DiskImage => {
            c.fill_round_rect(x + 12.2 * u, y + 17.5 * u, 15.6 * u, 11.0 * u, 2.2 * u, WHITE);
            line(c, (14.9, 24.8), (20.3, 24.8), 1.2, sheet);
            let (lx, ly) = at(24.8, 24.8);
            c.fill_circle(lx, ly, 1.0 * u, sheet);
        }
        // A framed view of hills under the sun.
        FileType::Picture => {
            c.stroke_round_rect(x + 11.6 * u, y + 15.6 * u, 16.8 * u, 14.4 * u, 2.6 * u, 1.6 * u, WHITE);
            let (sx, sy) = at(23.7, 19.8);
            c.fill_circle(sx, sy, 1.6 * u, WHITE);
            c.fill_convex(&[at(13.4, 28.6), at(18.2, 21.8), at(23.0, 28.6)], WHITE);
            c.fill_convex(&[at(19.8, 28.6), at(23.6, 24.2), at(27.0, 28.6)], WHITE);
        }
        // Play, its corners rounded by the strokes around it, a little
        // right of the middle as its weight lies left.
        FileType::Video => {
            let corners = [(15.7, 17.4), (24.7, 23.0), (15.7, 28.6)];
            c.fill_convex(&corners.map(|(v, w)| at(v, w)), WHITE);
            for k in 0..3 {
                line(c, corners[k], corners[(k + 1) % 3], 2.2, WHITE);
            }
        }
        // A quaver.
        FileType::Audio => {
            let (hx, hy) = at(17.25, 27.6);
            c.fill_circle(hx, hy, 2.9 * u, WHITE);
            line(c, (19.45, 27.6), (19.45, 16.7), 1.8, WHITE);
            line(c, (19.45, 16.7), (24.45, 18.5), 2.4, WHITE);
            line(c, (24.45, 18.5), (24.45, 20.9), 1.8, WHITE);
        }
        // A box seen from above, its faces parted.
        FileType::Package => {
            let corner = |deg: f32| {
                let a = deg.to_radians();
                (20.0 + 7.2 * a.cos(), 23.2 + 7.2 * a.sin())
            };
            c.fill_convex(&[30.0, 90.0, 150.0, 210.0, 270.0, 330.0].map(|deg| {
                let (v, w) = corner(deg);
                at(v, w)
            }), WHITE);
            for deg in [90.0, 210.0, 330.0] {
                line(c, (20.0, 23.2), corner(deg), 1.3, sheet);
            }
        }
        // Lines of text, the last one short.
        FileType::Text => {
            for (k, end) in [27.0, 27.0, 27.0, 21.6].into_iter().enumerate() {
                let row = 17.4 + 3.9 * k as f32;
                line(c, (13.0, row), (end, row), 1.8, WHITE);
            }
        }
        // Angle brackets around a slash.
        FileType::Code => {
            for (a, b) in [((17.0, 18.8), (13.6, 23.0)), ((13.6, 23.0), (17.0, 27.2)), ((23.0, 18.8), (26.4, 23.0)), ((26.4, 23.0), (23.0, 27.2))] {
                line(c, a, b, 1.8, WHITE);
            }
            line(c, (21.3, 17.8), (18.7, 28.2), 1.8, WHITE);
        }
        // A W, for a word processor's document.
        FileType::Document => {
            let w = [(13.4, 17.8), (16.2, 28.2), (20.0, 20.6), (23.8, 28.2), (26.6, 17.8)];
            for pair in w.windows(2) {
                line(c, pair[0], pair[1], 2.0, WHITE);
            }
        }
        // A table with a heading row and column.
        FileType::Spreadsheet => {
            c.stroke_round_rect(x + 12.6 * u, y + 16.6 * u, 14.8 * u, 12.8 * u, 1.8 * u, 1.6 * u, WHITE);
            line(c, (12.6, 21.2), (27.4, 21.2), 1.6, WHITE);
            line(c, (18.2, 16.6), (18.2, 29.4), 1.6, WHITE);
        }
        // A pie with a slice drawn out of it.
        FileType::Presentation => {
            let (cx, cy) = at(19.2, 23.8);
            c.fill_circle(cx, cy, 6.4 * u, WHITE);
            c.fill_convex(&quarter(18.8, 24.2, 7.6), sheet);
            c.fill_convex(&quarter(20.4, 22.6, 6.4), WHITE);
        }
        FileType::Pdf => {
            let (cx, cy) = at(19.85, 22.9);
            c.text_center(&f.strong, "PDF", cx, cy, 8.6 * u, WHITE);
        }
        FileType::Other => {
            let (cx, cy) = at(20.0, 23.0);
            c.text_center(&f.strong, "?", cx, cy, 18.0 * u, WHITE);
        }
    }
}

/// The list's chevron, 10 by 16 dp at the right margin.
fn chevron(c: &mut Canvas, t: &Theme, cy: f32, color: Rgba) {
    let m = &t.m;
    let (x0, y0) = (m.w as f32 - m.dp(SIDE_DP) - m.dp(10.0), cy - m.dp(8.0));
    let at = |v: f32, w: f32| (x0 + m.dp(v), y0 + m.dp(w));
    let (a, b, d) = (at(2.23, 1.94), at(8.28, 8.0), at(2.23, 14.06));
    let sw = m.dp(1.43);
    c.line(a.0, a.1, b.0, b.1, sw, color);
    c.line(b.0, b.1, d.0, d.1, sw, color);
}

/// The small arrow between two capsules, its 14 dp box starting at `x`.
fn arrow(c: &mut Canvas, t: &Theme, x: f32, cy: f32, color: Rgba) {
    let m = &t.m;
    let at = |v: f32, w: f32| (x + m.dp(v), cy + m.dp(w - 7.0));
    let (a, b, d) = (at(4.4, 2.11), at(9.2, 7.0), at(4.4, 11.89));
    let sw = m.dp(1.4);
    c.line(a.0, a.1, b.0, b.1, sw, color);
    c.line(b.0, b.1, d.0, d.1, sw, color);
}

/// The path over the list as capsules, the top of the list first, named
/// `top`, and the folder shown last in amber; each earlier one goes up to
/// its folder. A path longer than the strip shows its end, as the stock
/// strip does scrolled to it.
pub fn strip(t: &Theme, f: &Fonts, top: &str, folder: &str) -> Pinned {
    let m = &t.m;
    let p = palette(t);
    let mut c = Canvas::new(m.w, m.dp(STRIP_DP).round() as u32);
    c.clear(if t.wallpaper.is_some() { p.page.with_alpha(218) } else { p.page });
    let below = Path::new(folder).components().filter_map(|part| match part {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    });
    let names: Vec<String> = std::iter::once(top.to_string()).chain(below).collect();
    let px = m.dp(PILL_TEXT_DP);
    let widths: Vec<f32> = names.iter().map(|n| c.text_width(&f.value, n, px) + m.dp(PILL_PAD_DP) * 2.0).collect();
    let step = m.dp(ARROW_GAP_DP) * 2.0 + m.dp(ARROW_DP);
    let total = widths.iter().sum::<f32>() + step * (names.len() - 1) as f32;
    let mut x = m.dp(SIDE_DP).min(m.w as f32 - m.dp(SIDE_DP) - total);
    let (top, h) = (m.dp(PILL_TOP_DP), m.dp(PILL_DP));
    let mut hits = Vec::new();
    for (k, (name, &w)) in names.iter().zip(&widths).enumerate() {
        let here = k + 1 == names.len();
        let (fill, ink) = if here { (p.here, p.here_text) } else { (p.pill, p.pill_text) };
        c.fill_round_rect(x, top, w, h, h / 2.0, fill);
        c.text_center(&f.value, name, x + w / 2.0, top + h / 2.0, px, ink);
        if !here {
            hits.push(Hit { rect: Rect::new(x, top, w, h), action: Action::OpenAncestor(k) });
            arrow(&mut c, t, x + w + m.dp(ARROW_GAP_DP), top + h / 2.0, p.arrow);
        }
        x += w + step;
    }
    Pinned { canvas: c, top: m.dp(STRIP_TOP_DP), hits }
}

/// What the page says in place of the list, centred below the path, with
/// the reason under it when there is one.
pub fn note(c: &mut Canvas, t: &Theme, f: &Fonts, title: &str, why: Option<&str>) {
    let m = &t.m;
    let p = palette(t);
    let cx = m.w as f32 / 2.0;
    let cy = m.dp(ROWS_TOP_DP + NOTE_DP);
    c.text_center(&f.body, title, cx, cy, m.dp(NOTE_TEXT_DP), p.note);
    let Some(why) = why else { return };
    let lines = widgets::wrap(c, &f.body, why, m.sub_px, m.w as f32 - m.dp(SIDE_DP) * 2.0);
    for (k, line) in lines.iter().enumerate() {
        c.text_center(&f.body, line, cx, cy + m.dp(32.0) + m.sub_pitch * k as f32, m.sub_px, p.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_name_keeps_its_start_and_its_extension() {
        let f = crate::font::bundled();
        let (px, w) = (50.0, 750.0);
        let short = name_lines(&f.heading, px, "init.rc", w);
        assert_eq!(short, ["init.rc"]);
        let two = name_lines(&f.heading, px, "1d9bf122-130e-4db7-ae55-460c84819f0e.pdf", w);
        assert_eq!(two.len(), 2, "{two:?}");
        assert_eq!(two.concat(), "1d9bf122-130e-4db7-ae55-460c84819f0e.pdf", "nothing is left out of two lines");
        let long = "2021一带一路暨金砖大赛之物联网技术及其在智慧城市与工业互联网领域的应用创新赛表彰文件.pdf";
        let lines = name_lines(&f.heading, px, long, w);
        let c = Canvas::new(0, 0);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(long.starts_with(&lines[0]), "the first line is the start: {lines:?}");
        assert!(lines[1].contains('…') && lines[1].ends_with("文件.pdf"), "the middle goes, the end stays: {lines:?}");
        for line in &lines {
            assert!(c.text_width(&f.heading, line, px) <= w, "{line:?} fits");
        }
    }

    #[test]
    fn a_file_is_typed_by_its_extension_in_any_case() {
        let typed = [
            ("boot.img", FileType::DiskImage),
            ("OTA_ishtar.ZIP", FileType::Archive),
            ("rootfs.tar.gz", FileType::Archive),
            ("IMG_0001.HEIC", FileType::Picture),
            ("clip.mkv", FileType::Video),
            ("track.flac", FileType::Audio),
            ("app-release.apk", FileType::Package),
            ("recovery.log", FileType::Text),
            ("build.prop", FileType::Code),
            ("init.rc", FileType::Code),
            ("report.docx", FileType::Document),
            ("budget.csv", FileType::Spreadsheet),
            ("slides.pptx", FileType::Presentation),
            ("manual.pdf", FileType::Pdf),
        ];
        for (name, kind) in typed {
            assert_eq!(file_type(name), kind, "{name}");
        }
        for name in ["boot.img.lz4.000", ".bashrc", "toybox", "notes."] {
            assert_eq!(file_type(name), FileType::Other, "{name}");
        }
    }

    const KINDS: [FileType; 13] = [
        FileType::Archive,
        FileType::DiskImage,
        FileType::Picture,
        FileType::Video,
        FileType::Audio,
        FileType::Package,
        FileType::Text,
        FileType::Code,
        FileType::Document,
        FileType::Spreadsheet,
        FileType::Presentation,
        FileType::Pdf,
        FileType::Other,
    ];

    #[test]
    fn each_type_has_an_icon_of_its_own() {
        let f = crate::font::bundled();
        let drawn: Vec<Canvas> = KINDS
            .iter()
            .map(|&kind| {
                let mut c = Canvas::new(140, 140);
                c.clear(WHITE);
                file_icon(&mut c, &f, 0.0, 0.0, 140.0, WHITE, kind);
                c
            })
            .collect();
        for (i, a) in drawn.iter().enumerate() {
            for (j, b) in drawn.iter().enumerate().skip(i + 1) {
                assert_ne!(a.data, b.data, "{:?} and {:?} look alike", KINDS[i], KINDS[j]);
            }
        }
    }

    /// Where the white of a type's sign lies on its sheet, in the icon's 40
    /// units: its left, top, right and bottom, drawn ten pixels a unit on a
    /// black page.
    fn ink(f: &Fonts, kind: FileType) -> (f32, f32, f32, f32) {
        let px = 10.0;
        let side = (40.0 * px) as u32;
        let mut c = Canvas::new(side, side);
        let page = Rgba::rgb(0, 0, 0);
        c.clear(page);
        file_icon(&mut c, f, 0.0, 0.0, side as f32, page, kind);
        let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
        for (i, p) in c.data.chunks(4).enumerate() {
            if p[..3].iter().all(|&v| v > 230) {
                let (x, y) = (i as u32 % side, i as u32 / side);
                (l, t, r, b) = (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1));
            }
        }
        (l as f32 / px, t as f32 / px, r as f32 / px, b as f32 / px)
    }

    #[test]
    fn each_sign_sits_in_the_middle_of_its_sheet() {
        let f = crate::font::bundled();
        let mut off = Vec::new();
        for kind in KINDS {
            let (l, t, r, b) = ink(&f, kind);
            let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
            // The zip's teeth run down from the top edge: only its sideways
            // place counts.
            let level = kind == FileType::Archive || (cy - 23.0).abs() <= 0.3;
            if (cx - 20.0).abs() > 0.3 || !level {
                off.push(format!("{kind:?} at ({cx:.2}, {cy:.2}) over {l:.1}..{r:.1}, {t:.1}..{b:.1}"));
            }
        }
        assert!(off.is_empty(), "off the middle (20, 23):\n{}", off.join("\n"));
    }

    #[test]
    fn the_path_goes_up_by_its_capsules_and_shows_its_end_when_long() {
        let f = crate::font::bundled();
        let t = Theme::light(1440, 3200, 3.5);
        let path = strip(&t, &f, "根目录", "/system/bin");
        let ups: Vec<Action> = path.hits.iter().map(|h| h.action).collect();
        assert_eq!(ups, [Action::OpenAncestor(0), Action::OpenAncestor(1)], "the root and system go up; bin is where we are");
        let place = strip(&t, &f, "内部存储", "Download");
        assert_eq!(place.hits.iter().map(|h| h.action).collect::<Vec<_>>(), [Action::OpenAncestor(0)], "a place's top goes up");
        let deep = strip(&t, &f, "根目录", "/data/media/0/Android/data/com.example.something.long/files/cache/images");
        let last = deep.hits.last().expect("the capsules before the folder shown");
        assert!(last.rect.x + last.rect.w < t.m.w as f32, "the end shows");
        assert!(deep.hits[0].rect.x < 0.0, "the start runs off the left edge");
    }
}
