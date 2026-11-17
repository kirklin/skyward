use crate::color::{over, Rgba};
use crate::glow::{self, Glow};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Rgba,
    pub card: Rgba,
    pub text: Rgba,
    pub subtitle: Rgba,
    pub value: Rgba,
    pub section: Rgba,
    pub chevron: Rgba,
    pub accent: Rgba,
    pub track: Rgba,
    pub switch_off: Rgba,
    pub search: Rgba,
    pub search_icon: Rgba,
    pub placeholder: Rgba,
    pub badge: Rgba,
    pub avatar: Rgba,
    pub avatar_glyph: Rgba,
    pub status: Rgba,
    pub battery: BatteryInk,
    pub mono_icon: Rgba,
    pub danger: Rgba,
    pub danger_soft: Rgba,
    /// Titles and values on the parameters page and captions inside cards, a
    /// step softer than `text`.
    pub info: Rgba,
    /// Line icons on the parameters page.
    pub outline: Rgba,
    /// Captions under the About page's spec values.
    pub spec_label: Rgba,
    /// Laid over the card behind a row a finger holds down, under the
    /// row's words and icon.
    pub row_press: Rgba,
    /// Laid over the `<`, the search page's round button and the home
    /// page's search bar while a finger holds them down.
    pub button_press: Rgba,
    /// The round button at the end of a WLAN network's row, and the arrow
    /// in it.
    pub round: Rgba,
    pub round_mark: Rgba,
    /// The arrows up and down after the value of a row that opens a list of
    /// choices.
    pub updown: Rgba,
    /// A list of choices open over the page, and the shadow it casts where
    /// the shadow is whole.
    pub popup: Rgba,
    pub shadow: Rgba,
}

/// The status bar battery's inks, partly see-through as the stock status
/// bar's are, so they sit on whatever page is under them.
#[derive(Clone, Copy)]
pub struct BatteryInk {
    /// What the level fills while the cable is in, charging or full.
    pub charging: Rgba,
    /// What a low level fills with the cable out.
    pub low: Rgba,
    /// What any other level fills; the digits are cut out of it.
    pub level: Rgba,
    /// The empty part, cap included.
    pub empty: Rgba,
    pub bolt: Rgba,
}

/// The About page in a theme: the moving field behind it, the version
/// line under the wordmark, and the cards that scroll over the field.
#[derive(Clone, Copy)]
pub struct Hero {
    pub glow: &'static Glow,
    /// The version line: black or white, partly see-through.
    pub version: Rgba,
    pub card: Rgba,
    pub title: Rgba,
    pub value: Rgba,
    pub chevron: Rgba,
}

/// Layout metrics measured on a Xiaomi 17 Ultra's stock settings, in dp,
/// converted at the panel's density.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub w: u32,
    pub h: u32,
    pub px_per_dp: f32,
    pub margin: f32,
    pub card_radius: f32,
    pub card_gap: f32,
    pub section_block: f32,
    pub section_label_top: f32,
    pub row_h: f32,
    pub row_pad_v: f32,
    pub inner: f32,
    pub icon: f32,
    pub icon_gap: f32,
    pub title_px: f32,
    pub title_x: f32,
    pub title_top: f32,
    /// Back chevron: tip x, centre y, width, height and stroke.
    pub back_x: f32,
    pub back_cy: f32,
    pub back_w: f32,
    pub back_h: f32,
    pub back_stroke: f32,
    pub stroke: f32,
    pub content_top: f32,
    pub search_h: f32,
    pub search_text_inset: f32,
    pub after_search: f32,
    pub row_title_px: f32,
    pub sub_px: f32,
    pub sub_gap: f32,
    pub sub_pitch: f32,
    pub label_px: f32,
    pub account_title_px: f32,
    pub account_row_h: f32,
    pub avatar: f32,
    pub avatar_inset: f32,
    pub badge_px: f32,
    pub badge_w: f32,
    pub badge_h: f32,
    pub chevron_w: f32,
    pub chevron_h: f32,
    pub chevron_inset: f32,
    pub chevron_stroke: f32,
    pub value_gap: f32,
    pub switch_w: f32,
    pub switch_h: f32,
    pub knob: f32,
    pub slider_h: f32,
    pub slider_knob: f32,
    pub status_x: f32,
    /// Where the battery ends, from the right edge; with the cable in, the
    /// bolt's box ends at `status_bolt_right` instead and the battery moves
    /// left.
    pub status_right: f32,
    pub status_bolt_right: f32,
    pub status_cy: f32,
    pub status_px: f32,
}

impl Metrics {
    pub fn dp(&self, v: f32) -> f32 {
        v * self.px_per_dp
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub pal: Palette,
    pub hero: Hero,
    pub m: Metrics,
    pub dark: bool,
    pub(crate) wallpaper: Option<usize>,
    pub(crate) wallpaper_held: bool,
    pub(crate) wallpaper_level: u8,
    pub(crate) wallpaper_heavy: bool,
}

impl Theme {
    /// `px_per_dp` is the panel density over 160, e.g. 3.0 at 480 dpi.
    pub fn light(w: u32, h: u32, px_per_dp: f32) -> Self {
        Theme { pal: LIGHT, hero: LIGHT_HERO, m: metrics(w, h, px_per_dp), dark: false, wallpaper: None, wallpaper_held: false, wallpaper_level: 67, wallpaper_heavy: false }
    }

    pub fn dark(w: u32, h: u32, px_per_dp: f32) -> Self {
        Theme { pal: DARK, hero: DARK_HERO, m: metrics(w, h, px_per_dp), dark: true, wallpaper: None, wallpaper_held: false, wallpaper_level: 67, wallpaper_heavy: false }
    }

    pub(crate) fn over_wallpaper(mut self, image: usize) -> Self {
        self.wallpaper = Some(image);
        self.pal.bg = Rgba::rgb(19, 24, 29);
        self.pal.section = Rgba::rgb(224, 229, 234);
        self.pal.status = self.pal.section;
        self.pal.battery = DARK.battery;
        self.pal.card = if self.dark { Rgba::rgb(67, 75, 85) } else { Rgba::rgb(220, 226, 232) };
        self.pal.search = self.pal.card;
        self.pal.value = if self.dark { Rgba::rgb(196, 205, 214) } else { Rgba::rgb(29, 42, 53) };
        self.pal.chevron = if self.dark { Rgba::rgb(161, 174, 185) } else { Rgba::rgb(85, 97, 109) };
        self.pal.subtitle = self.pal.value;
        self.pal.search_icon = self.pal.value;
        self.pal.placeholder = self.pal.value;
        self
    }

    pub(crate) fn on_white_bar(mut self) -> Self {
        self.pal.status = LIGHT.status;
        self.pal.battery = LIGHT.battery;
        self
    }

    pub(crate) fn outside_ink(&self) -> Rgba {
        if self.wallpaper.is_some() { Rgba::rgb(246, 249, 252) } else { self.pal.text }
    }

    pub(crate) fn outside_note(&self) -> Rgba {
        if self.wallpaper.is_some() { self.pal.section } else { self.pal.subtitle }
    }

    pub(crate) fn outside_accent(&self) -> Rgba {
        if self.wallpaper.is_some() { Rgba::rgb(181, 211, 255) } else { self.pal.accent }
    }

    /// This theme with every card as it shows behind a row held down.
    pub fn pressed(&self) -> Theme {
        let mut t = *self;
        t.wallpaper_held = self.wallpaper.is_some();
        t.pal.card = over(self.pal.row_press, self.pal.card);
        t.hero.card = over(self.pal.row_press, self.hero.card);
        t
    }
}

fn metrics(w: u32, h: u32, px_per_dp: f32) -> Metrics {
    let dp = |v: f32| v * px_per_dp;
    Metrics {
        w,
        h,
        px_per_dp,
        margin: dp(12.0),
        card_radius: dp(20.0),
        card_gap: dp(12.0),
        section_block: dp(45.67),
        section_label_top: dp(23.0),
        row_h: dp(55.67),
        row_pad_v: dp(17.0),
        inner: dp(16.0),
        icon: dp(28.0),
        icon_gap: dp(20.67),
        title_px: dp(32.0),
        title_x: dp(28.6),
        title_top: dp(117.7),
        back_x: dp(26.3),
        back_cy: dp(75.7),
        back_w: dp(11.7),
        back_h: dp(21.0),
        back_stroke: dp(1.8),
        stroke: dp(2.0),
        content_top: dp(167.7),
        search_h: dp(43.7),
        search_text_inset: dp(43.7),
        after_search: dp(12.7),
        row_title_px: dp(16.9),
        sub_px: dp(14.0),
        sub_gap: dp(6.7),
        sub_pitch: dp(18.7),
        label_px: dp(13.2),
        account_title_px: dp(17.1),
        account_row_h: dp(69.7),
        avatar: dp(34.0),
        avatar_inset: dp(12.8),
        badge_px: dp(14.5),
        badge_w: dp(72.9),
        badge_h: dp(28.9),
        chevron_w: dp(7.7),
        chevron_h: dp(13.3),
        chevron_inset: dp(17.3),
        chevron_stroke: dp(1.15),
        value_gap: dp(11.3),
        switch_w: dp(48.7),
        switch_h: dp(28.0),
        knob: dp(19.3),
        slider_h: dp(27.5),
        slider_knob: dp(18.7),
        status_x: dp(37.0),
        status_right: dp(40.0),
        status_bolt_right: dp(36.0),
        status_cy: dp(25.7),
        status_px: dp(14.1),
    }
}

// Sampled from the stock Settings on a Xiaomi 17 Ultra.
const LIGHT: Palette = Palette {
    bg: Rgba::rgb(243, 243, 243),
    card: Rgba::rgb(255, 255, 255),
    text: Rgba::rgb(0, 0, 0),
    subtitle: Rgba::rgb(102, 102, 102),
    value: Rgba::rgb(153, 153, 153),
    section: Rgba::rgb(143, 156, 174),
    chevron: Rgba::rgb(178, 178, 178),
    accent: Rgba::rgb(74, 128, 247),
    track: Rgba::rgb(240, 240, 240),
    switch_off: Rgba::rgb(229, 229, 229),
    search: Rgba::rgb(229, 229, 229),
    search_icon: Rgba::rgb(161, 161, 161),
    // Black at 30%: 160 over the search field, and darker blue where it
    // crosses the cursor.
    placeholder: Rgba::rgb(0, 0, 0).with_alpha(77),
    badge: Rgba::rgb(235, 242, 254),
    avatar: Rgba::rgb(221, 223, 226),
    avatar_glyph: Rgba::rgb(255, 255, 255),
    status: Rgba::rgb(61, 61, 61),
    // The stock status bar's dark icons, sampled over light pages.
    battery: BatteryInk {
        charging: Rgba::rgb(29, 205, 58),
        low: Rgba::rgb(250, 56, 46),
        level: Rgba::rgb(0, 0, 0).with_alpha(196),
        empty: Rgba::rgb(0, 0, 0).with_alpha(77),
        bolt: Rgba::rgb(0, 0, 0).with_alpha(172),
    },
    mono_icon: Rgba::rgb(179, 179, 179),
    danger: Rgba::rgb(230, 66, 60),
    danger_soft: Rgba::rgb(253, 236, 235),
    info: Rgba::rgb(51, 51, 51),
    outline: Rgba::rgb(102, 102, 102),
    spec_label: Rgba::rgb(127, 127, 127),
    // A white card held down goes to 225; the page under `<` goes from 243
    // to 219, the search field and its round button from 229 to 206.
    row_press: Rgba::rgb(0, 0, 0).with_alpha(30),
    button_press: Rgba::rgb(0, 0, 0).with_alpha(26),
    // Sampled from the stock WLAN page.
    round: Rgba::rgb(242, 242, 242),
    round_mark: Rgba::rgb(169, 169, 169),
    // Sampled from the stock page that adds a network; the shadow as the
    // grey it leaves on the page beside the list is fitted.
    updown: Rgba::rgb(214, 214, 214),
    popup: Rgba::rgb(255, 255, 255),
    shadow: Rgba::rgb(0, 0, 0).with_alpha(27),
};

const DARK: Palette = Palette {
    bg: Rgba::rgb(0, 0, 0),
    card: Rgba::rgb(28, 28, 28),
    text: Rgba::rgb(237, 237, 237),
    subtitle: Rgba::rgb(140, 140, 140),
    value: Rgba::rgb(128, 128, 128),
    section: Rgba::rgb(107, 113, 134),
    chevron: Rgba::rgb(77, 77, 77),
    accent: Rgba::rgb(84, 136, 248),
    track: Rgba::rgb(46, 46, 46),
    switch_off: Rgba::rgb(58, 58, 58),
    search: Rgba::rgb(28, 28, 28),
    search_icon: Rgba::rgb(110, 110, 110),
    // White at 36%: 110 over the search field.
    placeholder: Rgba::rgb(255, 255, 255).with_alpha(92),
    badge: Rgba::rgb(29, 40, 64),
    avatar: Rgba::rgb(51, 51, 51),
    avatar_glyph: Rgba::rgb(140, 140, 140),
    status: Rgba::rgb(224, 224, 224),
    // The stock status bar's light icons, sampled over black and over the
    // lock screen's wallpaper.
    battery: BatteryInk {
        // As the stock Settings fill it over black and over a dark grey: the
        // green they fill with over light pages too.
        charging: Rgba::rgb(29, 205, 58),
        low: Rgba::rgb(230, 75, 59),
        level: Rgba::rgb(255, 255, 255).with_alpha(232),
        empty: Rgba::rgb(255, 255, 255).with_alpha(94),
        // As the stock Settings draw it over black and over a dark grey.
        bolt: Rgba::rgb(255, 255, 255).with_alpha(207),
    },
    mono_icon: Rgba::rgb(110, 110, 110),
    danger: Rgba::rgb(255, 92, 84),
    danger_soft: Rgba::rgb(52, 24, 24),
    info: Rgba::rgb(214, 214, 214),
    outline: Rgba::rgb(140, 140, 140),
    spec_label: Rgba::rgb(128, 128, 128),
    // White as strong as the black in the light theme.
    row_press: Rgba::rgb(255, 255, 255).with_alpha(30),
    button_press: Rgba::rgb(255, 255, 255).with_alpha(26),
    // As far from the card as the light theme's are from its own.
    round: Rgba::rgb(41, 41, 41),
    round_mark: Rgba::rgb(118, 118, 118),
    // The arrows as much softer than the chevron as in the light theme; the
    // list a step lighter than the cards it opens over, and its shadow
    // darker, for it to show on them.
    updown: Rgba::rgb(54, 54, 54),
    popup: Rgba::rgb(41, 41, 41),
    shadow: Rgba::rgb(0, 0, 0).with_alpha(77),
};

// Sampled from the stock「我的设备」page: in light mode the cards are
// the ordinary settings cards; in dark mode they are white at low opacity
// over the field.
const LIGHT_HERO: Hero = Hero {
    glow: &glow::LIGHT,
    version: Rgba::rgb(0, 0, 0).with_alpha(153),
    card: LIGHT.card,
    title: LIGHT.text,
    value: LIGHT.value,
    chevron: LIGHT.chevron,
};

const DARK_HERO: Hero = Hero {
    glow: &glow::DARK,
    version: Rgba::rgb(255, 255, 255).with_alpha(153),
    card: Rgba::rgb(255, 255, 255).with_alpha(36),
    title: Rgba::rgb(244, 245, 248),
    value: Rgba::rgb(157, 161, 188),
    chevron: Rgba::rgb(118, 122, 160),
};
