use crate::canvas::{Canvas, Cover, Run};
use crate::color::Rgba;
use crate::sprite::Sprite;

/// The moving colour field behind the About page, one per theme: four soft
/// discs whose centres circle slowly while their colours ease between three
/// sets, washed by a noise that pales and lightens them.
pub struct Glow {
    /// Disc centres and radii, in the field box with y up.
    pub discs: [(f32, f32, f32); 4],
    /// How far the centres circle.
    pub wander: f32,
    /// Scale of the noise across the screen.
    pub noise: f32,
    /// How much the noise drains colour, and how much light it adds.
    pub drain: f32,
    pub lift: f32,
    /// Field time between colour changes, doubled.
    pub period: f32,
    /// Field time runs this fast as the colours change, and this fast after.
    pub rush: f32,
    pub pace: f32,
    /// Three colour sets, RGBA per disc; the second is where it starts.
    pub colors: [[[f32; 4]; 4]; 3],
    /// How the wordmark is inked from the field under it.
    pub ink: Ink,
}

/// The wordmark's ink, worked out from the colour beneath it: a grey
/// burnt in (or dodged), a grey laid over in linear light, then lightness
/// raised toward a floor, leaving chroma and hue alone.
#[derive(Clone, Copy)]
pub struct Ink {
    /// Grey and opacity of the burn, or of the dodge when `dodge`.
    pub tone: (f32, f32),
    pub dodge: bool,
    pub linear_light: (f32, f32),
    /// Lightness below `floor` rises by `rise` of the way to it.
    pub floor: f32,
    pub rise: f32,
}

/// Disc centres shared by both themes.
const DISCS: [(f32, f32, f32); 4] = [(0.8, 0.2, 1.0), (0.8, 0.9, 1.0), (0.2, 0.9, 1.0), (0.2, 0.2, 1.0)];

pub const LIGHT: Glow = Glow {
    discs: DISCS,
    wander: 0.2,
    noise: 1.5,
    drain: 0.2,
    lift: 0.1,
    period: 5.0,
    rush: 1.6,
    pace: 1.05,
    colors: [
        [[1.0, 0.9, 0.94, 1.0], [1.0, 0.84, 0.89, 1.0], [0.97, 0.73, 0.82, 1.0], [0.64, 0.65, 0.98, 1.0]],
        [[0.58, 0.74, 1.0, 1.0], [1.0, 0.9, 0.93, 1.0], [0.74, 0.76, 1.0, 1.0], [0.97, 0.77, 0.84, 1.0]],
        [[0.98, 0.86, 0.9, 1.0], [0.6, 0.73, 0.98, 1.0], [0.92, 0.93, 1.0, 1.0], [0.56, 0.69, 1.0, 1.0]],
    ],
    ink: Ink { tone: (74.0 / 255.0, 0.8), dodge: false, linear_light: (79.0 / 255.0, 1.0), floor: 59.0, rise: 0.18 },
};

pub const DARK: Glow = Glow {
    discs: DISCS,
    wander: 0.4,
    noise: 1.5,
    drain: 0.17,
    lift: 0.0,
    period: 8.0,
    rush: 1.0,
    pace: 1.0,
    colors: [
        [[0.2, 0.06, 0.88, 0.4], [0.3, 0.14, 0.55, 0.5], [0.0, 0.64, 0.96, 0.5], [0.11, 0.16, 0.83, 0.4]],
        [[0.07, 0.15, 0.79, 0.5], [0.62, 0.21, 0.67, 0.5], [0.06, 0.25, 0.84, 0.5], [0.0, 0.2, 0.78, 0.5]],
        [[0.58, 0.3, 0.74, 0.4], [0.27, 0.18, 0.6, 0.5], [0.66, 0.26, 0.62, 0.5], [0.12, 0.16, 0.7, 0.6]],
    ],
    ink: Ink { tone: (161.0 / 255.0, 0.9), dodge: true, linear_light: (230.0 / 255.0, 0.3), floor: 59.0, rise: 0.18 },
};

/// A value easing toward a target on a damped spring.
#[derive(Clone, Copy, Debug)]
struct Eased {
    x: f32,
    v: f32,
    to: f32,
    /// Stiffness and damping of the spring.
    k: f32,
    c: f32,
}

impl Eased {
    fn at(x: f32) -> Eased {
        Eased { x, v: 0.0, to: x, k: 0.0, c: 0.0 }
    }

    /// Head for `to` on a spring of damping ratio `zeta` and response
    /// `period` seconds, keeping the current speed.
    fn aim(&mut self, to: f32, zeta: f32, period: f32) {
        let w = std::f32::consts::TAU / period;
        (self.to, self.k, self.c) = (to, w * w, 2.0 * zeta * w);
    }

    fn step(&mut self, dt: f32) {
        self.v += (self.k * (self.to - self.x) - self.c * self.v) * dt;
        self.x += self.v * dt;
    }
}

/// Where the field stands in its slow cycle.
pub struct GlowClock {
    look: &'static Glow,
    /// Wall seconds, run up to 120 and back down, and their direction.
    clock: f32,
    dir: f32,
    /// Field time, which drives the discs and the colour changes.
    time: f32,
    speed: Eased,
    blend: Eased,
    from: usize,
    to: usize,
    changes: u32,
    half: f32,
    /// When the speed eases back after a change.
    calm_at: Option<f32>,
    last: u64,
}

impl GlowClock {
    pub fn new(look: &'static Glow, now: u64) -> GlowClock {
        GlowClock {
            look,
            clock: 0.0,
            dir: 1.0,
            time: 0.0,
            speed: Eased::at(1.0),
            blend: Eased::at(0.0),
            from: 1,
            to: 1,
            changes: 0,
            half: 0.0,
            calm_at: None,
            last: now,
        }
    }

    /// Let wall time run on to `now`.
    pub fn advance(&mut self, now: u64) {
        let mut left = now.saturating_sub(self.last) as f32 / 1000.0;
        self.last = now;
        // Step at most 4 ms at a time so the springs stay true.
        while left > 0.0 {
            let dt = left.min(0.004);
            left -= dt;
            self.step(dt);
        }
    }

    fn step(&mut self, dt: f32) {
        let g = self.look;
        self.clock += dt * self.dir;
        if self.dir > 0.0 && self.clock >= 120.0 {
            self.dir = -1.0;
        } else if self.dir < 0.0 && self.clock <= 0.0 {
            self.dir = 1.0;
        }
        self.time += dt * self.dir * self.speed.x;
        // Every half period of field time the colours move on: second to
        // first, back, second to third, back.
        let d = self.time / g.period;
        let half = ((d - d.floor()) * 2.0).floor();
        if (self.half - half).abs() > 0.5 {
            (self.from, self.to) = [(1, 0), (0, 1), (1, 2), (2, 1)][self.changes as usize % 4];
            self.changes += 1;
            self.blend = Eased::at(0.0);
            self.blend.aim(1.0, 0.9, 1.3);
            self.speed.aim(g.rush, 0.9, 1.3);
            self.calm_at = Some(0.3);
        }
        self.half = half;
        if let Some(t) = self.calm_at.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.speed.aim(g.pace, 0.9, 0.6);
                self.calm_at = None;
            }
        }
        self.speed.step(dt);
        self.blend.step(dt);
    }

    /// The disc colours now.
    pub fn colors(&self) -> [[f32; 4]; 4] {
        let (a, b, t) = (&self.look.colors[self.from], &self.look.colors[self.to], self.blend.x);
        std::array::from_fn(|i| std::array::from_fn(|c| a[i][c] + (b[i][c] - a[i][c]) * t))
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn look(&self) -> &'static Glow {
        self.look
    }
}

/// The field sampled on a coarse grid over the screen, as colours already
/// laid over the page colour and faded by how far the page has scrolled:
/// it varies slowly enough to be filled in between grid points.
pub struct Field {
    pub cell: usize,
    pub cols: usize,
    pub rows: usize,
    /// RGB per grid point, 0 to 255.
    pub rgb: Vec<[f32; 3]>,
}

/// Sample the field over a `w`×`h` screen every `cell` pixels. `top` is the
/// height, as a fraction of the screen, of the band at the top the discs
/// are placed in; `fade` lets the page colour `bg` through.
pub fn sample(clock: &GlowClock, w: usize, h: usize, cell: usize, top: f32, bg: Rgba, fade: f32) -> Field {
    let g = clock.look();
    let t = clock.time();
    let colors = clock.colors();
    // Disc centres move with field time.
    let discs: [(f32, f32, f32); 4] = std::array::from_fn(|i| {
        let (x, y, r) = g.discs[i];
        let x = x + (t + y).sin() * g.wander;
        let y = y + (t + x).cos() * g.wander;
        (x, y, r)
    });
    let (cols, rows) = (w / cell + 2, h / cell + 2);
    let bg = [bg.r as f32, bg.g as f32, bg.b as f32];
    let mut rgb = vec![[0.0; 3]; cols * rows];
    for (j, line) in rgb.chunks_exact_mut(cols).enumerate() {
        for (i, px) in line.iter_mut().enumerate() {
            let (u, v) = (((i * cell) as f32 + 0.5) / w as f32, 1.0 - ((j * cell) as f32 + 0.5) / h as f32);
            let (c, a) = shade(g, &discs, &colors, t, u, v, top);
            *px = std::array::from_fn(|k| bg[k] + (c[k] * 255.0 * a + bg[k] * (1.0 - a) - bg[k]) * fade);
        }
    }
    Field { cell, cols, rows, rgb }
}

impl Field {
    /// Row `y` of the grid, in pixels from its first row, cell by cell:
    /// the colour at each cell's left edge in 16.16 fixed point, rounding
    /// included, and its change from one pixel to the next.
    fn row(&self, y: usize, start: &mut [[i32; 3]], step: &mut [[i32; 3]]) {
        const ONE: f32 = 65536.0;
        let gy = y as f32 / self.cell as f32;
        let j = (gy as usize).min(self.rows - 2);
        let fy = gy - j as f32;
        let mut left = [0.0f32; 3];
        for i in 0..self.cols {
            let (a, b) = (self.rgb[j * self.cols + i], self.rgb[(j + 1) * self.cols + i]);
            let c: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * fy);
            if i > 0 {
                start[i - 1] = left.map(|v| (v * ONE) as i32 + 0x8000);
                step[i - 1] = std::array::from_fn(|k| ((c[k] - left[k]) * ONE / self.cell as f32) as i32);
            }
            left = c;
        }
    }
}

/// Colour and opacity of the field at screen fraction (`u`, `v`), y up.
fn shade(g: &Glow, discs: &[(f32, f32, f32); 4], colors: &[[f32; 4]; 4], t: f32, u: f32, v: f32, top: f32) -> ([f32; 3], f32) {
    // Disc space: the band of height `top` at the top of the screen.
    let (bx, by) = (u, (v - (1.0 - top)) / top);
    let mut c = [0.0f32; 3];
    let mut a = 0.0f32;
    for (i, &(x, y, r)) in discs.iter().enumerate() {
        let d = (bx - x).hypot(by - y);
        if d >= r {
            continue;
        }
        let s = 1.0 - d / r;
        let k = s * s * (3.0 - 2.0 * s);
        let col = colors[i];
        for ch in 0..3 {
            c[ch] += (col[ch] * col[3] - c[ch]) * k;
        }
        a += (col[3] - a) * k;
    }
    if a <= 0.0 {
        return ([0.0; 3], 0.0);
    }
    let c = c.map(|x| x / a);
    let n = perlin(u * g.noise - t, v * g.noise - t);
    let pale = n * n * (3.0 - 2.0 * n);
    let (h, s, val) = to_hsv(c);
    let c = from_hsv(h, s * (1.0 - pale * g.drain), val).map(|x| x + pale * g.lift);
    (c, a.clamp(0.0, 1.0))
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn hash(x: f32, y: f32) -> f32 {
    let mut p = [fract(x * 0.13), fract(y * 0.13), fract(x * 0.13)];
    let d = p[0] * (p[1] + 3.333) + p[1] * (p[2] + 3.333) + p[2] * (p[0] + 3.333);
    p = p.map(|q| q + d);
    fract((p[0] + p[1]) * p[2])
}

/// Smooth value noise in 0..1.
pub(crate) fn perlin(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (a, b, c, d) = (hash(ix, iy), hash(ix + 1.0, iy), hash(ix, iy + 1.0), hash(ix + 1.0, iy + 1.0));
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    a + (b - a) * ux + (c - a) * uy * (1.0 - ux) + (d - b) * ux * uy
}

fn to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        fract(((g - b) / d) / 6.0)
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max > 0.0 { d / max } else { 0.0 }, max)
}

fn from_hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let k = |n: f32| {
        let p = (fract(h + n / 3.0) * 6.0 - 3.0).abs();
        v * (1.0 + s * ((p - 1.0).clamp(0.0, 1.0) - 1.0))
    };
    [k(0.0), k(2.0), k(1.0)]
}

/// The ±5-step dither the field is drawn with, at pixel (`x`, `y`), in
/// steps of one 255th.
pub fn dither(x: usize, y: usize) -> f32 {
    let n = fract(52.982_92 * fract(0.067_110_56 * (x as f32 + 0.5) + 0.005_837_15 * (y as f32 + 0.5)));
    10.0 * n - 5.0
}

/// The field's dither over a whole screen, in sixteenths of a step,
/// worked out once.
pub struct Dither {
    w: usize,
    steps: Vec<i8>,
}

impl Dither {
    /// Worked out a row at a time, many pixels at once.
    pub fn new(w: usize, h: usize) -> Dither {
        let mut steps = vec![0i8; w * h];
        for (y, row) in steps.chunks_exact_mut(w).enumerate() {
            for (x, step) in row.iter_mut().enumerate() {
                *step = (dither(x, y) * 16.0).round() as i8;
            }
        }
        Dither { w, steps }
    }
}

/// Cards scrolling over the field: their pixels, their coverage row by
/// row, and the page row at the top of the screen.
pub struct Layer<'a> {
    pub canvas: &'a Canvas,
    pub runs: &'a [Vec<Run>],
    pub offset: isize,
}

/// Fill `out` with the field, dithered as strongly as `fade` lets it show,
/// and lay `layer` over it.
pub fn paint(out: &mut Canvas, field: &Field, fade: f32, layer: &Layer, dither: &Dither) {
    let w = out.w as usize;
    assert_eq!(dither.w, w, "the dither was made for this screen");
    assert!(out.whole() && layer.canvas.whole(), "the field fills a whole screen under a whole page");
    let clear = [Run { x0: 0, x1: w as u16, cover: Cover::Clear }];
    let mut start = vec![[0i32; 3]; field.cols - 1];
    let mut step = vec![[0i32; 3]; field.cols - 1];
    // A dither value in sixteenths times this gives 16.16 fixed point.
    let shade = (fade * 4096.0).round() as i32;
    for (y, row) in out.data.chunks_exact_mut(w * 4).enumerate() {
        field.row(y, &mut start, &mut step);
        let row = row.as_chunks_mut::<4>().0;
        let noise = &dither.steps[y * w..(y + 1) * w];
        let py = y as isize + layer.offset;
        let inside = usize::try_from(py).ok().filter(|&p| p < layer.runs.len());
        let runs: &[Run] = inside.map_or(&clear, |p| &layer.runs[p]);
        let src = inside.map(|p| layer.canvas.data[p * w * 4..(p + 1) * w * 4].as_chunks::<4>().0);
        for run in runs {
            let (x0, x1) = (run.x0 as usize, run.x1 as usize);
            let px = &mut row[x0..x1];
            match (run.cover, src) {
                (Cover::Solid, Some(src)) => px.copy_from_slice(&src[x0..x1]),
                (Cover::Partial, Some(src)) => {
                    field_span(px, &noise[x0..x1], x0, field.cell, &start, &step, shade);
                    lay_span(px, &src[x0..x1]);
                }
                _ => field_span(px, &noise[x0..x1], x0, field.cell, &start, &step, shade),
            }
        }
    }
}

/// Pixels from column `x0` of a grid row laid out by [`Field::row`], one
/// per entry of `px`, dithered by `noise` at `shade`. Each cell's loop is
/// straight enough for the compiler to run on vector registers.
fn field_span(px: &mut [[u8; 4]], noise: &[i8], x0: usize, cell: usize, start: &[[i32; 3]], step: &[[i32; 3]], shade: i32) {
    let x1 = x0 + px.len();
    let mut x = x0;
    while x < x1 {
        let i = (x / cell).min(start.len() - 1);
        let end = if i + 1 == start.len() { x1 } else { ((i + 1) * cell).min(x1) };
        let (s, st, from) = (start[i], step[i], (x - i * cell) as i32);
        for (j, (p, &d)) in px[x - x0..end - x0].iter_mut().zip(&noise[x - x0..end - x0]).enumerate() {
            let (k, d) = (from + j as i32, d as i32 * shade);
            *p = [
                ((s[0] + st[0] * k + d) >> 16).clamp(0, 255) as u8,
                ((s[1] + st[1] * k + d) >> 16).clamp(0, 255) as u8,
                ((s[2] + st[2] * k + d) >> 16).clamp(0, 255) as u8,
                255,
            ];
        }
        x = end;
    }
}

/// Fill the rows of `out` from `top` down with `field`, whose first grid
/// row lies on row `top`, as many as the field reaches or the screen
/// holds, dithered at full strength.
pub fn paint_band(out: &mut Canvas, field: &Field, top: usize, dither: &Dither) {
    let w = out.w as usize;
    assert_eq!(dither.w, w, "the dither was made for this screen");
    assert!(out.whole(), "the field fills whole rows of a whole screen");
    let bottom = (top + (field.rows - 1) * field.cell).min(out.h as usize);
    let mut start = vec![[0i32; 3]; field.cols - 1];
    let mut step = vec![[0i32; 3]; field.cols - 1];
    for y in top..bottom {
        field.row(y - top, &mut start, &mut step);
        let row = out.data[y * w * 4..(y + 1) * w * 4].as_chunks_mut::<4>().0;
        field_span(row, &dither.steps[y * w..(y + 1) * w], 0, field.cell, &start, &step, 4096);
    }
}

/// Lay straight-alpha pixels `src` over opaque pixels `dst`.
fn lay_span(dst: &mut [[u8; 4]], src: &[[u8; 4]]) {
    for (p, s) in dst.iter_mut().zip(src) {
        let a = s[3] as u32 + (s[3] as u32 >> 7);
        for k in 0..3 {
            p[k] = ((s[k] as u32 * a + p[k] as u32 * (256 - a) + 128) >> 8) as u8;
        }
    }
}

/// Lay the wordmark `sprite`, grown by `scale` about its centre and at
/// `alpha`, in the ink `look` makes of the field under each of its
/// pixels, dithered as the field is.
#[allow(clippy::too_many_arguments)]
pub fn ink_wordmark(out: &mut Canvas, field: &Field, look: &Ink, sprite: &Sprite, scale: f32, alpha: f32, fade: f32, dither: &Dither) {
    assert!(out.whole(), "the wordmark is inked on a whole screen");
    let w = out.w as usize;
    let Some((x0, y0, x1, y1)) = sprite.bounds(0.0, 0.0, scale, w, out.h as usize) else { return };
    if alpha <= 0.0 {
        return;
    }
    // The ink at the grid points around the lettering, filled in between
    // as the field is.
    let cell = field.cell;
    let (i0, j0) = (x0 / cell, y0 / cell);
    let (i1, j1) = (((x1 - 1) / cell + 2).min(field.cols), ((y1 - 1) / cell + 2).min(field.rows));
    let inks = Field {
        cell,
        cols: i1 - i0,
        rows: j1 - j0,
        rgb: (j0..j1).flat_map(|j| (i0..i1).map(move |i| ink(look, field.rgb[j * field.cols + i]))).collect(),
    };
    let shade = (fade * 4096.0).round() as i32;
    let strength = (alpha * 256.0).round() as u32;
    let (mut start, mut step) = (vec![[0i32; 3]; inks.cols - 1], vec![[0i32; 3]; inks.cols - 1]);
    let (mut colors, mut cov) = (vec![[0u8; 4]; x1 - x0], vec![0u8; x1 - x0]);
    for (r, row) in out.data[y0 * w * 4..y1 * w * 4].chunks_exact_mut(w * 4).enumerate() {
        let y = y0 + r;
        sprite.cover_row(0.0, 0.0, scale, y, x0, &mut cov);
        inks.row(y - j0 * cell, &mut start, &mut step);
        field_span(&mut colors, &dither.steps[y * w + x0..y * w + x1], x0 - i0 * cell, cell, &start, &step, shade);
        for ((p, c), &v) in row.as_chunks_mut::<4>().0[x0..x1].iter_mut().zip(colors.iter()).zip(cov.iter()) {
            let k = ((v as u32 + (v as u32 >> 7)) * strength) >> 8;
            for ch in 0..3 {
                p[ch] = ((c[ch] as u32 * k + p[ch] as u32 * (256 - k) + 128) >> 8) as u8;
            }
        }
    }
}

/// The wordmark's colour over field colour `under` (0 to 255).
pub fn ink(look: &Ink, under: [f32; 3]) -> [f32; 3] {
    let (tone, tone_a) = look.tone;
    let (ll, ll_a) = look.linear_light;
    let c = under.map(|v| {
        let b = v / 255.0;
        let t = if look.dodge { (b / (1.0 - tone)).min(1.0) } else { 1.0 - ((1.0 - b) / tone).min(1.0) };
        let b = b + (t - b) * tone_a;
        b + ((b + 2.0 * ll - 1.0).clamp(0.0, 1.0) - b) * ll_a
    });
    let mut lab = to_lab(c);
    lab[0] += look.rise * (look.floor - lab[0]).max(0.0);
    from_lab(lab).map(|v| v * 255.0)
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB (0 to 1) to CIELAB under D65.
fn to_lab(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f32| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn from_lab([l, a, b]: [f32; 3]) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let inv = |t: f32| if t * t * t > 216.0 / 24389.0 { t * t * t } else { (116.0 * t - 16.0) * 27.0 / 24389.0 };
    let (x, y, z) = (inv(fx) * 0.950_47, inv(fy), inv(fz) * 1.088_83);
    let r = 3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z;
    let g = -0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z;
    let b = 0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z;
    [r, g, b].map(|c| from_linear(c.clamp(0.0, 1.0)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_ink_matches_the_wordmark_measured_over_its_backdrop() {
        // Backdrop between the letters and the letters' colour, each a 5×5
        // mean, from a recording of the 17U in light mode.
        for (under, seen) in [
            ([246.0, 203.0, 221.0], [149.0, 27.0, 74.0]),
            ([242.0, 218.0, 240.0], [131.0, 60.0, 126.0]),
            ([241.0, 213.0, 243.0], [130.0, 50.0, 141.0]),
            ([225.0, 202.0, 239.0], [90.0, 24.0, 134.0]),
            ([195.0, 206.0, 253.0], [48.0, 37.0, 176.0]),
            ([205.0, 223.0, 252.0], [44.0, 76.0, 174.0]),
            ([252.0, 219.0, 240.0], [155.0, 62.0, 128.0]),
            ([241.0, 228.0, 253.0], [125.0, 83.0, 159.0]),
        ] {
            // The screen's own dither moves each value by up to five steps.
            let got = ink(&LIGHT.ink, under);
            for c in 0..3 {
                assert!((got[c] - seen[c]).abs() < 12.0, "{under:?}: {got:?} vs {seen:?}");
            }
        }
    }

    #[test]
    fn the_dark_ink_matches_the_wordmark_measured_over_its_backdrop() {
        for (under, seen) in [([27.0, 41.0, 98.0], [127.0, 152.0, 240.0]), ([48.0, 32.0, 90.0], [159.0, 136.0, 238.0])] {
            let got = ink(&DARK.ink, under);
            for c in 0..3 {
                assert!((got[c] - seen[c]).abs() < 8.0, "{under:?}: {got:?} vs {seen:?}");
            }
        }
    }

    #[test]
    fn lab_round_trips() {
        for c in [[0.2, 0.5, 0.9], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.93, 0.78, 0.85]] {
            let back = from_lab(to_lab(c));
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-3, "{c:?} {back:?}");
            }
        }
    }

    #[test]
    fn hsv_round_trips() {
        for c in [[0.2, 0.5, 0.9], [0.93, 0.78, 0.85], [0.6, 0.6, 0.6]] {
            let (h, s, v) = to_hsv(c);
            let back = from_hsv(h, s, v);
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-4, "{c:?} {back:?}");
            }
        }
    }

    #[test]
    fn noise_and_dither_stay_in_range() {
        for i in 0..400 {
            let (x, y) = (i as f32 * 0.37 - 20.0, i as f32 * 0.11 - 5.0);
            let n = perlin(x, y);
            assert!((0.0..=1.0).contains(&n), "{n}");
            let d = dither(i * 7, i * 3);
            assert!((-5.0..5.0).contains(&d), "{d}");
        }
    }

    #[test]
    fn the_colours_move_on_every_half_period_and_speed_up_as_they_do() {
        let mut clock = GlowClock::new(&LIGHT, 0);
        assert_eq!(clock.colors(), LIGHT.colors[1], "it starts on the second set");
        // Half a period of field time passes at about a unit a second.
        clock.advance(2400);
        assert_eq!(clock.changes, 0);
        clock.advance(2600);
        assert_eq!(clock.changes, 1);
        clock.advance(2900);
        assert!(clock.speed.x > 1.1, "the field hurries as it changes: {}", clock.speed.x);
        clock.advance(3600);
        assert!(clock.blend.x > 0.5, "well on the way to the first set: {}", clock.blend.x);
        let mut lowest: f32 = 1.0;
        let mut highest: f32 = 0.0;
        for ms in (3600..63_600).step_by(16) {
            clock.advance(ms);
            for c in clock.colors() {
                lowest = lowest.min(c[..3].iter().cloned().fold(1.0, f32::min));
                highest = highest.max(c[..3].iter().cloned().fold(0.0, f32::max));
            }
        }
        assert!(clock.changes > 20, "{}", clock.changes);
        assert!(lowest > 0.5 && highest < 1.01, "the colours stay among the sets: {lowest} {highest}");
    }

    #[test]
    fn the_field_lies_over_the_page_colour_and_fades_with_scrolling() {
        let clock = GlowClock::new(&LIGHT, 0);
        let bg = Rgba::rgb(243, 243, 243);
        let full = sample(&clock, 1440, 3200, 16, 0.54, bg, 1.0);
        let gone = sample(&clock, 1440, 3200, 16, 0.54, bg, 0.0);
        assert!(gone.rgb.iter().all(|c| c.iter().all(|&v| (v - 243.0).abs() < 1e-3)));
        let top = full.rgb[full.cols * 10 + full.cols / 2];
        assert!(top.iter().any(|&v| (v - 243.0).abs() > 10.0), "colour at the top: {top:?}");
    }
}
