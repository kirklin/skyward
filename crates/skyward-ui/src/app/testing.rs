//! What the tests of the pages share: the panel they draw on, the phone
//! they describe, and fingers and keys to work the pages with.

use super::*;
use skyward_info::{Boot, Charge, Cpu, Storage};
use skyward_net::reach::Reach;

// A Xiaomi 13 Ultra panel: 1440 × 3200 at 560 dpi.
pub(super) const W: u32 = 1440;
pub(super) const H: u32 = 3200;
pub(super) const PPD: f32 = 3.5;

pub(super) fn fonts() -> Fonts {
    crate::font::bundled()
}

pub(super) fn info() -> DeviceInfo {
    DeviceInfo {
        model: "Xiaomi 13 Ultra".into(),
        codename: "ishtar".into(),
        soc: None,
        cpu: Cpu { cores: 8, max_khz: Some(3_187_200) },
        ram_kib: 15_556_972,
        panel: Some((W, H)),
        screen: Some(6.73),
        os_version: None,
        android: None,
        locale: None,
        security_patch: None,
        kernel: "5.15.178".into(),
        storage: Storage { total: 496_550_785_024, free: None },
        battery: None,
        slot: None,
        snapshot: SnapshotState::None,
        boot: Boot { unlocked: None, verified: None },
    }
}

/// Where on screen the first control doing `action` sits.
/// Where on screen the control doing `action` is: among the page's
/// controls, on its pinned strip, or a row of its long list.
pub(super) fn spot(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action) -> (f32, f32) {
    let (t, scroll) = (app.theme(), app.top().1);
    let fr = app.frame(f, i);
    if let Some(h) = fr.hits.iter().find(|h| h.action == action) {
        let (x, y) = h.rect.centre();
        return (x, y - scroll);
    }
    if let Some(p) = &fr.pinned
        && let Some(h) = p.hits.iter().find(|h| h.action == action)
    {
        let (x, y) = h.rect.centre();
        return (x, y + pinned_top(&t, p, scroll));
    }
    if let Some(d) = &fr.dock
        && let Some(h) = d.hits.iter().find(|h| h.action == action)
    {
        let (x, y) = h.rect.centre();
        return (x, y + H as f32 - d.canvas.h as f32);
    }
    let rows = fr.rows.as_ref().expect("control on the page");
    let answer = rows.action.expect("control on the page");
    let k = (0..rows.count()).find(|&k| answer(k) == action).expect("control on the page");
    let (x, y) = rows.rect(k, fr.canvas.w as f32).centre();
    (x, y - scroll)
}

pub(super) fn tap(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action) -> Vec<Effect> {
    tap_at(app, f, i, action, 0)
}

pub(super) fn tap_at(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action, now: u64) -> Vec<Effect> {
    let (x, y) = spot(app, f, i, action);
    let mut fx = app.touch(Touch::Down(x, y), f, now);
    fx.extend(app.touch(Touch::Up, f, now));
    fx
}

/// Tap the `<`, which stays at the top of the screen.
pub(super) fn tap_back(app: &mut App, f: &Fonts, i: &DeviceInfo, now: u64) {
    app.frame(f, i);
    let (x, y) = widgets::back_rect(&app.theme()).centre();
    app.touch(Touch::Down(x, y), f, now);
    app.touch(Touch::Up, f, now);
}

/// Tap the round button at the right end of the bar.
pub(super) fn tap_button(app: &mut App, f: &Fonts, i: &DeviceInfo, now: u64) -> Vec<Effect> {
    app.frame(f, i);
    let (x, y) = widgets::button_rect(&app.theme()).centre();
    let mut fx = app.touch(Touch::Down(x, y), f, now);
    fx.extend(app.touch(Touch::Up, f, now));
    fx
}

/// `screen` opened from the home page, so there is a page to go back
/// to.
pub(super) fn opened(screen: Screen, f: &Fonts, i: &DeviceInfo) -> App {
    let mut app = App::new(Screen::Home, W, H, PPD);
    tap(&mut app, f, i, Action::Open(screen));
    app
}

/// A battery at `level`, as `charge` has it, 35.2 °C, on a charger
/// unless it discharges.
pub(super) fn battery_at(level: u8, charge: Charge) -> skyward_info::Battery {
    let plugged = charge != Charge::Discharging;
    skyward_info::Battery { level, charge, temp: Some(352), health: None, cycles: None, design: None, full: None, voltage: None, quick: false, plugged }
}

/// The grey of the screen's pixel at (`x`, `y`).
pub(super) fn grey_at(c: &Canvas, x: f32, y: f32) -> u8 {
    c.data[((y as u32 * c.w + x as u32) * 4) as usize]
}

/// The darkest grey of the screen within `r`.
pub(super) fn darkest(c: &Canvas, r: Rect) -> u8 {
    let (x0, y0, x1, y1) = (r.x as u32, r.y as u32, (r.x + r.w) as u32, (r.y + r.h) as u32);
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| c.data[((y * c.w + x) * 4) as usize])).min().expect("a part of the screen")
}

/// A copy of the screen.
pub(super) fn shot(c: &Canvas) -> Canvas {
    c.clone()
}

/// How a slide opening a page, or going `back`, stands `ms` into it:
/// where the page in front stands, how far left the page behind is, and
/// how much of its light the page behind keeps, in 256ths.
pub(super) fn slid(back: bool, ms: u64) -> (u32, u32, u32) {
    let (x, _) = if back { Spring::new(0, -1000.0, 0.0, BACK_PERIOD) } else { Spring::new(0, 1000.0, -OPEN_SPEED * 1000.0, OPEN_PERIOD) }.at(ms);
    let along = if back { 1.0 + x / 1000.0 } else { x / 1000.0 };
    let front = (along * W as f32).round() as u32;
    let behind = (BEHIND_SHIFT * W as f32 * (1.0 - along)).round() as u32;
    let keep = ((1.0 - BEHIND_DIM * (1.0 - along)) * 256.0).round() as u32;
    (front, behind, keep)
}

/// Check row `y` of `mid`: the page `ahead` from `front` on, and left of
/// it the page `under`, `behind` pixels left of rest and keeping `keep`
/// 256ths of its light.
pub(super) fn check_slide(mid: &Canvas, ahead: &Canvas, under: &Canvas, y: u32, (front, behind, keep): (u32, u32, u32)) {
    for x in 0..W {
        let at = |c: &Canvas, x: u32| c.data[((y * W + x) * 4) as usize] as u32;
        let want = if x >= front { at(ahead, x - front) } else { (at(under, x + behind) * keep) >> 8 };
        let got = at(mid, x);
        assert!(got.abs_diff(want) <= 1, "column {x} of row {y}: {got}, not {want} (front {front}, behind {behind})");
    }
}

/// Flick the page up from `start`: the finger travels `step` px every
/// 8 ms five times and lifts 8 ms after its last move. Returns when it
/// lifted.
pub(super) fn flick(app: &mut App, f: &Fonts, start: u64, step: f32) -> u64 {
    app.touch(Touch::Down(700.0, 2400.0), f, start);
    for k in 1..=5u64 {
        app.touch(Touch::Move(700.0, 2400.0 - step * k as f32), f, start + 8 * k);
    }
    let lift = start + 48;
    app.touch(Touch::Up, f, lift);
    lift
}

/// Drag the page to scroll `to`, resting before lifting so it does not
/// glide, and let the time pass. The finger first goes 100 px, past the
/// touch slop, and comes back to the mark. It runs down the page's
/// margin, clear of the sliders, which a finger landing on moves
/// instead of the page.
pub(super) fn scroll_to(app: &mut App, f: &Fonts, to: f32, now: u64) {
    let (from, x) = (app.top().1, app.theme().m.margin / 2.0);
    app.touch(Touch::Down(x, 2400.0), f, now);
    app.touch(Touch::Move(x, 2300.0), f, now + 200);
    app.touch(Touch::Move(x, 2400.0 - (to - from)), f, now + 400);
    app.touch(Touch::Up, f, now + 800);
    app.tick(now + 800);
    assert!((app.top().1 - to).abs() < 0.01, "{} {to}", app.top().1);
}

/// A screen the home page's rows fit on with a little room to spare.
pub(super) const SNUG: u32 = 3000;

/// The installing page, opened from the install page, with the
/// installer started and waiting for the computer.
pub(super) fn sideloading(f: &Fonts, i: &DeviceInfo) -> App {
    let mut app = opened(Screen::Install, f, i);
    assert_eq!(tap(&mut app, f, i, Action::Sideload), vec![Effect::Sideload]);
    assert_eq!(app.screen(), Screen::Installing);
    assert_eq!(app.state.install, Install::Starting);
    app.install(Event::Waiting, 0);
    app
}

/// Press `key` and let it go.
pub(super) fn press(app: &mut App, key: Key, now: u64) -> Vec<Effect> {
    let mut fx = app.key(key, now);
    fx.extend(app.key_up(key, now));
    fx
}

/// The control the keys are on, on the page drawn.
pub(super) fn lit(app: &mut App, f: &Fonts, i: &DeviceInfo) -> Option<Stop> {
    app.frame(f, i);
    app.lit().map(|s| s.stop)
}

/// Move the light down the page until it is on `stop`.
pub(super) fn light_on(app: &mut App, f: &Fonts, i: &DeviceInfo, stop: Stop) {
    for _ in 0..100 {
        press(app, Key::VolumeDown, 0);
        if app.keys && lit(app, f, i) == Some(stop) {
            return;
        }
    }
    panic!("the keys never came to {stop:?}");
}

/// Drag the knob of a swipe-to-confirm track all the way across.
pub(super) fn swipe(app: &mut App, f: &Fonts, i: &DeviceInfo, which: Confirm) -> Vec<Effect> {
    let track = app.frame(f, i).hits.iter().find(|h| h.action == Action::Confirm(which)).expect("the track is on the page").rect;
    let y = track.centre().1 - app.top().1;
    let mut fx = app.touch(Touch::Down(track.x + track.h / 2.0, y), f, 0);
    fx.extend(app.touch(Touch::Move(track.x + track.w / 2.0, y), f, 0));
    fx.extend(app.touch(Touch::Move(track.x + track.w, y), f, 0));
    fx.extend(app.touch(Touch::Up, f, 0));
    fx
}

/// Type `text` on the keyboard of the page showing, one tap a key, in
/// the letters layer.
pub(super) fn type_text(app: &mut App, f: &Fonts, i: &DeviceInfo, text: &str) {
    for ch in text.chars() {
        tap(app, f, i, Action::Type(Cap::Char(ch)));
    }
}

pub(super) fn entry(name: &str, kind: FileKind) -> FileRow {
    FileRow { name: name.into(), kind, size: 4096, modified: 1_792_348_260, mode: 0o040_755, uid: 0, gid: 0 }
}

/// When the tests list folders: a day after the entries changed.
pub(super) const LISTED: i64 = 1_792_348_260 + 86_400;

pub(super) fn folder(items: usize) -> FileKind {
    FileKind::Folder { items: Some(items) }
}

/// The file manager in /sdcard, listing a folder and a file.
pub(super) fn in_sdcard(f: &Fonts, i: &DeviceInfo) -> App {
    let mut app = App::new(Screen::Advanced, W, H, PPD);
    tap(&mut app, f, i, Action::Open(Screen::Files));
    app.set_files("/sdcard".into(), Ok(vec![entry("DCIM", folder(2)), entry("a.zip", FileKind::File)]), LISTED);
    app
}

/// Hold the file manager's entry at `index` until the page of what can
/// be done with it opens.
pub(super) fn hold_entry(app: &mut App, f: &Fonts, i: &DeviceInfo, index: usize) {
    let (x, y) = spot(app, f, i, Action::OpenEntry(index));
    app.touch(Touch::Down(x, y), f, 0);
    app.tick(LONG_PRESS_MS);
    app.touch(Touch::Up, f, LONG_PRESS_MS);
    assert_eq!(app.screen(), Screen::FileInfo);
}

pub(super) fn toast(app: &App) -> Option<&str> {
    app.toast.as_ref().map(|t| t.0.as_str())
}

pub(super) fn network(name: &str, signal: i32, security: Security) -> Network {
    let bands = skyward_net::wlan::Bands::of(5805);
    Network { ssid: name.as_bytes().to_vec(), name: name.into(), signal, frequency: 5805, bands, security }
}

pub(super) fn joined(name: &str, stage: Joining) -> Wlan {
    let n = network(name, -34, Security::Psk);
    Wlan {
        power: Power::On,
        networks: vec![n.clone()],
        joined: Some(skyward_net::wlan::Joined { ssid: n.ssid, name: n.name, security: n.security, stage }),
    }
}

pub(super) fn connected(name: &str) -> Wlan {
    use std::net::Ipv4Addr;
    joined(
        name,
        Joining::Connected(skyward_net::wlan::Link {
            address: Ipv4Addr::new(192, 168, 1, 23),
            mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: vec![Ipv4Addr::new(192, 168, 1, 1)],
            signal: -40,
            frequency: 5805,
            speed: Some(866),
            generation: Some(5),
            mac: "02:00:00:00:00:04".into(),
            reach: Some(Reach::Online),
        }),
    )
}
