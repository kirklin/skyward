//! Bundled portrait wallpapers, prepared with two Gaussian blur radii.

use crate::{Canvas, Rgba};
use crate::geom::Rect;
use crate::theme::Theme;
use std::cell::RefCell;

const IMAGES: &[(&[u8], &[u8])] = &[
    (include_bytes!("../assets/wallpapers/00-soft.png"), include_bytes!("../assets/wallpapers/00-strong.png")),
    (include_bytes!("../assets/wallpapers/01-soft.png"), include_bytes!("../assets/wallpapers/01-strong.png")),
    (include_bytes!("../assets/wallpapers/02-soft.png"), include_bytes!("../assets/wallpapers/02-strong.png")),
    (include_bytes!("../assets/wallpapers/03-soft.png"), include_bytes!("../assets/wallpapers/03-strong.png")),
    (include_bytes!("../assets/wallpapers/04-soft.png"), include_bytes!("../assets/wallpapers/04-strong.png")),
    (include_bytes!("../assets/wallpapers/05-soft.png"), include_bytes!("../assets/wallpapers/05-strong.png")),
    (include_bytes!("../assets/wallpapers/06-soft.png"), include_bytes!("../assets/wallpapers/06-strong.png")),
    (include_bytes!("../assets/wallpapers/07-soft.png"), include_bytes!("../assets/wallpapers/07-strong.png")),
    (include_bytes!("../assets/wallpapers/08-soft.png"), include_bytes!("../assets/wallpapers/08-strong.png")),
    (include_bytes!("../assets/wallpapers/09-soft.png"), include_bytes!("../assets/wallpapers/09-strong.png")),
    (include_bytes!("../assets/wallpapers/10-soft.png"), include_bytes!("../assets/wallpapers/10-strong.png")),
    (include_bytes!("../assets/wallpapers/11-soft.png"), include_bytes!("../assets/wallpapers/11-strong.png")),
    (include_bytes!("../assets/wallpapers/12-soft.png"), include_bytes!("../assets/wallpapers/12-strong.png")),
    (include_bytes!("../assets/wallpapers/13-soft.png"), include_bytes!("../assets/wallpapers/13-strong.png")),
    (include_bytes!("../assets/wallpapers/14-soft.png"), include_bytes!("../assets/wallpapers/14-strong.png")),
    (include_bytes!("../assets/wallpapers/15-soft.png"), include_bytes!("../assets/wallpapers/15-strong.png")),
    (include_bytes!("../assets/wallpapers/16-soft.png"), include_bytes!("../assets/wallpapers/16-strong.png")),
    (include_bytes!("../assets/wallpapers/17-soft.png"), include_bytes!("../assets/wallpapers/17-strong.png")),
];

pub const COUNT: usize = IMAGES.len();

struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Image {
    fn load(index: usize, strong: bool) -> Option<Self> {
        let &(soft, heavy) = IMAGES.get(index)?;
        Self::decode(if strong { heavy } else { soft })
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut reader = png::Decoder::new(bytes).read_info().ok()?;
        let mut rgb = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut rgb).ok()?;
        if frame.color_type != png::ColorType::Rgb || frame.bit_depth != png::BitDepth::Eight {
            return None;
        }
        Some(Self { width: frame.width as usize, height: frame.height as usize, rgb })
    }

    fn sampler(&self, w: u32, h: u32) -> impl Fn(u32, u32) -> Rgba + '_ {
        let (w, h) = (w as usize, h as usize);
        let (sw, sh) = if w * self.height > h * self.width {
            (self.width, (h * self.width / w).max(1))
        } else {
            ((w * self.height / h).max(1), self.height)
        };
        let (left, top) = ((self.width - sw) / 2, (self.height - sh) / 2);
        move |x, y| {
            let u = (x as usize).min(w - 1) * (sw - 1) * 256 / w.saturating_sub(1).max(1);
            let v = (y as usize).min(h - 1) * (sh - 1) * 256 / h.saturating_sub(1).max(1);
            let x0 = left + u / 256;
            let y0 = top + v / 256;
            let x1 = (x0 + 1).min(left + sw - 1);
            let y1 = (y0 + 1).min(top + sh - 1);
            let mix = |a, b, w| (a * (256 - w) + b * w + 128) / 256;
            let channel = |k| {
                let at = |x, y| usize::from(self.rgb[(y * self.width + x) * 3 + k]);
                mix(mix(at(x0, y0), at(x1, y0), u % 256), mix(at(x0, y1), at(x1, y1), u % 256), v % 256) as u8
            };
            Rgba::rgb(channel(0), channel(1), channel(2))
        }
    }
}

thread_local! {
    // Keep both blur levels decoded when a page uses them together.
    static CURRENT: RefCell<[Option<(usize, Image)>; 2]> = const { RefCell::new([None, None]) };
}

fn with_image(index: usize, strong: bool, paint: impl FnOnce(&Image)) {
    CURRENT.with_borrow_mut(|images| {
        let cached = &mut images[usize::from(strong)];
        if cached.as_ref().is_none_or(|(current, _)| *current != index) {
            *cached = Image::load(index, strong).map(|image| (index, image));
        }
        if let Some((_, image)) = cached {
            paint(image);
        }
    });
}

fn limit(color: Rgba, ceiling: u8) -> Rgba {
    let peak = color.r.max(color.g).max(color.b);
    let knee = ceiling * 2 / 3;
    if peak <= knee {
        return color;
    }
    let compressed = u16::from(knee) + u16::from(peak - knee) * u16::from(ceiling - knee) / u16::from(255 - knee);
    let scale = |channel| (u16::from(channel) * compressed / u16::from(peak)) as u8;
    Rgba::rgb(scale(color.r), scale(color.g), scale(color.b))
}

fn exposure(level: u8, strong: bool) -> u16 {
    (50 + u16::from(level) * 3 / 4) * if strong { 78 } else { 100 } / 100
}

fn adjusted(pixel: Rgba, gain: u16) -> Rgba {
    let channel = |v| (u16::from(v) * gain / 100).min(255) as u8;
    Rgba::rgb(channel(pixel.r), channel(pixel.g), channel(pixel.b))
}

pub(crate) fn preview(canvas: &mut Canvas, index: usize, strong: bool, level: u8) {
    with_image(index, strong, |image| {
        let sample = image.sampler(canvas.w, canvas.h);
        for y in canvas.held() {
            canvas.paint_row(y, |x| adjusted(sample(x as u32, y), exposure(level, strong)));
        }
    });
}

pub(crate) fn thumbnail(canvas: &mut Canvas, index: usize, rect: Rect, radius: f32) {
    static ATLAS: std::sync::OnceLock<Image> = std::sync::OnceLock::new();
    let atlas = ATLAS.get_or_init(|| {
        Image::decode(include_bytes!("../assets/wallpapers/previews.png")).expect("embedded wallpaper previews")
    });
    let width = atlas.width / 3;
    let height = atlas.height / COUNT.div_ceil(3);
    let left = (index % 3) * width;
    let top = (index / 3) * height;
    canvas.fill_round_rect_sampled(rect, radius, |x, y| {
        let sx = (((x as f32 - rect.x) / rect.w) * width as f32) as usize;
        let sy = (((y as f32 - rect.y) / rect.h) * height as f32) as usize;
        let i = ((top + sy.min(height - 1)) * atlas.width + left + sx.min(width - 1)) * 3;
        Rgba::rgb(atlas.rgb[i], atlas.rgb[i + 1], atlas.rgb[i + 2])
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BackdropKey {
    index: usize,
    strong: bool,
    dark: bool,
    level: u8,
    width: u32,
    height: u32,
}

thread_local! {
    static BACKDROP: RefCell<Option<(BackdropKey, Canvas)>> = const { RefCell::new(None) };
}

pub(crate) fn compose(out: &mut Canvas, t: &Theme, strong: bool, panels: &[(Rect, f32)], offset: i32) {
    let Some(index) = t.wallpaper else { return };
    let key = BackdropKey { index, strong, dark: t.dark, level: t.wallpaper_level, width: out.w, height: out.h };
    BACKDROP.with_borrow_mut(|cached| {
        if cached.as_ref().is_none_or(|(current, _)| *current != key) {
            let mut background = Canvas::filled(key.width, key.height, t.pal.bg);
            with_image(index, strong, |image| {
                let sample = image.sampler(key.width, key.height);
                let ceiling = if t.dark { 84 } else { 117 };
                for y in background.held() {
                    background.paint_row(y, |x| limit(adjusted(sample(x as u32, y), exposure(key.level, strong)), ceiling));
                }
            });
            *cached = Some((key, background));
        }
        if let Some((_, background)) = cached {
            out.put(background, 0);
        }
    });
    with_image(index, true, |image| {
        let sample = image.sampler(key.width, key.height);
        for &(mut rect, radius) in panels {
            rect.y -= offset as f32;
            out.fill_round_rect_sampled(rect, radius, |x, y| card_material(sample(x, y), t));
        }
    });
}

pub(crate) fn panel(canvas: &mut Canvas, t: &Theme, rect: Rect, radius: f32, tint: Rgba) {
    let Some(index) = t.wallpaper else {
        canvas.fill_round_rect(rect.x, rect.y, rect.w, rect.h, radius, tint);
        return;
    };
    if let Some(panels) = &mut canvas.wallpaper_panels {
        panels.push((rect, radius));
        if t.wallpaper_held {
            canvas.fill_round_rect(rect.x, rect.y, rect.w, rect.h, radius, t.pal.row_press);
        }
        return;
    }
    with_image(index, true, |image| {
        let sample = image.sampler(t.m.w, t.m.h);
        canvas.fill_round_rect_sampled(rect, radius, |x, y| card_material(sample(x, y), t).with_alpha(tint.a));
    });
}

fn card_material(color: Rgba, t: &Theme) -> Rgba {
    let gain = exposure(t.wallpaper_level, t.wallpaper_heavy).clamp(88, 108);
    let paper = if t.dark { limit(color, 115) } else { color };
    let paper = adjusted(paper, gain);
    crate::color::mix(t.pal.card, paper, if t.dark { 0.46 } else { 0.31 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_wallpapers_decode_in_both_blur_levels() {
        assert_eq!(COUNT, 18);
        for index in 0..COUNT {
            for strong in [false, true] {
                let image = Image::load(index, strong).unwrap();
                assert_eq!((image.width, image.height, image.rgb.len()), (450, 990, 450 * 990 * 3));
            }
        }
    }

    #[test]
    fn bright_wallpapers_keep_their_hue_under_the_readability_limit() {
        assert_eq!(limit(Rgba::rgb(30, 48, 65), 117), Rgba::rgb(30, 48, 65));
        let bright = limit(Rgba::rgb(240, 120, 0), 117);
        assert!(bright.r < 117 && bright.r > 100);
        assert_eq!((bright.g, bright.b), (bright.r / 2, 0));
    }

    #[test]
    fn dark_cards_lift_shadows_and_keep_wallpaper_detail() {
        let theme = Theme::dark(120, 240, 1.0).over_wallpaper(0);
        let shadow = card_material(Rgba::rgb(0, 0, 0), &theme);
        let detail = card_material(Rgba::rgb(100, 90, 80), &theme);
        assert!(shadow.r >= theme.pal.bg.r + 16 && shadow.g >= theme.pal.bg.g + 16);
        assert!(detail.r > shadow.r + 20);
    }

    #[test]
    fn wallpaper_panels_skip_a_band_outside_the_card() {
        let theme = Theme::light(120, 240, 1.0).over_wallpaper(0);
        let base = theme.pal.bg;
        let mut band = Canvas::band(120, 240, 20..100, 0..10, base);
        let before = band.data.clone();
        panel(&mut band, &theme, Rect::new(15.0, 25.0, 90.0, 60.0), 6.0, theme.pal.card);
        assert_eq!(band.data, before);
    }

    #[test]
    fn scrolling_moves_the_blur_mask_but_keeps_wallpaper_at_the_same_pixel() {
        let theme = Theme::light(100, 200, 1.0).over_wallpaper(0);
        let mut before = Canvas::new(100, 200);
        let mut after = Canvas::new(100, 200);
        let panels = [(Rect::new(10.0, 80.0, 80.0, 50.0), 8.0)];
        compose(&mut before, &theme, false, &panels, 0);
        compose(&mut after, &theme, false, &panels, 60);
        let pixel = |canvas: &Canvas, x: usize, y: usize| {
            let at = (y * canvas.w as usize + x) * 4;
            <[u8; 4]>::try_from(&canvas.data[at..at + 4]).unwrap()
        };
        assert_eq!(pixel(&before, 1, 70), pixel(&after, 1, 70));
        assert_eq!(pixel(&before, 50, 160), pixel(&after, 50, 160));
        assert_ne!(pixel(&before, 50, 100), pixel(&after, 50, 100));
    }
}
