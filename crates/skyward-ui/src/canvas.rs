use crate::color::{blend_over, blend_px, Rgba};
use crate::font::{Face, SUBPIXELS};
use ab_glyph::{Font, GlyphId, PxScale, ScaleFont};

/// A straight-alpha RGBA8 raster target. All primitives are antialiased via
/// signed-distance coverage so the same code produces crisp output on the
/// device framebuffer and in host PNG previews.
///
/// A canvas may hold only a band of its rows, and of those only some
/// columns, for a page drawn a part at a time: everything is drawn where it
/// lies on the whole, and only what falls in the part held is written.
#[derive(Clone)]
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    /// The rows held, rows `top..bottom`, top to bottom; of each, the
    /// `span` columns from column `left`.
    pub data: Vec<u8>,
    top: u32,
    bottom: u32,
    left: u32,
    span: u32,
}

/// How a layer covers a stretch of pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Clear,
    Solid,
    Partial,
}

/// Pixels `x0..x1` of a row, all covered the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub x0: u16,
    pub x1: u16,
    pub cover: Cover,
}

/// Canvases at least this big leave their memory for the next ones, up to
/// so many kept: memory freshly mapped costs a page fault, the kernel
/// clearing the page, for every 4 KiB as it is first written, which costs
/// more than drawing the page.
const KEEP_BYTES: usize = 1 << 20;
const KEEP: usize = 6;

static KEPT: std::sync::Mutex<Vec<Vec<u8>>> = std::sync::Mutex::new(Vec::new());

/// Memory for `n` bytes of pixels, as it was left: the smallest kept that
/// was written that far, whose pages are mapped already; else the smallest
/// with room for them; else new.
fn kept(n: usize) -> Vec<u8> {
    let kept = if n >= KEEP_BYTES {
        let mut kept = KEPT.lock().expect("no one panics holding the kept memory");
        let best = |fits: &dyn Fn(&Vec<u8>) -> bool| {
            kept.iter().enumerate().filter(|(_, v)| fits(v)).min_by_key(|(_, v)| v.capacity()).map(|(i, _)| i)
        };
        let best = best(&|v| v.len() >= n).or_else(|| best(&|v| v.capacity() >= n));
        best.map(|i| kept.swap_remove(i))
    } else {
        None
    };
    kept.unwrap_or_else(|| Vec::with_capacity(n))
}

// Drawing primitives take geometry plus style as plain arguments.
#[allow(clippy::too_many_arguments)]
impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Self::filled(w, h, Rgba { r: 0, g: 0, b: 0, a: 0 })
    }

    /// A canvas every pixel of which is `color`.
    pub fn filled(w: u32, h: u32, color: Rgba) -> Self {
        Self::band(w, h, 0..h, 0..w, color)
    }

    /// A canvas `w` × `h` that holds only rows `rows` of it, and of those
    /// only columns `cols`, every pixel of them `color`.
    pub fn band(w: u32, h: u32, rows: std::ops::Range<u32>, cols: std::ops::Range<u32>, color: Rgba) -> Self {
        let (top, bottom) = (rows.start.min(h), rows.end.clamp(rows.start.min(h), h));
        let (left, right) = (cols.start.min(w), cols.end.clamp(cols.start.min(w), w));
        let span = right - left;
        let n = (span * (bottom - top) * 4) as usize;
        let mut data = kept(n);
        data.clear();
        if n > 0 {
            // One pixel, then all written so far copied after itself.
            data.extend_from_slice(&[color.r, color.g, color.b, color.a]);
            while data.len() < n {
                data.extend_from_within(..data.len().min(n - data.len()));
            }
        }
        Self { w, h, data, top, bottom, left, span }
    }

    /// A canvas `w` × `h` that holds rows `rows` of it whole, as whatever
    /// its memory held before: for drawing into a part at a time, where
    /// what is not drawn yet is never shown.
    pub fn unfilled(w: u32, h: u32, rows: std::ops::Range<u32>) -> Self {
        let (top, bottom) = (rows.start.min(h), rows.end.clamp(rows.start.min(h), h));
        let n = (w * (bottom - top) * 4) as usize;
        let mut data = kept(n);
        if data.len() >= n {
            data.truncate(n);
        } else {
            data.resize(n, 0);
        }
        Self { w, h, data, top, bottom, left: 0, span: w }
    }

    /// Keep memory for `n` canvases `w` × `h`, written to once, for the
    /// first canvases drawn on to take without the kernel mapping it a page
    /// at a time as they are.
    pub fn keep_ready(w: u32, h: u32, n: usize) {
        let canvases: Vec<Canvas> = (0..n).map(|_| Canvas::new(w, h)).collect();
        drop(canvases);
    }

    /// The rows the canvas holds.
    pub fn held(&self) -> std::ops::Range<u32> {
        self.top..self.bottom
    }

    /// The columns of the rows held the canvas holds.
    pub fn cols(&self) -> std::ops::Range<u32> {
        self.left..self.left + self.span
    }

    /// Whether the canvas holds all of itself.
    pub fn whole(&self) -> bool {
        self.held() == (0..self.h) && self.cols() == (0..self.w)
    }

    /// Write the canvas, which holds all of itself, as a PNG of what the
    /// panel shows of it: each pixel's color, without its alpha, which the
    /// panel does not show.
    pub fn write_png(&self, to: impl std::io::Write) -> anyhow::Result<()> {
        assert!(self.whole(), "a picture is made of a canvas that holds all of itself");
        let rgb: Vec<u8> = self.data.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
        let mut png = png::Encoder::new(to, self.w, self.h);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header()?;
        writer.write_image_data(&rgb)?;
        writer.finish()?;
        Ok(())
    }

    /// Take on the rows `band` holds, the same columns of them as this
    /// canvas holds, which run on from this canvas's last row or up to its
    /// first.
    pub fn join(&mut self, band: &Canvas) {
        assert_eq!((band.w, band.h, band.cols()), (self.w, self.h, self.cols()), "a band joins a canvas of its size, in its columns");
        let (mine, theirs) = (self.held(), band.held());
        if theirs.start == mine.end {
            self.data.extend_from_slice(&band.data);
            self.bottom = theirs.end;
        } else {
            assert_eq!(theirs.end, mine.start, "a band joins where the rows held end or begin");
            self.data.splice(0..0, band.data.iter().copied());
            self.top = theirs.start;
        }
    }

    /// Take what `band` holds in place of this canvas's own, where it holds
    /// the same.
    pub fn replace(&mut self, band: &Canvas) {
        assert_eq!((band.w, band.h), (self.w, self.h), "a band replaces part of a canvas of its size");
        let (rows, cols) = (self.rows(band.top as i32, band.held().end as i32), self.columns(band.left as i32, (band.left + band.span) as i32));
        if cols.0 >= cols.1 {
            return;
        }
        let n = (cols.1 - cols.0) as usize * 4;
        for y in rows.0..rows.1.min(band.held().end as i32) {
            let (to, from) = (self.at(cols.0, y), band.at(cols.0, y));
            self.data[to..to + n].copy_from_slice(&band.data[from..from + n]);
        }
    }

    /// The rows of `y0..y1` this canvas holds.
    #[inline]
    fn rows(&self, y0: i32, y1: i32) -> (i32, i32) {
        let held = self.held();
        (y0.max(held.start as i32), y1.min(held.end as i32))
    }

    /// The columns of `x0..x1` this canvas holds.
    #[inline]
    fn columns(&self, x0: i32, x1: i32) -> (i32, i32) {
        (x0.max(self.left as i32), x1.min((self.left + self.span) as i32))
    }

    /// Where the pixel (`x`, `y`) of a row and column held starts in `data`.
    #[inline]
    fn at(&self, x: i32, y: i32) -> usize {
        (((y - self.top as i32) * self.span as i32 + x - self.left as i32) * 4) as usize
    }

    #[inline]
    fn px(&mut self, x: i32, y: i32) -> Option<&mut [u8]> {
        if x < self.left as i32 || x >= (self.left + self.span) as i32 || y < self.top as i32 || y >= self.bottom as i32 {
            return None;
        }
        let i = self.at(x, y);
        Some(&mut self.data[i..i + 4])
    }

    /// Make this canvas, which holds all of itself, `h` tall: the rows it
    /// keeps as they were, any it gains clear.
    pub fn resize(&mut self, h: u32) {
        assert!(self.whole(), "only a canvas holding all of itself is resized");
        self.h = h;
        self.bottom = h;
        self.data.resize((self.w * h * 4) as usize, 0);
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, color: Rgba, cov: f32) {
        if let Some(p) = self.px(x, y) {
            blend_over(p, color, cov);
        }
    }

    pub fn clear(&mut self, color: Rgba) {
        for px in self.data.as_chunks_mut::<4>().0 {
            *px = [color.r, color.g, color.b, color.a];
        }
    }

    /// Overwrite a pixel-aligned rectangle with `color`.
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        let (x0, x1) = self.columns(x.floor().max(0.0) as i32, (x + w).ceil().max(0.0) as i32);
        let (y0, y1) = self.rows(y.floor().max(0.0) as i32, (y + h).ceil().max(0.0) as i32);
        if x0 >= x1 {
            return;
        }
        let px = [color.r, color.g, color.b, color.a];
        for row in y0..y1 {
            let (start, end) = (self.at(x0, row), self.at(x1, row));
            self.data[start..end].as_chunks_mut::<4>().0.fill(px);
        }
    }

    /// Paint rows `y0..y1` with a colour computed per pixel centre.
    pub fn shade_rows(&mut self, y0: u32, y1: u32, paint: impl Fn(f32, f32) -> Rgba) {
        let (y0, y1) = self.rows(y0 as i32, y1 as i32);
        let (x0, x1) = self.columns(0, self.w as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let c = paint(x as f32 + 0.5, y as f32 + 0.5);
                self.blend(x, y, c, 1.0);
            }
        }
    }

    /// Overwrite row `y`, which the canvas holds, with `paint(x)` for every
    /// column `x` it holds.
    pub fn paint_row(&mut self, y: u32, paint: impl Fn(usize) -> Rgba) {
        let start = self.at(self.left as i32, y as i32);
        let row = &mut self.data[start..start + (self.span * 4) as usize];
        for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let c = paint(self.left as usize + x);
            *p = [c.r, c.g, c.b, c.a];
        }
    }

    /// Dim this opaque canvas toward black, keeping `level` of its light.
    pub fn dim(&mut self, level: f32) {
        let keep = (level.clamp(0.0, 1.0) * 256.0).round() as u32;
        if keep >= 256 {
            return;
        }
        for p in self.data.as_chunks_mut::<4>().0 {
            *p = darken(*p, keep);
        }
    }

    /// Lay `layer`, whose top-left corner goes at (`x`, `y`), over this
    /// opaque canvas at `alpha` of its strength.
    pub fn lay(&mut self, layer: &Canvas, x: i32, y: i32, alpha: f32) {
        assert!(layer.whole(), "a layer laid holds all of itself");
        if alpha <= 0.0 {
            return;
        }
        let strength = (alpha * 256.0).round() as u32;
        let lw = layer.w as i32;
        let (from, to) = self.columns(x, x + lw);
        let (x0, x1) = (from - x, to - x);
        if x0 >= x1 {
            return;
        }
        let (first, last) = self.rows(y, y + layer.h as i32);
        for py in first..last {
            let ly = py - y;
            let src = &layer.data[((ly * lw + x0) * 4) as usize..((ly * lw + x1) * 4) as usize];
            let start = self.at(x + x0, py);
            let dst = &mut self.data[start..start + src.len()];
            if src.as_chunks::<4>().0.iter().fold(255, |all, s| all & s[3]) == 255 {
                // Where the layer is opaque, each pixel mixes at the same
                // strength.
                for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
                    *d = mix(*d, *s, strength);
                }
                continue;
            }
            for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
                if s[3] == 0 {
                    continue;
                }
                let a = ((s[3] as u32 + (s[3] as u32 >> 7)) * strength) >> 8;
                for k in 0..3 {
                    d[k] = ((s[k] as u32 * a + d[k] as u32 * (256 - a) + 128) >> 8) as u8;
                }
            }
        }
    }

    /// Where `layer` was laid with its top-left corner at (`x`, `y`), show
    /// `now` instead, `level` of the way: as if `now` had been laid there
    /// over what `layer` covered. `now` is as big as `layer` and covers
    /// wholly wherever `layer` does.
    pub fn relay(&mut self, layer: &Canvas, now: &Canvas, x: i32, y: i32, level: f32) {
        assert_eq!((layer.w, layer.h), (now.w, now.h), "the two layers are one size");
        assert!(self.whole() && layer.whole() && now.whole(), "layers are laid again whole on a whole canvas");
        if level <= 0.0 {
            return;
        }
        let (k, k8) = (level.min(1.0), (level.min(1.0) * 256.0).round() as i32);
        let (lw, sw) = (layer.w as i32, self.w as i32);
        let (x0, x1) = ((-x).max(0), (sw - x).min(lw));
        for ly in 0..layer.h as i32 {
            let py = y + ly;
            if py < 0 || py >= self.h as i32 || x0 >= x1 {
                continue;
            }
            let (from, to) = (((ly * lw + x0) * 4) as usize, ((ly * lw + x1) * 4) as usize);
            let start = ((py * sw + x + x0) * 4) as usize;
            let dst = &mut self.data[start..start + to - from];
            let (was, is) = (layer.data[from..to].as_chunks::<4>().0, now.data[from..to].as_chunks::<4>().0);
            if was.iter().fold(255, |all, p| all & p[3]) == 255 {
                // Where the layer was opaque, the pixel moves towards the
                // new layer's, except where the two layers are the same,
                // so what lies over them stays.
                for ((was, is), d) in was.iter().zip(is).zip(dst.as_chunks_mut::<4>().0) {
                    *d = mix(*d, *is, if was == is { 0 } else { k8 as u32 });
                }
                continue;
            }
            let pairs = was.iter().zip(is);
            for ((was, is), d) in pairs.zip(dst.as_chunks_mut::<4>().0) {
                if was == is {
                    continue;
                }
                if was[3] == 255 {
                    for c in 0..3 {
                        let o = d[c] as i32;
                        d[c] = (o + (((is[c] as i32 - o) * k8 + 128) >> 8)) as u8;
                    }
                    continue;
                }
                // What lay under the layer comes back from what it made
                // of it.
                let (wa, na) = (was[3] as f32 / 255.0, is[3] as f32 / 255.0);
                for c in 0..3 {
                    let o = d[c] as f32;
                    let under = (o - was[c] as f32 * wa) / (1.0 - wa);
                    let target = is[c] as f32 * na + under * (1.0 - na);
                    d[c] = (o + (target - o) * k).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// Show a page sliding over another, as pages open and go back: the
    /// page `front` in front, standing `at` pixels from the left edge with
    /// its left corners rounded to `radius`, over the page `behind`,
    /// standing `shift` pixels left of where it rests, darkened by `dim`
    /// (black laid over it, 0 to 1), and black past its right edge. Both
    /// pages are as big as this canvas, and each of its pixels is written
    /// once.
    pub fn slide(&mut self, front: &Canvas, behind: &Canvas, at: i32, shift: i32, dim: f32, radius: f32) {
        assert!((front.w, front.h) == (self.w, self.h) && (behind.w, behind.h) == (self.w, self.h), "the pages are one size");
        assert!(self.whole() && front.whole() && behind.whole(), "pages slide whole");
        let (w, h) = (self.w as i32, self.h as i32);
        let (at, shift) = (at.clamp(0, w), shift.clamp(0, w));
        let keep = ((1.0 - dim.clamp(0.0, 1.0)) * 256.0).round() as u32;
        let row = w as usize * 4;
        // Left of the front page's edge only the page behind shows; at the
        // edge, where the front page's corners round off, both do. The page
        // behind reaches the first `reach` columns, black past them.
        let plain = (at - 1).max(0) as usize;
        let reach = (w - shift) as usize;
        let lit = plain.min(reach);
        for y in 0..h {
            let rows = y as usize * row..(y as usize + 1) * row;
            let line = self.data[rows.clone()].as_chunks_mut::<4>().0;
            let ahead = front.data[rows.clone()].as_chunks::<4>().0;
            let back = behind.data[rows].as_chunks::<4>().0;
            for (d, s) in line[..lit].iter_mut().zip(&back[shift as usize..]) {
                *d = darken(*s, keep);
            }
            line[lit..plain].fill([0, 0, 0, 255]);
            let mut x = plain as i32;
            while x < w {
                let cover = front_cover(x, y, at, h, radius);
                if cover >= 1.0 {
                    break;
                }
                let k = x as usize;
                let under = if k >= reach { [0, 0, 0, 255] } else { darken(back[k + shift as usize], keep) };
                // A pixel the front page covers at all lies right of its
                // edge.
                line[k] = if cover <= 0.0 {
                    under
                } else {
                    let over = ahead[(x - at) as usize];
                    let a = (cover * 256.0).round() as u32;
                    let mix = |f: u8, b: u8| ((f as u32 * a + b as u32 * (256 - a) + 128) >> 8) as u8;
                    [mix(over[0], under[0]), mix(over[1], under[1]), mix(over[2], under[2]), 255]
                };
                x += 1;
            }
            if x < w {
                line[x as usize..].copy_from_slice(&ahead[(x - at) as usize..(w - at) as usize]);
            }
        }
    }

    /// A copy of the `w` × `h` pixels from (`x`, `y`), which lie in the rows
    /// this canvas holds.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Canvas {
        let (held, cols) = (self.held(), self.cols());
        assert!(cols.start <= x && x + w <= cols.end && held.start <= y && y + h <= held.end, "the part cropped lies in what is held");
        let mut out = Canvas::new(w, h);
        let part = w as usize * 4;
        for (k, dst) in out.data.chunks_exact_mut(part).enumerate() {
            let at = self.at(x as i32, (y as usize + k) as i32);
            dst.copy_from_slice(&self.data[at..at + part]);
        }
        out
    }

    /// Copy the opaque `strip`, as wide as this canvas, over it with its top
    /// row at `y`; rows past either edge are left out.
    pub fn put(&mut self, strip: &Canvas, y: i32) {
        assert_eq!(strip.w, self.w, "a strip spans the canvas");
        assert!(self.whole() && strip.whole(), "a strip is put whole on a whole canvas");
        let first = (-y).clamp(0, strip.h as i32);
        let last = (self.h as i32 - y).clamp(first, strip.h as i32);
        if first == last {
            return;
        }
        let row = self.w as usize * 4;
        let dst = (y + first) as usize * row;
        let n = (last - first) as usize * row;
        self.data[dst..dst + n].copy_from_slice(&strip.data[first as usize * row..first as usize * row + n]);
    }

    /// Copy rows `from..to` of the opaque `src`, as wide as this canvas,
    /// over it with the first of them at `y`; rows past either edge, of
    /// either canvas, are left out. Those that land on this canvas are rows
    /// `src` holds.
    pub fn put_rows(&mut self, src: &Canvas, from: i32, to: i32, y: i32) {
        assert_eq!(src.w, self.w, "the rows span the canvas");
        assert!(self.whole(), "rows are put on a whole canvas");
        // Rows asked for above `src`'s top are left out, the rest landing
        // where they would have.
        let skip = (-from).max(0);
        let (from, to, y) = (from + skip, to.min(src.h as i32), y + skip);
        let n = (to - from).max(0);
        let first = (-y).clamp(0, n);
        let last = (self.h as i32 - y).clamp(first, n);
        if first == last {
            return;
        }
        let held = src.held();
        assert!(held.start as i32 <= from + first && from + last <= held.end as i32, "the rows put are held");
        assert_eq!(src.cols(), 0..src.w, "the rows put are held whole");
        let row = self.w as usize * 4;
        let (dst, at) = ((y + first) as usize * row, src.at(0, from + first));
        let len = (last - first) as usize * row;
        self.data[dst..dst + len].copy_from_slice(&src.data[at..at + len]);
    }

    /// Each row as runs of clear, solid and partly covered pixels, for
    /// laying the canvas over something that keeps changing.
    pub fn runs(&self) -> Vec<Vec<Run>> {
        assert!(self.whole(), "runs are found on a whole canvas");
        let w = self.w as usize;
        let cover = |a: u8| match a {
            0 => Cover::Clear,
            255 => Cover::Solid,
            _ => Cover::Partial,
        };
        self.data
            .chunks_exact(w * 4)
            .map(|row| {
                let px = row.as_chunks::<4>().0;
                let mut runs = Vec::new();
                let (mut start, mut now) = (0, cover(px[0][3]));
                let mut x = 1;
                while x < w {
                    // Sixteen pixels as clear, or as solid, as the run goes
                    // on are passed over at once.
                    let same = match now {
                        Cover::Clear => Some(0),
                        Cover::Solid => Some(255),
                        Cover::Partial => None,
                    };
                    if let Some(a) = same.filter(|_| x + 16 <= w)
                        && px[x..x + 16].iter().all(|p| p[3] == a)
                    {
                        x += 16;
                        continue;
                    }
                    let c = cover(px[x][3]);
                    if c != now {
                        runs.push(Run { x0: start as u16, x1: x as u16, cover: now });
                        (start, now) = (x, c);
                    }
                    x += 1;
                }
                runs.push(Run { x0: start as u16, x1: w as u16, cover: now });
                runs
            })
            .collect()
    }

    /// Drop everything below row `h`, for pages measured after drawing.
    pub fn crop_height(mut self, h: u32) -> Canvas {
        let h = h.min(self.h);
        let held = self.held();
        self.data.truncate((self.span * (h.clamp(held.start, held.end) - held.start) * 4) as usize);
        self.top = self.top.min(h);
        self.bottom = self.bottom.min(h);
        self.h = h;
        self
    }

    /// A rounded rectangle in one colour.
    pub fn fill_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: Rgba) {
        self.fill_round_rect_graded(x, y, w, h, r, &|_| color);
    }

    /// A rounded rectangle shading from top to bottom, each row in the
    /// colour `grade` gives for the height of its centre. How far each row
    /// reaches is worked out from the outline: the run it covers wholly is
    /// written straight, and only the pixels the outline crosses are worked
    /// out one by one.
    pub fn fill_round_rect_graded(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, grade: &dyn Fn(f32) -> Rgba) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0, h / 2.0);
        let r = r.min(hx).min(hy);
        let (x0, x1) = self.columns((x - 1.0).floor() as i32, (x + w + 1.0).ceil() as i32);
        let (y0, y1) = self.rows((y - 1.0).floor() as i32, (y + h + 1.0).ceil() as i32);
        if x0 >= x1 {
            return;
        }
        // A pixel centre half a pixel inside the outline is covered wholly,
        // and one half a pixel outside not at all.
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            let qy = (fy - cy).abs() - (hy - r);
            let Some(outer) = round_reach(hx, r, qy, 0.5) else { continue };
            let color = grade(fy);
            let lo = ((cx - outer - 0.5).floor() as i32).max(x0);
            let hi = (((cx + outer - 0.5).ceil() as i32) + 1).min(x1).max(lo);
            let (a, b) = match round_reach(hx, r, qy, -0.5) {
                Some(inner) => centred(cx, inner, lo, hi),
                None => (hi, hi),
            };
            for px in (lo..a).chain(b..hi) {
                let cov = (0.5 - rr_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
            self.span(py, a, b, color);
        }
    }

    /// Lay `color` wholly over pixels `a..b` of row `y`, which lie on the
    /// canvas in a row it holds.
    fn span(&mut self, y: i32, a: i32, b: i32, color: Rgba) {
        if a >= b {
            return;
        }
        let (start, end) = (self.at(a, y), self.at(b, y));
        let run = self.data[start..end].as_chunks_mut::<4>().0;
        if color.a == 255 {
            run.fill([color.r, color.g, color.b, 255]);
        } else if run.iter().fold(255, |all, p| all & p[3]) == 255 {
            // Over opaque pixels each colour byte is the same mix, divided
            // by 255 with a shift and an add, exactly for the sums a mix
            // makes; red and blue together, then green.
            let a = color.a as u32;
            let lit_rb = (color.r as u32 | (color.b as u32) << 16) * a + 0x007f_007f;
            let lit_g = color.g as u32 * a + 0x7f;
            let by255 = |y: u32| ((y + 0x0001_0001 + ((y >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
            for p in run {
                let v = u32::from_le_bytes(*p);
                let rb = by255(lit_rb + (v & 0x00ff_00ff) * (255 - a));
                let g = by255(lit_g + ((v >> 8) & 0xff) * (255 - a)) & 0xff;
                *p = (rb | (g << 8) | 0xff00_0000).to_le_bytes();
            }
        } else {
            for p in run {
                blend_px(p, color, color.a as u32);
            }
        }
    }

    /// The soft shadow a rounded rectangle lifted off the page casts: its
    /// shape blurred by a Gaussian of deviation `sigma`, in `color` where
    /// the shape is whole. Only what lies outside the rectangle is laid,
    /// the rectangle being drawn over the rest: half the colour at its
    /// edge, fading with the distance from it as the blur has it.
    #[allow(clippy::too_many_arguments)]
    pub fn shadow_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, sigma: f32, color: Rgba) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0, h / 2.0);
        let r = r.min(hx).min(hy);
        let reach = SHADOW_REACH * sigma;
        let (x0, x1) = self.columns((x - reach).floor() as i32, (x + w + reach).ceil() as i32);
        let (y0, y1) = self.rows((y - reach).floor() as i32, (y + h + reach).ceil() as i32);
        if x0 >= x1 {
            return;
        }
        // How strong it is a whole number of pixels out, read between.
        let spread = sigma * std::f32::consts::SQRT_2;
        let level: Vec<f32> = (0..reach.ceil() as usize + 2).map(|d| 0.5 * erfc(d as f32 / spread)).collect();
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            let qy = (fy - cy).abs() - (hy - r);
            // The pixels wholly inside are left for the rectangle.
            let (a, b) = match round_reach(hx, r, qy, -0.5) {
                Some(inner) => centred(cx, inner, x0, x1),
                None => (x1, x1),
            };
            for px in (x0..a).chain(b..x1) {
                let d = rr_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r).max(0.0);
                if d >= reach {
                    continue;
                }
                let (i, k) = (d as usize, d.fract());
                self.blend(px, py, color, level[i] + (level[i + 1] - level[i]) * k);
            }
        }
    }

    /// The outline of a rounded rectangle, `t` wide, centred on its edge.
    /// Each row looks only at the pixels the outline crosses on either side
    /// of the middle it leaves clear, and writes straight those it covers
    /// wholly.
    pub fn stroke_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, t: f32, color: Rgba) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0, h / 2.0);
        let r = r.min(hx).min(hy);
        let half = t / 2.0;
        let pad = half + 1.0;
        let (x0, x1) = self.columns((x - pad).floor() as i32, (x + w + pad).ceil() as i32);
        let (y0, y1) = self.rows((y - pad).floor() as i32, (y + h + pad).ceil() as i32);
        if x0 >= x1 {
            return;
        }
        // A pixel centre no further than half the width less half a pixel
        // from the edge, either way, is covered wholly; one further than
        // half the width and half a pixel not at all.
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            let qy = (fy - cy).abs() - (hy - r);
            let reach = |by: f32| round_reach(hx, r, qy, by);
            let Some(outer) = reach(half + 0.5) else { continue };
            let lo = ((cx - outer - 0.5).floor() as i32).max(x0);
            let hi = (((cx + outer - 0.5).ceil() as i32) + 1).min(x1).max(lo);
            let clear = reach(-half - 0.5).map_or((lo, lo), |k| centred(cx, k, lo, hi));
            let whole = match reach(half - 0.5).filter(|_| half >= 0.5) {
                Some(k) => {
                    let (a, b) = centred(cx, k, lo, hi);
                    match reach(0.5 - half) {
                        Some(k) => {
                            let (c, d) = centred(cx, k, a, b);
                            [(a, c), (d, b)]
                        }
                        None => [(a, b), (b, b)],
                    }
                }
                None => [(lo, lo), (lo, lo)],
            };
            let mut skip = [whole[0], clear, whole[1]];
            skip.sort_unstable_by_key(|s| s.0);
            let mut px = lo;
            for (from, to) in skip.into_iter().chain([(hi, hi)]) {
                while px < from.min(hi) {
                    let d = rr_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r);
                    let cov = (0.5 - (d.abs() - half)).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.blend(px, py, color, cov);
                    }
                    px += 1;
                }
                px = px.max(to);
            }
            for (a, b) in whole {
                self.span(py, a, b, color);
            }
        }
    }

    /// A disc: each row's run inside it written straight, its edge pixel by
    /// pixel.
    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgba) {
        let (x0, x1) = self.columns((cx - r - 1.0).floor() as i32, (cx + r + 1.0).ceil() as i32);
        let (y0, y1) = self.rows((cy - r - 1.0).floor() as i32, (cy + r + 1.0).ceil() as i32);
        if x0 >= x1 {
            return;
        }
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            // Pixels whose centres lie half a pixel inside are covered
            // wholly.
            let inside = (r - 0.5) * (r - 0.5) - dy * dy;
            let (a, b) = if r >= 0.5 && inside >= 0.0 {
                let half = inside.sqrt();
                let a = ((cx - half - 0.5).ceil() as i32).clamp(x0, x1);
                (a, (((cx + half - 0.5).floor() as i32) + 1).clamp(a, x1))
            } else {
                (x1, x1)
            };
            for px in (x0..a).chain(b..x1) {
                let dx = px as f32 + 0.5 - cx;
                let cov = (0.5 - ((dx * dx + dy * dy).sqrt() - r)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
            self.span(py, a, b, color);
        }
    }

    /// A ring `t` wide: in each row only the pixels near it are looked at.
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, t: f32, color: Rgba) {
        let (x0, x1) = self.columns((cx - r - t).floor() as i32, (cx + r + t).ceil() as i32);
        let (y0, y1) = self.rows((cy - r - t).floor() as i32, (cy + r + t).ceil() as i32);
        if x0 >= x1 {
            return;
        }
        // Pixels this far or nearer the centre are clear of the ring.
        let hole = r - t / 2.0 - 1.0;
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            let clear = hole * hole - dy * dy;
            let (a, b) = if hole > 0.0 && clear > 0.0 {
                let half = clear.sqrt();
                (((cx - half).ceil() as i32).clamp(x0, x1), ((cx + half).floor() as i32).clamp(x0, x1))
            } else {
                (x1, x1)
            };
            for px in (x0..a).chain(b.max(a)..x1) {
                let dx = px as f32 + 0.5 - cx;
                let d = (dx * dx + dy * dy).sqrt() - r;
                let cov = (0.5 - (d.abs() - t / 2.0)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    /// The part of the ring about (`cx`, `cy`) from radius `r0` out to `r1`
    /// that lies within `half` radians either side of straight up, cut
    /// square along those two lines; with `r0` 0, a slice.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_sector(&mut self, cx: f32, cy: f32, r0: f32, r1: f32, half: f32, color: Rgba) {
        let (s, k) = (half.sin(), half.cos());
        let (x0, x1) = self.columns((cx - r1 * s - 1.0).floor() as i32, (cx + r1 * s + 1.0).ceil() as i32);
        let (y0, y1) = self.rows((cy - r1 - 1.0).floor() as i32, (cy + 1.0).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                let r = dx.hypot(dy);
                // How far out past the ring's edges or the cut on its side,
                // whichever is furthest.
                let d = (r - r1).max(r0 - r).max(dx.abs() * k + dy * s);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    /// Round-capped arc stroke. Angles are in radians in screen space (0 points
    /// right, positive turns clockwise because y grows downward); the arc runs
    /// from `start` through `sweep` radians.
    pub fn stroke_arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, sweep: f32, t: f32, color: Rgba) {
        let tau = std::f32::consts::TAU;
        let (sx, sy) = (cx + r * start.cos(), cy + r * start.sin());
        let end = start + sweep;
        let (ex, ey) = (cx + r * end.cos(), cy + r * end.sin());
        let (x0, x1) = self.columns((cx - r - t).floor() as i32, (cx + r + t).ceil() as i32);
        let (y0, y1) = self.rows((cy - r - t).floor() as i32, (cy + r + t).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                let (dx, dy) = (x - cx, y - cy);
                let rel = (dy.atan2(dx) - start).rem_euclid(tau);
                let d = if rel <= sweep {
                    ((dx * dx + dy * dy).sqrt() - r).abs()
                } else {
                    let a = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
                    let b = ((x - ex).powi(2) + (y - ey).powi(2)).sqrt();
                    a.min(b)
                };
                let cov = (0.5 - (d - t / 2.0)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    /// Ellipse outline using a first-order distance estimate, which is accurate
    /// for the thin strokes icons use.
    pub fn stroke_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, t: f32, color: Rgba) {
        let (x0, x1) = self.columns((cx - rx - t).floor() as i32, (cx + rx + t).ceil() as i32);
        let (y0, y1) = self.rows((cy - ry - t).floor() as i32, (cy + ry + t).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (x, y) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                let f = (x / rx).powi(2) + (y / ry).powi(2) - 1.0;
                let gx = 2.0 * x / (rx * rx);
                let gy = 2.0 * y / (ry * ry);
                let d = f.abs() / (gx * gx + gy * gy).sqrt().max(1e-6);
                let cov = (0.5 - (d - t / 2.0)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    /// A line `t` wide with round ends. Each row looks only at the pixels
    /// whose centres lie near enough the line for it to cover them, and
    /// writes straight those it covers wholly.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, color: Rgba) {
        let vx = x1 - x0;
        let vy = y1 - y0;
        let len2 = (vx * vx + vy * vy).max(1e-6);
        // A pixel centre this near the line is covered at all, and this near
        // wholly.
        let (outer, inner) = (t / 2.0 + 0.5, t / 2.0 - 0.5);
        let (top, bottom) = self.rows((y0.min(y1) - outer).floor() as i32, (y0.max(y1) + outer).ceil() as i32);
        for py in top..bottom {
            let fy = py as f32 + 0.5;
            let Some((lo, hi)) = near_segment(x0, y0, x1, y1, fy, outer) else { continue };
            let (lo, hi) = self.columns((lo - 0.5).ceil() as i32, ((hi - 0.5).floor() as i32) + 1);
            if lo >= hi {
                continue;
            }
            let (a, b) = match near_segment(x0, y0, x1, y1, fy, inner).filter(|_| inner > 0.0) {
                Some((a, b)) => {
                    let a = ((a - 0.5).ceil() as i32).clamp(lo, hi);
                    (a, (((b - 0.5).floor() as i32) + 1).clamp(a, hi))
                }
                None => (hi, hi),
            };
            for px in (lo..a).chain(b..hi) {
                let wx = px as f32 + 0.5 - x0;
                let wy = fy - y0;
                let s = ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0);
                let dx = wx - vx * s;
                let dy = wy - vy * s;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (0.5 - (d - t / 2.0)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
            self.span(py, a, b, color);
        }
    }

    /// Antialiased fill of a convex polygon (points in either winding).
    pub fn fill_convex(&mut self, pts: &[(f32, f32)], color: Rgba) {
        if pts.len() < 3 {
            return;
        }
        let n = pts.len() as f32;
        let (cx, cy) = pts.iter().fold((0.0, 0.0), |(sx, sy), &(x, y)| (sx + x / n, sy + y / n));
        // Per edge: unit normal pointing out of the polygon and its offset.
        let edges: Vec<(f32, f32, f32)> = (0..pts.len())
            .map(|i| {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % pts.len()];
                let (ex, ey) = (bx - ax, by - ay);
                let len = (ex * ex + ey * ey).sqrt().max(1e-6);
                let (mut nx, mut ny) = (ey / len, -ex / len);
                if nx * (cx - ax) + ny * (cy - ay) > 0.0 {
                    nx = -nx;
                    ny = -ny;
                }
                (nx, ny, nx * ax + ny * ay)
            })
            .collect();
        let minx = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32 - 1;
        let maxx = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32 + 1;
        let (minx, maxx) = self.columns(minx, maxx);
        let miny = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32 - 1;
        let maxy = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32 + 1;
        let (miny, maxy) = self.rows(miny, maxy);
        for py in miny..maxy {
            for px in minx..maxx {
                let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                let d = edges.iter().map(|&(nx, ny, c)| nx * x + ny * y - c).fold(f32::MIN, f32::max);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    pub fn text_width(&self, face: &Face, text: &str, px: f32) -> f32 {
        let scale = face.scale(px);
        let sf = face.font.as_scaled(scale);
        let mut w = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in text.chars() {
            let (id, advance) = face.glyph(ch, scale);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += advance;
            prev = Some(id);
        }
        w
    }

    /// Draw a run of text with its baseline at `(x, baseline)`. `px` is the em
    /// size. Returns the advance width; truncates with an ellipsis past `max_w`.
    pub fn text(
        &mut self,
        face: &Face,
        text: &str,
        x: f32,
        baseline: f32,
        px: f32,
        color: Rgba,
        max_w: Option<f32>,
    ) -> f32 {
        self.text_with(face, text, x, baseline, px, Ink::Solid(color), max_w)
    }

    /// [`Canvas::text`] with the ink colour chosen per pixel, for gradient
    /// lettering.
    pub fn text_paint(
        &mut self,
        face: &Face,
        text: &str,
        x: f32,
        baseline: f32,
        px: f32,
        paint: &dyn Fn(f32, f32) -> Rgba,
        max_w: Option<f32>,
    ) -> f32 {
        self.text_with(face, text, x, baseline, px, Ink::Paint(paint), max_w)
    }

    fn text_with(&mut self, face: &Face, text: &str, x: f32, baseline: f32, px: f32, ink: Ink, max_w: Option<f32>) -> f32 {
        let scale = face.scale(px);
        let sf = face.font.as_scaled(scale);
        let (ell, ell_w) = face.glyph('…', scale);
        let limit = max_w.map(|m| x + m);
        let chars: Vec<char> = text.chars().collect();
        let mut pen = x;
        let mut prev: Option<GlyphId> = None;
        for (i, &ch) in chars.iter().enumerate() {
            let (id, aw) = face.glyph(ch, scale);
            if let Some(p) = prev {
                pen += sf.kern(p, id);
            }
            if let Some(lim) = limit {
                let more = i + 1 < chars.len();
                if pen + aw + if more { ell_w } else { 0.0 } > lim {
                    self.glyph(face, ell, scale, pen, baseline, ink);
                    pen += ell_w;
                    break;
                }
            }
            self.glyph(face, id, scale, pen, baseline, ink);
            pen += aw;
            prev = Some(id);
        }
        pen - x
    }

    /// Text whose ink is vertically centred on `cy` (by the reference ideograph).
    pub fn text_vc(&mut self, face: &Face, text: &str, x: f32, cy: f32, px: f32, color: Rgba, max_w: Option<f32>) -> f32 {
        let (top, bottom) = face.ink(px);
        self.text(face, text, x, cy - (top + bottom) / 2.0, px, color, max_w)
    }

    /// Text whose ink top sits at `top_y`.
    pub fn text_top(&mut self, face: &Face, text: &str, x: f32, top_y: f32, px: f32, color: Rgba, max_w: Option<f32>) -> f32 {
        let (top, _) = face.ink(px);
        self.text(face, text, x, top_y - top, px, color, max_w)
    }

    /// Text centred horizontally on `cx` and vertically on `cy`.
    pub fn text_center(&mut self, face: &Face, text: &str, cx: f32, cy: f32, px: f32, color: Rgba) {
        let w = self.text_width(face, text, px);
        self.text_vc(face, text, cx - w / 2.0, cy, px, color, None);
    }

    /// Glyph `id` with its pen position at (`x`, `baseline`), from its ink
    /// drawn for the nearest quarter of a pixel.
    fn glyph(&mut self, face: &Face, id: GlyphId, scale: PxScale, x: f32, baseline: f32, ink: Ink) {
        let quarter = |v: f32| {
            let q = (v * SUBPIXELS).round();
            ((q / SUBPIXELS).floor() as i32, q.rem_euclid(SUBPIXELS) as u8)
        };
        let ((ix, qx), (iy, qy)) = (quarter(x), quarter(baseline));
        let Some(art) = face.ink_of(id, scale, qx, qy) else { return };
        let (ox, oy) = (ix + art.x, iy + art.y);
        let (from, to) = self.columns(ox, ox + art.w as i32);
        let (x0, x1) = (from - ox, to - ox);
        if x0 >= x1 {
            return;
        }
        let (first, last) = self.rows(oy, oy + art.h as i32);
        for py in first..last {
            let gy = py - oy;
            let covs = &art.cov[(gy * art.w as i32 + x0) as usize..(gy * art.w as i32 + x1) as usize];
            let start = self.at(ox + x0, py);
            let row = self.data[start..start + covs.len() * 4].as_chunks_mut::<4>().0;
            for (k, (&c, p)) in covs.iter().zip(row).enumerate() {
                if c == 0 {
                    continue;
                }
                let color = match ink {
                    Ink::Solid(color) => color,
                    Ink::Paint(paint) => paint((ox + x0 + k as i32) as f32 + 0.5, py as f32 + 0.5),
                };
                blend_px(p, color, (color.a as u32 * c as u32 + 127) / 255);
            }
        }
    }
}

/// How text is inked: one colour, or one chosen per pixel.
#[derive(Clone, Copy)]
enum Ink<'a> {
    Solid(Rgba),
    Paint(&'a dyn Fn(f32, f32) -> Rgba),
}

impl Drop for Canvas {
    /// The canvas's memory is kept for the next canvases, in place of the
    /// least kept once so many are: the biggest serve the most.
    fn drop(&mut self) {
        if self.data.capacity() >= KEEP_BYTES {
            let data = std::mem::take(&mut self.data);
            let mut kept = KEPT.lock().expect("no one panics holding the kept memory");
            if kept.len() < KEEP {
                kept.push(data);
            } else if let Some(least) = kept.iter_mut().min_by_key(|v| v.capacity()).filter(|v| v.capacity() < data.capacity()) {
                *least = data;
            }
        }
    }
}

/// The pixel `s` laid over the pixel `d` `a` 256ths of the way, `d`'s
/// alpha kept: red and blue mixed in one multiply, then green.
#[inline]
fn mix(d: [u8; 4], s: [u8; 4], a: u32) -> [u8; 4] {
    let (dv, sv) = (u32::from_le_bytes(d), u32::from_le_bytes(s));
    let rb = (((sv & 0x00ff_00ff) * a + (dv & 0x00ff_00ff) * (256 - a) + 0x0080_0080) >> 8) & 0x00ff_00ff;
    let g = ((((sv >> 8) & 0xff) * a + ((dv >> 8) & 0xff) * (256 - a) + 0x80) >> 8) & 0xff;
    (rb | (g << 8) | (dv & 0xff00_0000)).to_le_bytes()
}

/// The opaque pixel `p` keeping `keep` 256ths of its light: its red and
/// blue scaled in one multiply, then its green.
#[inline]
fn darken(p: [u8; 4], keep: u32) -> [u8; 4] {
    let v = u32::from_le_bytes(p);
    let rb = (((v & 0x00ff_00ff) * keep) >> 8) & 0x00ff_00ff;
    let g = ((((v >> 8) & 0xff) * keep) >> 8) & 0xff;
    (rb | (g << 8) | 0xff00_0000).to_le_bytes()
}

/// How much of pixel (`x`, `y`) a page whose left edge stands at `front`
/// covers, its left corners rounded to `radius` at the top and the foot of
/// a screen `h` tall.
fn front_cover(x: i32, y: i32, front: i32, h: i32, radius: f32) -> f32 {
    let (px, py, half) = (x as f32 + 0.5, y as f32 + 0.5, h as f32 / 2.0);
    let qx = front as f32 + radius - px;
    let qy = (py - half).abs() - (half - radius);
    let d = if qx > 0.0 && qy > 0.0 { qx.hypot(qy) - radius } else { qx.max(qy) - radius };
    (0.5 - d).clamp(0.0, 1.0)
}

/// How far either way from the middle a row of a rounded rectangle `hx`
/// half wide with corners `r` round reaches, for its outline moved `by`
/// outwards; `qy` is how far the row lies past where the corners start to
/// round. Nothing if the row misses it.
fn round_reach(hx: f32, r: f32, qy: f32, by: f32) -> Option<f32> {
    let rr = r + by;
    if rr <= 0.0 {
        // Corners sharper than the move: a plain rectangle.
        (qy <= rr).then_some(hx + by)
    } else if qy <= 0.0 {
        Some(hx + by)
    } else if qy < rr {
        Some(hx - r + (rr * rr - qy * qy).sqrt())
    } else {
        None
    }
}

/// The pixels of `lo..hi` whose centres lie no further than `reach` from
/// `cx` either way: the first, and one past the last.
fn centred(cx: f32, reach: f32, lo: i32, hi: i32) -> (i32, i32) {
    let a = ((cx - reach - 0.5).ceil() as i32).clamp(lo, hi);
    (a, (((cx + reach - 0.5).floor() as i32) + 1).clamp(a, hi))
}

/// Where along the row at height `y` points lie within `reach` of the
/// segment from (`x0`, `y0`) to (`x1`, `y1`): the round end about either
/// point, or beside the segment between them. The three meet, so together
/// they make one run.
fn near_segment(x0: f32, y0: f32, x1: f32, y1: f32, y: f32, reach: f32) -> Option<(f32, f32)> {
    let end = |ex: f32, ey: f32| {
        let dy = y - ey;
        (dy.abs() <= reach).then(|| {
            let half = (reach * reach - dy * dy).sqrt();
            (ex - half, ex + half)
        })
    };
    let (vx, vy) = (x1 - x0, y1 - y0);
    let len = (vx * vx + vy * vy).sqrt();
    let wy = y - y0;
    // Beside the segment: no further from its line than `reach`, and
    // between the lines across its two ends. Each is `lo` ≤ k u + `at` ≤
    // `hi` for u = x - x0, which bounds u from both sides, or holds for
    // all of the row or none of it.
    let between = |k: f32, at: f32, lo: f32, hi: f32| -> Option<(f32, f32)> {
        if k.abs() < 1e-6 {
            return (lo <= at && at <= hi).then_some((f32::MIN, f32::MAX));
        }
        let (a, b) = ((lo - at) / k, (hi - at) / k);
        Some((a.min(b), a.max(b)))
    };
    let side = (len > 1e-6)
        .then(|| {
            // Distance from the line: (u vy - wy vx) / len; how far along
            // it: (u vx + wy vy) / len.
            let across = between(vy, -wy * vx, -reach * len, reach * len)?;
            let along = between(vx, wy * vy, 0.0, len * len)?;
            let (lo, hi) = (across.0.max(along.0), across.1.min(along.1));
            (lo <= hi).then_some((x0 + lo, x0 + hi))
        })
        .flatten();
    [end(x0, y0), end(x1, y1), side].into_iter().flatten().reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// How many deviations out a shadow reaches: beyond, under a 255th of its
/// colour is left.
const SHADOW_REACH: f32 = 3.0;

/// The complementary error function, for `x` from 0 on, by a rational
/// approximation good to 1.5e-7.
fn erfc(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152 + t * 1.061_405_4))));
    poly * (-x * x).exp()
}

fn rr_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let dx = (px - cx).abs() - (hx - r);
    let dy = (py - cy).abs() - (hy - r);
    let ox = dx.max(0.0);
    let oy = dy.max(0.0);
    (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0) - r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_holds_the_colors_the_panel_shows() {
        let mut c = Canvas::filled(3, 2, Rgba::rgb(243, 243, 243));
        c.data[..4].copy_from_slice(&[10, 20, 30, 0]);
        c.data[20..24].copy_from_slice(&[200, 100, 50, 128]);
        let mut png = Vec::new();
        c.write_png(&mut png).unwrap();
        let mut read = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; read.output_buffer_size()];
        let info = read.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (3, 2, png::ColorType::Rgb));
        assert_eq!(&pixels[..3], &[10, 20, 30], "the color whatever the alpha, as the panel shows it");
        assert_eq!(&pixels[15..18], &[200, 100, 50]);
        assert_eq!(&pixels[3..6], &[243, 243, 243]);
    }

    #[test]
    fn a_shadow_fades_out_from_its_edge_as_the_stock_list_of_choices_casts_one() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
        assert!((erfc(1.0) - 0.157_299_2).abs() < 1e-6);
        assert!(erfc(3.0) < 3e-5);
        // The 17 Ultra's list over its grey page, 3 pixels a dp: the grey
        // it took on 0, 10, 20 … 60 pixels out from its right edge.
        let mut c = Canvas::filled(400, 300, Rgba::rgb(243, 243, 243));
        c.shadow_round_rect(50.0, 50.0, 200.0, 200.0, 72.0, 32.1, Rgba::rgb(0, 0, 0).with_alpha(27));
        let stock = [230.0, 233.1, 235.9, 238.2, 239.9, 241.0, 242.2];
        for (k, want) in stock.into_iter().enumerate() {
            let got = c.px(250 + 10 * k as i32, 150).expect("on the canvas")[0] as f32;
            assert!((got - want).abs() <= 1.5, "{} px out: {got}, the stock list {want}", 10 * k);
        }
        assert_eq!(c.px(150, 150).expect("on the canvas")[0], 243, "inside is left for the rectangle");
        assert_eq!(c.px(360, 150).expect("on the canvas")[0], 243, "it reaches no further than three deviations");
    }

    /// A canvas two pixels wide whose row k is grey k.
    fn graded(h: u32) -> Canvas {
        let mut c = Canvas::new(2, h);
        for (k, row) in c.data.chunks_mut(8).enumerate() {
            let g = k as u8;
            row.copy_from_slice(&[g, g, g, 255, g, g, g, 255]);
        }
        c
    }

    /// The grey of each row of `c`.
    fn greys(c: &Canvas) -> Vec<u8> {
        c.data.chunks(8).map(|row| row[0]).collect()
    }

    #[test]
    fn rows_land_where_asked_and_are_cut_at_either_canvas_edge() {
        let src = graded(10);
        let mut c = Canvas::new(2, 4);
        c.put_rows(&src, 2, 5, 1);
        assert_eq!(greys(&c), [0, 2, 3, 4]);
        let mut c = Canvas::new(2, 4);
        c.put_rows(&src, -2, 3, 0);
        assert_eq!(greys(&c), [0, 0, 0, 1], "rows above the source's top are not there to copy");
        let mut c = Canvas::new(2, 4);
        c.put_rows(&src, 6, 10, -1);
        assert_eq!(greys(&c), [7, 8, 9, 0], "rows above this canvas's top are left out");
    }

    #[test]
    fn a_layer_laid_again_in_place_shows_what_the_other_would_have_over_the_same_ground() {
        // A pixel of card and one of a black word on it, over a field that
        // is not one grey; then the card darker, opaque as the settings'
        // cards are and see-through as the About page's dark ones are.
        let card = |grey: u8, a: u8| {
            let mut c = Canvas::new(2, 1);
            c.data.copy_from_slice(&[grey, grey, grey, a, 0, 0, 0, 255]);
            c
        };
        for (was, is) in [(card(255, 255), card(225, 255)), (card(255, 36), card(255, 62))] {
            let ground = |c: &mut Canvas| c.data.copy_from_slice(&[20, 20, 20, 255, 90, 90, 90, 255]);
            let (mut shown, mut wanted) = (Canvas::new(2, 1), Canvas::new(2, 1));
            ground(&mut shown);
            ground(&mut wanted);
            shown.lay(&was, 0, 0, 1.0);
            wanted.lay(&is, 0, 0, 1.0);
            let before = shown.data.clone();
            shown.relay(&was, &is, 0, 0, 1.0);
            for (k, (&got, &want)) in shown.data.iter().zip(&wanted.data).enumerate() {
                assert!(got.abs_diff(want) <= 1, "byte {k}: {got} for {want}");
            }
            assert_eq!(shown.data[4..7], before[4..7], "the word is as it was");
            let mut half = Canvas::new(2, 1);
            half.data.copy_from_slice(&before);
            half.relay(&was, &is, 0, 0, 0.5);
            let mid = (before[0] as f32 + wanted.data[0] as f32) / 2.0;
            assert!((half.data[0] as f32 - mid).abs() <= 1.0, "half way: {} for {mid}", half.data[0]);
        }
    }

    /// A page `w` × `h` whose column x is grey `base` + 10 x.
    fn columns(w: u32, h: u32, base: u8) -> Canvas {
        let mut c = Canvas::new(w, h);
        for (i, p) in c.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let g = base + 10 * (i as u32 % w) as u8;
            *p = [g, g, g, 255];
        }
        c
    }

    /// The grey of each pixel along row `y`.
    fn row_greys(c: &Canvas, y: u32) -> Vec<u8> {
        (0..c.w).map(|x| c.data[((y * c.w + x) * 4) as usize]).collect()
    }

    #[test]
    fn a_page_slid_over_another_shows_the_one_behind_shifted_and_darkened() {
        let (front, behind) = (columns(10, 4, 100), columns(10, 4, 0));
        let mut c = Canvas::new(10, 4);
        c.slide(&front, &behind, 4, 2, 0.5, 0.0);
        assert_eq!(row_greys(&c, 1), [10, 15, 20, 25, 100, 110, 120, 130, 140, 150]);
        c.slide(&front, &behind, 6, 1, 0.5, 0.0);
        assert_eq!(row_greys(&c, 2), [5, 10, 15, 20, 25, 30, 100, 110, 120, 130]);
        // The page behind, shifted far left, is black past its right edge.
        c.slide(&front, &behind, 8, 3, 0.0, 0.0);
        assert_eq!(row_greys(&c, 0), [30, 40, 50, 60, 70, 80, 90, 0, 100, 110]);
    }

    #[test]
    fn a_page_sliding_rounds_its_leading_corners() {
        let behind = Canvas::new(20, 20);
        let mut c = Canvas::new(20, 20);
        c.slide(&columns(20, 20, 50), &behind, 5, 0, 0.0, 6.0);
        let at = |x: u32, y: u32| c.data[((y * 20 + x) * 4) as usize];
        assert_eq!(at(5, 10), 50, "the edge is straight between the corners");
        assert_eq!(at(4, 10), 0);
        for y in [0, 19] {
            assert_eq!(at(5, y), 0, "the corner's tip is cut away");
            assert_eq!(at(5 + 6, y), 50 + 60, "past the corner the page is whole");
        }
        assert!(at(6, 2) > 0 && at(6, 2) < 60, "the curve is smoothed: {}", at(6, 2));
    }

    #[test]
    fn a_crop_copies_the_part_asked_for() {
        let src = graded(6);
        let part = src.crop(1, 2, 1, 3);
        assert_eq!((part.w, part.h), (1, 3));
        assert_eq!(part.data.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), [2, 3, 4]);
    }
}

