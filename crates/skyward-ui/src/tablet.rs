//! Persistent navigation beside the existing recovery pages on large tablets.
use crate::app::{App, Screen, Touch};
use crate::canvas::Canvas;
use crate::color::{mix, Rgba};
use crate::font::Fonts;
use crate::geom::Rect;
use crate::icons::{self, Icon};
use crate::lang::{Lang, Text};
use crate::pages::{BLUE, GREEN, ORANGE, PURPLE, RED, SLATE};
use crate::theme::Theme;
use crate::widgets;

const SECTIONS: &[(Screen, Text, Icon)] = &[
    (Screen::About, ["我的设备", "我的裝置", "My device"], Icon::Phone),
    (Screen::Search, ["搜索", "搜尋", "Search"], Icon::More),
    (Screen::Wlan, ["WLAN", "Wi-Fi", "Wi-Fi"], Icon::Wlan),
    (Screen::Install, ["安装", "安裝", "Install"], Icon::Install),
    (Screen::Backup, ["备份", "備份", "Backup"], Icon::Backup),
    (Screen::Restore, ["恢复", "還原", "Restore"], Icon::Restore),
    (Screen::Mount, ["挂载", "掛載", "Mount"], Icon::Mount),
    (Screen::Decrypt, ["解密", "解密", "Decrypt"], Icon::Lock),
    (Screen::Wipe, ["清除数据", "清除資料", "Wipe data"], Icon::Wipe),
    (Screen::Advanced, ["高级", "進階", "Advanced"], Icon::More),
    (Screen::Settings, ["设置", "設定", "Settings"], Icon::Gear),
    (Screen::Reboot, ["重启", "重新啟動", "Reboot"], Icon::Power),
];

pub(crate) fn is_section(screen: Screen) -> bool {
    SECTIONS.iter().any(|(s, _, _)| *s == screen)
}

pub enum Input {
    Detail(Touch),
    Select(Screen),
    Sidebar,
}

#[derive(Clone, Copy)]
enum Gesture {
    Idle,
    Detail,
    Sidebar { start: f32, scroll: f32, pick: Option<Screen>, moved: bool },
}

pub struct Tablet {
    pub detail: Canvas,
    sidebar: Canvas,
    width: u32,
    height: u32,
    dp: f32,
    scroll: f32,
    max_scroll: f32,
    hits: Vec<(Rect, Screen)>,
    gesture: Gesture,
    cached: Option<(bool, Lang, Screen, bool, u32, Option<Screen>)>,
}

impl Tablet {
    pub fn new(width: u32, detail_width: u32, height: u32, dp: f32) -> Self {
        Self {
            detail: Canvas::new(detail_width, height), sidebar: Canvas::new(width, height),
            width, height, dp, scroll: 0.0, max_scroll: 0.0, hits: Vec::new(),
            gesture: Gesture::Idle, cached: None,
        }
    }

    pub fn cancel_gesture(&mut self) {
        self.gesture = Gesture::Idle;
    }

    pub fn paint(&mut self, out: &mut Canvas, f: &Fonts, app: &App) {
        let selected = app.tablet_selection();
        let wlan = app.state.offers(Screen::Wlan);
        let pressed = match self.gesture {
            Gesture::Sidebar { pick, moved: false, .. } => pick,
            _ => None,
        };
        let key = (app.state.dark, app.state.lang, selected, wlan, self.scroll.round() as u32, pressed);
        if self.cached != Some(key) {
            let t = if app.state.dark { Theme::dark(self.width, self.height, self.dp) }
                else { Theme::light(self.width, self.height, self.dp) };
            let (c, d) = (&mut self.sidebar, self.dp);
            c.clear(t.pal.bg);
            c.text_top(&f.title, "Skyward", 28.0*d, 94.0*d, 30.0*d, t.pal.text, None);
            let search = Rect::new(18.0*d, 148.0*d, self.width as f32-36.0*d, 42.0*d);
            widgets::search_bar(c, &t, search, 1.0);
            let words = widgets::search_words(&t, f, search,
                app.state.lang.pick(["搜索恢复功能", "搜尋復原功能", "Search recovery"]));
            c.lay(&words, search.x.floor() as i32, search.y.floor() as i32, 1.0);
            let entries: Vec<_> = SECTIONS.iter().filter(|(s, _, _)| *s != Screen::Search && (*s != Screen::Wlan || wlan)).collect();
            let top = 202.0*d;
            let row = 48.0*d;
            let gap = 12.0*d;
            let group = |s: Screen| match s {
                Screen::About => 0,
                Screen::Wlan => 1,
                Screen::Install | Screen::Backup | Screen::Restore | Screen::Mount | Screen::Decrypt => 2,
                _ => 3,
            };
            let gaps = entries.windows(2).filter(|pair| group(pair[0].0) != group(pair[1].0)).count();
            let content = entries.len() as f32 * row + gaps as f32 * gap;
            self.max_scroll = (content - (self.height as f32 - top - 24.0*d)).max(0.0);
            self.scroll = self.scroll.clamp(0.0, self.max_scroll);
            self.hits.clear();
            self.hits.push((search, Screen::Search));
            // Clip scrolling rows below the fixed title.
            let mut list = Canvas::band(self.width, self.height, top as u32..self.height, 0..self.width, t.pal.bg);
            let mut positions = Vec::new();
            let mut y = top - self.scroll;
            let mut first = 0;
            while first < entries.len() {
                let mut end = first+1;
                while end < entries.len() && group(entries[end].0) == group(entries[first].0) { end += 1; }
                let group_top = y;
                let height = (end-first) as f32 * row;
                let shade = mix(t.pal.card, t.pal.subtitle, if t.dark { 0.19 } else { 0.12 });
                let down = mix(t.pal.card, t.pal.subtitle, if t.dark { 0.28 } else { 0.18 });
                list.fill_round_rect_graded(18.0*d, group_top, self.width as f32-36.0*d, height,
                    t.m.card_radius, &|fy| {
                        let index = ((fy-group_top)/row).floor().max(0.0) as usize + first;
                        let screen = entries[index.min(end-1)].0;
                        if pressed == Some(screen) { down } else if selected == screen { shade } else { t.pal.card }
                    });
                for _ in first..end { positions.push(y); y += row; }
                y += gap;
                first = end;
            }
            for (index, &&(screen, title, icon)) in entries.iter().enumerate() {
                let y = positions[index];
                let r = Rect::new(18.0*d, y, self.width as f32 - 36.0*d, row);
                if y + row <= top || y >= self.height as f32 { continue; }
                let ink = t.pal.text;
                let color = match screen {
                    Screen::Install => GREEN,
                    Screen::Restore => PURPLE,
                    Screen::Wipe => RED,
                    Screen::Reboot => ORANGE,
                    Screen::About | Screen::Wlan | Screen::Backup | Screen::Decrypt => BLUE,
                    _ => SLATE,
                };
                // Exactly the home list's tile and glyph, at a compact sidebar size.
                let mut compact = t;
                compact.m.icon = 26.0*d;
                icons::tile(&mut list, &compact, 31.0*d, y + (row-compact.m.icon)/2.0, color, icon);
                list.text_vc(&f.body, app.state.lang.pick(title), 66.0*d, y + row/2.0,
                    17.0*d, ink, Some(self.width as f32 - 90.0*d));
                self.hits.push((Rect::new(r.x, r.y.max(top), r.w, (r.y+r.h).min(self.height as f32)-r.y.max(top)), screen));
            }
            c.replace(&list);
            c.line(self.width as f32 - 1.0, 32.0*d, self.width as f32 - 1.0, self.height as f32 - 24.0*d,
                1.0, mix(t.pal.bg, t.pal.subtitle, 0.2));
            if self.max_scroll > 0.0 {
                let available = self.height as f32 - top - 16.0*d;
                let thumb = available * available / (available + self.max_scroll);
                let y = top + (available-thumb)*self.scroll/self.max_scroll;
                c.fill_round_rect(self.width as f32 - 5.0*d, y, 2.0*d, thumb, d, t.pal.chevron);
            }
            self.cached = Some(key);
        }
        out.clear(Rgba::rgb(0, 0, 0));
        out.lay(&self.sidebar, 0, 0, 1.0);
        out.lay(&self.detail, self.width as i32, 0, 1.0);
    }

    #[cfg(test)]
    pub fn section_center(&self, screen: Screen) -> (f32, f32) {
        self.hits.iter().find(|(_, s)| *s == screen).unwrap().0.centre()
    }

    /// A gesture keeps the pane it began in, including when crossing the divider.
    pub fn route(&mut self, event: Touch) -> Input {
        if let Touch::Down(x, y) = event {
            self.gesture = if x >= self.width as f32 { Gesture::Detail } else {
                let pick = self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, s)| *s);
                Gesture::Sidebar { start: y, scroll: self.scroll, pick, moved: false }
            };
        }
        let result = match (self.gesture, event) {
            (Gesture::Detail, Touch::Down(x, y)) => Input::Detail(Touch::Down(x-self.width as f32, y)),
            (Gesture::Detail, Touch::Move(x, y)) => Input::Detail(Touch::Move(x-self.width as f32, y)),
            (Gesture::Detail, Touch::Up) => Input::Detail(Touch::Up),
            (Gesture::Sidebar { start, scroll, pick, moved }, Touch::Move(x, y)) => {
                let moved = moved || (y-start).abs() > 8.0*self.dp || x >= self.width as f32 || x < 0.0;
                self.scroll = (scroll + start - y).clamp(0.0, self.max_scroll);
                self.gesture = Gesture::Sidebar { start, scroll, pick, moved };
                Input::Sidebar
            }
            (Gesture::Sidebar { pick: Some(s), moved: false, .. }, Touch::Up) => Input::Select(s),
            _ => Input::Sidebar,
        };
        if event == Touch::Up { self.gesture = Gesture::Idle; }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_the_divider_never_changes_a_gestures_pane() {
        let mut t = Tablet::new(100, 200, 200, 1.0);
        t.hits.push((Rect::new(0.0, 0.0, 100.0, 100.0), Screen::Wipe));
        assert!(matches!(t.route(Touch::Down(30.0, 30.0)), Input::Sidebar));
        assert!(matches!(t.route(Touch::Move(180.0, 30.0)), Input::Sidebar));
        assert!(matches!(t.route(Touch::Up), Input::Sidebar));
        assert!(matches!(t.route(Touch::Down(130.0, 30.0)), Input::Detail(Touch::Down(30.0, 30.0))));
        assert!(matches!(t.route(Touch::Move(20.0, 30.0)), Input::Detail(Touch::Move(-80.0, 30.0))));
        assert!(matches!(t.route(Touch::Up), Input::Detail(Touch::Up)));
    }

    #[test]
    fn scrolling_cannot_select_a_section() {
        let mut t = Tablet::new(100, 200, 200, 1.0);
        t.hits.push((Rect::new(0.0, 0.0, 100.0, 100.0), Screen::Settings));
        t.route(Touch::Down(30.0, 30.0));
        assert!(matches!(t.route(Touch::Up), Input::Select(Screen::Settings)));
        t.route(Touch::Down(30.0, 30.0));
        t.route(Touch::Move(30.0, 60.0));
        assert!(matches!(t.route(Touch::Up), Input::Sidebar));
    }
}
