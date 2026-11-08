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
