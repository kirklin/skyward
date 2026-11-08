//! Logical display directions, independent of the panel's native scanout.
use crate::app::Touch;
use crate::Canvas;
use serde::{Deserialize, Serialize};

/// Clockwise turns of the UI relative to the native display.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    #[default]
    Native,
    Right,
    Inverted,
    Left,
}

impl Rotation {
    fn turns(self) -> i32 {
        match self { Self::Native => 0, Self::Right => 1, Self::Inverted => 2, Self::Left => 3 }
    }

    fn angle_to(self, next: Self) -> f32 {
        let turns = (next.turns() - self.turns() + 2).rem_euclid(4) - 2;
        turns as f32 * std::f32::consts::FRAC_PI_2
    }
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        match self {
            Self::Native | Self::Inverted => (width, height),
            Self::Right | Self::Left => (height, width),
        }
    }

    /// Native touch coordinates mapped into the logical canvas.
    pub fn touch(self, touch: Touch, width: u32, height: u32) -> Touch {
        let point = |x, y| match self {
            Self::Native => (x, y),
            Self::Right => (y, width.saturating_sub(1) as f32 - x),
            Self::Inverted => (width.saturating_sub(1) as f32 - x, height.saturating_sub(1) as f32 - y),
            Self::Left => (height.saturating_sub(1) as f32 - y, x),
        };
        match touch {
            Touch::Down(x, y) => { let (x, y) = point(x, y); Touch::Down(x, y) }
            Touch::Move(x, y) => { let (x, y) = point(x, y); Touch::Move(x, y) }
            Touch::Up => Touch::Up,
        }
    }

    /// Present a complete logical canvas on a native-sized target. Small tiles
    /// keep both source and destination close in cache during quarter turns.
    pub fn paint(self, source: &Canvas, native: &mut Canvas) {
        assert_eq!((source.w, source.h), self.size(native.w, native.h));
        assert_eq!(source.data.len(), (source.w * source.h * 4) as usize);
        assert_eq!(native.data.len(), source.data.len());
        let src = source.data.as_chunks::<4>().0;
        let dst = native.data.as_chunks_mut::<4>().0;
        let (nw, nh) = (native.w as usize, native.h as usize);
        match self {
            Self::Native => dst.copy_from_slice(src),
            Self::Inverted => {
                for (out, pixel) in dst.iter_mut().zip(src.iter().rev()) { *out = *pixel; }
            }
            Self::Right | Self::Left => {
                for top in (0..nh).step_by(32) {
                    for left in (0..nw).step_by(32) {
                        for y in top..(top + 32).min(nh) {
                            for x in left..(left + 32).min(nw) {
                                let from = match self {
                                    Self::Right => (nw - 1 - x) * nh + y,
                                    Self::Left => x * nh + nh - 1 - y,
                                    _ => unreachable!(),
                                };
                                dst[y * nw + x] = src[from];
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Require a clear, stable gravity direction; laying a tablet flat keeps its
/// previous orientation. Axes are right/up relative to the native display.
#[derive(Default)]
pub struct Gravity {
    filtered: Option<[f32; 3]>,
    candidate: Option<(Rotation, u64)>,
    at: Option<u64>,
}

impl Gravity {
    pub fn sample(&mut self, sample: [f32; 3], now: u64) -> Option<Rotation> {
        let [x, y, z] = sample;
        let magnitude = x*x + y*y + z*z;
        if !sample.iter().all(|v| v.is_finite()) || !(49.0..=169.0).contains(&magnitude) {
            *self = Self::default();
            return None;
        }
        let elapsed = self.at.map_or(1000, |at| now.saturating_sub(at));
        self.at = Some(now);
        if elapsed > 1000 { self.candidate = None; self.filtered = None; }
        let mix = 1.0 - (-(elapsed as f32) / 120.0).exp();
        let filtered = self.filtered.get_or_insert(sample);
        for (out, value) in filtered.iter_mut().zip(sample) { *out += mix * (value - *out); }
        let [x, y, z] = *filtered;
        let direction = if z.abs() > 8.5 || x.abs().max(y.abs()) < 5.5 {
            None
        } else if y.abs() > x.abs() * 1.35 {
            Some(if y > 0.0 { Rotation::Native } else { Rotation::Inverted })
        } else if x.abs() > y.abs() * 1.35 {
            Some(if x > 0.0 { Rotation::Right } else { Rotation::Left })
        } else { None };
        let Some(direction) = direction else { self.candidate = None; return None };
        match self.candidate {
            Some((prior, since)) if prior == direction => (now.saturating_sub(since) >= 400).then_some(direction),
            _ => { self.candidate = Some((direction, now)); None }
        }
    }
}

/// A rotating crossfade of the old and reflowed pages, in native panel space.
/// Motion snapshots are bounded to 1024 pixels on their long edge. The final
/// frame is the original full-resolution canvas, without an extra copy at rest.
pub struct Turn {
    original: Canvas,
    before: Canvas,
    after: Canvas,
    frame: Canvas,
    angle: f32,
    start: u64,
    columns: Vec<(usize, u32)>,
    rows: Vec<(usize, u32)>,
    upper: Vec<[u8; 4]>,
    lower: Vec<[u8; 4]>,
}

impl Turn {
    pub const MS: u64 = 420;

    /// Cubic-bezier (0.4, 0, 0.2, 1): a short acceleration and a long,
    /// gentle stop. Solve the time coordinate before reading the angle.
    fn progress(t: f32) -> f32 {
        if t <= 0.0 || t >= 1.0 { return t.clamp(0.0, 1.0); }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..12 {
            let s = (low + high) * 0.5;
            let x = 3.0*(1.0-s)*(1.0-s)*s*0.4 + 3.0*(1.0-s)*s*s*0.2 + s*s*s;
            if x < t { low = s; } else { high = s; }
        }
        let s = (low + high) * 0.5;
        s*s*(3.0-2.0*s)
    }

    pub fn new(before: &Canvas, after: &Canvas, from: Rotation, to: Rotation, now: u64) -> Self {
        assert_eq!((before.w, before.h), (after.w, after.h));
        let scale = before.w.max(before.h).div_ceil(1024).max(1);
        let small = |source: &Canvas| {
            let (w, h) = (source.w.div_ceil(scale), source.h.div_ceil(scale));
            let mut out = Canvas::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x)*4) as usize;
                    let j = (((y*scale).min(source.h-1)*source.w + (x*scale).min(source.w-1))*4) as usize;
                    out.data[i..i+4].copy_from_slice(&source.data[j..j+4]);
                }
            }
            out
        };
        let original = before.clone();
        let samples = |size: u32, small: u32| (0..size).map(|p| {
            let at = ((p as f32 + 0.5)/scale as f32 - 0.5).clamp(0.0, (small-1) as f32);
            (at.floor() as usize, ((at-at.floor())*256.0).round() as u32)
        }).collect();
        let before = small(before);
        let after = small(after);
        let frame = Canvas::new(before.w, before.h);
        let columns = samples(original.w, frame.w);
        let rows = samples(original.h, frame.h);
        let upper = vec![[0;4]; original.w as usize];
        let lower = upper.clone();
        Self { original, before, after, frame, angle: from.angle_to(to), start: now, columns, rows, upper, lower }
    }

    pub fn done(&self, now: u64) -> bool { now.saturating_sub(self.start) >= Self::MS }

    pub fn paint(&mut self, out: &mut Canvas, background: crate::Rgba, now: u64) {
        let t = (now.saturating_sub(self.start) as f32 / Self::MS as f32).clamp(0.0, 1.0);
        if t == 0.0 {
            out.data.copy_from_slice(&self.original.data);
            return;
        }
        let q = Self::progress(t);
        self.frame.clear(background);
        // Both snapshots' text turns through the same angle while their layout
        // dissolves from one set of dimensions to the other.
        Self::layer(&self.before, &mut self.frame, self.angle*q, 1.0);
        Self::layer(&self.after, &mut self.frame, -self.angle*(1.0-q), q);
        self.expand(out);
    }

    /// Separable bilinear sampling: expand each small row once, then interpolate
    /// between two cached rows. Motion never exposes nearest-neighbour blocks.
    fn expand(&mut self, out: &mut Canvas) {
        let width = self.frame.w as usize;
        let source = self.frame.data.as_chunks::<4>().0;
        let expand_row = |y: usize, row: &mut [[u8;4]]| {
            let source = &source[y*width..(y+1)*width];
            for (pixel, &(x, weight)) in row.iter_mut().zip(&self.columns) {
                *pixel = Self::mix(source[x], source[(x+1).min(width-1)], weight);
            }
        };
        let mut top: Option<usize> = None;
        for (y, &(index, weight)) in self.rows.iter().enumerate() {
            if top != Some(index) {
                if top.and_then(|v| v.checked_add(1)) == Some(index) {
                    std::mem::swap(&mut self.upper, &mut self.lower);
                } else {
                    expand_row(index, &mut self.upper);
                }
                expand_row((index+1).min(self.frame.h as usize-1), &mut self.lower);
                top = Some(index);
            }
            let stride = out.w as usize*4;
            let row = &mut out.data[y*stride..(y+1)*stride];
            let weight = weight as u16;
            for ((pixel, &a), &b) in row.iter_mut().zip(self.upper.as_flattened()).zip(self.lower.as_flattened()) {
                *pixel = ((a as u16*(256-weight) + b as u16*weight + 128) >> 8) as u8;
            }
        }
    }

    #[inline]
    fn mix(a: [u8;4], b: [u8;4], weight: u32) -> [u8;4] {
        // Two independent 16-bit lanes per word, with rounding before shifting.
        let (a, b) = (u32::from_le_bytes(a), u32::from_le_bytes(b));
        let lanes = |a, b| (a*(256-weight) + b*weight + 0x0080_0080) & 0xff00_ff00;
        let rb = lanes(a & 0x00ff_00ff, b & 0x00ff_00ff) >> 8;
        let ga = lanes((a >> 8) & 0x00ff_00ff, (b >> 8) & 0x00ff_00ff);
        (rb | ga).to_le_bytes()
    }

    fn layer(src: &Canvas, out: &mut Canvas, angle: f32, alpha: f32) {
        if alpha <= 0.0 { return; }
        let (sin, cos) = angle.sin_cos();
        let (w, h) = (out.w as f32, out.h as f32);
        // Keep the screen at its original size throughout the turn; corners
        // rotate beyond the viewport and are clipped instead of zooming out.
        let (a, b) = (cos, sin);
        let transform = tiny_skia::Transform::from_row(a, b, -b, a,
            (w-a*src.w as f32+b*src.h as f32)/2.0,
            (h-b*src.w as f32-a*src.h as f32)/2.0);
        let paint = tiny_skia::PixmapPaint { opacity: alpha, quality: tiny_skia::FilterQuality::Nearest, ..Default::default() };
        // Composed screen snapshots are opaque, so straight and premultiplied
        // RGBA coincide and the existing rasterizer can read them directly.
        let source = tiny_skia::PixmapRef::from_bytes(&src.data, src.w, src.h).unwrap();
        let mut target = tiny_skia::PixmapMut::from_bytes(&mut out.data, out.w, out.h).unwrap();
        target.draw_pixmap(0, 0, source, &paint, transform, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_ignores_flat_shaking_and_diagonal_positions_and_waits_for_stability() {
        let stable = |sample| {
            let mut gravity = Gravity::default();
            assert_eq!(gravity.sample(sample, 0), None);
            assert_eq!(gravity.sample(sample, 399), None);
            gravity.sample(sample, 400)
        };
        for sample in [[0.,0.,9.8], [7.,7.,0.], [20.,0.,0.], [f32::NAN,0.,9.8]] {
            assert_eq!(stable(sample), None);
        }
        for (sample, rotation) in [([0.,9.8,0.], Rotation::Native), ([9.8,0.,0.],Rotation::Right),
            ([0.,-9.8,0.],Rotation::Inverted), ([-9.8,0.,0.],Rotation::Left)] {
            assert_eq!(stable(sample), Some(rotation));
        }
        let mut gravity = Gravity::default();
        gravity.sample([9.8,0.,0.], 0);
        assert_eq!(gravity.sample([9.8,0.,0.], 2000), None, "a stale reading cannot complete a turn");
    }

    #[test]
    fn rotation_animation_uses_the_shortest_path_in_both_directions() {
        assert_eq!(Rotation::Native.angle_to(Rotation::Right), std::f32::consts::FRAC_PI_2);
        assert_eq!(Rotation::Right.angle_to(Rotation::Native), -std::f32::consts::FRAC_PI_2);
        assert_eq!(Rotation::Left.angle_to(Rotation::Native), std::f32::consts::FRAC_PI_2);
        let before = Canvas::filled(8, 6, crate::Rgba::rgb(255,0,0));
        let after = Canvas::filled(8, 6, crate::Rgba::rgb(0,0,255));
        let mut turn = Turn::new(&before, &after, Rotation::Native, Rotation::Right, 10);
        let mut out = Canvas::new(8,6);
        turn.paint(&mut out, crate::Rgba::rgb(0,0,0), 10);
        assert_eq!(out.data, before.data);
        turn.paint(&mut out, crate::Rgba::rgb(0,0,0), 10+Turn::MS);
        assert_eq!(out.data, after.data);
        assert!(!turn.done(10+Turn::MS-1));
        assert!(turn.done(10+Turn::MS));
    }

    #[test]
    fn motion_uses_smooth_samples_and_clips_without_shrinking() {
        assert_eq!(Turn::mix([0,128,255,255], [255,0,0,128], 128), [128,64,128,192]);
        let before = Canvas::filled(1201, 901, crate::Rgba::rgb(0,0,0));
        let mut turn = Turn::new(&before, &before, Rotation::Native, Rotation::Right, 0);
        for (i, pixel) in turn.frame.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *pixel = if (i%turn.frame.w as usize).is_multiple_of(2) { [0,0,0,255] } else { [255,255,255,255] };
        }
        let mut out = Canvas::new(1201,901);
        turn.expand(&mut out);
        assert_eq!(&out.data[4..12], &[64,64,64,255,191,191,191,255]);
        let red = Canvas::filled(80,60, crate::Rgba::rgb(255,0,0));
        let mut out = Canvas::filled(80,60, crate::Rgba::rgb(0,0,0));
        Turn::layer(&red, &mut out, std::f32::consts::FRAC_PI_4, 1.0);
        let pixel = ((30*80+1)*4) as usize;
        assert_eq!(&out.data[pixel..pixel+4], &[255,0,0,255]);
    }

    #[test]
    fn rotation_easing_accelerates_then_settles_without_overshoot() {
        assert_eq!(Turn::progress(0.0), 0.0);
        assert_eq!(Turn::progress(1.0), 1.0);
        assert!(Turn::progress(0.5) > 0.7);
        let mut previous = 0.0;
        for step in 0..=100 {
            let next = Turn::progress(step as f32/100.0);
            assert!((previous..=1.0).contains(&next));
            previous = next;
        }
        assert!(Turn::progress(0.1) < 0.05);
        assert!(Turn::progress(0.9) > 0.98);
    }

    #[test]
    fn bounded_animation_covers_the_last_row_and_column_of_odd_sized_panels() {
        let before = Canvas::filled(1201, 901, crate::Rgba::rgb(255,0,0));
        let after = Canvas::filled(1201, 901, crate::Rgba::rgb(0,0,255));
        let mut turn = Turn::new(&before, &after, Rotation::Native, Rotation::Left, 0);
        assert!(turn.frame.w.max(turn.frame.h) <= 1024);
        let mut out = Canvas::new(1201,901);
        turn.paint(&mut out, crate::Rgba::rgb(0,0,0), 0);
        assert_eq!(out.data, before.data);
        turn.paint(&mut out, crate::Rgba::rgb(0,0,0), Turn::MS);
        assert_eq!(out.data, after.data);
    }

    #[test]
    fn pixels_and_touches_agree_in_every_direction() {
        for rotation in [Rotation::Native, Rotation::Right, Rotation::Inverted, Rotation::Left] {
            let (w, h) = rotation.size(3, 2);
            let mut logical = Canvas::new(w, h);
            for (i, pixel) in logical.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *pixel = [i as u8, 0, 0, 255];
            }
            let mut native = Canvas::new(3, 2);
            rotation.paint(&logical, &mut native);
            for y in 0..2 {
                for x in 0..3 {
                    let Touch::Down(lx, ly) = rotation.touch(Touch::Down(x as f32, y as f32), 3, 2) else { panic!() };
                    assert!(lx >= 0.0 && lx < w as f32 && ly >= 0.0 && ly < h as f32);
                    assert_eq!(native.data[((y * 3 + x) * 4) as usize], (ly as u32 * w + lx as u32) as u8);
                }
            }
            assert!(matches!(rotation.touch(Touch::Up, 3, 2), Touch::Up));
        }
    }

    #[test]
    fn quarter_turns_have_the_expected_visual_direction() {
        let mut source = Canvas::new(2, 3);
        for (i, p) in source.data.as_chunks_mut::<4>().0.iter_mut().enumerate() { p[0] = i as u8; }
        let mut native = Canvas::new(3, 2);
        Rotation::Right.paint(&source, &mut native);
        assert_eq!(native.data.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<_>>(), [4, 2, 0, 5, 3, 1]);
        Rotation::Left.paint(&source, &mut native);
        assert_eq!(native.data.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<_>>(), [1, 3, 5, 0, 2, 4]);
    }
}
