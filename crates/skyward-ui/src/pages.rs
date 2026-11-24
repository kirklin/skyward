use crate::app::{
    Action, BackupAt, BackupEnd, BackupJob, Check, ClashChoice, Confirm, Damage, Data, Dock, DownloadEnd, EraseStop, Field, FileKind,
    FileOp, FileTask, Fold, Folder, Frame, FsAction, FsChoice, FsRow, Have, HeroArt, Hit, Install, JobEnd, Keys, Pick, Place, Region, Rows,
    Screen, SkywardCheck, SkywardEnd, SkywardJob, SkywardStage, Slider, Source, State, Switch, SystemsWork, Target, UsbDrive, UsbLink, Wipe,
    ADDABLE, BACKUP_PARTS, DATA_PART, DATA_PARTITION, DOWNLOADS, FASTBOOT_CHOICES, MEDIA_PART, SCREEN_OFF, TARGETS, TIME_ZONES, UTC, WIPES, installs, listed, starting,
};
use crate::canvas::Canvas;
use crate::color::Rgba;
use crate::editor;
use crate::files;
use crate::fmt::{self, Verb};
use crate::font::Fonts;
use crate::geom::Rect;
use crate::lang::{same, Lang, Text};
use crate::search;
use crate::term::{self, Term};
use crate::texts;
use crate::theme::Theme;
use crate::icons::{self, Icon};
use crate::widgets::{self, Caption, Detail, Fact, Outline, Row, Spec, SystemCard, SystemMark, Tone, Trail};
use skyward_info::{DeviceInfo, VERSION};
use skyward_install::{Outcome, Place as FlashPlace, Reflash, Refusal, Stage};
use skyward_net::download::Progress;
use skyward_net::supplicant::Security;
use skyward_net::wlan::{Bands, Network, Power, Stage as Joining};
use skyward_systems::{Held, Who};
use skyward_vab::{Slot, Standing};
use std::path::Path;

// The colours the stock Settings fills its icons' tiles with, the same in
// light and dark mode.
const BLUE: Rgba = Rgba::rgb(0x34, 0x82, 0xff);
const GREEN: Rgba = Rgba::rgb(0x1d, 0xcd, 0x3a);
const PURPLE: Rgba = Rgba::rgb(0x77, 0x67, 0xf9);
const SLATE: Rgba = Rgba::rgb(0x8c, 0x9d, 0xb0);
const ORANGE: Rgba = Rgba::rgb(0xff, 0x69, 0x00);
const RED: Rgba = Rgba::rgb(0xfa, 0x38, 0x2e);

/// What every page draws from: style, typefaces, the device readings, and
/// what the pages hold, from the choices made on them to where the data
/// partition stands.
pub struct Ctx<'a> {
    pub t: &'a Theme,
    pub f: &'a Fonts,
    pub info: &'a DeviceInfo,
    pub state: &'a State,
    /// The shell the terminal page shows, once one has started.
    pub term: Option<&'a Term>,
    /// The part of the page to draw; all of it if none is given. The page
    /// is laid out whole either way.
    pub band: Option<Band>,
}

/// Rows `rows` of a page, and of them the columns `cols`, each from the
/// first to one past the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub rows: (u32, u32),
    pub cols: (u32, u32),
}

pub fn render(screen: Screen, cx: &Ctx) -> Frame {
    match screen {
        Screen::Decrypt => decrypt(cx),
        Screen::Home => home(cx),
        Screen::Settings => settings(cx),
        Screen::Reboot => reboot(cx),
        Screen::Wipe => wipe(cx),
        Screen::Mount => mount(cx),
        Screen::Install => install(cx),
        Screen::Choose(place) => choose(cx, place),
        Screen::Installing => installing(cx),
        Screen::About => about(cx),
        Screen::Specs => specs(cx),
        Screen::ScreenOff => screen_off(cx),
        Screen::Search => search(cx),
        Screen::Backup => backup(cx),
        Screen::Restore => restore(cx),
        Screen::RestoreBackup => restore_backup(cx),
        Screen::BackupJob => backup_job(cx),
        Screen::Advanced => advanced(cx),
        Screen::Files => files(cx),
        Screen::FileInfo => file_info(cx),
        Screen::Permissions => permissions(cx),
        Screen::Editor => editor(cx),
        Screen::Terminal => terminal(cx),
        Screen::Relabel => relabel(cx),
        Screen::Filesystems => filesystems(cx),
        Screen::Filesystem => filesystem(cx),
        Screen::PartWork => part_work(cx),
        Screen::Language => language(cx),
        Screen::TimeZone => time_zone(cx),
        Screen::TimeZones(region) => time_zones(cx, region),
        Screen::Logs => logs(cx),
        Screen::Update => update(cx),
        Screen::SkywardUpdate => skyward_update(cx),
        Screen::License => license(cx),
        Screen::FontLicense => font_license(cx),
        Screen::Slot => slot(cx),
        Screen::Systems => systems(cx),
        Screen::Fastboot => fastboot(cx),
        Screen::Erase => erase(cx),
        Screen::CantStart => cant_start(cx),
        Screen::Unhandled => unhandled(cx),
        Screen::Rename => rename(cx),
        Screen::Clash => clash(cx),
        Screen::FileJob => file_job(cx),
        Screen::Wlan => wlan(cx),
        Screen::WlanPassword => wlan_password(cx),
        Screen::WlanNetwork => wlan_network(cx),
        Screen::WlanAdd => wlan_add(cx),
    }
}

/// Label and colour of each swipe-to-confirm control: the filesystem
/// page's as `s` says what it is to do.
pub fn confirm_label(which: Confirm, s: &State) -> (&'static str, bool) {
    use texts::confirm;
    let (label, danger) = match which {
        Confirm::Filesystem => match s.fs_action {
            FsAction::Repair => (confirm::REPAIR, true),
            FsAction::Remake(_) => (confirm::CHANGE, true),
        },
        Confirm::Reboot => (confirm::REBOOT, false),
        Confirm::Wipe => (confirm::WIPE, true),
        Confirm::Flash => (confirm::FLASH, true),
        Confirm::Backup => (confirm::BACK_UP, false),
        Confirm::Restore => (confirm::RESTORE, true),
        Confirm::Slot => (confirm::SWITCH, true),
        Confirm::FactoryReset => (confirm::FACTORY_RESET, true),
        Confirm::DeleteEntry | Confirm::DeleteBackup => (confirm::DELETE, true),
        Confirm::Install => (confirm::INSTALL, false),
        Confirm::InstallWipe => (confirm::INSTALL_AND_WIPE, true),
        Confirm::RunScript => (confirm::INSTALL, true),
        Confirm::Merge => (confirm::MERGE, true),
        Confirm::Relabel => (confirm::FIX, true),
        Confirm::SwitchSystem => (confirm::SWITCH_AND_REBOOT, false),
        Confirm::RemoveSystem => (confirm::REMOVE, true),
        Confirm::InstallSystem => (confirm::INSTALL, false),
    };
    (s.lang.pick(label), danger)
}

/// The home page's large title.
pub const HOME_TITLE: &str = "Skyward";

/// Where the phone can restart to, and what is said under the name of one
/// that wants it.
const REBOOT_TARGETS: [(Text, Option<Text>); 6] = [
    (texts::reboot::target::SYSTEM, None),
    (texts::reboot::target::RECOVERY, None),
    (same("Fastboot"), Some(texts::reboot::target::FASTBOOT_NOTE)),
    (same("Bootloader"), None),
    (same("EDL"), Some(texts::reboot::target::EDL_NOTE)),
    (texts::reboot::target::POWER_OFF, None),
];

/// A canvas for a page `h` tall in the colour `bg`, holding the rows `cx`
/// asks to be drawn.
fn sheet(cx: &Ctx, h: u32, bg: Rgba) -> Canvas {
    let w = cx.t.m.w;
    let Band { rows, cols } = cx.band.unwrap_or(Band { rows: (0, h), cols: (0, w) });
    Canvas::band(w, h, rows.0..rows.1, cols.0..cols.1, bg)
}

fn page(cx: &Ctx) -> Canvas {
    sheet(cx, cx.t.m.h, cx.t.pal.bg)
}

/// A canvas tall enough for a scrolling page; [`fit`] trims it to the content.
fn long_page(cx: &Ctx) -> Canvas {
    sheet(cx, cx.t.m.h * 3, cx.t.pal.bg)
}

/// How tall a long page whose content ends at `bottom` is: to the bottom
/// margin after it, and never shorter than one screen.
fn fitted(t: &Theme, bottom: f32) -> u32 {
    ((bottom + t.m.pad_bottom).ceil() as u32).max(t.m.h)
}

/// Trim a long page to its content plus the bottom margin, and never shorter
/// than one screen.
fn fit(c: Canvas, t: &Theme, bottom: f32) -> Canvas {
    c.crop_height(fitted(t, bottom))
}

fn hits(rects: &[Rect], actions: &[Action]) -> Vec<Hit> {
    rects.iter().zip(actions).map(|(&rect, &action)| Hit { rect, action }).collect()
}

/// Recovery home, laid out like the Settings root: title, search, a device
/// card, then grouped entries. The status bar carries the readings: the
/// slot, the data partition's lock and the battery's temperature.
pub fn home(cx: &Ctx) -> Frame {
    home_with_active(cx, None)
}

/// [`home`] with `active` screen highlighted for tablet navigation sidebars.
pub fn home_with_active(cx: &Ctx, active: Option<Screen>) -> Frame {
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    // The search bar is left room on the page and drawn over it, folding
    // away as the page scrolls up.
    let bar = Rect::new(t.m.margin, t.m.content_top, t.m.w as f32 - t.m.margin * 2.0, t.m.search_h);
    let mut y = bar.y + bar.h + t.m.after_search;
    let cards_top = y;
    let fold = Fold { bar, hint: l.pick(texts::search::HINT), span: cards_top - bar.y };
    out.push(Hit { rect: bar, action: Action::Open(Screen::Search) });

    let sub = match &i.os_version {
        Some(v) => format!("{} · {v}", i.codename),
        None => i.codename.clone(),
    };
    // Under the phone's name the card holds only what asks for something to
    // be done: a battery too low to install on, an update waiting to merge,
    // and a restart into the slot the system does not run from now.
    let low = match i.battery.as_ref().and_then(skyward_install::battery_refusal) {
        Some(Refusal::Battery { level, charging }) => Some(fmt::battery_low(l, level, charging)),
        _ => None,
    };
    let next = match (i.slot, &cx.state.boot_slots) {
        (Some(now), Some(Ok(slots))) if slots.active != now => Some(fmt::next_boot(l, slots.active, now)),
        _ => None,
    };
    let mut rows = Vec::new();
    let mut opens = Vec::new();
    if let Some((title, note)) = &low {
        rows.push(Row::new(title).icon(RED, Icon::Battery).subtitle(note).trail(Trail::None));
        opens.push(None);
    }
    if let Some(note) = fmt::snapshot_note(l, &i.snapshot) {
        rows.push(Row::new(fmt::snapshot(l, &i.snapshot)).icon(ORANGE, Icon::Layers).subtitle(note).trail(Trail::Arrow));
        opens.push(Some(Action::Open(Screen::Update)));
    }
    if let Some((title, note)) = &next {
        rows.push(Row::new(title).icon(ORANGE, Icon::Power).subtitle(note).trail(Trail::Arrow));
        opens.push(Some(Action::Open(Screen::Slot)));
    }
    if cx.state.system_notice.is_some() {
        let why = l.pick(texts::home::TAP_TO_SEE_WHY);
        rows.push(Row::new(l.pick(texts::systems::SECOND_FAILED)).icon(RED, Icon::Systems).subtitle(why).trail(Trail::Arrow));
        opens.push(Some(Action::Open(Screen::Systems)));
    }
    let card_active = if active == Some(Screen::About) {
        Some(0)
    } else {
        opens.iter().position(|open| active.is_some_and(|a| *open == Some(Action::Open(a)))).map(|k| k + 1)
    };
    let card = widgets::device_card_active(&mut c, t, f, y, &i.model, &sub, &rows, card_active);
    out.push(Hit { rect: card.rows[0], action: Action::Open(Screen::About) });
    out.extend(card.rows[1..].iter().zip(&opens).filter_map(|(&rect, open)| open.map(|action| Hit { rect, action })));
    y = card.bottom + t.m.card_gap;
    // On a phone whose WLAN the recovery brings up, the WLAN comes first
    // under the device, as on the stock settings home; with it, where the
    // phone can hold a second system, the two systems.
    let mut rows = Vec::new();
    let mut opens = Vec::new();
    if let Some(w) = &cx.state.wlan {
        rows.push(Row::new(l.pick(texts::page::WLAN)).icon(BLUE, Icon::Wlan).trail(Trail::Value(fmt::wlan_value(l, w))));
        opens.push(Action::Open(Screen::Wlan));
    }
    let starts = systems_value(cx);
    if cx.state.offers(Screen::Systems) {
        let row = Row::new(l.pick(texts::page::DUAL_BOOT)).icon(PURPLE, Icon::Systems);
        rows.push(match &starts {
            Some(value) => row.trail(Trail::Value(value)),
            None => row,
        });
        opens.push(Action::Open(Screen::Systems));
    }
    if !rows.is_empty() {
        let first_active = opens.iter().position(|open| active.is_some_and(|a| *open == Action::Open(a)));
        let first = widgets::list_active(&mut c, t, f, y, &rows, first_active);
        out.extend(hits(&first.rows, &opens));
        y = first.bottom + t.m.card_gap;
    }
    let tool_screens = [Screen::Install, Screen::Backup, Screen::Restore, Screen::Mount, Screen::Decrypt];
    let tools_active = active.and_then(|a| tool_screens.iter().position(|&s| s == a));
    let tools = widgets::list_active(&mut c, t, f, y, &[
        Row::new(l.pick(texts::page::INSTALL)).icon(GREEN, Icon::Install),
        Row::new(l.pick(texts::page::BACKUP)).icon(BLUE, Icon::Backup),
        Row::new(l.pick(texts::page::RESTORE)).icon(PURPLE, Icon::Restore),
        Row::new(l.pick(texts::page::MOUNT)).icon(SLATE, Icon::Mount),
        Row::new(l.pick(texts::page::DECRYPT)).icon(BLUE, Icon::Lock).trail(Trail::Value(fmt::data(l, &cx.state.data))),
    ], tools_active);
    out.extend(hits(&tools.rows, &tool_screens.map(Action::Open)));
    y = tools.bottom + t.m.card_gap;
    let target = l.pick(REBOOT_TARGETS[cx.state.reboot].0);
    let system_screens = [Screen::Wipe, Screen::Advanced, Screen::Settings, Screen::Reboot];
    let system_active = active.and_then(|a| system_screens.iter().position(|&s| s == a));
    let system = widgets::list_active(&mut c, t, f, y, &[
        Row::new(l.pick(texts::page::WIPE)).icon(RED, Icon::Wipe),
        Row::new(l.pick(texts::page::ADVANCED)).icon(SLATE, Icon::More),
        Row::new(l.pick(texts::page::SETTINGS)).icon(SLATE, Icon::Gear),
        Row::new(l.pick(texts::page::REBOOT)).icon(ORANGE, Icon::Power).trail(Trail::Value(target)),
    ], system_active);
    out.extend(hits(&system.rows, &system_screens.map(Action::Open)));
    // However little it holds, the page scrolls far enough to fold its bar
    // away.
    let height = fitted(t, system.bottom).max(t.m.h + fold.span.ceil() as u32);
    Frame { back: false, fold: Some(fold), ..Frame::new(t, c.crop_height(height), out, HOME_TITLE, cards_top) }
}

/// What the home page says beside its entry to the systems: the one that
/// starts, where a second system is installed, once they are read.
fn systems_value(cx: &Ctx) -> Option<String> {
    let l = cx.state.lang;
    match &cx.state.systems {
        Some(Ok(systems)) => Some(match (&systems.other, starting(systems)) {
            (None, _) => l.pick(texts::home::NO_SECOND_SYSTEM).into(),
            (Some(_), Who::Installed) => systems.installed.name.clone(),
            (Some(other), Who::Other) => other.name.clone(),
        }),
        _ => None,
    }
}

/// What the advanced page says beside its system update entry: a download
/// under way, a newer system found, or where an update the system installed
/// stands.
fn update_value(cx: &Ctx) -> &'static str {
    let (s, l) = (cx.state, cx.state.lang);
    if s.downloading() {
        return l.pick(texts::update::DOWNLOADING);
    }
    if s.newest().is_some_and(|r| cx.info.os_version.as_deref() != Some(r.version.as_str())) {
        return l.pick(texts::update::NEW_VERSION);
    }
    fmt::update_value(l, &s.update)
}

/// Preferences, opening with the stock「显示与亮度」theme previews.
pub fn settings(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let themes = widgets::theme_picker(&mut c, t, f, l, top + t.m.dp(8.9));
    out.extend(hits(&themes.rows, &[Action::Theme { dark: false }, Action::Theme { dark: true }]));
    let mut y = widgets::section(&mut c, t, f, themes.bottom, l.pick(texts::settings::BRIGHTNESS));
    let off = fmt::screen_off(l, SCREEN_OFF[s.screen_off]);
    let below = Row::new(l.pick(texts::page::SCREEN_TIMEOUT)).trail(Trail::Value(&off));
    let slider = widgets::slider_card(&mut c, t, f, y, Caption::Sun, l.pick(texts::settings::SCREEN_BRIGHTNESS), s.brightness, Some(below));
    out.push(Hit { rect: slider.track, action: Action::Slider(Slider::Brightness) });
    if let Some(r) = slider.below {
        out.push(Hit { rect: r, action: Action::Open(Screen::ScreenOff) });
    }
    y = widgets::section(&mut c, t, f, slider.bottom, l.pick(texts::settings::VIBRATION));
    let below = Row::new(l.pick(texts::settings::TOUCH_VIBRATION)).trail(Trail::Switch(s.vibrate));
    let vibration = widgets::slider_card(&mut c, t, f, y, Caption::Vibrate, l.pick(texts::settings::VIBRATION_INTENSITY), s.vibration, Some(below));
    out.push(Hit { rect: vibration.track, action: Action::Slider(Slider::Vibration) });
    if let Some(r) = vibration.below {
        out.push(Hit { rect: r, action: Action::Toggle(Switch::Vibrate) });
    }
    y = widgets::section(&mut c, t, f, vibration.bottom, l.pick(texts::settings::GENERAL));
    let general = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(texts::page::LANGUAGE)).icon(BLUE, Icon::Globe).trail(Trail::Value(l.name())),
        Row::new(l.pick(texts::page::TIME_ZONE)).icon(ORANGE, Icon::Clock).trail(Trail::Value(s.zone_name())),
    ]);
    out.extend(hits(&general.rows, &[Action::Open(Screen::Language), Action::Open(Screen::TimeZone)]));
    y = widgets::section(&mut c, t, f, general.bottom, "USB");
    let usb = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::settings::FILE_TRANSFER)).icon(BLUE, Icon::Usb).trail(Trail::Switch(s.file_transfer))]);
    out.extend(hits(&usb.rows, &[Action::Toggle(Switch::FileTransfer)]));
    y = widgets::footnote(&mut c, t, f, usb.bottom, l.pick(texts::settings::FILE_TRANSFER_NOTE));
    y = widgets::section(&mut c, t, f, y, l.pick(texts::settings::DEBUGGING));
    let debug = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::page::TERMINAL)).icon(SLATE, Icon::Terminal)]);
    out.extend(hits(&debug.rows, &[Action::Open(Screen::Terminal)]));
    y = widgets::section(&mut c, t, f, debug.bottom, l.pick(texts::settings::ABOUT));
    let mut rows = vec![Row::new(l.pick(texts::page::ABOUT)).trail(Trail::Value(VERSION))];
    let mut opens = vec![Action::Open(Screen::About)];
    if s.offers(Screen::SkywardUpdate) {
        let row = Row::new(l.pick(texts::page::SKYWARD_UPDATE));
        rows.push(match skyward_value(cx) {
            Some(value) => row.trail(Trail::Value(value)),
            None => row,
        });
        opens.push(Action::Open(Screen::SkywardUpdate));
    }
    let about = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&about.rows, &opens));
    Frame::new(t, fit(c, t, about.bottom), out, l.pick(texts::page::SETTINGS), top)
}

/// What the settings page says beside its Skyward update entry: an update
/// under way, or a newer release found.
fn skyward_value(cx: &Ctx) -> Option<&'static str> {
    let (s, l) = (cx.state, cx.state.lang);
    if s.updating_skyward() {
        return Some(l.pick(texts::skyward_update::UPDATING));
    }
    match (s.skyward_newest(), &s.skyward_job) {
        (_, Some(SkywardJob { outcome: Some(SkywardEnd::Updated(_)), .. })) => Some(l.pick(texts::skyward_update::UPDATED)),
        (Some(n), _) if n.newer && n.build.is_some() => Some(l.pick(texts::update::NEW_VERSION)),
        _ => None,
    }
}

/// How long the screen stays on with nothing touched, opened from the
/// settings.
pub fn screen_off(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let top = t.m.content_top;
    let labels = SCREEN_OFF.map(|ms| fmt::screen_off(l, ms));
    let rows: Vec<Row> = labels.iter().enumerate().map(|(k, label)| Row::new(label).trail(Trail::Check(s.screen_off == k))).collect();
    let times = widgets::list(&mut c, t, f, top, &rows);
    let picks: Vec<Action> = (0..SCREEN_OFF.len()).map(Action::ScreenOff).collect();
    let y = widgets::footnote(&mut c, t, f, times.bottom, l.pick(texts::screen_off::NOTE));
    if SCREEN_OFF[s.screen_off].is_none() {
        widgets::footnote(&mut c, t, f, y, l.pick(texts::screen_off::STAYS_ON));
    }
    Frame::new(t, c, hits(&times.rows, &picks), l.pick(texts::page::SCREEN_TIMEOUT), top)
}

/// Mount manager: the slot's partitions, each with a switch to mount it
/// read-only or unmount it; one that programs are using says which, and
/// stays.
pub fn mount(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;

    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::mount::PARTITIONS));
    let bottom = match &s.mounts {
        Err(why) => {
            let y = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::mount::UNREADABLE)).trail(Trail::None)]).bottom;
            widgets::footnote(&mut c, t, f, y, why)
        }
        Ok(mounts) if mounts.is_empty() => widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::mount::NONE)).trail(Trail::None)]).bottom,
        Ok(mounts) => {
            let subs: Vec<String> = mounts
                .iter()
                .map(|m| match m.holders.is_empty() {
                    true => m.point.clone(),
                    false => texts::mount::in_use_by(l, &m.point, m.holders.join(l.pick(texts::list::SEPARATOR))),
                })
                .collect();
            let rows: Vec<Row> = mounts
                .iter()
                .zip(&subs)
                .map(|(m, sub)| {
                    let trail = if m.holders.is_empty() { Trail::Switch(m.mounted) } else { Trail::Text(l.pick(texts::mount::IN_USE)) };
                    Row::new(&m.base).subtitle(sub).trail(trail)
                })
                .collect();
            let list = widgets::list(&mut c, t, f, y, &rows);
            out.extend(list.rows.iter().zip(mounts).enumerate().map(|(i, (&rect, m))| {
                let action = match m.holders.is_empty() {
                    true => Action::MountToggle(i),
                    false => Action::Unavailable(l.pick(texts::mount::CANT_UNMOUNT_IN_USE)),
                };
                Hit { rect, action }
            }));
            let y = match &s.mount_failure {
                Some(failure) => widgets::footnote(&mut c, t, f, list.bottom, &fmt::mount_failure(l, failure)),
                None => list.bottom,
            };
            let about = match cx.info.slot.map(|s| s.letter()) {
                Some(slot) => texts::mount::shows_slot(l, slot),
                None => l.pick(texts::mount::SHOWS).into(),
            };
            widgets::footnote(&mut c, t, f, y, &about)
        }
    };

    Frame::new(t, fit(c, t, bottom), out, l.pick(texts::page::MOUNT), top)
}

/// Who made the recovery, as the About page names them under its version.
const AUTHOR: &str = "© 2026 Kirk Lin";

/// About this device, modelled on the stock「我的设备」: the wordmark, the
/// version and its author over a moving colour field, and the summary,
/// hardware and link cards scrolling over it. Only the cards are drawn
/// here; the canvas is clear wherever the field shows.
pub fn about(cx: &Ctx) -> Frame {
    let (t, f) = (cx.t, cx.f);
    let m = &t.m;
    // The page is drawn whole, under the field: laid out first to learn
    // how tall it comes to, then drawn that tall.
    let (_, _, bottom) = about_cards(cx, &mut Canvas::band(m.w, m.h * 3, 0..0, 0..0, Rgba { r: 0, g: 0, b: 0, a: 0 }));
    let mut c = Canvas::new(m.w, fitted(t, bottom));
    let (out, summary_top, bottom) = about_cards(cx, &mut c);
    let canvas = fit(c, t, bottom);
    let art = HeroArt {
        wordmark: widgets::wordmark(t, f, "Skyward", m.dp(233.7), m.dp(37.8)),
        version: widgets::version_line(t, f, VERSION),
        author: widgets::author_line(t, f, AUTHOR),
        runs: canvas.runs(),
    };
    Frame { header: false, hero: Some(art), ..Frame::new(t, canvas, out, cx.state.lang.pick(texts::page::ABOUT), summary_top) }
}

/// The phone's processor by its marketing name in `l`, from the phone's
/// settings, or else by its cores.
fn soc(l: Lang, i: &DeviceInfo) -> Option<&str> {
    i.soc.as_ref().map(|n| l.among(n.hans.as_str(), n.hant.as_str(), n.en.as_str()))
}

/// The About page's cards on `c`: what touching them does, where the
/// first starts, and where the last ends.
fn about_cards(cx: &Ctx, c: &mut Canvas) -> (Vec<Hit>, f32, f32) {
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let m = &t.m;
    let tone = Tone::hero(t);
    let mut out = Vec::new();

    let storage = fmt::storage_used(&i.storage).unwrap_or_else(|| fmt::capacity(i.storage.total));
    let mut facts = vec![
        Fact { title: l.pick(texts::specs::MODEL), value: i.model.clone(), link: false },
        Fact { title: l.pick(texts::specs::STORAGE), value: storage, link: true },
    ];
    if let Some(v) = &i.os_version {
        facts.push(Fact { title: l.pick(texts::specs::OS_VERSION), value: v.clone(), link: false });
    }
    if let Some(r) = &i.android {
        facts.push(Fact { title: l.pick(texts::specs::ANDROID_VERSION), value: r.clone(), link: false });
    }
    let summary = widgets::facts(c, t, f, m.dp(490.7), &facts, &tone);
    out.push(Hit { rect: summary.rows[1], action: Action::Open(Screen::Specs) });

    let processor = soc(l, i).map_or_else(|| fmt::cpu(l, &i.cpu), str::to_string);
    let mut specs = vec![(l.pick(texts::specs::PROCESSOR), processor), (l.pick(texts::specs::RAM), fmt::ram(i.ram_kib))];
    if let Some(design) = i.battery.as_ref().and_then(|b| b.design) {
        specs.push((l.pick(texts::specs::BATTERY_CAPACITY), fmt::mah(design)));
    }
    if let Some(d) = i.screen {
        specs.push((l.pick(texts::specs::SCREEN_SIZE), fmt::inches(d)));
    }
    if let Some(p) = i.panel {
        specs.push((l.pick(texts::specs::RESOLUTION), fmt::panel(p)));
    }
    let y = widgets::spec_card(c, t, f, summary.bottom + m.card_gap, &fmt::model_lines(&i.model), &specs, &tone);
    let links = widgets::list_toned(c, t, f, y + m.card_gap, &[Row::new(l.pick(texts::page::SPECS)), Row::new(l.pick(texts::page::LOGS))], &tone);
    out.extend(hits(&links.rows, &[Action::Open(Screen::Specs), Action::Open(Screen::Logs)]));
    (out, summary.rows[0].y, links.bottom)
}

/// Every reading the recovery has, modelled on the stock「全部参数与信息」.
pub fn specs(cx: &Ctx) -> Frame {
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let gap = t.m.card_gap;
    let mut c = long_page(cx);
    let mut out = Vec::new();
    // This page leaves more room under the title than the settings pages.
    let top = t.m.dp(188.0);

    let mut grid = vec![Spec { icon: Outline::Phone, label: l.pick(texts::specs::MODEL), value: fmt::model_lines(&i.model) }];
    if let Some(design) = i.battery.as_ref().and_then(|b| b.design) {
        grid.push(Spec { icon: Outline::Battery, label: l.pick(texts::specs::BATTERY_CAPACITY), value: fmt::mah(design) });
    }
    let cpu = match soc(l, i) {
        Some(soc) => format!("{soc}\n{}", fmt::cpu(l, &i.cpu)),
        None => fmt::cpu(l, &i.cpu),
    };
    grid.push(Spec { icon: Outline::Chip, label: l.pick(texts::specs::PROCESSOR), value: cpu });
    grid.push(Spec { icon: Outline::Memory, label: l.pick(texts::specs::RAM), value: fmt::ram(i.ram_kib) });
    if let Some(p) = i.panel {
        grid.push(Spec { icon: Outline::Resolution, label: l.pick(texts::specs::RESOLUTION), value: fmt::panel(p) });
    }
    if let Some(d) = i.screen {
        grid.push(Spec { icon: Outline::Screen, label: l.pick(texts::specs::SCREEN_SIZE), value: fmt::inches(d) });
    }
    grid.push(Spec { icon: Outline::Storage, label: l.pick(texts::specs::STORAGE_CAPACITY), value: fmt::capacity(i.storage.total) });
    grid.push(Spec { icon: Outline::Tag, label: l.pick(texts::specs::CODENAME), value: i.codename.clone() });
    let mut y = widgets::spec_grid(&mut c, t, f, top, &grid);

    if let Some(v) = &i.os_version {
        y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(texts::specs::OS_VERSION), v.clone())]);
    }
    if let Some(r) = &i.android {
        y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(texts::specs::ANDROID_VERSION), fmt::android(l, r, i.security_patch.as_ref()))]);
    }
    y = widgets::info_card(&mut c, t, f, y + gap, &[
        (l.pick(texts::specs::RECOVERY_VERSION), format!("Skyward {VERSION}")),
        (l.pick(texts::specs::KERNEL_VERSION), i.kernel.clone()),
    ]);

    let mut boot = Vec::new();
    if let Some(s) = i.slot {
        boot.push((l.pick(texts::slot::CURRENT), s.letter().to_string()));
        boot.push((l.pick(texts::page::UPDATE), fmt::snapshot(l, &i.snapshot).to_string()));
    }
    if let Some(unlocked) = i.boot.unlocked {
        let state = if unlocked { texts::specs::UNLOCKED } else { texts::specs::LOCKED };
        boot.push(("Bootloader", l.pick(state).to_string()));
    }
    if let Some(v) = &i.boot.verified {
        boot.push((l.pick(texts::specs::VERIFIED_BOOT), fmt::verified(l, v)));
    }
    boot.push((l.pick(texts::specs::DATA_PARTITION), fmt::data(l, &cx.state.data).to_string()));
    y = widgets::info_card(&mut c, t, f, y + gap, &boot);

    if let Some(b) = &i.battery {
        let mut rows = vec![(l.pick(texts::specs::LEVEL), fmt::level(l, b))];
        if let Some(v) = b.temp {
            rows.push((l.pick(texts::specs::TEMPERATURE), fmt::celsius(v)));
        }
        if let Some(v) = fmt::wear(l, b) {
            rows.push((l.pick(texts::specs::HEALTH), v));
        }
        if let Some(v) = b.cycles {
            rows.push((l.pick(texts::specs::CHARGE_CYCLES), texts::specs::cycles(l, v)));
        }
        if let Some(v) = b.voltage {
            rows.push((l.pick(texts::specs::VOLTAGE), fmt::volts(v)));
        }
        y = widgets::info_card(&mut c, t, f, y + gap, &rows);
    }

    let total = fmt::capacity(i.storage.total);
    let storage = match i.storage.free.map(fmt::gb) {
        Some(free) => texts::specs::free_of(l, free, total),
        None => texts::specs::total_unmounted(l, total),
    };
    y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(texts::specs::INTERNAL_STORAGE), storage)]);
    let links = widgets::list(&mut c, t, f, y + gap, &[
        Row::new(l.pick(texts::page::LICENSE)).trail(Trail::Value("GPL-3.0")),
        Row::new(l.pick(texts::page::FONTS)).trail(Trail::Value("MiSans")),
    ]);
    out.extend(hits(&links.rows, &[Action::Open(Screen::License), Action::Open(Screen::FontLicense)]));
    Frame::new(t, fit(c, t, links.bottom), out, l.pick(texts::page::SPECS), top)
}

/// The data partition and its users' own files. The page opens the
/// partition as it shows, which is how it learns each user's screen lock;
/// a user whose files wait on one is asked with the control that lock is
/// answered with, from the start.
pub fn decrypt(cx: &Ctx) -> Frame {
    let (c, l) = (page(cx), cx.state.lang);
    match &cx.state.data {
        Data::Locked { user, keys, checking } => code(cx, c, *user, *keys, *checking),
        Data::Closed | Data::Opening => data_status(
            cx,
            c,
            Icon::Lock,
            l.pick(texts::decrypt::DATA_PARTITION),
            Some(l.pick(texts::decrypt::READING_LOCK)),
            None,
        ),
        Data::Open => data_status(cx, c, Icon::Folder, l.pick(texts::decrypt::DECRYPTED), Some(l.pick(texts::decrypt::SEE_FILES)), None),
        Data::Failed(why) => data_status(cx, c, Icon::Lock, l.pick(texts::decrypt::FAILED), None, Some(why)),
        Data::Erased => data_status(cx, c, Icon::Wipe, l.pick(texts::data::ERASED), Some(l.pick(texts::decrypt::MADE_ANEW)), None),
    }
}

/// How far apart, in dp, the lines of the Decrypt page's title are where
/// it takes more than one.
const TITLE_PITCH: f32 = 32.0;

/// Where the partition stands, under its sign: a line on it, or why it
/// did not open and a way to try again; then the way off the page.
fn data_status(cx: &Ctx, mut c: Canvas, icon: Icon, title: &str, note: Option<&str>, why: Option<&str>) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let m = &t.m;
    let x = m.w as f32 / 2.0;
    let sign_cy = m.dp(150.0);
    c.fill_circle(x, sign_cy, m.dp(30.0), t.pal.card);
    icons::glyph(&mut c, icon, x, sign_cy, m.dp(40.0), t.pal.accent);
    // What stands under the title moves down by the lines the title takes.
    let room = m.w as f32 - m.title_x * 2.0;
    let more = widgets::centred(&mut c, &f.heading, title, x, sign_cy + m.dp(64.0), m.dp(24.0), m.dp(TITLE_PITCH), t.pal.text, room);
    if let Some(note) = note {
        widgets::centred(&mut c, &f.body, note, x, sign_cy + m.dp(98.0) + more, m.sub_px, m.sub_pitch, t.pal.subtitle, room);
    }
    let mut out = Vec::new();
    if let Some(why) = why {
        widgets::paragraph(&mut c, t, f, sign_cy + m.dp(88.0) + more, why);
        let again = widgets::button(&mut c, t, f, m.h as f32 * 0.62, l.pick(texts::common::TRY_AGAIN));
        out.push(Hit { rect: again, action: Action::OpenData });
    }
    let leave_cy = m.h as f32 - m.dp(64.0);
    let leave = l.pick(if matches!(cx.state.data, Data::Open | Data::Erased) { texts::common::DONE } else { texts::decrypt::NOT_NOW });
    c.text_center(&f.heading, leave, x, leave_cy, m.row_title_px, t.pal.accent);
    out.push(Hit { rect: Rect::new(x - m.dp(80.0), leave_cy - m.dp(24.0), m.dp(160.0), m.dp(48.0)), action: Action::Skip });
    Frame { header: false, ..Frame::new(t, c, out, l.pick(texts::page::DECRYPT), 0.0) }
}

/// A user's own files wait on their screen lock: the control it is
/// answered with, which takes nothing while what was entered is checked.
fn code(cx: &Ctx, mut c: Canvas, user: u32, keys: Keys, checking: bool) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let m = &t.m;
    let x = m.w as f32 / 2.0;
    let mut out = Vec::new();
    let title = match (keys == Keys::Pattern, user) {
        (true, 0) => l.pick(texts::decrypt::DRAW_PATTERN).to_string(),
        (true, user) => texts::decrypt::draw_pattern_of(l, user),
        (false, 0) => l.pick(texts::decrypt::ENTER_LOCK).to_string(),
        (false, user) => texts::decrypt::enter_lock_of(l, user),
    };
    let note = l.pick(if checking { texts::decrypt::CHECKING } else { texts::decrypt::TO_SEE_FILES });
    let room = m.w as f32 - m.title_x * 2.0;
    let more = widgets::centred(&mut c, &f.heading, &title, x, m.dp(130.0), m.dp(24.0), m.dp(TITLE_PITCH), t.pal.text, room);
    widgets::centred(&mut c, &f.body, note, x, m.dp(166.0) + more, m.sub_px, m.sub_pitch, t.pal.subtitle, room);
    let mut pattern = None;
    let bottom = match keys {
        Keys::Letters => {
            let top = m.h as f32 - widgets::keyboard_height(t);
            let caps = widgets::keyboard(&mut c, t, f, l, top, s.layer, s.key_held(), l.pick(texts::page::DECRYPT));
            if !checking {
                out.extend(caps.into_iter().map(|(rect, cap)| Hit { rect, action: Action::Type(cap) }));
            }
            top
        }
        Keys::Pattern => {
            let span = m.dp(300.0);
            let cy = m.dp(300.0) + span / 2.0;
            widgets::pattern_lock(&mut c, t, x, cy, span, &s.pattern);
            if !checking {
                out.push(Hit { rect: Rect::new(x - span, cy - span, span * 2.0, span * 2.0), action: Action::Pattern });
                pattern = Some((x, cy, span));
            }
            m.h as f32 - m.dp(26.0)
        }
        Keys::Digits => {
            let top = m.h as f32 - widgets::keypad_height(t);
            let caps = widgets::keypad(&mut c, t, f, l, top, s.key_held());
            if !checking {
                out.extend(caps.into_iter().map(|(rect, cap)| Hit { rect, action: Action::Type(cap) }));
            }
            top
        }
    };
    // The dots sit just above whatever types into them; a pattern is its
    // own showing of what has been drawn.
    if keys != Keys::Pattern {
        widgets::code_dots(&mut c, t, x, bottom - m.dp(92.0), s.code.chars().count());
    }
    let skip_cy = bottom - m.dp(34.0);
    c.text_center(&f.heading, l.pick(texts::decrypt::NOT_NOW), x, skip_cy, m.row_title_px, t.pal.accent);
    out.push(Hit { rect: Rect::new(x - m.dp(80.0), skip_cy - m.dp(24.0), m.dp(160.0), m.dp(48.0)), action: Action::Skip });
    Frame { header: false, pattern, ..Frame::new(t, c, out, l.pick(texts::page::DECRYPT), 0.0) }
}

/// Reboot target picker with a swipe-to-confirm control.
pub fn reboot(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let mut y = widgets::section_first(&mut c, t, f, top, l.pick(texts::reboot::TO));
    let rows: Vec<Row> = REBOOT_TARGETS
        .iter()
        .enumerate()
        .map(|(k, &(title, sub))| {
            let r = Row::new(l.pick(title)).trail(Trail::Check(s.reboot == k));
            match sub {
                Some(text) => r.subtitle(l.pick(text)),
                None => r,
            }
        })
        .collect();
    let targets = widgets::list(&mut c, t, f, y, &rows);
    let picks: Vec<Action> = (0..TARGETS.len()).map(Action::RebootTarget).collect();
    out.extend(hits(&targets.rows, &picks));
    y = targets.bottom;
    if let Some(slot) = cx.info.slot {
        let next = match &s.boot_slots {
            Some(Ok(slots)) if slots.active == slot => texts::reboot::current(l, slot.letter()),
            Some(Ok(slots)) => slots.active.letter().to_string(),
            Some(Err(_)) => l.pick(texts::common::UNREADABLE).to_string(),
            None => String::new(),
        };
        y = widgets::section(&mut c, t, f, y, l.pick(texts::slot::TITLE));
        let slots = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::reboot::NEXT_BOOT)).trail(Trail::Value(&next))]);
        out.extend(hits(&slots.rows, &[Action::Open(Screen::Slot)]));
        y = slots.bottom;
    }
    let (label, danger) = confirm_label(Confirm::Reboot, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Reboot) });
    Frame::new(t, c, out, l.pick(texts::page::REBOOT), top)
}

/// Installing, opened from the home page: the system on the phone now, and
/// where a package can come from.
pub fn install(cx: &Ctx) -> Frame {
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mut facts = Vec::new();
    if let Some(v) = &i.os_version {
        facts.push((l.pick(texts::install::CURRENT_SYSTEM), v.clone()));
    }
    if let Some(p) = &i.security_patch {
        facts.push((l.pick(texts::install::SECURITY_PATCH), fmt::patch(l, p)));
    }
    if let Some(s) = i.slot {
        facts.push((l.pick(texts::slot::CURRENT), texts::install::slot_detail(l, s.letter(), s.other().letter())));
    }
    let ways = l.pick(texts::install::WAYS);
    let y = if facts.is_empty() {
        widgets::section_first(&mut c, t, f, top, ways)
    } else {
        let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::common::THIS_PHONE));
        let y = widgets::info_card(&mut c, t, f, y, &facts);
        widgets::section(&mut c, t, f, y, ways)
    };
    let mut rows = Vec::new();
    if let Some(note) = fmt::snapshot_note(l, &i.snapshot) {
        rows.push(Row::new(fmt::snapshot(l, &i.snapshot)).icon(ORANGE, Icon::Layers).subtitle(note).trail(Trail::None));
    }
    // Internal storage opens with the data partition, which the decrypt
    // page opens or tells why it does not.
    let data = &cx.state.data;
    let (storage, from_storage) = if data.storage_open() {
        (texts::install::CHOOSE_IN_STORAGE, Action::Open(Screen::Choose(Place::Storage)))
    } else if *data == Data::Erased {
        (texts::data::ERASED, Action::Open(Screen::Decrypt))
    } else {
        (texts::decrypt::NEEDED, Action::Open(Screen::Decrypt))
    };
    // The drive's page waits for one to be plugged in, and says why one
    // that is cannot be read.
    let usb = match &cx.state.usb_drive {
        Some(UsbDrive::Shown) => texts::install::CHOOSE_ON_USB_DRIVE,
        Some(UsbDrive::Unreadable(_)) => texts::usb_drive::UNREADABLE,
        None => texts::usb_drive::WANTED,
    };
    rows.push(Row::new(install_title(l, Source::File(Place::Storage))).icon(BLUE, Icon::Phone).subtitle(l.pick(storage)));
    rows.push(Row::new(install_title(l, Source::File(Place::UsbDrive))).icon(PURPLE, Icon::UsbDrive).subtitle(l.pick(usb)));
    let sideload = l.pick(texts::install::SIDELOAD);
    rows.push(Row::new(install_title(l, Source::Computer)).icon(GREEN, Icon::Usb).subtitle(sideload));
    let methods = widgets::list(&mut c, t, f, y, &rows);
    let from_usb = Action::Open(Screen::Choose(Place::UsbDrive));
    let mut out = hits(&methods.rows[methods.rows.len() - 3..], &[from_storage, from_usb, Action::Sideload]);
    // A phone that can hold a second system installs one from here too.
    let what = l.pick(match cx.state.dual_boot {
        true => texts::install::WHAT_WITH_SECOND_SYSTEM,
        false => texts::install::WHAT,
    });
    let y = widgets::footnote(&mut c, t, f, methods.bottom, what);
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::common::OPTIONS));
    let check = Row::new(l.pick(texts::install::VERIFY_SIGNATURES)).trail(Trail::Switch(cx.state.check_signature));
    let signature = widgets::list(&mut c, t, f, y, &[check]);
    out.extend(hits(&signature.rows, &[Action::Toggle(Switch::Signature)]));
    let mut y = widgets::footnote(&mut c, t, f, signature.bottom, l.pick(texts::install::VERIFY_SIGNATURES_NOTE));
    if cx.state.recovery_partitions {
        let row = Row::new(l.pick(texts::install::FLASH_SKYWARD_AGAIN)).trail(Trail::Switch(cx.state.keep_recovery));
        let options = widgets::list(&mut c, t, f, y + t.m.card_gap, &[row]);
        out.extend(hits(&options.rows, &[Action::Toggle(Switch::KeepRecovery)]));
        y = widgets::footnote(&mut c, t, f, options.bottom, l.pick(texts::install::FLASH_SKYWARD_AGAIN_NOTE));
    }
    Frame::new(t, fit(c, t, y), out, l.pick(texts::page::INSTALL), top)
}

/// How installing from `source` is named: on the install page, and as the
/// title of the page that installs.
fn install_title(l: Lang, source: Source) -> &'static str {
    use texts::install::title;
    l.pick(match source {
        Source::Computer => title::FROM_COMPUTER,
        Source::File(Place::Storage) => title::FROM_STORAGE,
        Source::File(Place::UsbDrive) => title::FROM_USB_DRIVE,
        Source::Opened { .. } => title::FROM_FILES,
        Source::Asked { security: false } => title::SYSTEM_UPDATE,
        Source::Asked { security: true } => title::SECURITY_UPDATE,
        Source::Rescue => title::RESCUE,
    })
}

/// What a place is called at the top of its folders.
fn place_name(l: Lang, place: Place) -> &'static str {
    l.pick(match place {
        Place::Storage => texts::place::STORAGE,
        Place::UsbDrive => texts::place::USB_DRIVE,
    })
}

/// A one-line card on how things stand.
fn standing<'a>(title: &'a str, color: Rgba, icon: Icon, detail: &'a str) -> Row<'a> {
    Row::new(title).icon(color, icon).subtitle(detail).trail(Trail::None)
}

fn share(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64) as f32
    }
}

fn percent(share: f32) -> u32 {
    (share * 100.0).floor() as u32
}

/// Where an update stands, and the share of its step done when known.
fn update_stage(l: Lang, stage: Option<&Stage>) -> (String, Option<f32>) {
    match stage {
        Some(Stage::Writing { done, total }) => {
            let s = share(*done, *total);
            (texts::installing::update_writing(l, percent(s)), Some(s))
        }
        Some(Stage::Verifying { index, count, partition }) => {
            (texts::installing::update_verifying(l, partition, index + 1, count), Some(*index as f32 / *count as f32))
        }
        Some(Stage::Finishing) => (l.pick(texts::installing::UPDATE_FINISHING).into(), None),
        Some(Stage::Reflashing { done, total }) => (l.pick(texts::installing::REFLASHING).into(), Some(*done as f32 / *total as f32)),
        Some(
            Stage::Resizing | Stage::Flashing { .. } | Stage::Script { .. } | Stage::Signature { .. } | Stage::Allocating { .. } | Stage::Root { .. } | Stage::Storing,
        )
        | None => {
            (l.pick(texts::installing::GETTING_READY).into(), None)
        }
    }
}

/// Where installing a system beside the installed one stands, and the share
/// of its step done when known.
fn system_stage(l: Lang, stage: Option<&Stage>) -> (String, Option<f32>) {
    match stage {
        Some(Stage::Allocating { done, total }) => {
            let s = share(*done, *total);
            (texts::installing::system_allocating(l, percent(s)), Some(s))
        }
        Some(Stage::Root { done, total }) => {
            let s = share(*done, *total);
            let said = match done * 2 <= *total {
                true => texts::installing::system_writing(l, percent(s)),
                false => texts::installing::system_verifying(l, percent(s)),
            };
            (said, Some(s))
        }
        Some(Stage::Storing) => (l.pick(texts::installing::SYSTEM_STORING).into(), None),
        _ => (l.pick(texts::installing::GETTING_READY).into(), None),
    }
}

/// What the card says while an install script runs, and how far it is: the
/// last line the script showed and how far it says it is, then Skyward
/// going back into the recovery partitions it wrote over.
fn script_stage<'a>(l: Lang, stage: Option<&Stage>, said: &'a [String]) -> (&'a str, Option<f32>) {
    let running = l.pick(texts::installing::RUNNING_SCRIPT);
    let last = said.iter().rev().map(|line| line.trim()).find(|line| !line.is_empty()).unwrap_or(running);
    match stage {
        Some(Stage::Reflashing { done, total }) => (l.pick(texts::installing::REFLASHING), Some(*done as f32 / *total as f32)),
        Some(Stage::Script { thousandths }) => (last, Some(f32::from(*thousandths) / 1000.0)),
        _ => (last, None),
    }
}

/// How Skyward went back into the recovery partitions the install wrote
/// over, where it did.
fn reflashed(c: &mut Canvas, cx: &Ctx, y: f32) -> f32 {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    match &cx.state.reflash {
        None => y,
        Some(Reflash::Written(written)) => {
            let said = match written.as_slice() {
                [slot] => texts::installing::reflashed_one(l, slot.letter()),
                [a, b] => texts::installing::reflashed_two(l, a.letter(), b.letter()),
                other => unreachable!("a phone has two slots, and Skyward went back into {other:?}"),
            };
            widgets::footnote(c, t, f, y, &said)
        }
        Some(Reflash::Failed(why)) => {
            let y = widgets::footnote(c, t, f, y, l.pick(texts::installing::REFLASH_FAILED));
            widgets::footnote(c, t, f, y, why)
        }
    }
}

/// What the install script showed, where there is any: its heading, and
/// under it the lines as the page's long list, the last thing on the page.
fn script_output(c: &mut Canvas, cx: &Ctx, y: f32, rows: &mut Option<Rows>) -> f32 {
    let lines = &cx.state.script_output;
    if lines.is_empty() {
        return y;
    }
    let y = widgets::section(c, cx.t, cx.f, y, cx.state.lang.pick(texts::installing::SCRIPT_OUTPUT));
    *rows = Some(Rows { edges: widgets::output_edges(c, cx.t, cx.f, y, lines), action: None });
    y
}

/// The slots an image can be flashed to, each by its letter where the
/// slot the phone runs from is known.
fn pick_title(l: Lang, pick: Pick, current: Option<Slot>) -> String {
    match (pick, current) {
        (Pick::Current, Some(slot)) => texts::flash::current_slot(l, slot.letter()),
        (Pick::Current, None) => l.pick(texts::slot::CURRENT).into(),
        (Pick::Other, Some(slot)) => texts::flash::other_slot(l, slot.other().letter()),
        (Pick::Other, None) => l.pick(texts::flash::OTHER_SLOT_UNNAMED).into(),
        (Pick::Both, _) => l.pick(texts::flash::BOTH_SLOTS).into(),
    }
}

/// Installing a package step by step: receiving what the computer sends
/// with `adb sideload`, or reading the file chosen, then installing it.
pub fn installing(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let (mut out, mut rows) = (Vec::new(), None);
    let top = t.m.content_top;
    let computer = s.source == Source::Computer;
    let bottom = match &s.install {
        Install::Idle | Install::Starting | Install::Ending if computer => {
            let row = standing(l.pick(texts::installing::SWITCHING_USB), GREEN, Icon::Usb, l.pick(texts::installing::TO_SIDELOAD));
            widgets::list(&mut c, t, f, top, &[row]).bottom
        }
        Install::Idle | Install::Starting | Install::Ending if s.source == Source::Rescue => {
            let row = standing(l.pick(texts::installing::SWITCHING_USB), GREEN, Icon::Usb, l.pick(texts::installing::TO_RESCUE));
            widgets::list(&mut c, t, f, top, &[row]).bottom
        }
        Install::Idle | Install::Starting | Install::Ending => {
            widgets::list(&mut c, t, f, top, &[Row::new(l.pick(texts::installing::READING_FILE)).icon(GREEN, Icon::Install).trail(Trail::None)]).bottom
        }
        Install::Opening => {
            let opening = Row::new(l.pick(texts::installing::OPENING_DATA)).icon(GREEN, Icon::Lock).trail(Trail::None);
            let y = widgets::list(&mut c, t, f, top, &[opening]).bottom;
            asked_package(&mut c, cx, y)
        }
        Install::Locked(_) => {
            let row = standing(l.pick(texts::installing::CANT_READ_YET), ORANGE, Icon::Lock, l.pick(texts::decrypt::NEEDED));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let open = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::page::DECRYPT))]);
            out.extend(hits(&open.rows, &[Action::Open(Screen::Decrypt)]));
            let y = widgets::footnote(&mut c, t, f, open.bottom, l.pick(texts::installing::INSTALLS_ONCE_DECRYPTED));
            asked_package(&mut c, cx, y)
        }
        Install::Waiting if s.source == Source::Rescue => {
            let row = standing(l.pick(texts::installing::WAITING), GREEN, Icon::Usb, l.pick(texts::installing::CONNECT));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(texts::installing::RUN_ON_COMPUTER));
            let y = widgets::info_card(&mut c, t, f, y, &[
                (l.pick(texts::installing::RESCUE_INSTALL), l.pick(texts::installing::RESCUE_INSTALL_NOTE).into()),
                ("adb rescue getprop", l.pick(texts::installing::RESCUE_GETPROP_NOTE).into()),
                ("adb rescue wipe userdata", l.pick(texts::installing::RESCUE_WIPE_NOTE).into()),
            ]);
            widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::RESCUE_NOTE))
        }
        Install::Waiting => {
            let row = standing(l.pick(texts::installing::WAITING), GREEN, Icon::Usb, l.pick(texts::installing::CONNECT));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(texts::installing::RUN_ON_COMPUTER));
            let y = widgets::info_card(&mut c, t, f, y, &[(l.pick(texts::installing::SIDELOAD), l.pick(texts::installing::SIDELOAD_FILE_NOTE).into())]);
            widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::SIDELOAD_NOTE))
        }
        Install::Checking { stage: Some(Stage::Signature { done, total }), .. } => {
            let read = share(*done, *total);
            let detail = texts::installing::read(l, percent(read));
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(texts::installing::VERIFYING_SIGNATURE), &detail, Some(read));
            let note = l.pick(match s.source {
                Source::Computer => texts::installing::CANCEL_ENDS_SIDELOAD,
                Source::Rescue => texts::installing::CANCEL_LEAVES_RESCUE,
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => texts::installing::CANCEL,
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => texts::installing::CANCEL_KEEP_USB_DRIVE,
            });
            widgets::footnote(&mut c, t, f, y, note)
        }
        Install::Checking { size, .. } => {
            let detail = texts::installing::file_size(l, fmt::bytes(*size));
            let y = widgets::list(&mut c, t, f, top, &[standing(l.pick(texts::installing::READING_FILE), GREEN, Icon::Install, &detail)]).bottom;
            match s.source {
                Source::Computer | Source::Rescue => widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::KEEP_CABLE)),
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => y,
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::KEEP_USB_DRIVE)),
            }
        }
        Install::Choosing(choice) => {
            let (name, holds) = fmt::image_kind(l, choice.kind);
            let detail = texts::installing::size_and_holds(l, fmt::bytes(choice.size), &holds);
            let y = widgets::list(&mut c, t, f, top, &[standing(name, GREEN, Icon::Install, &detail)]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(texts::flash::PARTITION));
            let rows: Vec<Row> = choice
                .partitions
                .iter()
                .enumerate()
                .map(|(k, p)| Row::new(&p.name).subtitle(fmt::partition_role(l, p)).trail(Trail::Check(k == choice.partition)))
                .collect();
            let parts = widgets::list(&mut c, t, f, y, &rows);
            let choose: Vec<Action> = (0..choice.partitions.len()).map(Action::FlashPartition).collect();
            out.extend(hits(&parts.rows, &choose));
            // A partition the phone has in each slot goes to the slots
            // picked, and a logical one to the slot the phone runs from,
            // which is shown with nothing to pick; super is in no slot.
            let bottom = match choice.partitions[choice.partition].place {
                FlashPlace::Slotted => {
                    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(texts::slot::TITLE));
                    let titles = Pick::ALL.map(|p| pick_title(l, p, i.slot));
                    let rows: Vec<Row> =
                        Pick::ALL.iter().zip(&titles).map(|(p, title)| Row::new(title).trail(Trail::Check(*p == choice.pick))).collect();
                    let picks = widgets::list(&mut c, t, f, y, &rows);
                    out.extend(hits(&picks.rows, &Pick::ALL.map(Action::FlashSlots)));
                    picks.bottom
                }
                FlashPlace::Logical => {
                    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(texts::slot::TITLE));
                    let title = pick_title(l, Pick::Current, i.slot);
                    widgets::list(&mut c, t, f, y, &[Row::new(&title).trail(Trail::Check(true))]).bottom
                }
                FlashPlace::Single => parts.bottom,
            };
            let (label, danger) = confirm_label(Confirm::Flash, cx.state);
            let track = widgets::confirm(&mut c, t, f, bottom + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::Flash) });
            let note = l.pick(match s.source {
                Source::Computer => texts::flash::OVERWRITES_ENDS_SIDELOAD,
                Source::Rescue => texts::flash::OVERWRITES_LEAVES_RESCUE,
                Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => texts::flash::OVERWRITES,
            });
            widgets::footnote(&mut c, t, f, track.y + track.h, note)
        }
        Install::Confirming { version, wipe } => {
            use texts::installing::confirm;
            let (title, color, icon, detail, which) = if *wipe {
                let detail = match (version, s.cache_partition) {
                    (Some(v), true) => confirm::version_resets_with_cache(l, v),
                    (Some(v), false) => confirm::version_resets(l, v),
                    (None, true) => l.pick(confirm::UPDATE_RESETS_WITH_CACHE).into(),
                    (None, false) => l.pick(confirm::UPDATE_RESETS).into(),
                };
                (l.pick(confirm::WIPE_NEEDED).to_string(), ORANGE, Icon::Wipe, detail, Confirm::InstallWipe)
            } else {
                let title = match version {
                    Some(v) => confirm::version(l, v),
                    None => l.pick(confirm::UPDATE).into(),
                };
                let detail = match i.slot.map(|s| s.other().letter()) {
                    Some(slot) => confirm::to_slot(l, slot),
                    None => l.pick(confirm::STARTS_AFTER_REBOOT).into(),
                };
                (title, GREEN, Icon::Install, detail, Confirm::Install)
            };
            let y = widgets::list(&mut c, t, f, top, &[standing(&title, color, icon, &detail)]).bottom;
            let (label, danger) = confirm_label(which, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(which) });
            widgets::footnote(&mut c, t, f, track.y + track.h, l.pick(texts::installing::CANCEL_WRITES_NOTHING))
        }
        Install::Updating { version, stage, overwriting } => {
            let title = match version {
                Some(v) => texts::installing::named(l, v),
                None => l.pick(texts::installing::UPDATE).into(),
            };
            let (detail, done) = update_stage(l, stage.as_ref());
            let y = widgets::progress_card(&mut c, t, f, top, &title, &detail, done);
            let until = until_installed(l, s.source);
            let note = match i.slot.map(|s| s.other().letter()) {
                Some(slot) => texts::installing::until_and_slot(l, until, slot),
                None => until.to_string(),
            };
            let y = widgets::footnote(&mut c, t, f, y, &note);
            if *overwriting {
                widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::OVERWRITING))
            } else {
                y
            }
        }
        Install::Flashing { partition, stage } => {
            let (detail, done) = match stage {
                Some(Stage::Flashing { done, total }) => {
                    let s = share(*done, *total);
                    let detail = match done * 2 < *total {
                        true => texts::flash::writing(l, percent(s)),
                        false => texts::flash::verifying(l, percent(s)),
                    };
                    (detail, Some(s))
                }
                _ => (l.pick(texts::installing::GETTING_READY).into(), None),
            };
            let y = widgets::progress_card(&mut c, t, f, top, &texts::flash::flashing(l, partition), &detail, done);
            let keep = l.pick(match s.source {
                Source::Computer | Source::Rescue => texts::flash::KEEP_CABLE,
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => texts::flash::KEEP_POWER,
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => texts::flash::KEEP_USB_DRIVE,
            });
            widgets::footnote(&mut c, t, f, y, keep)
        }
        Install::SystemChoosing(choice) => {
            let detail = texts::installing::as_second_system(l, &choice.version);
            let y = widgets::list(&mut c, t, f, top, &[standing(&choice.name, PURPLE, Icon::Systems, &detail)]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(texts::installing::ROOM));
            let sizes: Vec<String> = choice.sizes.iter().map(|&b| fmt::bytes(b)).collect();
            let least = l.pick(texts::installing::LEAST_ROOM);
            let rows: Vec<Row> = sizes
                .iter()
                .enumerate()
                .map(|(k, size)| {
                    let row = Row::new(size).trail(Trail::Check(k == choice.size));
                    if k == 0 { row.subtitle(least) } else { row }
                })
                .collect();
            let list = widgets::list(&mut c, t, f, y, &rows);
            let picks: Vec<Action> = (0..choice.sizes.len()).map(Action::SystemSize).collect();
            out.extend(hits(&list.rows, &picks));
            let y = widgets::footnote(&mut c, t, f, list.bottom, l.pick(texts::installing::ROOM_NOTE));
            let (label, danger) = confirm_label(Confirm::InstallSystem, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::InstallSystem) });
            widgets::footnote(&mut c, t, f, track.y + track.h, l.pick(texts::installing::CANCEL_WRITES_NOTHING))
        }
        Install::Systeming { name, stage } => {
            let (detail, done) = system_stage(l, stage.as_ref());
            let y = widgets::progress_card(&mut c, t, f, top, &texts::installing::named(l, name), &detail, done);
            widgets::footnote(&mut c, t, f, y, until_installed(l, s.source))
        }
        Install::ConfirmingScript => {
            let name = s.package_name.as_deref().unwrap_or(l.pick(texts::installing::FLASHABLE_ZIP));
            let detail = l.pick(texts::installing::SCRIPT_WARNING);
            let mut y = widgets::list(&mut c, t, f, top, &[standing(name, ORANGE, Icon::Install, detail)]).bottom;
            if let Some(note) = fmt::snapshot_script_note(l, &i.snapshot) {
                y = widgets::footnote(&mut c, t, f, y, note);
            }
            let (label, danger) = confirm_label(Confirm::RunScript, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::RunScript) });
            widgets::footnote(&mut c, t, f, track.y + track.h, l.pick(texts::installing::CANCEL_RUNS_NOTHING))
        }
        Install::Scripting { stage, .. } => {
            let (detail, done) = script_stage(l, stage.as_ref(), &s.script_output);
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(texts::installing::SCRIPT), detail, done);
            let y = widgets::footnote(&mut c, t, f, y, until_installed(l, s.source));
            script_output(&mut c, cx, y, &mut rows)
        }
        Install::Wiping => {
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(texts::wipe::WIPING), factory_reset(s), None);
            widgets::footnote(&mut c, t, f, y, l.pick(texts::wipe::KEEP_POWER))
        }
        Install::Wiped(Ok(())) => {
            let made_anew = l.pick(texts::installing::DATA_MADE_ANEW);
            let y = widgets::list(&mut c, t, f, top, &[standing(l.pick(texts::wipe::WIPED), GREEN, Icon::Wipe, made_anew)]).bottom;
            widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::MORE_COMMANDS))
        }
        Install::Wiped(Err(stop)) => {
            let (what, detail) = fmt::erase_stop(l, stop);
            let mut y = widgets::list(&mut c, t, f, top, &[standing(l.pick(texts::wipe::INCOMPLETE), RED, Icon::Wipe, &what)]).bottom;
            if let Some(d) = detail {
                y = widgets::footnote(&mut c, t, f, y, &d);
            }
            widgets::footnote(&mut c, t, f, y, l.pick(texts::installing::MORE_COMMANDS))
        }
        Install::Finished { outcome, damage } => finished(&mut c, cx, top, outcome, damage.as_ref(), &mut out, &mut rows),
    };
    // What the system asked for says why, where it said.
    let bottom = match (s.source, &s.asked.reason) {
        (Source::Asked { .. }, Some(reason)) => widgets::info_card(&mut c, t, f, bottom + t.m.dp(12.0), &[(l.pick(texts::installing::REASON), reason.clone())]),
        _ => bottom,
    };
    Frame { rows, ..Frame::new(t, fit(c, t, bottom), out, install_title(l, s.source), top) }
}

/// The package the system asked for, by the path it gave, at `y`.
fn asked_package(c: &mut Canvas, cx: &Ctx, y: f32) -> f32 {
    let name = cx.state.package_name.clone().unwrap_or_default();
    widgets::info_card(c, cx.t, cx.f, y + cx.t.m.dp(12.0), &[(cx.state.lang.pick(texts::installing::PACKAGE), name)])
}

/// What not to do while a package from `source` is installed.
fn until_installed(l: Lang, source: Source) -> &'static str {
    l.pick(match source {
        Source::Computer | Source::Rescue => texts::installing::UNTIL_KEEP_CABLE,
        Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => texts::installing::UNTIL_KEEP_POWER,
        Source::File(Place::UsbDrive) | Source::Opened { drive: true } => texts::installing::UNTIL_KEEP_USB_DRIVE,
    })
}

/// How an install ended, and where to go from there: another package from
/// the computer, or back to choose another file.
fn finished(
    c: &mut Canvas,
    cx: &Ctx,
    top: f32,
    outcome: &Outcome,
    damage: Option<&Damage>,
    out: &mut Vec<Hit>,
    rows: &mut Option<Rows>,
) -> f32 {
    let (t, f, source, l) = (cx.t, cx.f, cx.state.source, cx.state.lang);
    let gap = t.m.card_gap;
    use texts::installed;
    let choose_another = installed::CHOOSE_ANOTHER;
    let (again, another) = match source {
        Source::Computer => (Row::new(l.pick(installed::SEND_AGAIN)).icon(GREEN, Icon::Usb), Action::Sideload),
        Source::File(_) | Source::Opened { .. } => (Row::new(l.pick(choose_another)).icon(GREEN, Icon::Folder), Action::Back),
        Source::Asked { .. } => (Row::new(l.pick(choose_another)).icon(GREEN, Icon::Folder), Action::Open(Screen::Install)),
        Source::Rescue => (Row::new(l.pick(installed::LEAVE_RESCUE)).icon(GREEN, Icon::Usb), Action::Back),
    };
    let restart = Row::new(l.pick(installed::REBOOT_TO_SYSTEM)).icon(ORANGE, Icon::Power);
    let done = l.pick(installed::INSTALLED);
    let nothing_written = l.pick(installed::NOTHING_WRITTEN);
    match outcome {
        Outcome::Installed { slot, .. } => {
            let detail = installed::to_slot(l, slot.letter());
            let y = widgets::list(c, t, f, top, &[standing(done, GREEN, Icon::Install, &detail)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[restart]);
            out.extend(hits(&next.rows, &[Action::RebootTo(Target::System)]));
            let y = reflashed(c, cx, next.bottom);
            afterwards_of(c, cx, y, true)
        }
        Outcome::Flashed { partition, slots } => {
            let detail = match slots.as_slice() {
                [] => installed::flashed_whole(l, partition),
                [slot] => installed::flashed_one(l, partition, slot.letter()),
                [a, b] => installed::flashed_two(l, partition, a.letter(), b.letter()),
                other => unreachable!("a phone has two slots, and the image went to {other:?}"),
            };
            let y = widgets::list(c, t, f, top, &[standing(l.pick(installed::FLASHED), GREEN, Icon::Install, &detail)]).bottom;
            let mut rows = vec![restart];
            let mut acts = vec![Action::RebootTo(Target::System)];
            if *partition == "recovery" {
                rows.push(Row::new(l.pick(installed::REBOOT_TO_RECOVERY)).icon(ORANGE, Icon::Power));
                acts.push(Action::RebootTo(Target::Recovery));
            }
            rows.push(again);
            acts.push(another);
            let next = widgets::list(c, t, f, y + gap, &rows);
            out.extend(hits(&next.rows, &acts));
            afterwards_of(c, cx, next.bottom, true)
        }
        Outcome::Ran { .. } => {
            let ran = l.pick(installed::SCRIPT_RAN);
            let y = widgets::list(c, t, f, top, &[standing(done, GREEN, Icon::Install, ran)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[restart, again]);
            out.extend(hits(&next.rows, &[Action::RebootTo(Target::System), another]));
            let y = reflashed(c, cx, next.bottom);
            let y = afterwards_of(c, cx, y, true);
            script_output(c, cx, y, rows)
        }
        Outcome::SystemInstalled { name } => {
            let detail = installed::second_system(l, name);
            let y = widgets::list(c, t, f, top, &[standing(done, GREEN, Icon::Systems, &detail)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(texts::page::DUAL_BOOT)).icon(PURPLE, Icon::Systems)]);
            out.extend(hits(&next.rows, &[Action::Open(Screen::Systems)]));
            afterwards_of(c, cx, next.bottom, true)
        }
        Outcome::Cancelled => {
            let y = widgets::list(c, t, f, top, &[standing(l.pick(installed::CANCELLED), SLATE, Icon::Install, nothing_written)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[again]);
            out.extend(hits(&next.rows, &[another]));
            next.bottom
        }
        Outcome::Refused(r) => {
            let reason = fmt::refusal(l, r, &cx.info.codename, source, cx.state.zone());
            let y = widgets::list(c, t, f, top, &[standing(l.pick(installed::CANT_INSTALL), ORANGE, Icon::Install, &reason)]).bottom;
            // What the system asked for and did not go in leaves the phone
            // where it was, from which it may go back to the system.
            let (rows, acts) = match source {
                Source::Asked { .. } => (vec![again, restart], vec![another, Action::RebootTo(Target::System)]),
                _ => (vec![again], vec![another]),
            };
            let next = widgets::list(c, t, f, y + gap, &rows);
            out.extend(hits(&next.rows, &acts));
            let y = widgets::footnote(c, t, f, next.bottom, nothing_written);
            afterwards_of(c, cx, y, false)
        }
        Outcome::Failed(failure) => {
            let (what, detail) = fmt::failure(l, failure, source);
            let y = widgets::list(c, t, f, top, &[standing(l.pick(installed::FAILED), RED, Icon::Install, &what)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[again, restart]);
            out.extend(hits(&next.rows, &[another, Action::RebootTo(Target::System)]));
            // The package is sent again, or installed or flashed again.
            let sent = matches!(source, Source::Computer | Source::Rescue);
            let warning = match (damage, sent) {
                (Some(Damage::System), true) => l.pick(installed::damage::SYSTEM_SEND).into(),
                (Some(Damage::System), false) => l.pick(installed::damage::SYSTEM_INSTALL).into(),
                (Some(Damage::Partition(p)), true) => installed::damage::partition_send(l, p),
                (Some(Damage::Partition(p)), false) => installed::damage::partition_flash(l, p),
                (Some(Damage::Script), true) => l.pick(installed::damage::SCRIPT_SEND).into(),
                (Some(Damage::Script), false) => l.pick(installed::damage::SCRIPT_INSTALL).into(),
                (None, _) => l.pick(installed::damage::NONE).to_string(),
            };
            let mut y = widgets::footnote(c, t, f, next.bottom, &warning);
            if let Some(d) = detail {
                y = widgets::footnote(c, t, f, y, &d);
            }
            let y = reflashed(c, cx, y);
            let y = afterwards_of(c, cx, y, false);
            script_output(c, cx, y, rows)
        }
    }
}

/// What comes after an install, where it is not the user's to choose:
/// rescue waits for the computer's next command, and a package the system
/// asked for, once in, has the phone leave on its own.
fn afterwards_of(c: &mut Canvas, cx: &Ctx, y: f32, installed: bool) -> f32 {
    let l = cx.state.lang;
    let leaving = match cx.state.asked.shutdown_after {
        true => texts::installed::leaving::POWER_OFF,
        false => texts::installed::leaving::REBOOT,
    };
    match cx.state.source {
        Source::Rescue => widgets::footnote(c, cx.t, cx.f, y, l.pick(texts::installing::MORE_COMMANDS)),
        Source::Asked { .. } if installed => widgets::footnote(c, cx.t, cx.f, y, l.pick(leaving)),
        _ => y,
    }
}

/// What the wipe page lists of [`WIPES`] beyond everything, under the
/// partition each is in.
const WIPE_GROUPS: [(Text, &[Wipe]); 2] = [
    (texts::wipe::part::DATA, &[Wipe::AppData, Wipe::Media, Wipe::AppCaches, Wipe::SystemCaches, Wipe::Art, Wipe::Logs, Wipe::Updates]),
    (texts::wipe::part::CACHE, &[Wipe::Cache]),
];

/// What a factory reset erases, as the wipe page tells it of its first
/// choice, and the page that carries one out for the system tells it.
fn factory_reset(s: &State) -> &'static str {
    s.lang.pick(match s.cache_partition {
        true => texts::wipe::factory_reset::WITH_CACHE,
        false => texts::wipe::factory_reset::WITHOUT_CACHE,
    })
}

/// A choice of the wipe page as it lists it: its title, and exactly what it
/// erases.
fn wipe_row(w: Wipe, s: &State) -> (&'static str, String) {
    use texts::wipe::{part, what};
    let l = s.lang;
    let (title, what): (Text, String) = match w {
        Wipe::All => (texts::wipe::ALL, factory_reset(s).into()),
        Wipe::AppData => (part::APP_DATA, l.pick(what::APP_DATA).into()),
        Wipe::Media => (part::MEDIA, l.pick(what::MEDIA).into()),
        Wipe::AppCaches => (part::APP_CACHES, l.pick(what::APP_CACHES).into()),
        Wipe::SystemCaches => (part::SYSTEM_CACHES, l.pick(what::SYSTEM_CACHES).into()),
        Wipe::Art => (part::ART, l.pick(what::ART).into()),
        Wipe::Logs => match s.phone_logs {
            0 => (part::LOGS, l.pick(what::LOGS).into()),
            n => (part::LOGS, what::logs_and_own(l, n as u64)),
        },
        Wipe::Updates => (part::UPDATES, l.pick(what::UPDATES).into()),
        Wipe::Cache => (texts::wipe::EVERYTHING, l.pick(what::CACHE).into()),
    };
    (l.pick(title), what)
}

/// What to erase: everything, as a factory reset does, or parts of it on
/// their own, the cache partition among them where the phone has one; then
/// a swipe erases what is chosen.
pub fn wipe(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let chosen = |w: Wipe| s.wipe[WIPES.iter().position(|&x| x == w).expect("every choice is one of WIPES")];
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::wipe::RESET));
    let (name, sub) = wipe_row(Wipe::All, s);
    let all = widgets::list(&mut c, t, f, y, &[Row::new(name).subtitle(&sub).trail(Trail::Check(chosen(Wipe::All)))]);
    out.extend(hits(&all.rows, &[Action::WipeItem(Wipe::All)]));
    let mut y = widgets::footnote(&mut c, t, f, all.bottom, l.pick(texts::wipe::STAYS));
    for (title, items) in WIPE_GROUPS {
        let shown: Vec<Wipe> = items.iter().copied().filter(|&w| w != Wipe::Cache || s.cache_partition).collect();
        if shown.is_empty() {
            continue;
        }
        let below = widgets::section(&mut c, t, f, y, l.pick(title));
        let lines: Vec<(&str, String)> = shown.iter().map(|&w| wipe_row(w, s)).collect();
        let rows: Vec<Row> =
            shown.iter().zip(&lines).map(|(&w, (name, sub))| Row::new(name).subtitle(sub).trail(Trail::Check(chosen(w)))).collect();
        let list = widgets::list(&mut c, t, f, below, &rows);
        out.extend(hits(&list.rows, &shown.iter().map(|&w| Action::WipeItem(w)).collect::<Vec<_>>()));
        y = list.bottom;
    }
    let y = widgets::footnote(&mut c, t, f, y, l.pick(texts::wipe::NOTE));
    let (label, danger) = confirm_label(Confirm::Wipe, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Wipe) });
    Frame::new(t, fit(c, t, track.y + track.h), out, l.pick(texts::page::WIPE), top)
}

/// A page laid out above the on-screen keyboard: where the keyboard's top
/// is, and its keys as touch targets.
fn keyboard_hits(c: &mut Canvas, t: &Theme, f: &Fonts, s: &State, enter: &str, out: &mut Vec<Hit>) -> f32 {
    let top = t.m.h as f32 - widgets::keyboard_height(t);
    let keys = widgets::keyboard(c, t, f, s.lang, top, s.layer, s.key_held(), enter);
    out.extend(keys.into_iter().map(|(rect, cap)| Hit { rect, action: Action::Type(cap) }));
    top
}

/// Finding a feature or partition: the field where the title would be, the
/// button that leaves the search beside it, what it finds, and the
/// keyboard.
pub fn search(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let m = &t.m;
    let mut c = page(cx);
    let mut out = Vec::new();
    let (at, cancel) = widgets::search_layout(t);
    let field = widgets::search_field(&mut c, t, f, at.x, at.w, at.y + at.h / 2.0, &s.search, l.pick(texts::search::HINT));
    widgets::cancel_button(&mut c, t, cancel);
    out.push(Hit { rect: cancel, action: Action::CloseSearch });
    let top = field.field.y + field.field.h + m.dp(12.0);
    let kb_top = m.h as f32 - widgets::keyboard_height(t);
    if s.search.trim().is_empty() {
        widgets::footnote(&mut c, t, f, top, l.pick(texts::search::HOW));
    } else {
        let found = search::find_on(&s.search, l, |screen| s.offers(screen));
        if found.is_empty() {
            let none = texts::search::none(l, s.search.trim());
            widgets::list(&mut c, t, f, top, &[Row::new(&none).trail(Trail::None)]);
        } else {
            let rows: Vec<Row> = found.iter().map(|e| Row::new(l.pick(e.title)).subtitle(l.pick(e.place))).collect();
            let list = widgets::list(&mut c, t, f, top, &rows);
            // The keyboard covers what does not fit above it.
            let shown = list.rows.iter().zip(&found).filter(|(r, _)| r.y + r.h <= kb_top);
            out.extend(shown.map(|(&rect, e)| Hit { rect, action: Action::Open(e.screen) }));
        }
    }
    let find = l.pick(texts::search::SEARCH);
    keyboard_hits(&mut c, t, f, s, find, &mut out);
    if let Some(rect) = field.clear {
        out.push(Hit { rect, action: Action::ClearSearch });
    }
    Frame { back: false, header: false, caret: Some(field.caret), ..Frame::new(t, c, out, find, 0.0) }
}

/// Backing up: what to take, where it goes, and how.
pub fn backup(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::backup::CONTENTS));
    let system = fmt::snapshot_backup_note(l, &i.snapshot).unwrap_or(l.pick(texts::backup::SYSTEM_HOLDS));
    let (data, media) = match s.data == Data::Open {
        true => (texts::backup::DATA_HOLDS, texts::backup::MEDIA_HOLDS),
        false => (texts::backup::DATA_HOLDS_LOCKED, texts::backup::MEDIA_HOLDS_LOCKED),
    };
    let boot = l.pick(texts::backup::BOOT_HOLDS);
    let modem = l.pick(texts::backup::MODEM_HOLDS);
    let subs = [boot, system, l.pick(data), l.pick(media), modem];
    let rows: Vec<Row> = BACKUP_PARTS
        .iter()
        .zip(subs)
        .enumerate()
        .map(|(k, (name, sub))| Row::new(l.pick(*name)).subtitle(sub).trail(Trail::Check(s.backup[k])))
        .collect();
    let parts = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&parts.rows, &[0, 1, 2, 3, 4].map(Action::BackupPart)));
    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(texts::backup::SAVE_TO));
    let storage = match (s.data.storage_open(), i.storage.free.map(fmt::gb)) {
        (true, Some(free)) => texts::backup::free(l, free),
        (true, None) => l.pick(texts::backup::FREE_UNKNOWN).to_string(),
        (false, _) if s.data == Data::Erased => l.pick(texts::data::ERASED).to_string(),
        (false, _) => l.pick(texts::decrypt::NEEDED).to_string(),
    };
    let usb = l.pick(match &s.usb_drive {
        Some(UsbDrive::Shown) => texts::backup::CONNECTED,
        Some(UsbDrive::Unreadable(_)) => texts::usb_drive::UNREADABLE,
        None => texts::usb_drive::WANTED,
    });
    let places = widgets::list(&mut c, t, f, y, &[
        Row::new(place_name(l, Place::Storage)).subtitle(&storage).trail(Trail::Check(s.backup_to == Place::Storage)),
        Row::new(place_name(l, Place::UsbDrive)).subtitle(usb).trail(Trail::Check(s.backup_to == Place::UsbDrive)),
    ]);
    out.extend(hits(&places.rows, &[Action::BackupTo(Place::Storage), Action::BackupTo(Place::UsbDrive)]));
    let y = widgets::section(&mut c, t, f, places.bottom, l.pick(texts::common::OPTIONS));
    let options = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(texts::backup::COMPRESS)).subtitle(l.pick(texts::backup::COMPRESS_NOTE)).trail(Trail::Switch(s.compress)),
        Row::new(l.pick(texts::backup::VERIFY)).subtitle(l.pick(texts::backup::VERIFY_NOTE)).trail(Trail::Switch(s.verify)),
    ]);
    out.extend(hits(&options.rows, &[Action::Toggle(Switch::Compress), Action::Toggle(Switch::Verify)]));
    let y = widgets::footnote(&mut c, t, f, options.bottom, l.pick(texts::backup::NOTE));
    let (label, danger) = confirm_label(Confirm::Backup, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Backup) });
    Frame::new(t, fit(c, t, track.y + track.h), out, l.pick(texts::page::BACKUP), top)
}

/// The backups found, newest first, to restore one; those that cannot be
/// read are listed after them, to be deleted.
pub fn restore(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::restore::BACKUPS));
    let bottom = if s.backups.is_empty() {
        widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::restore::NONE)).trail(Trail::None)]).bottom
    } else {
        let lines: Vec<(String, String)> = s
            .backups
            .iter()
            .map(|b| match &b.held {
                Ok(h) => (fmt::datetime(l, h.made, s.zone()), format!("{} · {} · {}", fmt::parts(l, &h.parts), fmt::bytes(h.size), place_name(l, b.place))),
                Err(_) => (b.name.clone(), format!("{} · {}", l.pick(texts::common::UNREADABLE), place_name(l, b.place))),
            })
            .collect();
        let rows: Vec<Row> = lines.iter().map(|(title, sub)| Row::new(title).subtitle(sub)).collect();
        let list = widgets::list(&mut c, t, f, y, &rows);
        out.extend(list.rows.iter().enumerate().map(|(k, &rect)| Hit { rect, action: Action::OpenBackup(k) }));
        list.bottom
    };
    let bottom = widgets::footnote(&mut c, t, f, bottom, l.pick(texts::restore::NOTE));
    Frame::new(t, fit(c, t, bottom), out, l.pick(texts::page::RESTORE), top)
}

/// One backup: when and where it was made, and which of its parts to
/// restore. One made on another phone is not restored here, and one that
/// cannot be read can only be deleted.
pub fn restore_backup(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let (entry, chosen) = s.restore.as_ref().expect("the restore page shows a backup");
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let location = l.pick(texts::common::LOCATION);
    let y = match &entry.held {
        Err(why) => {
            let facts = [(l.pick(texts::restore::FOLDER), entry.name.clone()), (location, place_name(l, entry.place).to_string())];
            let y = widgets::info_card(&mut c, t, f, top, &facts);
            widgets::footnote(&mut c, t, f, y, &texts::restore::unreadable(l, why))
        }
        Ok(held) => {
            let mut facts = vec![
                (l.pick(texts::restore::MADE), fmt::datetime(l, held.made, s.zone())),
                (l.pick(texts::common::SIZE), fmt::bytes(held.size)),
                (location, place_name(l, entry.place).to_string()),
            ];
            if let Some(system) = &held.system {
                facts.push((l.pick(texts::restore::SYSTEM_VERSION), system.clone()));
            }
            if let Some(slot) = held.slot {
                facts.push((l.pick(texts::slot::TITLE), slot.letter().to_string()));
            }
            let y = widgets::info_card(&mut c, t, f, top, &facts);
            if held.device != i.codename {
                widgets::footnote(&mut c, t, f, y, &texts::backup::refused::other_phone(l, &held.device))
            } else {
                let y = widgets::section(&mut c, t, f, y, l.pick(texts::restore::CONTENTS));
                let kept: Vec<usize> = (0..BACKUP_PARTS.len()).filter(|&k| held.parts[k]).collect();
                let rows: Vec<Row> = kept.iter().map(|&k| Row::new(l.pick(BACKUP_PARTS[k])).trail(Trail::Check(chosen[k]))).collect();
                let list = widgets::list(&mut c, t, f, y, &rows);
                out.extend(list.rows.iter().zip(&kept).map(|(&rect, &k)| Hit { rect, action: Action::RestorePart(k) }));
                let mut y = widgets::footnote(&mut c, t, f, list.bottom, l.pick(texts::restore::OVERWRITES));
                if chosen[DATA_PART] {
                    y = widgets::footnote(&mut c, t, f, y, l.pick(texts::restore::DATA_NOTE));
                }
                if chosen[MEDIA_PART] {
                    y = widgets::footnote(&mut c, t, f, y, l.pick(texts::restore::MEDIA_NOTE));
                }
                // The boot partitions' vbmeta checks the system's: one of
                // another version without the other may not start.
                if let (Some(was), Some(now)) = (&held.system, &i.os_version)
                    && was != now
                    && chosen[0] != chosen[1]
                {
                    y = widgets::footnote(&mut c, t, f, y, &texts::restore::other_version(l, was, now));
                }
                let (label, danger) = confirm_label(Confirm::Restore, cx.state);
                let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
                out.push(Hit { rect: track, action: Action::Confirm(Confirm::Restore) });
                track.y + track.h
            }
        }
    };
    let delete = widgets::list(&mut c, t, f, y + t.m.dp(28.0), &[Row::new(l.pick(texts::restore::DELETE)).tint(t.pal.danger).trail(Trail::None)]);
    out.extend(hits(&delete.rows, &[Action::AskDelete]));
    let mut bottom = delete.bottom;
    if s.asking_delete {
        let y = widgets::footnote(&mut c, t, f, delete.bottom, l.pick(texts::restore::ASK_DELETE));
        let (label, danger) = confirm_label(Confirm::DeleteBackup, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::DeleteBackup) });
        bottom = track.y + track.h;
    }
    Frame::new(t, fit(c, t, bottom), out, l.pick(texts::restore::TITLE), top)
}

/// The share of its pass the part under way has done.
fn part_share(a: &BackupAt) -> f32 {
    if a.of == 0 {
        1.0
    } else {
        share(a.done, a.of).min(1.0)
    }
}

/// How far backing up or restoring has come over all, 0 to 1, and the
/// bytes of the parts the pass under way has gone through, the part under
/// way by its share. Each part counts by its bytes in each pass: a restore
/// checks all its parts first, and a backup checks them last.
fn job_progress(job: &BackupJob, a: &BackupAt) -> (f32, u64) {
    let total: u64 = job.plan.iter().map(|(_, n)| n).sum();
    let before: u64 = job.plan[..a.item].iter().map(|(_, n)| n).sum();
    let passed = before as f64 + f64::from(part_share(a)) * job.plan[a.item].1 as f64;
    let second = job.checks && a.checking != job.restore;
    let passes = if job.checks { 2.0 } else { 1.0 };
    let over = (if second { total as f64 } else { 0.0 } + passed) / (passes * total.max(1) as f64);
    (over as f32, passed as u64)
}

/// What a backup or a restore does to a part as it goes through them:
/// takes it, puts it back, or checks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    BackUp,
    Restore,
    Verify,
}

impl Pass {
    /// The pass that writes: a restore's, or a backup's.
    fn writing(restore: bool) -> Pass {
        if restore { Pass::Restore } else { Pass::BackUp }
    }

    /// The pass a job is on at `a`.
    fn at(job: &BackupJob, a: &BackupAt) -> Pass {
        if a.checking { Pass::Verify } else { Pass::writing(job.restore) }
    }

    /// Under way: 正在备份, Backing up.
    fn doing(self) -> Text {
        match self {
            Pass::BackUp => texts::backup::job::BACKING_UP,
            Pass::Restore => texts::backup::job::RESTORING,
            Pass::Verify => texts::backup::job::VERIFYING,
        }
    }

    /// Done to a part: 已备份, Backed up.
    fn done(self) -> Text {
        match self {
            Pass::BackUp => texts::backup::job::BACKED_UP,
            Pass::Restore => texts::backup::job::RESTORED,
            Pass::Verify => texts::backup::job::VERIFIED,
        }
    }

    /// Under way on the part `name`: 正在备份 boot.
    fn doing_part(self, l: Lang, name: &str) -> String {
        match self {
            Pass::BackUp => texts::backup::job::backing_up_part(l, name),
            Pass::Restore => texts::backup::job::restoring_part(l, name),
            Pass::Verify => texts::backup::job::verifying_part(l, name),
        }
    }

    /// Under way on a part, `p` hundredths of it done: 正在备份 37%.
    fn doing_share(self, l: Lang, p: u32) -> String {
        match self {
            Pass::BackUp => texts::backup::job::backing_up_share(l, p),
            Pass::Restore => texts::backup::job::restoring_share(l, p),
            Pass::Verify => texts::backup::job::verifying_share(l, p),
        }
    }

    /// How much of the whole is done: 已备份 1.2GB，共 5.0GB.
    fn done_of(self, l: Lang, passed: &str, total: &str) -> String {
        match self {
            Pass::BackUp => texts::backup::job::backed_up_of(l, passed, total),
            Pass::Restore => texts::backup::job::restored_of(l, passed, total),
            Pass::Verify => texts::backup::job::verified_of(l, passed, total),
        }
    }
}

/// Where part `k` of a backup or restore stands, as its row says it.
fn part_state(l: Lang, job: &BackupJob, k: usize) -> String {
    use std::cmp::Ordering;
    use texts::backup::job as said;
    let wrote = Pass::writing(job.restore).done();
    let ended = job.outcome.is_some();
    let a = match (&job.outcome, job.at) {
        (Some(BackupEnd::Saved(_)), _) => return l.pick(if job.checks { said::VERIFIED } else { wrote }).to_string(),
        (Some(BackupEnd::Restored { .. }), _) => return l.pick(wrote).to_string(),
        (_, None) => return l.pick(if ended { said::NOT_STARTED } else { texts::common::WAITING }).to_string(),
        (_, Some(a)) => a,
    };
    let state = match k.cmp(&a.item) {
        Ordering::Less if a.checking => said::VERIFIED,
        Ordering::Less => wrote,
        Ordering::Equal if ended => texts::common::UNFINISHED,
        Ordering::Equal => return Pass::at(job, &a).doing_share(l, percent(part_share(&a))),
        // Ahead of the pass under way: a backup checks its parts once all
        // are written, and a restore writes them once all are checked.
        Ordering::Greater => match (a.checking, job.restore, ended) {
            (true, false, false) => wrote,
            (true, false, true) => said::NOT_VERIFIED,
            (false, false, false) | (true, true, false) => texts::common::WAITING,
            (false, false, true) => said::NOT_BACKED_UP,
            (false, true, false) => said::VERIFIED,
            (_, true, true) => said::NOT_RESTORED,
        },
    };
    l.pick(state).to_string()
}

/// Backing up or restoring: how far it has come over all, each part and
/// where it stands, and a way to stop while stopping breaks nothing. Once
/// it has ended, how, and where to go.
pub fn backup_job(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let job = s.backup_job.as_ref().expect("the backup page shows work");
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    use texts::backup::job as said;
    let icon = if job.restore { Icon::Restore } else { Icon::Backup };
    let writing = job.at.is_some_and(|a| !a.checking);
    let mut y = match &job.outcome {
        None => {
            let (title, detail, done) = match job.at {
                Some(a) => {
                    let (over, passed) = job_progress(job, &a);
                    let total = fmt::bytes(job.plan.iter().map(|(_, n)| n).sum());
                    let pass = Pass::at(job, &a);
                    (pass.doing_part(l, &job.plan[a.item].0), pass.done_of(l, &fmt::bytes(passed), &total), over)
                }
                None if job.restore => (l.pick(said::READING).to_string(), l.pick(said::CHECKING).to_string(), 0.0),
                None => (l.pick(texts::installing::GETTING_READY).to_string(), l.pick(said::SIZING).to_string(), 0.0),
            };
            widgets::progress_card(&mut c, t, f, top, &title, &detail, Some(done))
        }
        Some(end) => {
            let gone = l.pick(if job.restore { texts::installed::NOTHING_WRITTEN } else { said::NOTHING_SAVED });
            let failed = l.pick(if job.restore { said::RESTORE_FAILED } else { said::BACKUP_FAILED });
            let (title, color, detail) = match end {
                BackupEnd::Saved(name) => {
                    let saved = match job.place {
                        Place::Storage => said::saved_to_storage(l, name),
                        Place::UsbDrive => said::saved_to_usb_drive(l, name),
                    };
                    (l.pick(said::BACKUP_COMPLETE), GREEN, saved)
                }
                BackupEnd::Restored { .. } => (l.pick(said::RESTORE_COMPLETE), GREEN, l.pick(said::USES_RESTORED).to_string()),
                BackupEnd::Stopped => (l.pick(texts::common::STOPPED), SLATE, gone.to_string()),
                BackupEnd::Refused(r) => {
                    let title = l.pick(if job.restore { said::CANT_RESTORE } else { said::CANT_BACK_UP });
                    (title, ORANGE, fmt::backup_refusal(l, r, job.restore))
                }
                BackupEnd::Failed(_) if job.restore && writing => (failed, RED, l.pick(said::PARTLY_WRITTEN).to_string()),
                BackupEnd::Failed(_) => (failed, RED, gone.to_string()),
            };
            widgets::list(&mut c, t, f, top, &[standing(title, color, icon, &detail)]).bottom
        }
    };
    match &job.outcome {
        None => {
            let note = match (job.restore, job.at, job.place) {
                (false, _, Place::Storage) => said::BACKUP_KEEP_POWER,
                (false, _, Place::UsbDrive) => said::BACKUP_KEEP_USB_DRIVE,
                (true, Some(a), Place::Storage) if !a.checking => said::RESTORE_KEEP_POWER,
                (true, Some(a), Place::UsbDrive) if !a.checking => said::RESTORE_KEEP_USB_DRIVE,
                (true, _, _) => said::NOTHING_UNTIL_VERIFIED,
            };
            y = widgets::footnote(&mut c, t, f, y, l.pick(note));
            // A restore that has begun writing goes on to its end.
            if !(job.restore && writing) {
                let label = l.pick(if job.stopping { texts::common::STOPPING } else { texts::common::STOP });
                let stop = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
                if !job.stopping {
                    out.extend(hits(&stop.rows, &[Action::StopBackup]));
                }
                y = stop.bottom;
            }
        }
        Some(end) => {
            match end {
                BackupEnd::Restored { unmade } if !unmade.is_empty() => {
                    let note = said::users_gone(l, unmade.join(l.pick(texts::list::SEPARATOR)));
                    y = widgets::footnote(&mut c, t, f, y, &note);
                }
                BackupEnd::Failed(why) => y = widgets::footnote(&mut c, t, f, y, why),
                _ => {}
            }
            let (rows, acts): (Vec<Row>, Vec<Action>) = match end {
                BackupEnd::Restored { .. } => (
                    vec![Row::new(l.pick(texts::installed::REBOOT_TO_SYSTEM)).icon(ORANGE, Icon::Power), Row::new(l.pick(texts::common::DONE)).trail(Trail::None)],
                    vec![Action::RebootTo(Target::System), Action::Back],
                ),
                _ => (vec![Row::new(l.pick(texts::common::DONE)).trail(Trail::None)], vec![Action::Back]),
            };
            let next = widgets::list(&mut c, t, f, y + t.m.card_gap, &rows);
            out.extend(hits(&next.rows, &acts));
            y = next.bottom;
        }
    }
    // The parts come after what can be done, which a long list would push
    // off the screen.
    if !job.plan.is_empty() {
        let heading = l.pick(if job.restore { texts::restore::CONTENTS } else { texts::backup::CONTENTS });
        let y0 = widgets::section(&mut c, t, f, y, heading);
        let states: Vec<String> = (0..job.plan.len()).map(|k| part_state(l, job, k)).collect();
        let rows: Vec<Row> = job.plan.iter().zip(&states).map(|((name, _), state)| Row::new(name).trail(Trail::Text(state))).collect();
        y = widgets::list(&mut c, t, f, y0, &rows).bottom;
    }
    let title = l.pick(match (&job.outcome, job.restore) {
        (None, restore) => Pass::writing(restore).doing(),
        (Some(BackupEnd::Saved(_)), _) => said::BACKUP_COMPLETE,
        (Some(BackupEnd::Restored { .. }), _) => said::RESTORE_COMPLETE,
        (Some(_), false) => said::BACKUP_INCOMPLETE,
        (Some(_), true) => said::RESTORE_INCOMPLETE,
    });
    Frame { back: job.outcome.is_some(), ..Frame::new(t, fit(c, t, y), out, title, top) }
}

/// Tools for the experienced.
pub fn advanced(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = page(cx);
    let top = t.m.content_top;
    let tools = widgets::list(&mut c, t, f, top, &[
        Row::new(l.pick(texts::page::FILES)).icon(BLUE, Icon::Folder),
        Row::new(l.pick(texts::page::TERMINAL)).icon(SLATE, Icon::Terminal),
        Row::new(l.pick(texts::page::RELABEL)).icon(PURPLE, Icon::Shield),
        Row::new(l.pick(texts::page::FILESYSTEMS)).icon(RED, Icon::Mount),
        Row::new(l.pick(texts::page::LOGS)).icon(ORANGE, Icon::Log),
        Row::new(l.pick(texts::page::UPDATE)).icon(GREEN, Icon::Layers).trail(Trail::Value(update_value(cx))),
    ]);
    let out = hits(&tools.rows, &[
        Action::Open(Screen::Files),
        Action::Open(Screen::Terminal),
        Action::Open(Screen::Relabel),
        Action::Open(Screen::Filesystems),
        Action::Open(Screen::Logs),
        Action::Open(Screen::Update),
    ]);
    Frame::new(t, c, out, l.pick(texts::page::ADVANCED), top)
}

/// Giving the data partition's files the SELinux labels the installed
/// system's rules give them: what it does, and a swipe to start it; then
/// how far it has come, a way to stop it, and how it ended.
pub fn relabel(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let title = l.pick(texts::page::RELABEL);
    let Some(job) = &s.relabel else {
        let mut y = widgets::paragraph(&mut c, t, f, top, l.pick(texts::relabel::WHAT));
        let note = match s.data {
            Data::Open | Data::Locked { .. } => None,
            Data::Erased => Some(texts::relabel::DATA_ERASED),
            Data::Closed | Data::Opening | Data::Failed(_) => Some(texts::relabel::OPEN_FIRST),
        };
        if let Some(note) = note {
            y = widgets::footnote(&mut c, t, f, y, l.pick(note));
        }
        let (label, danger) = confirm_label(Confirm::Relabel, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::Relabel) });
        return Frame::new(t, c, out, title, top);
    };
    let repair = Verb::of(&crate::app::PartKind::Repair);
    let heading = match &job.outcome {
        None => repair.going(l),
        Some(JobEnd::Done) => repair.complete(l),
        Some(JobEnd::Stopped) => l.pick(texts::common::STOPPED).into(),
        Some(JobEnd::Failed(_)) => repair.failed(l),
    };
    let detail = match (job.total, job.checked) {
        (0, _) if job.outcome.is_none() => l.pick(texts::installing::GETTING_READY).to_string(),
        (total, checked) => texts::relabel::checked(l, checked, total),
    };
    let y = widgets::progress_card(&mut c, t, f, top, &heading, &detail, Some(share(job.checked, job.total)));
    match &job.outcome {
        None => {
            let label = l.pick(if job.stopping { texts::common::STOPPING } else { texts::common::STOP });
            let stop = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopRelabel]));
            }
        }
        Some(end) => {
            let changed = job.changed;
            let note = match end {
                JobEnd::Done if changed == 0 => l.pick(texts::relabel::NOTHING_NEEDED).to_string(),
                JobEnd::Done => texts::relabel::changed(l, changed),
                JobEnd::Stopped => texts::relabel::stopped(l, changed),
                JobEnd::Failed(why) => texts::relabel::failed(l, why),
            };
            let y = widgets::footnote(&mut c, t, f, y, &note);
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::common::DONE)).trail(Trail::None)]);
            out.extend(hits(&finish.rows, &[Action::Back]));
        }
    }
    Frame { back: job.outcome.is_some(), ..Frame::new(t, c, out, title, top) }
}

/// The partitions whose filesystems can be repaired or made afresh as
/// another, each with what it holds and its size.
pub fn filesystems(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::mount::PARTITIONS));
    let bottom = match &s.filesystems {
        None => widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::filesystems::READING)).trail(Trail::None)]).bottom,
        Some(Err(why)) => {
            let y = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::mount::UNREADABLE)).trail(Trail::None)]).bottom;
            widgets::footnote(&mut c, t, f, y, why)
        }
        Some(Ok(rows)) if rows.is_empty() => widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::filesystems::NONE)).trail(Trail::None)]).bottom,
        Some(Ok(rows)) => {
            let subs: Vec<String> = rows.iter().map(|r| format!("{} · {}", fs_held(l, r, &s.data), fmt::bytes(r.size))).collect();
            let shown: Vec<Row> = rows.iter().zip(&subs).map(|(r, sub)| Row::new(&r.name).subtitle(sub)).collect();
            let list = widgets::list(&mut c, t, f, y, &shown);
            out.extend(hits(&list.rows, &(0..rows.len()).map(Action::OpenFilesystem).collect::<Vec<_>>()));
            widgets::footnote(&mut c, t, f, list.bottom, l.pick(texts::filesystems::NOTE))
        }
    };
    Frame::new(t, fit(c, t, bottom), out, l.pick(texts::page::FILESYSTEMS), top)
}

/// What a partition holds, as its row and page name it: the data
/// partition's is read only once it is open, and until then the row says
/// where it stands, as the home page does.
fn fs_held(l: Lang, row: &FsRow, data: &Data) -> String {
    match (&row.filesystem, row.name == DATA_PARTITION) {
        (Some(filesystem), _) => filesystem.clone(),
        (None, true) => fmt::data(l, data).into(),
        (None, false) => l.pick(texts::filesystems::UNRECOGNIZED).into(),
    }
}

/// One partition: what it holds, what can be done to it, and a swipe to do
/// what is chosen.
pub fn filesystem(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let Some(Ok(rows)) = &s.filesystems else { unreachable!("a partition's page opens from the list read") };
    let row = &rows[s.fs_chosen];
    let data = row.name == DATA_PARTITION;
    let facts = [
        (l.pick(texts::filesystems::MOUNT_POINT), row.mount_point.clone()),
        (l.pick(texts::filesystems::FILESYSTEM), fs_held(l, row, &s.data)),
        (l.pick(texts::common::SIZE), fmt::bytes(row.size)),
    ];
    let y = widgets::info_card(&mut c, t, f, top, &facts);
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::common::ACTIONS));
    let chosen = |action: &FsAction| s.fs_action == *action;
    let remakes: Vec<String> = row.others.iter().map(|fs| texts::filesystems::change_to(l, fs)).collect();
    let made: Vec<String> = row.others.iter().map(|_| remade(row, s)).collect();
    let repair = Row::new(l.pick(texts::filesystems::REPAIR))
        .subtitle(l.pick(texts::filesystems::REPAIR_NOTE))
        .trail(Trail::Check(chosen(&FsAction::Repair)));
    let mut shown = vec![repair];
    let mut actions = vec![Action::ChooseFs(FsChoice::Repair)];
    for (k, fs) in row.others.iter().enumerate() {
        shown.push(Row::new(&remakes[k]).subtitle(&made[k]).trail(Trail::Check(chosen(&FsAction::Remake(fs.clone())))));
        actions.push(Action::ChooseFs(FsChoice::Remake(k)));
    }
    let list = widgets::list(&mut c, t, f, y, &shown);
    out.extend(hits(&list.rows, &actions));
    let mut y = list.bottom;
    if row.others.is_empty() {
        y = widgets::footnote(&mut c, t, f, y, l.pick(texts::filesystems::CANT_CHANGE));
    }
    if data {
        y = widgets::footnote(&mut c, t, f, y, l.pick(texts::filesystems::DATA_CLOSES));
    }
    if !row.others.is_empty() {
        y = widgets::footnote(&mut c, t, f, y, l.pick(texts::filesystems::CANT_UNDO));
    }
    let (label, danger) = confirm_label(Confirm::Filesystem, s);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Filesystem) });
    Frame::new(t, fit(c, t, track.y + track.h), out, partition_title(l, &row.name), top)
}

/// The title of the page of a partition the filesystem page lists.
fn partition_title(l: Lang, name: &str) -> &'static str {
    l.pick(match name {
        DATA_PARTITION => texts::wipe::part::DATA,
        "metadata" => texts::wipe::part::METADATA,
        "cache" => texts::wipe::part::CACHE,
        _ => texts::flash::PARTITION,
    })
}

/// What making `row` afresh as another filesystem erases: the data
/// partition goes as a factory reset goes, with /metadata and the cache
/// partition beside it.
fn remade(row: &FsRow, s: &State) -> String {
    match (row.name == DATA_PARTITION, s.cache_partition) {
        (true, true) => s.lang.pick(texts::filesystems::FORMATS_DATA_AND_CACHE).into(),
        (true, false) => s.lang.pick(texts::filesystems::FORMATS_DATA).into(),
        (false, _) => texts::filesystems::formats(s.lang, &row.name),
    }
}

/// Work on whole partitions: each partition and where it stands, what the
/// program said of the last, and once it has ended, how. What the system
/// asked for leaves on its own once it went through every partition, and
/// offers where to go otherwise; what the user chose goes back to its page.
pub fn part_work(cx: &Ctx) -> Frame {
    use crate::app::{ItemEnd, ItemState, PartKind};
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let work = s.part_work.as_ref().expect("the page of partition work shows work");
    let mut c = long_page(cx);
    let (mut out, mut rows) = (Vec::new(), None);
    let top = t.m.content_top;
    let verb = Verb::of(&work.kind);
    // Work on one partition fails; work through several may get partway.
    let (title, several) = match &work.kind {
        PartKind::Disable => (texts::parts::title::WIPE, true),
        PartKind::Unshare => (texts::parts::title::UNSHARE, true),
        PartKind::Repair => (texts::parts::title::REPAIR, false),
        PartKind::Remake(_) => (texts::parts::title::FORMAT, false),
    };
    let failed = |state: &ItemState| matches!(state, ItemState::Ended(ItemEnd::Failed(_)));
    let short = matches!(work.outcome, Some(Err(_))) || work.items.iter().any(|(_, state)| failed(state));
    let heading = match (&work.outcome, short, several) {
        (None, _, _) => verb.going(l),
        (Some(_), false, _) => verb.complete(l),
        (Some(_), true, true) => verb.incomplete(l),
        (Some(_), true, false) => verb.failed(l),
    };
    let mut y = widgets::section_first(&mut c, t, f, top, &heading);
    if !work.items.is_empty() {
        let states: Vec<String> = work
            .items
            .iter()
            .map(|(_, state)| match state {
                ItemState::Waiting if work.outcome.is_some() => verb.not_done(l),
                ItemState::Waiting => l.pick(texts::common::WAITING).into(),
                ItemState::Working => verb.going(l),
                ItemState::Ended(end) => fmt::item_end(l, end, verb),
            })
            .collect();
        let held: Vec<Option<&str>> = work
            .items
            .iter()
            .map(|(_, state)| match state {
                ItemState::Ended(ItemEnd::NotShared(fs)) if !fs.is_empty() => Some(fs.as_str()),
                _ => None,
            })
            .collect();
        let shown: Vec<Row> = work
            .items
            .iter()
            .zip(&states)
            .zip(&held)
            .map(|(((name, _), state), held)| {
                let row = Row::new(name).trail(Trail::Text(state));
                match held {
                    Some(fs) => row.subtitle(fs),
                    None => row,
                }
            })
            .collect();
        y = widgets::list(&mut c, t, f, y, &shown).bottom;
    }
    let leaving = match s.asked.shutdown_after {
        true => texts::parts::leaving::POWER_OFF,
        false => texts::parts::leaving::REBOOT,
    };
    let mut notes: Vec<String> = Vec::new();
    match &work.outcome {
        None if work.asked => notes.push(l.pick(leaving).into()),
        None => notes.push(verb.hold_on(l)),
        Some(Err(stop)) => {
            let (what, detail) = fmt::part_stop(l, stop, &cx.info.codename);
            notes.push(what);
            notes.extend(detail);
        }
        Some(Ok(())) => {}
    }
    for (name, state) in &work.items {
        if let ItemState::Ended(ItemEnd::Failed(Some(why))) = state {
            notes.push(texts::common::with_reason(l, verb.failed_on(l, name), why));
        }
    }
    for note in &notes {
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if let (true, Some(reason)) = (work.asked, &s.asked.reason) {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(texts::installing::REASON), reason.clone())]);
    }
    match (&work.outcome, work.asked) {
        (None, _) => {}
        (Some(_), true) if !short => {}
        // The system's work ran without the slot's partitions, which going
        // on in the recovery brings up first, as leaving fastboot does.
        (Some(_), true) => {
            let below = widgets::section(&mut c, t, f, y, l.pick(texts::common::ACTIONS));
            let rows: Vec<Row> = AFTERWARDS.iter().map(|&(title, _)| Row::new(l.pick(title)).trail(Trail::None)).collect();
            let actions = AFTERWARDS.map(|(_, action)| if action == Action::Dismiss { Action::EnterRecovery } else { action });
            let choices = widgets::list(&mut c, t, f, below, &rows);
            out.extend(hits(&choices.rows, &actions));
            y = choices.bottom;
        }
        (Some(_), false) => {
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::common::DONE)).trail(Trail::None)]);
            out.extend(hits(&finish.rows, &[Action::Back]));
            y = finish.bottom;
        }
    }
    if !work.output.is_empty() {
        let below = widgets::section(&mut c, t, f, y, l.pick(texts::parts::OUTPUT));
        rows = Some(Rows { edges: widgets::output_edges(&c, t, f, below, &work.output), action: None });
        y = below;
    }
    let back = work.outcome.is_some() && !work.asked;
    Frame { rows, back, ..Frame::new(t, fit(c, t, y), out, l.pick(title), top) }
}

/// The system on the phone now; the newest system the phone maker's
/// server knows, and downloading its full package to install it; and
/// where an update the system installed stands, merging it into the slot it
/// went to: how far a merge under way has come, and a swipe to merge one
/// that waits there for the system to start it.
pub fn update(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mut out = Vec::new();
    if let Standing::Merging(done) = s.update {
        let share = done as f32;
        let detail = texts::common::share_done(l, percent(share));
        let y = widgets::progress_card(&mut c, t, f, top, l.pick(texts::snapshot::MERGING), &detail, Some(share));
        let y = widgets::footnote(&mut c, t, f, y, l.pick(texts::update::MERGING_NOTE));
        return Frame::new(t, fit(c, t, y), out, l.pick(texts::page::UPDATE), top);
    }
    let mut y = top;
    if let Some(v) = &i.os_version {
        y = widgets::section_first(&mut c, t, f, y, l.pick(texts::common::THIS_PHONE));
        y = widgets::info_card(&mut c, t, f, y, &[(l.pick(texts::update::CURRENT_VERSION), v.clone())]);
    }
    if s.update_server {
        let online_update = l.pick(texts::update::ONLINE);
        y = if y == top { widgets::section_first(&mut c, t, f, y, online_update) } else { widgets::section(&mut c, t, f, y, online_update) };
        y = online(&mut c, cx, y, &mut out);
    }
    let next = match &s.boot_slots {
        Some(Ok(slots)) => Some(slots.active),
        _ => None,
    };
    let (title, detail) = fmt::update(l, &s.update, next);
    let color = match s.update {
        Standing::None => SLATE,
        Standing::Merged => GREEN,
        Standing::Failed(_) | Standing::Unknown => RED,
        _ => ORANGE,
    };
    let mut row = Row::new(title).icon(color, Icon::Layers).trail(Trail::None);
    if let Some(detail) = &detail {
        row = row.subtitle(detail);
    }
    let merging = l.pick(texts::update::MERGING);
    y = if y == top { widgets::section_first(&mut c, t, f, y, merging) } else { widgets::section(&mut c, t, f, y, merging) };
    y = widgets::list(&mut c, t, f, y, &[row]).bottom;
    if s.update == Standing::Waiting {
        let (label, danger) = confirm_label(Confirm::Merge, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::Merge) });
        y = widgets::footnote(&mut c, t, f, track.y + track.h, l.pick(texts::update::CANT_GO_BACK));
    }
    Frame::new(t, fit(c, t, y), out, l.pick(texts::page::UPDATE), top)
}

/// Asking the update server for the newest system, then its full package:
/// downloading it, how far that has come, and installing it once it is
/// whole; then what to note before installing it.
fn online(c: &mut Canvas, cx: &Ctx, y: f32, out: &mut Vec<Hit>) -> f32 {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let release = match &s.check {
        Check::Asking => return widgets::list(c, t, f, y, &[Row::new(l.pick(texts::update::CHECKING)).trail(Trail::None)]).bottom,
        Check::Answered(Some(r)) => r,
        other => {
            let mut row = Row::new(l.pick(texts::update::CHECK));
            if !s.online() {
                row = row.subtitle(l.pick(texts::update::WLAN_FIRST));
            }
            let check = widgets::list(c, t, f, y, &[row]);
            out.extend(hits(&check.rows, &[Action::CheckUpdate]));
            return match other {
                Check::Failed(why) => widgets::footnote(c, t, f, check.bottom, &fmt::unanswered(l, why)),
                Check::Answered(None) => widgets::footnote(c, t, f, check.bottom, l.pick(texts::update::NONE_KNOWN)),
                _ => check.bottom,
            };
        }
    };
    let newer = i.os_version.as_deref() != Some(release.version.as_str());
    let title = if newer { texts::update::new_version_named(l, &release.version) } else { l.pick(texts::update::UP_TO_DATE).to_string() };
    let size = match release.length {
        Some(length) => fmt::file_size(length),
        None => fmt::package_size(&release.size),
    };
    let package = texts::update::full_package(l, size);
    let mut y = widgets::list(c, t, f, y, &[Row::new(&title).icon(GREEN, Icon::Layers).subtitle(&package).trail(Trail::None)]).bottom;
    match (&s.download, s.have) {
        (Some(job), _) if job.outcome.is_none() => {
            let (heading, detail, share) = match job.progress {
                None => (l.pick(texts::installing::GETTING_READY), String::new(), None),
                Some(p) => downloaded(l, p, job.rate),
            };
            y = widgets::progress_card(c, t, f, y + t.m.card_gap, heading, &detail, share);
            let label = l.pick(if job.stopping { texts::update::PAUSING } else { texts::update::PAUSE });
            let stop = widgets::list(c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopDownload]));
            }
            y = stop.bottom;
        }
        (_, Have::Whole(_)) => {
            let row = Row::new(l.pick(texts::page::INSTALL)).subtitle(l.pick(texts::update::DOWNLOADED_TO_STORAGE));
            let install = widgets::list(c, t, f, y + t.m.card_gap, &[row]);
            out.extend(hits(&install.rows, &[Action::InstallDownloaded]));
            y = install.bottom;
        }
        (job, have) => {
            match job.as_ref().and_then(|j| j.outcome.as_ref()) {
                Some(DownloadEnd::Failed(why)) => y = widgets::footnote(c, t, f, y, &fmt::download_failure(l, why)),
                Some(DownloadEnd::Stopped) => y = widgets::footnote(c, t, f, y, l.pick(texts::update::PAUSED)),
                _ => {}
            }
            let title = l.pick(if matches!(have, Have::Part(_)) { texts::update::RESUME } else { texts::update::DOWNLOAD });
            let had = match have {
                Have::Part(done) => Some(texts::update::downloaded(l, fmt::file_size(done))),
                _ => None,
            };
            let sub = if !s.data.storage_open() {
                Some(l.pick(texts::decrypt::NEEDED))
            } else if !s.online() {
                Some(l.pick(texts::update::WLAN_FIRST))
            } else {
                had.as_deref()
            };
            let row = Row::new(title);
            let download = widgets::list(c, t, f, y + t.m.card_gap, &[sub.map_or(row, |sub| row.subtitle(sub))]);
            out.extend(hits(&download.rows, &[Action::Download]));
            y = download.bottom;
        }
    }
    let saved = texts::update::saved_in(l, DOWNLOADS);
    let points = [l.pick(texts::update::KEEPS_DATA), saved.as_str(), l.pick(texts::update::RESUMABLE), l.pick(texts::update::OPTIMIZES)];
    widgets::notes(c, t, f, y + t.m.card_gap, l.pick(texts::common::NOTES), &points)
}

/// What a progress card says of a download, and the share of it done:
/// what an earlier download left being checked, or the rest coming at
/// `rate` bytes a second.
fn downloaded(l: Lang, progress: Progress, rate: u64) -> (&'static str, String, Option<f32>) {
    match progress {
        Progress::Checking { done, total } => (l.pick(texts::update::CHECKING_PART), format!("{} / {}", fmt::file_size(done), fmt::file_size(total)), Some(share(done, total))),
        Progress::Downloading { done, total } => (
            l.pick(texts::update::DOWNLOADING),
            format!("{} / {} · {}/s", fmt::file_size(done), fmt::file_size(total), fmt::file_size(rate)),
            Some(share(done, total)),
        ),
    }
}

/// Skyward's own update: the build running now; Skyward's newest release
/// on GitHub, and updating to it; then what to note before updating.
pub fn skyward_update(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mut out = Vec::new();
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::common::THIS_PHONE));
    let y = widgets::info_card(&mut c, t, f, y, &[(l.pick(texts::update::CURRENT_VERSION), VERSION.to_string())]);
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::update::ONLINE));
    let y = skyward_online(&mut c, cx, y, &mut out);
    let points = [l.pick(texts::skyward_update::FROM_GITHUB), l.pick(texts::skyward_update::BOTH_SLOTS), l.pick(texts::skyward_update::REBOOT_TO_USE)];
    let y = widgets::notes(&mut c, t, f, y + t.m.card_gap, l.pick(texts::common::NOTES), &points);
    Frame::new(t, fit(c, t, y), out, l.pick(texts::page::SKYWARD_UPDATE), top)
}

/// Asking GitHub for Skyward's newest release, then updating to it: its
/// build for the phone downloaded and written into the recovery partitions,
/// how far that has come, and how it ended.
fn skyward_online(c: &mut Canvas, cx: &Ctx, y: f32, out: &mut Vec<Hit>) -> f32 {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let gap = t.m.card_gap;
    let newest = match &s.skyward_check {
        SkywardCheck::Asking => return widgets::list(c, t, f, y, &[Row::new(l.pick(texts::update::CHECKING)).trail(Trail::None)]).bottom,
        SkywardCheck::Answered(n) => n,
        other => {
            let mut row = Row::new(l.pick(texts::update::CHECK));
            if !s.online() {
                row = row.subtitle(l.pick(texts::update::WLAN_FIRST));
            }
            let check = widgets::list(c, t, f, y, &[row]);
            out.extend(hits(&check.rows, &[Action::CheckSkyward]));
            return match other {
                SkywardCheck::Failed(why) => widgets::footnote(c, t, f, check.bottom, &fmt::unanswered(l, why)),
                _ => check.bottom,
            };
        }
    };
    let version = &newest.version;
    let (title, detail) = match (newest.newer, &newest.build) {
        (true, Some(build)) => (texts::update::new_version_named(l, version), texts::skyward_update::package(l, fmt::file_size(build.size))),
        (true, None) => (texts::update::new_version_named(l, version), l.pick(texts::skyward_update::NO_PACKAGE).to_string()),
        (false, _) => (l.pick(texts::update::UP_TO_DATE).to_string(), texts::skyward_update::newest_is(l, version)),
    };
    let y = widgets::list(c, t, f, y, &[Row::new(&title).icon(GREEN, Icon::Install).subtitle(&detail).trail(Trail::None)]).bottom;
    if !newest.newer || newest.build.is_none() {
        return y;
    }
    match &s.skyward_job {
        Some(job) if job.outcome.is_none() => {
            let (heading, detail, share) = match job.stage {
                None => (l.pick(texts::installing::GETTING_READY), String::new(), None),
                Some(SkywardStage::Download(p)) => downloaded(l, p, job.rate),
                Some(SkywardStage::Write { done, total }) => {
                    let part = share(done, total);
                    (l.pick(texts::skyward_update::WRITING), texts::common::share_done(l, percent(part)), Some(part))
                }
            };
            let y = widgets::progress_card(c, t, f, y + gap, heading, &detail, share);
            if matches!(job.stage, Some(SkywardStage::Write { .. })) {
                return widgets::footnote(c, t, f, y, l.pick(texts::skyward_update::KEEP_POWER));
            }
            let stop = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(if job.stopping { texts::common::STOPPING } else { texts::common::STOP })).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopSkywardUpdate]));
            }
            stop.bottom
        }
        Some(SkywardJob { outcome: Some(SkywardEnd::Updated(slots)), .. }) => {
            let detail = match slots.as_slice() {
                [slot] => texts::skyward_update::in_one(l, version, slot.letter()),
                [a, b] => texts::skyward_update::in_two(l, version, a.letter(), b.letter()),
                other => unreachable!("a phone has two slots, and Skyward went into {other:?}"),
            };
            let y = widgets::list(c, t, f, y + gap, &[standing(l.pick(texts::skyward_update::SUCCEEDED), GREEN, Icon::Install, &detail)]).bottom;
            let restart = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(texts::installed::REBOOT_TO_RECOVERY)).icon(ORANGE, Icon::Power)]);
            out.extend(hits(&restart.rows, &[Action::RebootTo(Target::Recovery)]));
            restart.bottom
        }
        job => {
            let mut y = y;
            match job.as_ref().and_then(|j| j.outcome.as_ref()) {
                Some(SkywardEnd::Stopped) => y = widgets::footnote(c, t, f, y, l.pick(texts::skyward_update::STOPPED)),
                Some(SkywardEnd::Download(why)) => y = widgets::footnote(c, t, f, y, &fmt::skyward_download_failure(l, why)),
                Some(SkywardEnd::NotWritten(why)) => {
                    y = widgets::footnote(c, t, f, y, l.pick(texts::skyward_update::NOT_WRITTEN));
                    y = widgets::footnote(c, t, f, y, why);
                }
                Some(SkywardEnd::WriteFailed(slots, why)) => {
                    let warning = match slots.as_slice() {
                        [slot] => texts::skyward_update::partly_one(l, slot.letter()),
                        [a, b] => texts::skyward_update::partly_two(l, a.letter(), b.letter()),
                        other => unreachable!("a phone has two slots, and Skyward was written to {other:?}"),
                    };
                    y = widgets::footnote(c, t, f, y, &warning);
                    y = widgets::footnote(c, t, f, y, why);
                }
                Some(SkywardEnd::Updated(_)) => unreachable!("an update that went in shows how it did"),
                None => {}
            }
            let mut row = Row::new(l.pick(texts::skyward_update::DOWNLOAD_AND_UPDATE));
            if !s.online() {
                row = row.subtitle(l.pick(texts::update::WLAN_FIRST));
            }
            let update = widgets::list(c, t, f, y + gap, &[row]);
            out.extend(hits(&update.rows, &[Action::UpdateSkyward]));
            update.bottom
        }
    }
}

/// The WLAN switch; then the network chosen and how joining it goes, and
/// the networks in range, strongest first.
pub fn wlan(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let w = s.wlan.as_ref().expect("the WLAN page shows on a phone whose WLAN is brought up");
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let on = !matches!(w.power, Power::Off | Power::Failed(_));
    let switch = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(texts::page::WLAN)).trail(Trail::Switch(on))]);
    out.extend(hits(&switch.rows, &[Action::Toggle(Switch::Wlan)]));
    let mut y = switch.bottom;
    match &w.power {
        Power::Off => y = widgets::footnote(&mut c, t, f, y, l.pick(texts::wlan::TURN_ON_NOTE)),
        Power::Failed(why) => y = widgets::footnote(&mut c, t, f, y, &texts::wlan::cant_turn_on(l, why)),
        Power::Starting => y = widgets::footnote(&mut c, t, f, y, l.pick(texts::wlan::TURNING_ON_WLAN)),
        Power::On => {
            let locked = |s: Security| !matches!(s, Security::Open | Security::Owe);
            // The network chosen stands out in the accent, as the stock
            // page shows the one it is on.
            if let Some(j) = &w.joined {
                let found = w.networks.iter().find(|n| n.ssid == j.ssid);
                let (signal, bands) = match (&j.stage, found) {
                    (Joining::Connected(link), found) => (Some(link.signal), found.map_or(Bands::of(link.frequency), |n| n.bands)),
                    (_, Some(n)) => (Some(n.signal), n.bands),
                    (_, None) => (None, Bands::default()),
                };
                let tag = fmt::band_tag(bands);
                let mut row = Row::new(&j.name).fan(signal.map_or(0, fmt::signal_level)).subtitle(fmt::joining(l, &j.stage));
                row = row.trail(Trail::Network { locked: locked(j.security) });
                if let Some(tag) = &tag {
                    row = row.tag(tag);
                }
                let joined = widgets::list_toned(&mut c, t, f, y + t.m.card_gap, &[row], &Tone::lit(t));
                out.extend(hits(&joined.rows, &[Action::OpenJoined]));
                out.push(Hit { rect: widgets::round_button(t, joined.rows[0]), action: Action::OpenJoined });
                y = joined.bottom;
                if let Joining::Connected(link) = &j.stage
                    && let Some(note) = link.reach.as_ref().and_then(|r| fmt::reach(l, r))
                {
                    y = widgets::footnote(&mut c, t, f, y, &note);
                }
            }
            let others: Vec<(usize, &Network)> =
                w.networks.iter().enumerate().filter(|(_, n)| w.joined.as_ref().is_none_or(|j| j.ssid != n.ssid)).collect();
            let (below, rescan) = widgets::section_action(&mut c, t, f, y, l.pick(texts::wlan::NEARBY), l.pick(texts::wlan::REFRESH));
            out.push(Hit { rect: rescan, action: Action::Rescan });
            y = below;
            if others.is_empty() {
                y = widgets::footnote(&mut c, t, f, y - t.m.dp(10.0), l.pick(texts::wlan::LOOKING)) + t.m.card_gap;
            }
            let tags: Vec<Option<String>> = others.iter().map(|(_, n)| fmt::band_tag(n.bands)).collect();
            let mut rows: Vec<Row> = others
                .iter()
                .zip(&tags)
                .map(|((_, n), tag)| {
                    let row = Row::new(&n.name).fan(fmt::signal_level(n.signal)).trail(Trail::Network { locked: locked(n.security) });
                    match tag {
                        Some(tag) => row.tag(tag),
                        None => row,
                    }
                })
                .collect();
            // The list ends with adding a network by hand, as the stock one
            // does.
            rows.push(Row::new(l.pick(texts::wlan::ADD_NETWORK)).tint(t.pal.accent).trail(Trail::None));
            let list = widgets::list(&mut c, t, f, y, &rows);
            let mut picks: Vec<Action> = others.iter().map(|(k, _)| Action::JoinNetwork(*k)).collect();
            picks.push(Action::AddNetwork);
            out.extend(hits(&list.rows, &picks));
            // The round buttons come after the rows they sit on, and a touch
            // finds the last control under it.
            for (rect, (k, _)) in list.rows.iter().zip(&others) {
                out.push(Hit { rect: widgets::round_button(t, *rect), action: Action::NetworkDetails(*k) });
            }
            y = list.bottom;
        }
    }
    Frame::new(t, fit(c, t, y), out, l.pick(texts::page::WLAN), top)
}

/// The password of the network chosen: its name, what is typed, a switch
/// to show it, and the keyboard.
pub fn wlan_password(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let network = s.joining.as_ref().expect("the password page is for a network");
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, &network.name);
    let shown = if s.show_password { s.password.clone() } else { "•".repeat(s.password.chars().count()) };
    let (y, caret) = widgets::text_field(&mut c, t, f, y, &shown);
    let show = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::wlan::SHOW_PASSWORD)).trail(Trail::Switch(s.show_password))]);
    out.extend(hits(&show.rows, &[Action::Toggle(Switch::ShowPassword)]));
    keyboard_hits(&mut c, t, f, s, l.pick(texts::wlan::CONNECT), &mut out);
    Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(texts::wlan::ENTER_PASSWORD), top) }
}

/// A network added by hand: its name, how it is protected, its password
/// where it takes one with the switch that shows it, and the keyboard,
/// which types into the field touched. The protections open over the page
/// from their row, which alone then answers a touch.
pub fn wlan_add(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let m = &t.m;
    let a = s.adding.as_ref().expect("the page adds a network");
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = m.content_top;
    let field = |y: f32| Rect::new(m.margin, y, m.w as f32 - m.margin * 2.0, m.dp(56.0));
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::wlan::NETWORK_NAME));
    let (below, mut caret) = widgets::text_field(&mut c, t, f, y, &a.name);
    out.push(Hit { rect: field(y), action: Action::TypeInto(Field::Name) });
    // Held down while its list is open, as the stock row stays.
    let tone = if a.choosing { Tone::page(&t.pressed()) } else { Tone::page(t) };
    let protection = [Row::new(l.pick(texts::wlan::SECURITY_TITLE)).trail(Trail::Choice(fmt::security(l, a.security)))];
    let security = widgets::list_toned(&mut c, t, f, below + m.card_gap, &protection, &tone);
    out.extend(hits(&security.rows, &[Action::ChooseSecurity]));
    if a.security.password() {
        let y = widgets::section(&mut c, t, f, security.bottom, l.pick(texts::wlan::PASSWORD));
        let shown = if s.show_password { s.password.clone() } else { "•".repeat(s.password.chars().count()) };
        let (below, typed) = widgets::text_field(&mut c, t, f, y, &shown);
        out.push(Hit { rect: field(y), action: Action::TypeInto(Field::Password) });
        if a.field == Field::Password {
            caret = typed;
        }
        let show = widgets::list(&mut c, t, f, below + m.card_gap, &[Row::new(l.pick(texts::wlan::SHOW_PASSWORD)).trail(Trail::Switch(s.show_password))]);
        out.extend(hits(&show.rows, &[Action::Toggle(Switch::ShowPassword)]));
    }
    keyboard_hits(&mut c, t, f, s, l.pick(texts::wlan::CONNECT), &mut out);
    if !a.choosing {
        return Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(texts::wlan::ADD_NETWORK), top) };
    }
    // A touch anywhere off the list closes it.
    out.clear();
    out.push(Hit { rect: Rect::new(0.0, 0.0, m.w as f32, m.h as f32), action: Action::CloseChoices });
    let names = ADDABLE.map(|p| fmt::security(l, p));
    let chosen = ADDABLE.iter().position(|&p| p == a.security);
    let held = ADDABLE.iter().position(|&p| s.held == Some(Action::PickSecurity(p)));
    let list = widgets::choices(&mut c, t, f, security.rows[0], &names, chosen, held);
    out.extend(hits(&list, &ADDABLE.map(Action::PickSecurity)));
    Frame::new(t, c, out, l.pick(texts::wlan::ADD_NETWORK), top)
}

/// A network's details, in the stock page's grid: for the one chosen, how
/// joining it goes and what it gave, and forgetting it; for any other, how
/// strong and how protected it is.
pub fn wlan_network(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let network = s.details.as_ref().expect("the details page is of a network");
    let joined = s.wlan.as_ref().and_then(|w| w.joined.as_ref()).filter(|j| j.ssid == network.ssid);
    let y = widgets::section_first(&mut c, t, f, top, &network.name);
    let detail = |icon, title, value: String| Detail { icon, title, value };
    let security = detail(Outline::Shield, l.pick(texts::wlan::SECURITY_TITLE), fmt::security(l, network.security).to_string());
    let signal = l.pick(texts::wlan::SIGNAL_STRENGTH);
    let title = l.pick(texts::wlan::DETAILS);
    let Some(j) = joined else {
        let items = [
            detail(Outline::Wifi, l.pick(texts::wlan::BANDS), fmt::bands(l, network.bands)),
            detail(Outline::Signal, signal, fmt::signal(l, network.signal).to_string()),
            security,
        ];
        let y = widgets::detail_grid(&mut c, t, f, y, &items);
        return Frame::new(t, fit(c, t, y), out, title, top);
    };
    let mut items = vec![detail(Outline::Info, l.pick(texts::wlan::STATUS), fmt::joining(l, &j.stage).to_string())];
    match &j.stage {
        Joining::Connected(link) => {
            if let Some(n) = link.generation {
                items.push(detail(Outline::Wifi, l.pick(texts::wlan::STANDARD), fmt::generation(l, n)));
            }
            if let Some(speed) = link.speed {
                items.push(detail(Outline::Speed, l.pick(texts::wlan::LINK_SPEED), format!("{speed} Mbps")));
            }
            items.push(detail(Outline::Signal, signal, fmt::signal(l, link.signal).to_string()));
            items.push(security);
            items.push(detail(Outline::Address, l.pick(texts::wlan::IP_ADDRESS), link.address.to_string()));
            items.push(detail(Outline::Mask, l.pick(texts::wlan::SUBNET_MASK), link.mask.to_string()));
            if let Some(router) = link.router {
                items.push(detail(Outline::Router, l.pick(texts::wlan::ROUTER), router.to_string()));
            }
        }
        _ => {
            items.push(detail(Outline::Signal, signal, fmt::signal(l, network.signal).to_string()));
            items.push(security);
        }
    }
    let y = widgets::detail_grid(&mut c, t, f, y, &items);
    let forget = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::wlan::FORGET)).tint(t.pal.danger).trail(Trail::None)]);
    out.extend(hits(&forget.rows, &[Action::LeaveNetwork]));
    Frame::new(t, fit(c, t, forget.bottom), out, title, top)
}

/// The file manager, every entry of the folder shown, folders first.
pub fn files(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    // An entry picked to copy or move goes to the folder shown, from the
    // bar held at the bottom.
    let dock = s.clip.as_ref().map(|clip| {
        let go = l.pick(match clip.op {
            FileOp::Copy => texts::files::COPY_HERE,
            FileOp::Move => texts::files::MOVE_HERE,
        });
        let (canvas, rects) = widgets::dock_bar(t, f, files::page(t), &[l.pick(texts::common::CANCEL), go]);
        Dock { canvas, hits: hits(&rects, &[Action::Unpick, Action::Paste]) }
    });
    let button = Some((widgets::Glyph::Terminal, Action::TerminalHere));
    let root = l.pick(texts::files::ROOT);
    Frame { dock, button, ..listing(cx, &s.files, root, None, l.pick(texts::files::NONE), Action::OpenEntry, l.pick(texts::page::FILES)) }
}

/// A place's folders and the files in them that can be installed, to
/// choose one. A USB drive's page waits for one to be plugged in, and the
/// drive's files show as soon as it is.
pub fn choose(cx: &Ctx, place: Place) -> Frame {
    let (s, l) = (cx.state, cx.state.lang);
    let unreadable;
    let absent = match (place, &s.usb_drive) {
        (Place::UsbDrive, None) => Some((l.pick(texts::choose::NO_USB_DRIVE), l.pick(texts::choose::CONNECT_USB_DRIVE))),
        (Place::UsbDrive, Some(UsbDrive::Unreadable(why))) => {
            unreadable = texts::choose::usb_drive_format(l, why);
            Some((l.pick(texts::usb_drive::UNREADABLE), unreadable.as_str()))
        }
        _ => None,
    };
    let empty = l.pick(texts::choose::NONE);
    listing(cx, s.folder(Screen::Choose(place)), place_name(l, place), absent, empty, Action::ChooseEntry, l.pick(texts::choose::TITLE))
}

/// A folder laid out as the stock file manager lays one out: the path as
/// capsules from `top`, held under the bar once scrolled there, then the
/// entries in one list on the plain page, each row drawn as it scrolls into
/// view and touched for `action`. In place of the list, `absent` says why
/// nothing of the folder can be shown, and `empty` that it holds nothing.
fn listing(
    cx: &Ctx,
    folder: &Folder,
    top: &str,
    absent: Option<(&str, &str)>,
    empty: &str,
    action: fn(usize) -> Action,
    title: &'static str,
) -> Frame {
    let (t, f) = (cx.t, cx.f);
    let bg = files::page(t);
    let mut c = sheet(cx, t.m.h, bg);
    let rows = match (absent, &folder.entries) {
        (Some((note, why)), _) => {
            files::note(&mut c, t, f, note, Some(why));
            None
        }
        (None, Err(why)) => {
            files::note(&mut c, t, f, cx.state.lang.pick(texts::files::UNREADABLE), Some(why));
            None
        }
        (None, Ok(entries)) if entries.is_empty() => {
            files::note(&mut c, t, f, empty, None);
            None
        }
        (None, Ok(entries)) => Some(files::rows(t, f, entries, action)),
    };
    let pinned = Some(files::strip(t, f, top, &folder.path));
    Frame { bg, rows, pinned, ..Frame::new(t, c, Vec::new(), title, t.m.dp(files::STRIP_TOP_DP)) }
}

/// Row `i` of the page's long list over the whole of `c`, sized for it,
/// drawn as the row comes into view.
pub fn row(screen: Screen, cx: &Ctx, i: usize, c: &mut Canvas) {
    match screen {
        Screen::Files | Screen::Choose(_) => {
            let folder = cx.state.folder(screen);
            let entries = folder.entries.as_ref().expect("a folder listed draws its rows");
            files::row(c, cx.t, cx.f, cx.state.lang, &entries[i], folder.listed, cx.state.zone());
        }
        Screen::Installing => widgets::output_row(c, cx.t, cx.f, &cx.state.script_output, i),
        Screen::PartWork => {
            let work = cx.state.part_work.as_ref().expect("the page of partition work shows work");
            widgets::output_row(c, cx.t, cx.f, &work.output, i)
        }
        other => unreachable!("{other:?} draws no list row by row"),
    }
}

/// One entry of a folder, a file or a folder: what it is, and what can be
/// done with it. Deleting asks first, with a swipe.
pub fn file_info(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let e = s.file.as_ref().expect("the file page shows an entry");
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let folder = matches!(e.kind, FileKind::Folder { .. });
    let mut facts = vec![
        (l.pick(texts::files::NAME), e.name.clone()),
        (l.pick(texts::common::LOCATION), Path::new(&s.files.path).join(&e.name).to_string_lossy().into_owned()),
    ];
    let holds = l.pick(texts::files::HOLDS);
    match &e.kind {
        FileKind::Link { target, .. } => facts.push((l.pick(texts::files::POINTS_TO), target.clone())),
        FileKind::Folder { items: Some(n) } => facts.push((holds, texts::files::line::items(l, *n as u64))),
        FileKind::Folder { items: None } => facts.push((holds, l.pick(texts::common::UNREADABLE).to_string())),
        FileKind::File | FileKind::Special => facts.push((l.pick(texts::common::SIZE), texts::files::size(l, e.size, fmt::file_size(e.size)))),
    }
    facts.push((l.pick(texts::files::MODIFIED), fmt::datetime(l, e.modified, s.zone())));
    facts.push((l.pick(texts::files::PERMISSIONS), texts::files::owned_by(l, fmt::mode(e.mode), e.uid, e.gid)));
    let y = widgets::info_card(&mut c, t, f, top, &facts);
    let mut rows = Vec::new();
    let mut acts = Vec::new();
    // Images are flashed; packages and zips that install themselves are
    // installed.
    if installs(e) {
        let image = Path::new(&e.name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("img"));
        rows.push(Row::new(l.pick(if image { texts::files::FLASH } else { texts::page::INSTALL })));
        acts.push(Action::InstallEntry);
    }
    if e.kind == FileKind::File {
        rows.push(Row::new(l.pick(texts::files::EDIT)));
        acts.push(Action::EditEntry);
    }
    rows.extend([Row::new(l.pick(texts::files::COPY)), Row::new(l.pick(texts::files::MOVE_TO)), Row::new(l.pick(texts::files::RENAME))]);
    acts.extend([Action::PickEntry(FileOp::Copy), Action::PickEntry(FileOp::Move), Action::RenameEntry]);
    // A link's own permissions mean nothing; its target is changed where
    // it is.
    if !matches!(e.kind, FileKind::Link { .. }) {
        rows.push(Row::new(l.pick(texts::files::CHANGE_PERMISSIONS)));
        acts.push(Action::OpenPermissions);
    }
    rows.push(Row::new(l.pick(texts::files::DELETE)).tint(t.pal.danger).trail(Trail::None));
    acts.push(Action::AskDelete);
    let actions = widgets::list(&mut c, t, f, y + t.m.card_gap, &rows);
    let mut out = hits(&actions.rows, &acts);
    let mut bottom = actions.bottom;
    if s.asking_delete {
        let what = match folder {
            true => texts::files::ask_delete_folder(l, &e.name),
            false => texts::files::ask_delete(l, &e.name),
        };
        let y = widgets::footnote(&mut c, t, f, actions.bottom, &what);
        let (label, danger) = confirm_label(Confirm::DeleteEntry, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::DeleteEntry) });
        bottom = track.y + track.h;
    }
    let title = l.pick(if folder { texts::files::FOLDER_INFO } else { texts::files::FILE_INFO });
    Frame::new(t, fit(c, t, bottom), out, title, top)
}

/// The permission bits of the entry the file page shows, a switch each that
/// takes effect at once: reading, writing and running for its owner, its
/// group and everyone else, then the set-ID and sticky bits.
pub fn permissions(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let e = s.file.as_ref().expect("the permissions page shows an entry");
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mode = e.mode & 0o7777;
    let bits = fmt::mode(e.mode);
    use texts::files::mode as said;
    let facts = [(l.pick(texts::files::NAME), e.name.clone()), (l.pick(texts::files::PERMISSIONS), said::bits(l, bits, format!("{mode:04o}")))];
    let mut y = widgets::info_card(&mut c, t, f, top, &facts);
    let mut out = Vec::new();
    let classes = [(said::OWNER, 6), (said::GROUP, 3), (said::OTHERS, 0)];
    let (read, write, run) = (l.pick(said::READ), l.pick(said::WRITE), l.pick(said::EXECUTE));
    let groups = classes.map(|(title, shift)| (l.pick(title), [(read, 4 << shift), (write, 2 << shift), (run, 1 << shift)]));
    let special = (l.pick(said::SPECIAL), [("SUID", 0o4000), ("SGID", 0o2000), (l.pick(said::STICKY), 0o1000)]);
    for (title, bits) in groups.into_iter().chain([special]) {
        y = widgets::section(&mut c, t, f, y, title);
        let rows = bits.map(|(name, bit)| Row::new(name).trail(Trail::Switch(mode & bit != 0)));
        let list = widgets::list(&mut c, t, f, y, &rows);
        out.extend(hits(&list.rows, &bits.map(|(_, bit)| Action::ModeBit(bit))));
        y = list.bottom;
    }
    Frame::new(t, fit(c, t, y), out, l.pick(texts::files::CHANGE_PERMISSIONS), top)
}

/// A new name for the entry the file page shows, typed on the keyboard.
pub fn rename(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let (y, caret) = widgets::text_field(&mut c, t, f, top, &s.new_name);
    widgets::footnote(&mut c, t, f, y, l.pick(texts::files::RENAME_NOTE));
    keyboard_hits(&mut c, t, f, s, l.pick(texts::common::DONE), &mut out);
    Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(texts::files::RENAME), top) }
}

/// The folder an entry goes to already holds its name: what to do.
pub fn clash(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let (clip, into, _) = s.clash.as_ref().expect("the clash page shows a clash");
    let name = Path::new(&clip.path).file_name().map_or_else(|| clip.path.clone(), |n| n.to_string_lossy().into_owned());
    use texts::files::clash as said;
    let skip = l.pick(match clip.op {
        FileOp::Copy => said::SKIP_COPY,
        FileOp::Move => said::SKIP_MOVE,
    });
    let mut c = page(cx);
    let top = t.m.content_top;
    let y = widgets::paragraph(&mut c, t, f, top, &said::there(l, into, name));
    let choices = widgets::list(&mut c, t, f, y + t.m.dp(12.0), &[
        Row::new(l.pick(said::REPLACE)).subtitle(l.pick(said::REPLACE_NOTE)),
        Row::new(l.pick(said::SKIP)).subtitle(skip),
        Row::new(l.pick(said::KEEP_BOTH)).subtitle(l.pick(said::KEEP_BOTH_NOTE)),
    ]);
    let out = hits(&choices.rows, &[
        Action::Resolve(ClashChoice::Replace),
        Action::Resolve(ClashChoice::Skip),
        Action::Resolve(ClashChoice::KeepBoth),
    ]);
    Frame::new(t, c, out, l.pick(said::TITLE), top)
}

/// Copying, moving or deleting: how far it has come and a way to stop it,
/// then, if it did not finish, why.
pub fn file_job(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let job = s.job.as_ref().expect("the job page shows work on files");
    let path = match &job.task {
        FileTask::Copy { path, .. } | FileTask::Move { path, .. } | FileTask::Delete { path } => path,
    };
    use texts::files::job as said;
    let name = Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    let share = |(done, total): (u64, u64)| if total == 0 { 0.0 } else { done as f32 / total as f32 };
    // The bar stays while the work measures, so that the button under it
    // does not move. Deleting, and copying entries that hold no bytes, is
    // counted in entries.
    let (detail, done) = match (job.progress, &job.task) {
        (None, _) => (l.pick(texts::installing::GETTING_READY).to_string(), 0.0),
        (Some((bytes, _)), FileTask::Copy { .. }) if bytes.1 > 0 => (said::copied_of(l, fmt::file_size(bytes.0), fmt::file_size(bytes.1)), share(bytes)),
        (Some((bytes, _)), FileTask::Move { .. }) if bytes.1 > 0 => (said::moved_of(l, fmt::file_size(bytes.0), fmt::file_size(bytes.1)), share(bytes)),
        (Some((_, entries)), FileTask::Copy { .. }) => (said::copied_items(l, entries.0, entries.1), share(entries)),
        (Some((_, entries)), FileTask::Move { .. }) => (said::moved_items(l, entries.0, entries.1), share(entries)),
        (Some((_, entries)), FileTask::Delete { .. }) => (said::deleted_items(l, entries.0, entries.1), share(entries)),
    };
    let mut c = page(cx);
    let top = t.m.content_top;
    let y = widgets::progress_card(&mut c, t, f, top, &name, &detail, Some(done));
    let mut out = Vec::new();
    match &job.outcome {
        None => {
            let label = l.pick(if job.stopping { texts::common::STOPPING } else { texts::common::STOP });
            let stop = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopJob]));
            }
        }
        Some(end) => {
            let why = match (end, &job.task) {
                (JobEnd::Stopped, FileTask::Copy { .. }) => l.pick(said::COPY_STOPPED).to_string(),
                (JobEnd::Stopped, FileTask::Move { .. }) => l.pick(said::MOVE_STOPPED).to_string(),
                (JobEnd::Stopped, FileTask::Delete { .. }) => l.pick(said::DELETE_STOPPED).to_string(),
                (JobEnd::Failed(why), FileTask::Copy { .. }) => said::copy_failed(l, why),
                (JobEnd::Failed(why), FileTask::Move { .. }) => said::move_failed(l, why),
                (JobEnd::Failed(why), FileTask::Delete { .. }) => said::delete_failed(l, why),
                (JobEnd::Done, _) => unreachable!("work done leaves its page"),
            };
            let y = widgets::footnote(&mut c, t, f, y, &why);
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(texts::common::DONE)).trail(Trail::None)]);
            out.extend(hits(&finish.rows, &[Action::Back]));
        }
    }
    let title = l.pick(match (&job.task, job.outcome.is_none()) {
        (FileTask::Copy { .. }, true) => said::COPYING,
        (FileTask::Copy { .. }, false) => said::COPY_INCOMPLETE,
        (FileTask::Move { .. }, true) => said::MOVING,
        (FileTask::Move { .. }, false) => said::MOVE_INCOMPLETE,
        (FileTask::Delete { .. }, true) => said::DELETING,
        (FileTask::Delete { .. }, false) => said::DELETE_INCOMPLETE,
    });
    Frame { back: job.outcome.is_some(), ..Frame::new(t, c, out, title, top) }
}

/// A shell on a terminal: the screen it writes, the keys a phone keyboard
/// lacks, and the keyboard, every key of which goes to the shell.
pub fn terminal(cx: &Ctx) -> Frame {
    let (t, f, s) = (cx.t, cx.f, cx.state);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    keyboard_hits(&mut c, t, f, s, s.lang.pick(texts::keyboard::ENTER), &mut out);
    let l = term::layout(t, f, top);
    term::draw(&mut c, t, f, &l, cx.term);
    let held = cx.term.is_some_and(|term| term.ctrl);
    let keys = term::keys(&mut c, t, f, &l, &term::TERM_KEYS, held);
    out.extend(keys.into_iter().map(|(rect, key)| Hit { rect, action: Action::TermKey(key) }));
    Frame { scrollback: Some((l.card, l.pitch)), ..Frame::new(t, c, out, s.lang.pick(texts::page::TERMINAL), top) }
}

/// A text file being edited: where it is, its lines wrapped on a card, the
/// keys a phone keyboard lacks for moving the cursor, and the keyboard.
/// Once it is changed, the round button saves it.
pub fn editor(cx: &Ctx) -> Frame {
    let (t, f, s) = (cx.t, cx.f, cx.state);
    let ed = s.editor.as_ref().expect("the editor page shows a file");
    let mut c = page(cx);
    let mut out = Vec::new();
    keyboard_hits(&mut c, t, f, s, s.lang.pick(texts::keyboard::NEW_LINE), &mut out);
    editor::caption(&mut c, t, f, &ed.path);
    let l = editor::layout(t, f);
    let caret = editor::draw(&mut c, t, f, &l, ed);
    let keys = term::keys(&mut c, t, f, &l, &editor::EDITOR_KEYS, false);
    out.extend(keys.into_iter().map(|(rect, key)| Hit { rect, action: Action::TermKey(key) }));
    let button = ed.changed.then_some((widgets::Glyph::Done, Action::SaveText));
    Frame { scrollback: Some((l.card, l.pitch)), caret, button, ..Frame::new(t, c, out, s.lang.pick(texts::files::EDIT), t.m.content_top) }
}

/// The languages the pages speak, each named in itself, to speak another.
pub fn language(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let rows: Vec<Row> = Lang::ALL.iter().map(|&k| Row::new(k.name()).trail(Trail::Check(k == l))).collect();
    let list = widgets::list(&mut c, t, f, top, &rows);
    let out = hits(&list.rows, &Lang::ALL.map(Action::Language));
    Frame::new(t, fit(c, t, list.bottom), out, l.pick(texts::page::LANGUAGE), top)
}

/// The time zone the status bar's clock and the times on the pages keep.
/// On a phone whose installed system's zone is known, a switch follows it
/// and tells which it is; a phone with no system, or one that has set no
/// zone, shows none. Under it a zone is chosen from a part of the world,
/// whose row names the zone kept while that is one of its own.
pub fn time_zone(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = match &s.system_zone {
        Some(zone) => {
            let name = listed(&zone.name).map_or(zone.name.as_str(), |at| l.pick(TIME_ZONES[at].name));
            let system = format!("{} · {name}", fmt::utc_offset(s.now, &zone.rules));
            let follow = Row::new(l.pick(texts::time_zone::FOLLOW_SYSTEM)).subtitle(&system).trail(Trail::Switch(s.follow_system_zone));
            let follow = widgets::list(&mut c, t, f, top, &[follow]);
            out.extend(hits(&follow.rows, &[Action::Toggle(Switch::SystemTimeZone)]));
            widgets::section(&mut c, t, f, follow.bottom, l.pick(texts::time_zone::CHOOSE))
        }
        None => top,
    };
    let kept = s.zone_listed();
    let rows = Region::ALL.map(|region| {
        let row = Row::new(l.pick(region.name()));
        match kept.filter(|&at| TIME_ZONES[at].region == Some(region)) {
            Some(at) => row.trail(Trail::Value(l.pick(TIME_ZONES[at].name))),
            None => row,
        }
    });
    let regions = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&regions.rows, &Region::ALL.map(|region| Action::Open(Screen::TimeZones(region)))));
    let utc = listed(UTC).expect("UTC is a zone listed");
    let offset = fmt::utc_offset(s.now, fmt::rules(utc));
    let universal = Row::new(l.pick(TIME_ZONES[utc].name)).subtitle(&offset).trail(Trail::Check(kept == Some(utc)));
    let universal = widgets::list(&mut c, t, f, regions.bottom + t.m.card_gap, &[universal]);
    out.extend(hits(&universal.rows, &[Action::TimeZone(utc)]));
    let bottom = widgets::footnote(&mut c, t, f, universal.bottom, l.pick(texts::time_zone::NOTE));
    Frame::new(t, fit(c, t, bottom), out, l.pick(texts::page::TIME_ZONE), top)
}

/// The time zones of one part of the world, from the one furthest behind
/// UTC now to the one furthest ahead, each with how far that is and the
/// cities it is known by.
pub fn time_zones(cx: &Ctx, region: Region) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mut zones: Vec<usize> = (0..TIME_ZONES.len()).filter(|&at| TIME_ZONES[at].region == Some(region)).collect();
    zones.sort_by_key(|&at| fmt::utc_offset_seconds(s.now, fmt::rules(at)));
    let lines: Vec<String> = zones.iter().map(|&at| format!("{} · {}", fmt::utc_offset(s.now, fmt::rules(at)), l.pick(TIME_ZONES[at].cities))).collect();
    let kept = s.zone_listed();
    let rows: Vec<Row> = zones.iter().zip(&lines).map(|(&at, line)| Row::new(l.pick(TIME_ZONES[at].name)).subtitle(line).trail(Trail::Check(kept == Some(at)))).collect();
    let list = widgets::list(&mut c, t, f, top, &rows);
    let picks: Vec<Action> = zones.iter().map(|&at| Action::TimeZone(at)).collect();
    Frame::new(t, fit(c, t, list.bottom), hits(&list.rows, &picks), l.pick(region.name()), top)
}

/// Saving the logs, as a bug report asks for them.
pub fn logs(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::logs::CONTENTS));
    let parts = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(texts::logs::RECOVERY_LOG)).subtitle(l.pick(texts::logs::RECOVERY_LOG_NOTE)).trail(Trail::Check(s.logs[0])),
        Row::new(l.pick(texts::logs::KERNEL_LOG)).subtitle(l.pick(texts::logs::KERNEL_LOG_NOTE)).trail(Trail::Check(s.logs[1])),
    ]);
    out.extend(hits(&parts.rows, &[Action::LogPart(0), Action::LogPart(1)]));
    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(texts::logs::SAVE_TO));
    let storage = l.pick(match cx.state.data.storage_open() {
        true => texts::logs::FOLDER,
        false => texts::decrypt::NEEDED,
    });
    let places = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(texts::logs::TEMPORARY)).subtitle(l.pick(texts::logs::TEMPORARY_NOTE)).trail(Trail::Check(s.logs_to == 0)),
        Row::new(place_name(l, Place::Storage)).subtitle(storage).trail(Trail::Check(s.logs_to == 1)),
    ]);
    out.extend(hits(&places.rows, &[Action::LogsTo(0), Action::LogsTo(1)]));
    let y = widgets::footnote(&mut c, t, f, places.bottom, l.pick(texts::logs::NOTE));
    let export = widgets::button(&mut c, t, f, y + t.m.dp(24.0), l.pick(texts::logs::EXPORT));
    out.push(Hit { rect: export, action: Action::ExportLogs });
    Frame::new(t, c, out, l.pick(texts::page::LOGS), top)
}

/// The words the licence gives for a program's notice, which every
/// language but English reads its own rendering ahead of.
const LICENSE_ORIGINAL: &str = "Skyward\nCopyright (C) 2026 Kirk Lin\n\n\
This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.\n\n\
This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.\n\n\
You should have received a copy of the GNU General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.";

/// Skyward's own licence.
pub fn license(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let y = widgets::info_card(&mut c, t, f, top, &[(l.pick(texts::license::TERMS_TITLE), l.pick(texts::license::TERMS).to_string())]);
    let bottom = match l.pick(texts::license::ORIGINAL_ALONE) {
        true => widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), LICENSE_ORIGINAL),
        false => {
            let y = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), l.pick(texts::license::NOTICE));
            let y = widgets::section(&mut c, t, f, y, l.pick(texts::license::ORIGINAL));
            widgets::paragraph(&mut c, t, f, y - t.m.dp(16.0), LICENSE_ORIGINAL)
        }
    };
    Frame::new(t, fit(c, t, bottom), Vec::new(), l.pick(texts::page::LICENSE), top)
}

/// The monospaced typeface's licence, which names the copyright holders on
/// its first lines; the image carries the same file beside the font.
const MONO_LICENSE: &str = include_str!("../../../resources/fonts/KirkMono-OFL.txt");

/// The typefaces, as they name themselves, and the terms each is used under.
pub fn font_license(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let (typeface, version, copyright) = (l.pick(texts::fonts::TYPEFACE), l.pick(texts::fonts::VERSION), l.pick(texts::fonts::COPYRIGHT));
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::fonts::INTERFACE_TEXT));
    let y = widgets::info_card(&mut c, t, f, y, &[
        (typeface, f.info.family.clone()),
        (version, f.info.version.clone()),
        (copyright, f.info.copyright.clone().expect("MiSans names its copyright holder")),
    ]);
    let y = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), l.pick(texts::fonts::MISANS));
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::fonts::TERMINAL_TEXT));
    let holders: Vec<&str> = MONO_LICENSE.lines().take_while(|l| l.starts_with("Copyright")).collect();
    let y = widgets::info_card(&mut c, t, f, y, &[
        (typeface, f.mono_info.family.clone()),
        (version, f.mono_info.version.clone()),
        (copyright, holders.join("\n")),
        (l.pick(texts::license::TERMS_TITLE), l.pick(texts::fonts::OFL).to_string()),
    ]);
    let bottom = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), l.pick(texts::fonts::MONO));
    Frame::new(t, fit(c, t, bottom), Vec::new(), l.pick(texts::page::FONTS), top)
}

/// Which slot the phone starts from next, as the boot control holds it,
/// and what it holds of each; picking the other and swiping switches.
/// While an update is in flight the slot stays as the update set it.
pub fn slot(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let title = l.pick(texts::slot::BOOT_SLOT);
    let Some(current) = i.slot else {
        widgets::list(&mut c, t, f, top, &[Row::new(l.pick(texts::slot::NO_SLOTS)).trail(Trail::None)]);
        return Frame::new(t, c, out, title, top);
    };
    let slots = match &s.boot_slots {
        Some(Ok(slots)) => *slots,
        Some(Err(why)) => {
            let list = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(texts::slot::UNREADABLE)).trail(Trail::None)]);
            widgets::footnote(&mut c, t, f, list.bottom, why);
            return Frame::new(t, c, out, title, top);
        }
        None => return Frame::new(t, c, out, title, top),
    };
    let chosen = s.next_slot.unwrap_or(slots.active);
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::reboot::NEXT_BOOT));
    let titles = [Slot::A, Slot::B].map(|slot| texts::slot::named(l, slot.letter()));
    let subs = [Slot::A, Slot::B].map(|slot| {
        let k = slot.number() as usize;
        l.pick(match (slot == current, slots.bootable[k], slots.successful[k]) {
            (true, false, _) => texts::slot::RUNNING_UNBOOTABLE,
            (true, true, false) => texts::slot::RUNNING_UNSTARTED,
            (true, true, true) => texts::slot::RUNNING,
            (false, false, _) => texts::slot::OTHER_UNBOOTABLE,
            (false, true, false) => texts::slot::OTHER_UNSTARTED,
            (false, true, true) => texts::slot::OTHER,
        })
    });
    let rows: Vec<Row> =
        [Slot::A, Slot::B].iter().enumerate().map(|(k, &slot)| Row::new(&titles[k]).subtitle(subs[k]).trail(Trail::Check(slot == chosen))).collect();
    let list = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&list.rows, &[Slot::A, Slot::B].map(Action::PickSlot)));
    if let Some(note) = fmt::snapshot_slot_note(l, &i.snapshot) {
        widgets::footnote(&mut c, t, f, list.bottom, note);
        return Frame::new(t, c, out, title, top);
    }
    let y = widgets::footnote(&mut c, t, f, list.bottom, l.pick(texts::slot::SWITCH_NOTE));
    let (label, danger) = confirm_label(Confirm::Slot, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Slot) });
    Frame::new(t, c, out, title, top)
}

/// The two systems as big cards one over the other, laid out as the boot
/// slot page lays out its slots: the main system, in the phone's own
/// partitions, and the second, whose files lie in the data partition, or
/// the place to install one, the one that starts next checked. Picking the
/// other and swiping writes its boot images into the slot that starts
/// next, and restarts into it. Under them, the second system, and removing
/// it.
pub fn systems(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let title = l.pick(texts::page::DUAL_BOOT);
    // While the boot partitions are written, or the second system removed,
    // the page only says so: of a switch the system asked for, perhaps
    // before the systems are read.
    if let Some(work) = s.systems_work {
        let (heading, detail) = match work {
            SystemsWork::Switching(to) => {
                let name = match (&s.systems, to) {
                    (Some(Ok(systems)), Who::Installed) => Some(systems.installed.name.as_str()),
                    (Some(Ok(systems)), Who::Other) => systems.other.as_ref().map(|o| o.name.as_str()),
                    _ => None,
                };
                let heading = match name {
                    Some(name) => texts::systems::switching_to(l, name),
                    None => l.pick(texts::systems::SWITCHING).to_string(),
                };
                (heading, l.pick(texts::systems::WRITING_BOOT_IMAGES))
            }
            SystemsWork::Removing => (l.pick(texts::systems::REMOVING).to_string(), l.pick(texts::systems::GIVING_ROOM_BACK)),
        };
        let y = widgets::progress_card(&mut c, t, f, top, &heading, detail, None);
        let y = widgets::footnote(&mut c, t, f, y, l.pick(texts::systems::KEEP_POWER));
        return Frame::new(t, fit(c, t, y), out, title, top);
    }
    let systems = match &s.systems {
        Some(Ok(systems)) => systems,
        Some(Err(why)) => {
            let list = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(texts::systems::UNREADABLE)).trail(Trail::None)]);
            let y = widgets::footnote(&mut c, t, f, list.bottom, why);
            return Frame::new(t, fit(c, t, y), out, title, top);
        }
        None => return Frame::new(t, fit(c, t, top), out, title, top),
    };
    let starts = starting(systems);
    let chosen = s.system_pick.unwrap_or(starts);
    let second_role = l.pick(texts::systems::SECOND);
    // Boot partitions that hold a second system's image nothing is kept of
    // start neither system the page knows.
    let main = SystemCard {
        role: l.pick(texts::systems::MAIN),
        name: &systems.installed.name,
        detail: systems.installed.version.as_deref().unwrap_or_default(),
        mark: SystemMark::Check(chosen == Who::Installed && systems.held != Held::Unrecorded),
    };
    let second = match &systems.other {
        Some(other) => SystemCard { role: second_role, name: &other.name, detail: &other.version, mark: SystemMark::Check(chosen == Who::Other) },
        None => SystemCard {
            role: second_role,
            name: l.pick(texts::systems::INSTALL),
            detail: l.pick(texts::systems::CHOOSE_PACKAGE),
            mark: SystemMark::Opens,
        },
    };
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::reboot::NEXT_BOOT));
    let shown = widgets::system_cards(&mut c, t, f, y, &[main, second]);
    let second_opens = match systems.other {
        Some(_) => Action::PickSystem(Who::Other),
        None => Action::Open(Screen::Install),
    };
    out.extend(hits(&shown.rows, &[Action::PickSystem(Who::Installed), second_opens]));
    let mut y = shown.bottom + t.m.dp(12.0);
    if let Some(why) = &s.system_notice {
        y = widgets::info_card(&mut c, t, f, y, &[(l.pick(texts::systems::SECOND_FAILED), why.clone())]);
    }
    if chosen != starts {
        match (chosen, fmt::snapshot_system_note(l, &i.snapshot)) {
            (Who::Other, Some(note)) => y = widgets::footnote(&mut c, t, f, y, note),
            _ => {
                let (label, danger) = confirm_label(Confirm::SwitchSystem, s);
                let track = widgets::confirm(&mut c, t, f, y + t.m.dp(12.0), label, danger, s.confirm);
                out.push(Hit { rect: track, action: Action::Confirm(Confirm::SwitchSystem) });
                y = track.y + track.h;
            }
        }
    }
    y = widgets::footnote(&mut c, t, f, y, l.pick(texts::systems::WHAT));
    match systems.held {
        Held::Recorded => {}
        Held::Rewritten => y = widgets::footnote(&mut c, t, f, y, l.pick(texts::systems::REWRITTEN)),
        Held::Unrecorded => y = widgets::footnote(&mut c, t, f, y, l.pick(texts::systems::UNRECORDED)),
    }
    if systems.other.is_some() && systems.changed {
        y = widgets::footnote(&mut c, t, f, y, l.pick(texts::systems::CHANGED));
    }
    if let Some(other) = &systems.other {
        y = widgets::section(&mut c, t, f, y, second_role);
        let size = fmt::bytes(other.size);
        let facts = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(texts::systems::SPACE_USED)).trail(Trail::Text(&size))]);
        let mounted = matches!(s.data, Data::Open | Data::Locked { .. });
        let remove = Row::new(l.pick(texts::systems::REMOVE)).tint(t.pal.danger);
        let (remove, act) = match mounted {
            true => (remove.trail(Trail::None), Action::AskDelete),
            false => (remove.subtitle(l.pick(texts::decrypt::NEEDED)), Action::Open(Screen::Decrypt)),
        };
        let removing = widgets::list(&mut c, t, f, facts.bottom + t.m.card_gap, &[remove]);
        out.extend(hits(&removing.rows, &[act]));
        y = removing.bottom;
        if s.asking_delete && mounted {
            let ask = l.pick(match starts {
                Who::Other => texts::systems::ASK_REMOVE_RUNNING,
                Who::Installed => texts::systems::ASK_REMOVE,
            });
            let y2 = widgets::footnote(&mut c, t, f, y, ask);
            let (label, danger) = confirm_label(Confirm::RemoveSystem, s);
            let track = widgets::confirm(&mut c, t, f, y2 + t.m.dp(24.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::RemoveSystem) });
            y = track.y + track.h;
        }
    }
    Frame::new(t, fit(c, t, y), out, title, top)
}

/// The phone's own fastbootd serving the computer: whether it runs, how far
/// the computer has the port, and where to go from here, chosen with the
/// keys and lit where they are.
pub fn fastboot(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = page(cx);
    let top = t.m.content_top;
    let running = l.pick(if s.fastboot.running { texts::fastboot::RUNNING } else { texts::fastboot::NOT_RUNNING });
    let link = l.pick(match s.fastboot.link {
        UsbLink::Connected => texts::fastboot::CONNECTED,
        UsbLink::Connecting => texts::fastboot::CONNECTING,
        UsbLink::Unplugged => texts::fastboot::NO_COMPUTER,
    });
    let slot = i.slot.map(|s| s.letter());
    let mut facts = vec![
        Row::new("fastbootd").trail(Trail::Text(running)),
        Row::new("USB").trail(Trail::Text(link)),
        Row::new(l.pick(texts::specs::CODENAME)).trail(Trail::Text(&i.codename)),
    ];
    if let Some(letter) = slot {
        facts.push(Row::new(l.pick(texts::slot::CURRENT)).trail(Trail::Text(letter)));
    }
    let y = widgets::section_first(&mut c, t, f, top, l.pick(texts::fastboot::STATUS));
    let state = widgets::list(&mut c, t, f, y, &facts);
    let y = widgets::footnote(&mut c, t, f, state.bottom, l.pick(texts::fastboot::NOTE));
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::fastboot::LEAVE));
    let rows: Vec<Row> = FASTBOOT_CHOICES.iter().map(|&(title, _)| Row::new(l.pick(title)).trail(Trail::None)).collect();
    let choices = widgets::list(&mut c, t, f, y, &rows);
    let actions: Vec<Action> = FASTBOOT_CHOICES.iter().map(|&(_, action)| action).collect();
    let out = hits(&choices.rows, &actions);
    widgets::footnote(&mut c, t, f, choices.bottom, l.pick(texts::fastboot::KEYS));
    Frame { back: false, ..Frame::new(t, c, out, "Fastboot", top) }
}

/// Where to go once what the system asked for has stopped or cannot be
/// done, in the order the pages list them.
const AFTERWARDS: [(Text, Action); 3] = [
    (texts::installed::REBOOT_TO_SYSTEM, Action::RebootTo(Target::System)),
    (texts::afterwards::ENTER_RECOVERY, Action::Dismiss),
    (texts::reboot::target::POWER_OFF, Action::RebootTo(Target::PowerOff)),
];

/// The list of [`AFTERWARDS`] at `y`, and where it ends. Work the user
/// chose in the recovery goes back to its home page, rather than into it.
fn afterwards(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, y: f32, asked: bool, out: &mut Vec<Hit>) -> f32 {
    let rows: Vec<Row> = AFTERWARDS
        .iter()
        .map(|&(title, action)| Row::new(l.pick(if action == Action::Dismiss && !asked { texts::afterwards::BACK_HOME } else { title })).trail(Trail::None))
        .collect();
    let choices = widgets::list(c, t, f, y, &rows);
    out.extend(hits(&choices.rows, &AFTERWARDS.map(|(_, action)| action)));
    choices.bottom
}

/// Erasing: each part and how far it has come, and once erasing has
/// stopped, why. What the system asked for leaves on its own once it is
/// done, and offers where to go only if it stops; what the user chose
/// offers where to go once it ends either way.
pub fn erase(cx: &Ctx) -> Frame {
    use std::cmp::Ordering;
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let e = &s.erase;
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let failed = matches!(e.outcome, Some(Err(_)));
    // Only a part that went wrong was under way; whatever else stopped
    // erasing stopped it before any part began.
    let begun = !failed || matches!(e.outcome, Some(Err(EraseStop::Failed(..))));
    let heading = l.pick(match &e.outcome {
        None => texts::wipe::WIPING,
        Some(Ok(())) => texts::wipe::WIPED,
        Some(Err(_)) => texts::wipe::INCOMPLETE,
    });
    let mut y = widgets::section_first(&mut c, t, f, top, heading);
    if !e.parts.is_empty() {
        let states: Vec<String> = (0..e.parts.len())
            .map(|k| match (k.cmp(&e.done), failed) {
                (Ordering::Less, _) => l.pick(texts::wipe::WIPED).into(),
                (Ordering::Equal, false) => match e.progress {
                    Some((done, total)) if total > 0 => texts::wipe::wiping_share(l, percent(share(done, total))),
                    _ => l.pick(texts::wipe::WIPING).into(),
                },
                (Ordering::Equal, true) if begun => l.pick(texts::common::UNFINISHED).into(),
                (Ordering::Greater, false) => l.pick(texts::common::WAITING).into(),
                (Ordering::Equal | Ordering::Greater, true) => l.pick(texts::wipe::NOT_WIPED).into(),
            })
            .collect();
        let rows: Vec<Row> = e.parts.iter().zip(&states).map(|(&part, state)| Row::new(fmt::part(l, part)).trail(Trail::Text(state))).collect();
        y = widgets::list(&mut c, t, f, y, &rows).bottom;
    }
    let notes: Vec<String> = match (&e.outcome, e.asked, e.reset) {
        (Some(Err(stop)), _, _) => {
            let (what, detail) = fmt::erase_stop(l, stop);
            std::iter::once(what).chain(detail).collect()
        }
        (_, true, true) => {
            let leaving = match s.asked.shutdown_after {
                true => texts::erase::ALL_THEN_POWER_OFF,
                false => texts::erase::ALL_THEN_REBOOT,
            };
            vec![l.pick(leaving).into()]
        }
        (_, true, false) => vec![l.pick(if s.asked.shutdown_after { texts::parts::leaving::POWER_OFF } else { texts::parts::leaving::REBOOT }).into()],
        (None, false, _) => vec![l.pick(texts::wipe::KEEP_POWER).into()],
        (Some(Ok(())), false, true) => vec![l.pick(texts::erase::MADE_ANEW).into()],
        (Some(Ok(())), false, false) => Vec::new(),
    };
    for note in &notes {
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if let (true, Some(reason)) = (e.asked, &s.asked.reason) {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(texts::installing::REASON), reason.clone())]);
    }
    if failed || (!e.asked && e.outcome.is_some()) {
        let below = widgets::section(&mut c, t, f, y, l.pick(texts::common::ACTIONS));
        y = afterwards(&mut c, t, f, l, below, e.asked, &mut out);
    }
    let title = l.pick(match (e.reset, e.asked) {
        (true, _) => texts::wipe::RESET,
        (false, true) => texts::erase::WIPE_CACHE,
        (false, false) => texts::page::WIPE,
    });
    Frame { back: false, ..Frame::new(t, fit(c, t, y), out, title, top) }
}

/// The system could not start Android and asks whether to erase all user
/// data: start it again, look around the recovery first, or erase.
pub fn cant_start(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let mut y = widgets::paragraph(&mut c, t, f, top, l.pick(texts::cant_start::WHAT));
    if let Some(reason) = &s.asked.reason {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(texts::installing::REASON), reason.clone())]);
    }
    y = widgets::section(&mut c, t, f, y, l.pick(texts::common::ACTIONS));
    let rows = [
        Row::new(l.pick(texts::common::TRY_AGAIN)).subtitle(l.pick(texts::cant_start::STARTS_AGAIN)).trail(Trail::None),
        Row::new(l.pick(texts::afterwards::ENTER_RECOVERY)).subtitle(l.pick(texts::cant_start::WIPES_NOTHING)).trail(Trail::None),
    ];
    let choices = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&choices.rows, &[Action::RebootTo(Target::System), Action::Dismiss]));
    let y = widgets::footnote(&mut c, t, f, choices.bottom, l.pick(texts::cant_start::CANT_UNDO));
    let (label, danger) = confirm_label(Confirm::FactoryReset, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::FactoryReset) });
    Frame { back: false, ..Frame::new(t, c, out, l.pick(texts::cant_start::TITLE), top) }
}

/// Something the system asked of the recovery that it does not do: what
/// was asked, and where to go instead.
pub fn unhandled(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let mut items = vec![(l.pick(texts::unhandled::REQUEST), s.asked.unhandled.clone().unwrap_or_default())];
    if let Some(reason) = &s.asked.reason {
        items.push((l.pick(texts::installing::REASON), reason.clone()));
    }
    let y = widgets::info_card(&mut c, t, f, top, &items);
    let y = widgets::footnote(&mut c, t, f, y, l.pick(texts::unhandled::NOTE));
    let y = widgets::section(&mut c, t, f, y, l.pick(texts::common::ACTIONS));
    afterwards(&mut c, t, f, l, y, true, &mut out);
    Frame { back: false, ..Frame::new(t, c, out, l.pick(texts::unhandled::TITLE), top) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::BackupRefusal;

    fn job(restore: bool, checks: bool) -> BackupJob {
        let plan = vec![("boot".into(), 200), ("system".into(), 800)];
        BackupJob { restore, place: Place::Storage, checks, plan, at: None, outcome: None, stopping: false }
    }

    fn at(item: usize, done: u64, of: u64, checking: bool) -> BackupAt {
        BackupAt { item, done, of, checking }
    }

    #[test]
    fn a_script_shows_its_last_line_until_skyward_goes_back() {
        let said = ["- Flashing boot".to_string(), "  ".to_string()];
        let l = Lang::Hans;
        assert_eq!(script_stage(l, None, &[]), ("正在运行安装脚本", None));
        assert_eq!(script_stage(Lang::En, None, &[]), ("Running the install script", None));
        assert_eq!(script_stage(l, Some(&Stage::Script { thousandths: 250 }), &said), ("- Flashing boot", Some(0.25)), "past a blank line");
        assert_eq!(script_stage(l, Some(&Stage::Reflashing { done: 1, total: 4 }), &said), ("正在重新刷入 Skyward", Some(0.25)));
        assert_eq!(update_stage(Lang::En, Some(&Stage::Reflashing { done: 3, total: 4 })), ("Flashing Skyward again".to_string(), Some(0.75)));
        assert_eq!(update_stage(Lang::Hant, Some(&Stage::Writing { done: 1, total: 4 })).0, "第 1 步，共 3 步：寫入新系統 25%");
    }

    #[test]
    fn a_backup_checks_its_parts_last_and_a_restore_first() {
        // Halfway through writing system, 600 of the 1000 bytes: the first
        // of two passes, then the second.
        assert_eq!(job_progress(&job(false, true), &at(1, 400, 800, false)), (0.3, 600));
        assert_eq!(job_progress(&job(false, true), &at(1, 400, 800, true)), (0.8, 600));
        assert_eq!(job_progress(&job(false, false), &at(1, 400, 800, false)), (0.6, 600), "without a check, writing is all");
        assert_eq!(job_progress(&job(true, true), &at(0, 100, 200, true)), (0.05, 100));
        assert_eq!(job_progress(&job(true, true), &at(0, 100, 200, false)), (0.55, 100));
    }

    #[test]
    fn each_part_says_where_it_stands() {
        let states = |j: &BackupJob| (0..2).map(|k| part_state(Lang::Hans, j, k)).collect::<Vec<_>>();
        let mut b = job(false, true);
        assert_eq!(states(&b), ["等待中", "等待中"]);
        b.at = Some(at(1, 400, 800, false));
        assert_eq!(states(&b), ["已备份", "正在备份 50%"]);
        b.at = Some(at(0, 50, 200, true));
        assert_eq!(states(&b), ["正在校验 25%", "已备份"]);
        b.outcome = Some(BackupEnd::Failed("the backup does not hold what was stored".into()));
        assert_eq!(states(&b), ["未完成", "未校验"]);
        b.outcome = Some(BackupEnd::Saved("20261021-141639".into()));
        assert_eq!(states(&b), ["已校验", "已校验"]);
        let mut r = job(true, true);
        r.at = Some(at(1, 0, 800, true));
        assert_eq!(states(&r), ["已校验", "正在校验 0%"]);
        r.at = Some(at(0, 200, 400, false));
        assert_eq!(states(&r), ["正在恢复 50%", "已校验"]);
        r.outcome = Some(BackupEnd::Failed("write /dev/block/by-name/boot_b".into()));
        assert_eq!(states(&r), ["未完成", "未恢复"]);
        r.at = None;
        r.outcome = Some(BackupEnd::Refused(BackupRefusal::OlderAndroid { was: "17".into(), now: "16".into() }));
        assert_eq!(states(&r), ["未开始", "未开始"]);
    }

    #[test]
    fn each_part_says_where_it_stands_in_the_language() {
        let mut b = job(false, true);
        b.at = Some(at(1, 400, 800, false));
        assert_eq!((0..2).map(|k| part_state(Lang::En, &b, k)).collect::<Vec<_>>(), ["Backed up", "Backing up 50%"]);
        let mut r = job(true, true);
        r.at = Some(at(0, 200, 400, false));
        assert_eq!((0..2).map(|k| part_state(Lang::Hant, &r, k)).collect::<Vec<_>>(), ["正在還原 50%", "已驗證"]);
    }
}
