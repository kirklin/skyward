use crate::app::{
    Action, BackupAt, BackupEnd, BackupJob, Check, ClashChoice, Confirm, Damage, Data, Dock, DownloadEnd, EraseStop, Field, FileKind,
    FileOp, FileTask, Fold, Folder, Frame, FsAction, FsChoice, FsRow, Have, HeroArt, Hit, Install, JobEnd, Keys, Pick, Place, Rows, Screen,
    SkywardCheck, SkywardEnd, SkywardJob, SkywardStage, Slider, Source, State, Switch, SystemsWork, Target, UsbDrive, UsbLink, Wipe, ADDABLE,
    BACKUP_PARTS, DATA_ERASED, DATA_PARTITION, DOWNLOADS, FASTBOOT_CHOICES, SCREEN_OFF, TARGETS, TIME_ZONES, USB_DRIVE_UNREADABLE,
    USB_DRIVE_WANTED, WIPES, installs, starting,
};
use crate::canvas::Canvas;
use crate::color::Rgba;
use crate::editor;
use crate::files;
use crate::fmt::{self, Verb};
use crate::font::Fonts;
use crate::geom::Rect;
use crate::lang::{Lang, Text};
use crate::search;
use crate::term::{self, Term};
use crate::theme::Theme;
use crate::tr;
use crate::icons::{self, Icon};
use crate::widgets::{self, Caption, Detail, Fact, Outline, Row, Spec, SystemCard, SystemMark, Tone, Trail};
use skyward_info::{DeviceInfo, VERSION};
use skyward_install::{Outcome, Reflash, Refusal, Stage};
use skyward_net::download::Progress;
use skyward_net::supplicant::Security;
use skyward_net::wlan::{Bands, Network, Power, Stage as Joining};
use skyward_systems::Who;
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
    const INSTALL: Text = ["滑动以安装", "滑動以安裝", "Slide to install"];
    let (label, danger) = match which {
        Confirm::Filesystem => match s.fs_action {
            FsAction::Repair => (["滑动以修复", "滑動以修復", "Slide to repair"], true),
            FsAction::Remake(_) => (["滑动以更换", "滑動以更換", "Slide to change"], true),
        },
        Confirm::Reboot => (["滑动以重启", "滑動以重新啟動", "Slide to reboot"], false),
        Confirm::Wipe => (["滑动以清除", "滑動以清除", "Slide to wipe"], true),
        Confirm::Flash => (["滑动以刷入", "滑動以刷入", "Slide to flash"], true),
        Confirm::Backup => (["滑动以备份", "滑動以備份", "Slide to back up"], false),
        Confirm::Restore => (["滑动以恢复", "滑動以還原", "Slide to restore"], true),
        Confirm::Slot => (["滑动以切换", "滑動以切換", "Slide to switch"], true),
        Confirm::FactoryReset => (["滑动以恢复出厂设置", "滑動以恢復原廠設定", "Slide to factory reset"], true),
        Confirm::DeleteEntry | Confirm::DeleteBackup => (["滑动以删除", "滑動以刪除", "Slide to delete"], true),
        Confirm::Install => (INSTALL, false),
        Confirm::InstallWipe => (["滑动以安装并清除", "滑動以安裝並清除", "Slide to install and wipe"], true),
        Confirm::RunScript => (INSTALL, true),
        Confirm::Merge => (["滑动以合并", "滑動以合併", "Slide to merge"], true),
        Confirm::Relabel => (["滑动以修复", "滑動以修復", "Slide to fix"], true),
        Confirm::SwitchSystem => (["滑动以切换并重启", "滑動以切換並重新啟動", "Slide to switch and reboot"], false),
        Confirm::RemoveSystem => (["滑动以删除", "滑動以刪除", "Slide to remove"], true),
        Confirm::InstallSystem => (INSTALL, false),
    };
    (s.lang.pick(label), danger)
}

/// The home page's large title, and what its search bar and the search
/// page's field say while nothing is typed.
pub const HOME_TITLE: &str = "Skyward";
pub const SEARCH_HINT: Text = ["搜索功能与分区", "搜尋功能與分割區", "Search features and partitions"];

const REBOOT_TARGETS: [(Text, Option<Text>); 6] = [
    (["系统", "系統", "System"], None),
    (["恢复模式", "復原模式", "Recovery"], None),
    (["Fastboot"; 3], Some(["用户空间 fastbootd", "使用者空間 fastbootd", "Userspace fastbootd"])),
    (["Bootloader"; 3], None),
    (["EDL"; 3], Some(["高通紧急下载模式", "高通緊急下載模式", "Qualcomm emergency download"])),
    (["关机", "關機", "Power off"], None),
];

/// What pages are called both on the rows that open them and in their own
/// titles.
const WLAN: Text = ["WLAN", "Wi-Fi", "Wi-Fi"];
const DECRYPT: Text = ["解密", "解密", "Decrypt"];
const ADVANCED: Text = ["高级", "進階", "Advanced"];
const SETTINGS: Text = ["设置", "設定", "Settings"];
const REBOOT: Text = ["重启", "重新啟動", "Reboot"];
const ABOUT: Text = ["关于本机", "關於手機", "About phone"];
const SPECS: Text = ["全部参数与信息", "全部規格與資訊", "All specs"];
const LOGS: Text = ["导出日志", "匯出記錄", "Export logs"];
const SCREEN_TIMEOUT: Text = ["息屏时间", "螢幕自動關閉", "Screen timeout"];
const UPDATE: Text = ["系统更新", "系統更新", "System update"];
const SKYWARD_UPDATE: Text = ["Skyward 更新", "Skyward 更新", "Skyward update"];
const FILES: Text = ["文件管理", "檔案管理", "File manager"];
const TERMINAL: Text = ["终端", "終端機", "Terminal"];
const LANGUAGE: Text = ["语言", "語言", "Language"];
const TIME_ZONE: Text = ["时区", "時區", "Time zone"];
const LICENSE: Text = ["开源许可", "開放原始碼授權", "Open source licenses"];
const FONTS: Text = ["界面字体", "介面字型", "UI fonts"];
const DUAL_BOOT: Text = ["双系统", "雙系統", "Dual boot"];

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
    ((bottom + t.m.margin * 2.0).ceil() as u32).max(t.m.h)
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
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    // The search bar is left room on the page and drawn over it, folding
    // away as the page scrolls up.
    let bar = Rect::new(t.m.margin, t.m.content_top, t.m.w as f32 - t.m.margin * 2.0, t.m.search_h);
    let mut y = bar.y + bar.h + t.m.after_search;
    let cards_top = y;
    let fold = Fold { bar, hint: l.pick(SEARCH_HINT), span: cards_top - bar.y };
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
        let why = l.pick(["点按查看原因", "點按查看原因", "Tap to see why"]);
        rows.push(Row::new(l.pick(SECOND_FAILED)).icon(RED, Icon::Systems).subtitle(why).trail(Trail::Arrow));
        opens.push(Some(Action::Open(Screen::Systems)));
    }
    let card = widgets::device_card(&mut c, t, f, y, &i.model, &sub, &rows);
    out.push(Hit { rect: card.rows[0], action: Action::Open(Screen::About) });
    out.extend(card.rows[1..].iter().zip(&opens).filter_map(|(&rect, open)| open.map(|action| Hit { rect, action })));
    y = card.bottom + t.m.card_gap;
    // On a phone whose WLAN the recovery brings up, the WLAN comes first
    // under the device, as on the stock settings home; with it, where the
    // phone can hold a second system, the two systems.
    let mut rows = Vec::new();
    let mut opens = Vec::new();
    if let Some(w) = &cx.state.wlan {
        rows.push(Row::new(l.pick(WLAN)).icon(BLUE, Icon::Wlan).trail(Trail::Value(fmt::wlan_value(l, w))));
        opens.push(Action::Open(Screen::Wlan));
    }
    let starts = systems_value(cx);
    if cx.state.offers(Screen::Systems) {
        let row = Row::new(l.pick(DUAL_BOOT)).icon(PURPLE, Icon::Systems);
        rows.push(match &starts {
            Some(value) => row.trail(Trail::Value(value)),
            None => row,
        });
        opens.push(Action::Open(Screen::Systems));
    }
    if !rows.is_empty() {
        let first = widgets::list(&mut c, t, f, y, &rows);
        out.extend(hits(&first.rows, &opens));
        y = first.bottom + t.m.card_gap;
    }
    let tools = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(["安装", "安裝", "Install"])).icon(GREEN, Icon::Install),
        Row::new(l.pick(["备份", "備份", "Backup"])).icon(BLUE, Icon::Backup),
        Row::new(l.pick(["恢复", "還原", "Restore"])).icon(PURPLE, Icon::Restore),
        Row::new(l.pick(["挂载", "掛載", "Mount"])).icon(SLATE, Icon::Mount),
        Row::new(l.pick(DECRYPT)).icon(BLUE, Icon::Lock).trail(Trail::Value(fmt::data(l, &cx.state.data))),
    ]);
    out.extend(hits(&tools.rows, &[
        Action::Open(Screen::Install),
        Action::Open(Screen::Backup),
        Action::Open(Screen::Restore),
        Action::Open(Screen::Mount),
        Action::Open(Screen::Decrypt),
    ]));
    y = tools.bottom + t.m.card_gap;
    let target = l.pick(REBOOT_TARGETS[cx.state.reboot].0);
    let system = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(["清除数据", "清除資料", "Wipe data"])).icon(RED, Icon::Wipe),
        Row::new(l.pick(ADVANCED)).icon(SLATE, Icon::More),
        Row::new(l.pick(SETTINGS)).icon(SLATE, Icon::Gear),
        Row::new(l.pick(REBOOT)).icon(ORANGE, Icon::Power).trail(Trail::Value(target)),
    ]);
    out.extend(hits(&system.rows, &[
        Action::Open(Screen::Wipe),
        Action::Open(Screen::Advanced),
        Action::Open(Screen::Settings),
        Action::Open(Screen::Reboot),
    ]));
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
            (None, _) => l.pick(["未安装第二系统", "未安裝第二系統", "No second system"]).into(),
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
        return l.pick(DOWNLOADING);
    }
    if s.newest().is_some_and(|r| cx.info.os_version.as_deref() != Some(r.version.as_str())) {
        return l.pick(["有新版本", "有新版本", "New version"]);
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
    let mut y = widgets::section(&mut c, t, f, themes.bottom, l.pick(["亮度", "亮度", "Brightness"]));
    let off = fmt::screen_off(l, SCREEN_OFF[s.screen_off]);
    let below = Row::new(l.pick(SCREEN_TIMEOUT)).trail(Trail::Value(&off));
    let slider = widgets::slider_card(&mut c, t, f, y, Caption::Sun, l.pick(["屏幕亮度", "螢幕亮度", "Screen brightness"]), s.brightness, Some(below));
    out.push(Hit { rect: slider.track, action: Action::Slider(Slider::Brightness) });
    if let Some(r) = slider.below {
        out.push(Hit { rect: r, action: Action::Open(Screen::ScreenOff) });
    }
    y = widgets::section(&mut c, t, f, slider.bottom, l.pick(["振动", "震動", "Vibration"]));
    let below = Row::new(l.pick(["触摸振动", "觸控震動", "Touch vibration"])).trail(Trail::Switch(s.vibrate));
    let vibration = widgets::slider_card(&mut c, t, f, y, Caption::Vibrate, l.pick(["振动强度", "震動強度", "Vibration intensity"]), s.vibration, Some(below));
    out.push(Hit { rect: vibration.track, action: Action::Slider(Slider::Vibration) });
    if let Some(r) = vibration.below {
        out.push(Hit { rect: r, action: Action::Toggle(Switch::Vibrate) });
    }
    y = widgets::section(&mut c, t, f, vibration.bottom, l.pick(["通用", "一般", "General"]));
    let general = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(LANGUAGE)).icon(BLUE, Icon::Globe).trail(Trail::Value(l.name())),
        Row::new(l.pick(TIME_ZONE)).icon(ORANGE, Icon::Clock).trail(Trail::Value(l.pick(TIME_ZONES[s.time_zone].0))),
    ]);
    out.extend(hits(&general.rows, &[Action::Open(Screen::Language), Action::Open(Screen::TimeZone)]));
    y = widgets::section(&mut c, t, f, general.bottom, "USB");
    let usb = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["文件传输", "檔案傳輸", "File transfer"])).icon(BLUE, Icon::Usb).trail(Trail::Switch(s.file_transfer))]);
    out.extend(hits(&usb.rows, &[Action::Toggle(Switch::FileTransfer)]));
    let note = l.pick([
        "开启后，可在电脑上管理内部存储中的文件，需先完成「解密」。",
        "開啟後，可在電腦上管理內部儲存空間中的檔案，需先完成「解密」。",
        "When on, a computer can manage the files in internal storage, once it is decrypted.",
    ]);
    y = widgets::footnote(&mut c, t, f, usb.bottom, note);
    y = widgets::section(&mut c, t, f, y, l.pick(["调试", "偵錯", "Debugging"]));
    let debug = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(TERMINAL)).icon(SLATE, Icon::Terminal)]);
    out.extend(hits(&debug.rows, &[Action::Open(Screen::Terminal)]));
    y = widgets::section(&mut c, t, f, debug.bottom, l.pick(["关于", "關於", "About"]));
    let mut rows = vec![Row::new(l.pick(ABOUT)).trail(Trail::Value(VERSION))];
    let mut opens = vec![Action::Open(Screen::About)];
    if s.offers(Screen::SkywardUpdate) {
        let row = Row::new(l.pick(SKYWARD_UPDATE));
        rows.push(match skyward_value(cx) {
            Some(value) => row.trail(Trail::Value(value)),
            None => row,
        });
        opens.push(Action::Open(Screen::SkywardUpdate));
    }
    let about = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&about.rows, &opens));
    Frame::new(t, fit(c, t, about.bottom), out, l.pick(SETTINGS), top)
}

/// What the settings page says beside its Skyward update entry: an update
/// under way, or a newer release found.
fn skyward_value(cx: &Ctx) -> Option<&'static str> {
    let (s, l) = (cx.state, cx.state.lang);
    if s.updating_skyward() {
        return Some(l.pick(["正在更新", "正在更新", "Updating"]));
    }
    match (s.skyward_newest(), &s.skyward_job) {
        (_, Some(SkywardJob { outcome: Some(SkywardEnd::Updated(_)), .. })) => Some(l.pick(["已更新", "已更新", "Updated"])),
        (Some(n), _) if n.newer && n.build.is_some() => Some(l.pick(["有新版本", "有新版本", "New version"])),
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
    let note = l.pick([
        "超过所选时间未操作，屏幕将变暗并熄灭，按电源键即可点亮。",
        "超過所選時間未操作，螢幕將變暗並關閉，按電源鍵即可喚醒。",
        "After the time chosen with nothing touched, the screen dims and turns off. Press the power key to light it again.",
    ]);
    let y = widgets::footnote(&mut c, t, f, times.bottom, note);
    if SCREEN_OFF[s.screen_off].is_none() {
        let note = l.pick([
            "屏幕将保持常亮，长时间显示同一画面可能留下残影。",
            "螢幕將保持恆亮，長時間顯示同一畫面可能留下殘影。",
            "The screen stays on. Showing the same image for long may leave it burnt in.",
        ]);
        widgets::footnote(&mut c, t, f, y, note);
    }
    Frame::new(t, c, hits(&times.rows, &picks), l.pick(SCREEN_TIMEOUT), top)
}

/// Mount manager: the slot's partitions, each with a switch to mount it
/// read-only or unmount it; one that programs are using says which, and
/// stays.
pub fn mount(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;

    let y = widgets::section_first(&mut c, t, f, top, l.pick(["分区", "分割區", "Partitions"]));
    let bottom = match &s.mounts {
        Err(why) => {
            let y = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["无法读取分区", "無法讀取分割區", "Couldn't read the partitions"])).trail(Trail::None)]).bottom;
            widgets::footnote(&mut c, t, f, y, why)
        }
        Ok(mounts) if mounts.is_empty() => {
            widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["暂无可挂载的分区", "沒有可掛載的分割區", "No partitions to mount"])).trail(Trail::None)]).bottom
        }
        Ok(mounts) => {
            let subs: Vec<String> = mounts
                .iter()
                .map(|m| {
                    let (point, holders) = (&m.point, m.holders.join(l.pick(["、", "、", ", "])));
                    match m.holders.is_empty() {
                        true => point.clone(),
                        false => tr!(l, "{point} · {holders} 正在使用", "{point} · {holders} 正在使用", "{point} · in use by {holders}"),
                    }
                })
                .collect();
            let rows: Vec<Row> = mounts
                .iter()
                .zip(&subs)
                .map(|(m, sub)| {
                    let trail = if m.holders.is_empty() { Trail::Switch(m.mounted) } else { Trail::Text(l.pick(["使用中", "使用中", "In use"])) };
                    Row::new(&m.base).subtitle(sub).trail(trail)
                })
                .collect();
            let list = widgets::list(&mut c, t, f, y, &rows);
            out.extend(list.rows.iter().zip(mounts).enumerate().map(|(i, (&rect, m))| {
                let action = match m.holders.is_empty() {
                    true => Action::MountToggle(i),
                    false => Action::Unavailable(l.pick(["无法卸载正在使用的分区", "無法卸載正在使用的分割區", "A partition in use can't be unmounted"])),
                };
                Hit { rect, action }
            }));
            let y = match &s.mount_failure {
                Some(failure) => widgets::footnote(&mut c, t, f, list.bottom, &fmt::mount_failure(l, failure)),
                None => list.bottom,
            };
            let about = match cx.info.slot.map(|s| s.letter()) {
                Some(slot) => tr!(
                    l,
                    "仅显示当前槽位 {slot} 在 super 中的分区，挂载后均为只读。",
                    "僅顯示目前槽位 {slot} 在 super 中的分割區，掛載後均為唯讀。",
                    "Only the current slot {slot}'s partitions in super show here, all mounted read-only."
                ),
                None => l
                    .pick(["仅显示 super 中的分区，挂载后均为只读。", "僅顯示 super 中的分割區，掛載後均為唯讀。", "Only partitions in super show here, all mounted read-only."])
                    .into(),
            };
            widgets::footnote(&mut c, t, f, y, &about)
        }
    };

    Frame::new(t, fit(c, t, bottom), out, l.pick(["挂载", "掛載", "Mount"]), top)
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
    Frame { header: false, hero: Some(art), ..Frame::new(t, canvas, out, cx.state.lang.pick(ABOUT), summary_top) }
}

const MODEL: Text = ["设备型号", "裝置型號", "Model"];
const PROCESSOR: Text = ["处理器", "處理器", "Processor"];
const RAM: Text = ["运行内存", "記憶體", "RAM"];
const BATTERY_CAPACITY: Text = ["电池容量", "電池容量", "Battery capacity"];
const SCREEN_SIZE: Text = ["屏幕尺寸", "螢幕尺寸", "Screen size"];
const RESOLUTION: Text = ["分辨率", "解析度", "Resolution"];
const OS_VERSION: Text = ["OS 版本", "OS 版本", "OS version"];
const ANDROID_VERSION: Text = ["Android 版本", "Android 版本", "Android version"];
const CURRENT_SLOT: Text = ["当前槽位", "目前槽位", "Current slot"];
const DATA_PART: Text = ["data 分区", "data 分割區", "data partition"];

/// The phone's processor by its marketing name in `l`, from the phone's
/// settings, or else by its cores.
fn soc(l: Lang, i: &DeviceInfo) -> Option<&str> {
    i.soc.as_ref().map(|n| l.pick([n.hans.as_str(), n.hant.as_str(), n.en.as_str()]))
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
        Fact { title: l.pick(MODEL), value: i.model.clone(), link: false },
        Fact { title: l.pick(["存储空间", "儲存空間", "Storage"]), value: storage, link: true },
    ];
    if let Some(v) = &i.os_version {
        facts.push(Fact { title: l.pick(OS_VERSION), value: v.clone(), link: false });
    }
    if let Some(r) = &i.android {
        facts.push(Fact { title: l.pick(ANDROID_VERSION), value: r.clone(), link: false });
    }
    let summary = widgets::facts(c, t, f, m.dp(490.7), &facts, &tone);
    out.push(Hit { rect: summary.rows[1], action: Action::Open(Screen::Specs) });

    let processor = soc(l, i).map_or_else(|| fmt::cpu(l, &i.cpu), str::to_string);
    let mut specs = vec![(l.pick(PROCESSOR), processor), (l.pick(RAM), fmt::ram(i.ram_kib))];
    if let Some(design) = i.battery.as_ref().and_then(|b| b.design) {
        specs.push((l.pick(BATTERY_CAPACITY), fmt::mah(design)));
    }
    if let Some(d) = i.screen {
        specs.push((l.pick(SCREEN_SIZE), fmt::inches(d)));
    }
    if let Some(p) = i.panel {
        specs.push((l.pick(RESOLUTION), fmt::panel(p)));
    }
    let y = widgets::spec_card(c, t, f, summary.bottom + m.card_gap, &fmt::model_lines(&i.model), &specs, &tone);
    let links = widgets::list_toned(c, t, f, y + m.card_gap, &[Row::new(l.pick(SPECS)), Row::new(l.pick(LOGS))], &tone);
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

    let mut grid = vec![Spec { icon: Outline::Phone, label: l.pick(MODEL), value: fmt::model_lines(&i.model) }];
    if let Some(design) = i.battery.as_ref().and_then(|b| b.design) {
        grid.push(Spec { icon: Outline::Battery, label: l.pick(BATTERY_CAPACITY), value: fmt::mah(design) });
    }
    let cpu = match soc(l, i) {
        Some(soc) => format!("{soc}\n{}", fmt::cpu(l, &i.cpu)),
        None => fmt::cpu(l, &i.cpu),
    };
    grid.push(Spec { icon: Outline::Chip, label: l.pick(PROCESSOR), value: cpu });
    grid.push(Spec { icon: Outline::Memory, label: l.pick(RAM), value: fmt::ram(i.ram_kib) });
    if let Some(p) = i.panel {
        grid.push(Spec { icon: Outline::Resolution, label: l.pick(RESOLUTION), value: fmt::panel(p) });
    }
    if let Some(d) = i.screen {
        grid.push(Spec { icon: Outline::Screen, label: l.pick(SCREEN_SIZE), value: fmt::inches(d) });
    }
    grid.push(Spec { icon: Outline::Storage, label: l.pick(["存储容量", "儲存容量", "Storage capacity"]), value: fmt::capacity(i.storage.total) });
    grid.push(Spec { icon: Outline::Tag, label: l.pick(["设备代号", "裝置代號", "Codename"]), value: i.codename.clone() });
    let mut y = widgets::spec_grid(&mut c, t, f, top, &grid);

    if let Some(v) = &i.os_version {
        y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(OS_VERSION), v.clone())]);
    }
    if let Some(r) = &i.android {
        y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(ANDROID_VERSION), fmt::android(l, r, i.security_patch.as_ref()))]);
    }
    y = widgets::info_card(&mut c, t, f, y + gap, &[
        (l.pick(["Recovery 版本", "Recovery 版本", "Recovery version"]), format!("Skyward {VERSION}")),
        (l.pick(["内核版本", "核心版本", "Kernel version"]), i.kernel.clone()),
    ]);

    let mut boot = Vec::new();
    if let Some(s) = i.slot {
        boot.push((l.pick(CURRENT_SLOT), s.letter().to_string()));
        boot.push((l.pick(UPDATE), fmt::snapshot(l, &i.snapshot).to_string()));
    }
    if let Some(unlocked) = i.boot.unlocked {
        let state = if unlocked { ["已解锁", "已解鎖", "Unlocked"] } else { ["已锁定", "已鎖定", "Locked"] };
        boot.push(("Bootloader", l.pick(state).to_string()));
    }
    if let Some(v) = &i.boot.verified {
        boot.push((l.pick(["验证启动", "驗證開機", "Verified boot"]), fmt::verified(l, v)));
    }
    boot.push((l.pick(DATA_PART), fmt::data(l, &cx.state.data).to_string()));
    y = widgets::info_card(&mut c, t, f, y + gap, &boot);

    if let Some(b) = &i.battery {
        let mut rows = vec![(l.pick(["电量", "電量", "Level"]), fmt::level(l, b))];
        if let Some(v) = b.temp {
            rows.push((l.pick(["温度", "溫度", "Temperature"]), fmt::celsius(v)));
        }
        if let Some(v) = fmt::wear(l, b) {
            rows.push((l.pick(["健康", "健康狀態", "Health"]), v));
        }
        if let Some(v) = b.cycles {
            rows.push((l.pick(["充电循环", "充電循環", "Charge cycles"]), tr!(l, "{v} 次", "{v} 次", "{v}")));
        }
        if let Some(v) = b.voltage {
            rows.push((l.pick(["电压", "電壓", "Voltage"]), fmt::volts(v)));
        }
        y = widgets::info_card(&mut c, t, f, y + gap, &rows);
    }

    let total = fmt::capacity(i.storage.total);
    let storage = match i.storage.free.map(fmt::gb) {
        Some(free) => tr!(l, "可用存储 {free} / 总存储 {total}", "可用空間 {free} / 總容量 {total}", "{free} free of {total}"),
        None => tr!(l, "总存储 {total} · data 分区未挂载", "總容量 {total} · data 分割區未掛載", "{total} in all · data partition not mounted"),
    };
    y = widgets::info_card(&mut c, t, f, y + gap, &[(l.pick(["机身存储", "內建儲存空間", "Internal storage"]), storage)]);
    let links = widgets::list(&mut c, t, f, y + gap, &[
        Row::new(l.pick(LICENSE)).trail(Trail::Value("GPL-3.0")),
        Row::new(l.pick(FONTS)).trail(Trail::Value("MiSans")),
    ]);
    out.extend(hits(&links.rows, &[Action::Open(Screen::License), Action::Open(Screen::FontLicense)]));
    Frame::new(t, fit(c, t, links.bottom), out, l.pick(SPECS), top)
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
            l.pick(["解密 data 分区", "解密 data 分割區", "Decrypt the data partition"]),
            Some(l.pick(["正在读取锁屏方式", "正在讀取螢幕鎖定方式", "Reading the screen lock"])),
            None,
        ),
        Data::Open => data_status(
            cx,
            c,
            Icon::Folder,
            l.pick(["data 分区已解密", "data 分割區已解密", "The data partition is decrypted"]),
            Some(l.pick(["你可以在「文件管理」中查看本机文件", "你可以在「檔案管理」中查看本機檔案", "You can see the phone's files in File manager"])),
            None,
        ),
        Data::Failed(why) => data_status(cx, c, Icon::Lock, l.pick(["未能打开 data 分区", "無法開啟 data 分割區", "Couldn't open the data partition"]), None, Some(why)),
        Data::Erased => data_status(
            cx,
            c,
            Icon::Wipe,
            l.pick(DATA_ERASED),
            Some(l.pick(["下次开机时将重新创建", "下次開機時將重新建立", "It's made anew when the phone next starts"])),
            None,
        ),
    }
}

/// Where the partition stands, under its sign: a line on it, or why it
/// did not open and a way to try again; then the way off the page.
fn data_status(cx: &Ctx, mut c: Canvas, icon: Icon, title: &str, note: Option<&str>, why: Option<&str>) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let m = &t.m;
    let x = m.w as f32 / 2.0;
    let sign_cy = m.dp(150.0);
    c.fill_circle(x, sign_cy, m.dp(30.0), t.pal.card);
    icons::glyph(&mut c, icon, x, sign_cy, m.dp(40.0), t.pal.accent);
    c.text_center(&f.heading, title, x, sign_cy + m.dp(64.0), m.dp(24.0), t.pal.text);
    if let Some(note) = note {
        c.text_center(&f.body, note, x, sign_cy + m.dp(98.0), m.sub_px, t.pal.subtitle);
    }
    let mut out = Vec::new();
    if let Some(why) = why {
        widgets::paragraph(&mut c, t, f, sign_cy + m.dp(88.0), why);
        let again = widgets::button(&mut c, t, f, m.h as f32 * 0.62, l.pick(["重试", "重試", "Try again"]));
        out.push(Hit { rect: again, action: Action::OpenData });
    }
    let leave_cy = m.h as f32 - m.dp(64.0);
    let leave = l.pick(if matches!(cx.state.data, Data::Open | Data::Erased) { ["完成", "完成", "Done"] } else { NOT_NOW });
    c.text_center(&f.heading, leave, x, leave_cy, m.row_title_px, t.pal.accent);
    out.push(Hit { rect: Rect::new(x - m.dp(80.0), leave_cy - m.dp(24.0), m.dp(160.0), m.dp(48.0)), action: Action::Skip });
    Frame { header: false, ..Frame::new(t, c, out, l.pick(DECRYPT), 0.0) }
}

/// What leaves the decrypt page with the data partition left as it is.
const NOT_NOW: Text = ["暂不解密", "暫不解密", "Not now"];

/// What a row whose work waits on the data partition says under it until
/// that is decrypted.
const DECRYPT_NEEDED: Text = ["需先完成「解密」", "需先完成「解密」", "Decrypt first"];

/// A user's own files wait on their screen lock: the control it is
/// answered with, which takes nothing while what was entered is checked.
fn code(cx: &Ctx, mut c: Canvas, user: u32, keys: Keys, checking: bool) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let m = &t.m;
    let x = m.w as f32 / 2.0;
    let mut out = Vec::new();
    let title = match (keys == Keys::Pattern, user) {
        (true, 0) => l.pick(["绘制锁屏图案", "繪製螢幕鎖定圖案", "Draw the screen lock pattern"]).to_string(),
        (true, user) => tr!(l, "绘制用户 {user} 的锁屏图案", "繪製使用者 {user} 的螢幕鎖定圖案", "Draw user {user}'s screen lock pattern"),
        (false, 0) => l.pick(["输入锁屏密码", "輸入螢幕鎖定密碼", "Enter the screen lock"]).to_string(),
        (false, user) => tr!(l, "输入用户 {user} 的锁屏密码", "輸入使用者 {user} 的螢幕鎖定密碼", "Enter user {user}'s screen lock"),
    };
    let note = l.pick(if checking { ["正在验证", "正在驗證", "Checking"] } else { ["解密后可查看本机文件", "解密後可查看本機檔案", "Decrypt to see the phone's files"] });
    c.text_center(&f.heading, &title, x, m.dp(130.0), m.dp(24.0), t.pal.text);
    c.text_center(&f.body, note, x, m.dp(166.0), m.sub_px, t.pal.subtitle);
    let mut pattern = None;
    let bottom = match keys {
        Keys::Letters => {
            let top = m.h as f32 - widgets::keyboard_height(t);
            let caps = widgets::keyboard(&mut c, t, f, l, top, s.layer, s.key_held(), l.pick(DECRYPT));
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
    c.text_center(&f.heading, l.pick(NOT_NOW), x, skip_cy, m.row_title_px, t.pal.accent);
    out.push(Hit { rect: Rect::new(x - m.dp(80.0), skip_cy - m.dp(24.0), m.dp(160.0), m.dp(48.0)), action: Action::Skip });
    Frame { header: false, pattern, ..Frame::new(t, c, out, l.pick(DECRYPT), 0.0) }
}

/// What a slot is shown as where the boot control could not be read.
const UNREADABLE: Text = ["无法读取", "無法讀取", "Unreadable"];

/// Reboot target picker with a swipe-to-confirm control.
pub fn reboot(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let mut y = widgets::section_first(&mut c, t, f, top, l.pick(["重启到", "重新啟動到", "Reboot to"]));
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
            Some(Ok(slots)) if slots.active == slot => {
                let slot = slot.letter();
                tr!(l, "{slot}（当前）", "{slot}（目前）", "{slot} (current)")
            }
            Some(Ok(slots)) => slots.active.letter().to_string(),
            Some(Err(_)) => l.pick(UNREADABLE).to_string(),
            None => String::new(),
        };
        y = widgets::section(&mut c, t, f, y, l.pick(["槽位", "槽位", "Slot"]));
        let slots = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["下次启动", "下次啟動", "Next boot"])).trail(Trail::Value(&next))]);
        out.extend(hits(&slots.rows, &[Action::Open(Screen::Slot)]));
        y = slots.bottom;
    }
    let (label, danger) = confirm_label(Confirm::Reboot, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Reboot) });
    Frame::new(t, c, out, l.pick(REBOOT), top)
}

/// Installing, opened from the home page: the system on the phone now, and
/// where a package can come from.
pub fn install(cx: &Ctx) -> Frame {
    let (t, f, i, l) = (cx.t, cx.f, cx.info, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let mut facts = Vec::new();
    if let Some(v) = &i.os_version {
        facts.push((l.pick(["当前系统", "目前系統", "Current system"]), v.clone()));
    }
    if let Some(p) = &i.security_patch {
        facts.push((l.pick(SECURITY_PATCH), fmt::patch(l, p)));
    }
    if let Some(s) = i.slot {
        let (slot, other) = (s.letter(), s.other().letter());
        let detail = tr!(l, "{slot}，系统更新将安装到槽位 {other}", "{slot}，系統更新將安裝到槽位 {other}", "{slot}, and a system update installs to slot {other}");
        facts.push((l.pick(CURRENT_SLOT), detail));
    }
    let ways = l.pick(["安装方式", "安裝方式", "Ways to install"]);
    let y = if facts.is_empty() {
        widgets::section_first(&mut c, t, f, top, ways)
    } else {
        let y = widgets::section_first(&mut c, t, f, top, l.pick(["本机", "本機", "This phone"]));
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
        (
            ["选择内部存储中的更新包或镜像", "選擇內部儲存空間中的更新檔或映像檔", "Choose a package or image in internal storage"],
            Action::Open(Screen::Choose(Place::Storage)),
        )
    } else if *data == Data::Erased {
        (DATA_ERASED, Action::Open(Screen::Decrypt))
    } else {
        (DECRYPT_NEEDED, Action::Open(Screen::Decrypt))
    };
    // The drive's page waits for one to be plugged in, and says why one
    // that is cannot be read.
    let usb = match &cx.state.usb_drive {
        Some(UsbDrive::Shown) => ["选择 U 盘中的更新包或镜像", "選擇 USB 隨身碟中的更新檔或映像檔", "Choose a package or image on the USB drive"],
        Some(UsbDrive::Unreadable(_)) => USB_DRIVE_UNREADABLE,
        None => USB_DRIVE_WANTED,
    };
    rows.push(Row::new(install_title(l, Source::File(Place::Storage))).icon(BLUE, Icon::Phone).subtitle(l.pick(storage)));
    rows.push(Row::new(install_title(l, Source::File(Place::UsbDrive))).icon(PURPLE, Icon::UsbDrive).subtitle(l.pick(usb)));
    let sideload = l.pick(["在电脑上运行 adb sideload", "在電腦上執行 adb sideload", "Run adb sideload on the computer"]);
    rows.push(Row::new(install_title(l, Source::Computer)).icon(GREEN, Icon::Usb).subtitle(sideload));
    let methods = widgets::list(&mut c, t, f, y, &rows);
    let from_usb = Action::Open(Screen::Choose(Place::UsbDrive));
    let mut out = hits(&methods.rows[methods.rows.len() - 3..], &[from_storage, from_usb, Action::Sideload]);
    // A phone that can hold a second system installs one from here too.
    let what = l.pick(match cx.state.dual_boot {
        true => [
            "支持安装 A/B 完整更新包、带安装脚本的刷机包和第二系统的安装包，以及刷入 boot、init_boot、vendor_boot、recovery、dtbo、vbmeta 分区镜像。系统更新将安装到另一个槽位，重启后从该槽位启动；第二系统安装在 data 分区中，可在「双系统」中切换。选择文件时仅列出文件夹以及 .zip、.img 文件。",
            "支援安裝 A/B 完整更新檔、帶有安裝指令碼的刷機包和第二系統的安裝檔，以及刷入 boot、init_boot、vendor_boot、recovery、dtbo、vbmeta 分割區映像檔。系統更新將安裝到另一個槽位，重新啟動後從該槽位啟動；第二系統安裝在 data 分割區中，可在「雙系統」中切換。選擇檔案時僅列出資料夾以及 .zip、.img 檔案。",
            "Installs A/B full OTA packages, flashable zips with an install script and second systems' packages, and flashes boot, init_boot, vendor_boot, recovery, dtbo and vbmeta partition images. A system update installs to the other slot, which starts after a reboot; a second system installs into the data partition, and Dual boot switches to it. Choosing a file lists only folders and .zip and .img files.",
        ],
        false => [
            "支持安装 A/B 完整更新包和带安装脚本的刷机包，以及刷入 boot、init_boot、vendor_boot、recovery、dtbo、vbmeta 分区镜像。系统更新将安装到另一个槽位，重启后从该槽位启动。选择文件时仅列出文件夹以及 .zip、.img 文件。",
            "支援安裝 A/B 完整更新檔和帶有安裝指令碼的刷機包，以及刷入 boot、init_boot、vendor_boot、recovery、dtbo、vbmeta 分割區映像檔。系統更新將安裝到另一個槽位，重新啟動後從該槽位啟動。選擇檔案時僅列出資料夾以及 .zip、.img 檔案。",
            "Installs A/B full OTA packages and flashable zips with an install script, and flashes boot, init_boot, vendor_boot, recovery, dtbo and vbmeta partition images. A system update installs to the other slot, which starts after a reboot. Choosing a file lists only folders and .zip and .img files.",
        ],
    });
    let y = widgets::footnote(&mut c, t, f, methods.bottom, what);
    let y = widgets::section(&mut c, t, f, y, l.pick(["选项", "選項", "Options"]));
    let check = Row::new(l.pick(["验证刷机包签名", "驗證刷機包簽章", "Verify zip signatures"])).trail(Trail::Switch(cx.state.check_signature));
    let signature = widgets::list(&mut c, t, f, y, &[check]);
    out.extend(hits(&signature.rows, &[Action::Toggle(Switch::Signature)]));
    let note = l.pick(["开启后，仅安装带有本机官方签名的刷机包。", "開啟後，僅安裝帶有本機官方簽章的刷機包。", "When on, only zips signed with this phone's official keys install."]);
    let mut y = widgets::footnote(&mut c, t, f, signature.bottom, note);
    if cx.state.recovery_partitions {
        let row = Row::new(l.pick(["自动重新刷入 Skyward", "自動重新刷入 Skyward", "Flash Skyward again"])).trail(Trail::Switch(cx.state.keep_recovery));
        let options = widgets::list(&mut c, t, f, y + t.m.card_gap, &[row]);
        out.extend(hits(&options.rows, &[Action::Toggle(Switch::KeepRecovery)]));
        let note = l.pick([
            "开启后，安装系统更新或刷机包时被覆盖的 recovery 分区将重新刷入 Skyward。",
            "開啟後，安裝系統更新或刷機包時被覆寫的 recovery 分割區將重新刷入 Skyward。",
            "When on, Skyward goes back into the recovery partitions a system update or flashable zip writes over.",
        ]);
        y = widgets::footnote(&mut c, t, f, options.bottom, note);
    }
    Frame::new(t, fit(c, t, y), out, l.pick(["安装", "安裝", "Install"]), top)
}

const SECURITY_PATCH: Text = ["安全补丁", "安全性修補程式", "Security patch"];

/// How installing from `source` is named: on the install page, and as the
/// title of the page that installs.
fn install_title(l: Lang, source: Source) -> &'static str {
    l.pick(match source {
        Source::Computer => ["通过电脑安装", "透過電腦安裝", "Install from a computer"],
        Source::File(Place::Storage) => ["从内部存储安装", "從內部儲存空間安裝", "Install from internal storage"],
        Source::File(Place::UsbDrive) => ["从 U 盘安装", "從 USB 隨身碟安裝", "Install from a USB drive"],
        Source::Opened { .. } => ["从文件管理安装", "從檔案管理安裝", "Install from File manager"],
        Source::Asked { security: false } => ["安装系统更新", "安裝系統更新", "Install a system update"],
        Source::Asked { security: true } => ["安装安全更新", "安裝安全性更新", "Install a security update"],
        Source::Rescue => ["救援模式", "救援模式", "Rescue mode"],
    })
}

/// What a place is called at the top of its folders.
fn place_name(l: Lang, place: Place) -> &'static str {
    l.pick(match place {
        Place::Storage => ["内部存储", "內部儲存空間", "Internal storage"],
        Place::UsbDrive => ["U 盘", "USB 隨身碟", "USB drive"],
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
            let p = percent(s);
            (tr!(l, "第 1 步，共 3 步：写入新系统 {p}%", "第 1 步，共 3 步：寫入新系統 {p}%", "Step 1 of 3: writing the new system {p}%"), Some(s))
        }
        Some(Stage::Verifying { index, count, partition }) => {
            let n = index + 1;
            let said = tr!(l, "第 2 步，共 3 步：校验 {partition}（{n}/{count}）", "第 2 步，共 3 步：驗證 {partition}（{n}/{count}）", "Step 2 of 3: verifying {partition} ({n}/{count})");
            (said, Some(*index as f32 / *count as f32))
        }
        Some(Stage::Finishing) => (l.pick(["第 3 步，共 3 步：完成安装", "第 3 步，共 3 步：完成安裝", "Step 3 of 3: finishing"]).into(), None),
        Some(Stage::Reflashing { done, total }) => (l.pick(REFLASHING).into(), Some(*done as f32 / *total as f32)),
        Some(Stage::Flashing { .. } | Stage::Script { .. } | Stage::Signature { .. } | Stage::Allocating { .. } | Stage::Root { .. } | Stage::Storing) | None => {
            (l.pick(GETTING_READY).into(), None)
        }
    }
}

/// Where installing a system beside the installed one stands, and the share
/// of its step done when known.
fn system_stage(l: Lang, stage: Option<&Stage>) -> (String, Option<f32>) {
    match stage {
        Some(Stage::Allocating { done, total }) => {
            let s = share(*done, *total);
            let p = percent(s);
            (tr!(l, "第 1 步，共 3 步：在 data 分区中分配空间 {p}%", "第 1 步，共 3 步：在 data 分割區中分配空間 {p}%", "Step 1 of 3: setting room aside in the data partition {p}%"), Some(s))
        }
        Some(Stage::Root { done, total }) => {
            let s = share(*done, *total);
            let p = percent(s);
            let said = match done * 2 <= *total {
                true => tr!(l, "第 2 步，共 3 步：写入系统 {p}%", "第 2 步，共 3 步：寫入系統 {p}%", "Step 2 of 3: writing the system {p}%"),
                false => tr!(l, "第 2 步，共 3 步：校验系统 {p}%", "第 2 步，共 3 步：驗證系統 {p}%", "Step 2 of 3: verifying the system {p}%"),
            };
            (said, Some(s))
        }
        Some(Stage::Storing) => (l.pick(["第 3 步，共 3 步：保存启动镜像", "第 3 步，共 3 步：儲存啟動映像檔", "Step 3 of 3: keeping its boot images"]).into(), None),
        _ => (l.pick(GETTING_READY).into(), None),
    }
}

const REFLASHING: Text = ["正在重新刷入 Skyward", "正在重新刷入 Skyward", "Flashing Skyward again"];

/// What the card says while an install script runs, and how far it is: the
/// last line the script showed and how far it says it is, then Skyward
/// going back into the recovery partitions it wrote over.
fn script_stage<'a>(l: Lang, stage: Option<&Stage>, said: &'a [String]) -> (&'a str, Option<f32>) {
    let running = l.pick(["正在运行安装脚本", "正在執行安裝指令碼", "Running the install script"]);
    let last = said.iter().rev().map(|line| line.trim()).find(|line| !line.is_empty()).unwrap_or(running);
    match stage {
        Some(Stage::Reflashing { done, total }) => (l.pick(REFLASHING), Some(*done as f32 / *total as f32)),
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
            let slots = fmt::slots(l, written);
            let said = match (l, written.len()) {
                (Lang::En, 1) => format!("Skyward went back into {slots}'s recovery partition."),
                _ => tr!(l, "已将 Skyward 重新刷入{slots} 的 recovery 分区。", "已將 Skyward 重新刷入{slots} 的 recovery 分割區。", "Skyward went back into the recovery partitions of {slots}."),
            };
            widgets::footnote(c, t, f, y, &said)
        }
        Some(Reflash::Failed(why)) => {
            let note = l.pick(["未能将 Skyward 重新刷入 recovery 分区。", "無法將 Skyward 重新刷入 recovery 分割區。", "Couldn't flash Skyward back into the recovery partitions."]);
            let y = widgets::footnote(c, t, f, y, note);
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
    let y = widgets::section(c, cx.t, cx.f, y, cx.state.lang.pick(["安装脚本输出", "安裝指令碼輸出", "Install script output"]));
    *rows = Some(Rows { edges: widgets::output_edges(c, cx.t, cx.f, y, lines), action: None });
    y
}

fn pick_title(l: Lang, pick: Pick, current: Option<Slot>) -> String {
    let letter = |s: Option<Slot>| s.map(|s| format!(" {}", s.letter())).unwrap_or_default();
    match pick {
        Pick::Current => format!("{}{}", l.pick(CURRENT_SLOT), letter(current)),
        Pick::Other => format!("{}{}", l.pick(["另一个槽位", "另一個槽位", "Other slot"]), letter(current.map(Slot::other))),
        Pick::Both => l.pick(["两个槽位", "兩個槽位", "Both slots"]).into(),
    }
}

/// Words the installing page says of more than one of its steps.
const SWITCHING_USB: Text = ["正在切换 USB 模式", "正在切換 USB 模式", "Switching USB mode"];
const READING_FILE: Text = ["正在读取文件", "正在讀取檔案", "Reading the file"];
const WAITING: Text = ["等待电脑发送", "等待電腦傳送", "Waiting for the computer"];
const CONNECT: Text = ["请用数据线连接电脑，并在电脑上运行以下命令", "請用傳輸線連接電腦，並在電腦上執行以下指令", "Connect a computer with a cable, and run these commands on it"];
const RUN_ON_COMPUTER: Text = ["在电脑上运行", "在電腦上執行", "Run on the computer"];
const KEEP_CABLE: Text = ["读取完成前，请不要断开数据线。", "讀取完成前，請不要拔除傳輸線。", "Keep the cable in until it's read."];
const KEEP_DRIVE: Text = ["读取完成前，请不要拔出 U 盘。", "讀取完成前，請不要拔出 USB 隨身碟。", "Keep the USB drive in until it's read."];
const GETTING_READY: Text = ["正在准备", "正在準備", "Getting ready"];

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
            let row = standing(l.pick(SWITCHING_USB), GREEN, Icon::Usb, l.pick(["从 adb 调试切换到 sideload 模式", "從 adb 偵錯切換到 sideload 模式", "From adb debugging to sideload"]));
            widgets::list(&mut c, t, f, top, &[row]).bottom
        }
        Install::Idle | Install::Starting | Install::Ending if s.source == Source::Rescue => {
            let row = standing(l.pick(SWITCHING_USB), GREEN, Icon::Usb, l.pick(["从 adb 调试切换到救援模式", "從 adb 偵錯切換到救援模式", "From adb debugging to rescue mode"]));
            widgets::list(&mut c, t, f, top, &[row]).bottom
        }
        Install::Idle | Install::Starting | Install::Ending => {
            widgets::list(&mut c, t, f, top, &[Row::new(l.pick(READING_FILE)).icon(GREEN, Icon::Install).trail(Trail::None)]).bottom
        }
        Install::Opening => {
            let opening = Row::new(l.pick(["正在打开 data 分区", "正在開啟 data 分割區", "Opening the data partition"])).icon(GREEN, Icon::Lock).trail(Trail::None);
            let y = widgets::list(&mut c, t, f, top, &[opening]).bottom;
            asked_package(&mut c, cx, y)
        }
        Install::Locked(_) => {
            let row = standing(l.pick(["暂无法读取更新包", "暫時無法讀取更新檔", "The package can't be read yet"]), ORANGE, Icon::Lock, l.pick(DECRYPT_NEEDED));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let open = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(DECRYPT))]);
            out.extend(hits(&open.rows, &[Action::Open(Screen::Decrypt)]));
            let y = widgets::footnote(&mut c, t, f, open.bottom, l.pick(["解密后将自动安装。", "解密後將自動安裝。", "It installs once the data partition is decrypted."]));
            asked_package(&mut c, cx, y)
        }
        Install::Waiting if s.source == Source::Rescue => {
            let row = standing(l.pick(WAITING), GREEN, Icon::Usb, l.pick(CONNECT));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(RUN_ON_COMPUTER));
            let y = widgets::info_card(&mut c, t, f, y, &[
                (
                    l.pick(["adb rescue install 文件名", "adb rescue install 檔案名稱", "adb rescue install <file>"]),
                    l.pick([
                        "将文件名替换为系统更新包、刷机包或分区镜像",
                        "將檔案名稱替換為系統更新檔、刷機包或分割區映像檔",
                        "With a system update package, flashable zip or partition image as <file>",
                    ])
                    .into(),
                ),
                ("adb rescue getprop", l.pick(["读取本机的版本和设备信息", "讀取本機的版本和裝置資訊", "Reads this phone's versions and device details"]).into()),
                ("adb rescue wipe userdata", l.pick(["清除本机的全部数据", "清除本機的全部資料", "Wipes all of this phone's data"]).into()),
            ]);
            let note = l.pick([
                "需在电脑上安装 Android SDK Platform-Tools。返回后将退出救援模式，并将 USB 切换回 adb 调试。",
                "需在電腦上安裝 Android SDK Platform-Tools。返回後將結束救援模式，並將 USB 切換回 adb 偵錯。",
                "The computer needs Android SDK Platform-Tools. Going back leaves rescue mode and switches USB back to adb debugging.",
            ]);
            widgets::footnote(&mut c, t, f, y, note)
        }
        Install::Waiting => {
            let row = standing(l.pick(WAITING), GREEN, Icon::Usb, l.pick(CONNECT));
            let y = widgets::list(&mut c, t, f, top, &[row]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(RUN_ON_COMPUTER));
            let y = widgets::info_card(&mut c, t, f, y, &[(
                l.pick(["adb sideload 文件名", "adb sideload 檔案名稱", "adb sideload <file>"]),
                l.pick([
                    "将文件名替换为系统更新包、刷机包或分区镜像，例如 boot.img",
                    "將檔案名稱替換為系統更新檔、刷機包或分割區映像檔，例如 boot.img",
                    "With a system update package, flashable zip or partition image as <file>, such as boot.img",
                ])
                .into(),
            )]);
            let note = l.pick([
                "需在电脑上安装 Android SDK Platform-Tools。返回后将停止等待，并将 USB 切换回 adb 调试。",
                "需在電腦上安裝 Android SDK Platform-Tools。返回後將停止等待，並將 USB 切換回 adb 偵錯。",
                "The computer needs Android SDK Platform-Tools. Going back stops waiting and switches USB back to adb debugging.",
            ]);
            widgets::footnote(&mut c, t, f, y, note)
        }
        Install::Checking { stage: Some(Stage::Signature { done, total }), .. } => {
            let read = share(*done, *total);
            let p = percent(read);
            let detail = tr!(l, "已读取 {p}%", "已讀取 {p}%", "{p}% read");
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(["正在验证签名", "正在驗證簽章", "Verifying the signature"]), &detail, Some(read));
            let note = l.pick(match s.source {
                Source::Computer => [
                    "读取完成前，请不要断开数据线。返回将取消安装，并结束电脑上的 adb sideload。",
                    "讀取完成前，請不要拔除傳輸線。返回將取消安裝，並結束電腦上的 adb sideload。",
                    "Keep the cable in until it's read. Going back cancels the install and ends adb sideload on the computer.",
                ],
                Source::Rescue => [
                    "读取完成前，请不要断开数据线。返回将取消安装，并退出救援模式。",
                    "讀取完成前，請不要拔除傳輸線。返回將取消安裝，並結束救援模式。",
                    "Keep the cable in until it's read. Going back cancels the install and leaves rescue mode.",
                ],
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => ["返回将取消安装。", "返回將取消安裝。", "Going back cancels the install."],
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => [
                    "读取完成前，请不要拔出 U 盘。返回将取消安装。",
                    "讀取完成前，請不要拔出 USB 隨身碟。返回將取消安裝。",
                    "Keep the USB drive in until it's read. Going back cancels the install.",
                ],
            });
            widgets::footnote(&mut c, t, f, y, note)
        }
        Install::Checking { size, .. } => {
            let size = fmt::bytes(*size);
            let detail = tr!(l, "文件大小 {size}", "檔案大小 {size}", "File size {size}");
            let y = widgets::list(&mut c, t, f, top, &[standing(l.pick(READING_FILE), GREEN, Icon::Install, &detail)]).bottom;
            match s.source {
                Source::Computer | Source::Rescue => widgets::footnote(&mut c, t, f, y, l.pick(KEEP_CABLE)),
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => y,
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => widgets::footnote(&mut c, t, f, y, l.pick(KEEP_DRIVE)),
            }
        }
        Install::Choosing(choice) => {
            let (name, holds) = fmt::image_kind(l, choice.kind);
            let size = fmt::bytes(choice.size);
            let detail = tr!(l, "{size}，{holds}", "{size}，{holds}", "{size} · {holds}");
            let y = widgets::list(&mut c, t, f, top, &[standing(name, GREEN, Icon::Install, &detail)]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(["分区", "分割區", "Partition"]));
            let rows: Vec<Row> = choice
                .partitions
                .iter()
                .enumerate()
                .map(|(k, p)| Row::new(p).subtitle(fmt::partition_role(l, p)).trail(Trail::Check(k == choice.partition)))
                .collect();
            let parts = widgets::list(&mut c, t, f, y, &rows);
            let choose: Vec<Action> = (0..choice.partitions.len()).map(Action::FlashPartition).collect();
            out.extend(hits(&parts.rows, &choose));
            let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(["槽位", "槽位", "Slot"]));
            let titles = Pick::ALL.map(|p| pick_title(l, p, i.slot));
            let rows: Vec<Row> =
                Pick::ALL.iter().zip(&titles).map(|(p, title)| Row::new(title).trail(Trail::Check(*p == choice.pick))).collect();
            let picks = widgets::list(&mut c, t, f, y, &rows);
            out.extend(hits(&picks.rows, &Pick::ALL.map(Action::FlashSlots)));
            let (label, danger) = confirm_label(Confirm::Flash, cx.state);
            let track = widgets::confirm(&mut c, t, f, picks.bottom + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::Flash) });
            let note = l.pick(match s.source {
                Source::Computer => [
                    "刷入将覆盖分区中的原有内容。返回将取消刷入，并结束电脑上的 adb sideload。",
                    "刷入將覆寫分割區中的原有內容。返回將取消刷入，並結束電腦上的 adb sideload。",
                    "Flashing writes over what the partition holds. Going back cancels it and ends adb sideload on the computer.",
                ],
                Source::Rescue => [
                    "刷入将覆盖分区中的原有内容。返回将取消刷入，并退出救援模式。",
                    "刷入將覆寫分割區中的原有內容。返回將取消刷入，並結束救援模式。",
                    "Flashing writes over what the partition holds. Going back cancels it and leaves rescue mode.",
                ],
                Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => [
                    "刷入将覆盖分区中的原有内容。返回将取消刷入。",
                    "刷入將覆寫分割區中的原有內容。返回將取消刷入。",
                    "Flashing writes over what the partition holds. Going back cancels it.",
                ],
            });
            widgets::footnote(&mut c, t, f, track.y + track.h, note)
        }
        Install::Confirming { version, wipe } => {
            let (title, color, icon, detail, which) = if *wipe {
                let reset = factory_reset(s);
                let detail = match version {
                    Some(v) => tr!(l, "安装 {v} 后将{reset}。", "安裝 {v} 後將{reset}。", "Installing {v} then {reset}."),
                    None => tr!(l, "安装该更新后将{reset}。", "安裝這個更新後將{reset}。", "Installing this update then {reset}."),
                };
                (l.pick(["需清除全部数据", "需清除全部資料", "All data has to be wiped"]).to_string(), ORANGE, Icon::Wipe, detail, Confirm::InstallWipe)
            } else {
                let title = match version {
                    Some(v) => tr!(l, "安装 {v}", "安裝 {v}", "Install {v}"),
                    None => l.pick(["安装系统更新", "安裝系統更新", "Install the system update"]).into(),
                };
                let detail = match i.slot.map(|s| s.other().letter()) {
                    Some(slot) => tr!(
                        l,
                        "新系统将安装到槽位 {slot}，重启后将从该槽位启动。",
                        "新系統將安裝到槽位 {slot}，重新啟動後將從該槽位啟動。",
                        "The new system installs to slot {slot}, which starts after a reboot."
                    ),
                    None => l.pick(["重启后将从新系统启动。", "重新啟動後將從新系統啟動。", "The new system starts after a reboot."]).into(),
                };
                (title, GREEN, Icon::Install, detail, Confirm::Install)
            };
            let y = widgets::list(&mut c, t, f, top, &[standing(&title, color, icon, &detail)]).bottom;
            let (label, danger) = confirm_label(which, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(which) });
            let note = l.pick(["返回将取消安装，不会写入任何内容。", "返回將取消安裝，不會寫入任何內容。", "Going back cancels the install without writing anything."]);
            widgets::footnote(&mut c, t, f, track.y + track.h, note)
        }
        Install::Updating { version, stage, overwriting } => {
            let title = match version {
                Some(v) => tr!(l, "正在安装 {v}", "正在安裝 {v}", "Installing {v}"),
                None => l.pick(["正在安装系统更新", "正在安裝系統更新", "Installing the system update"]).into(),
            };
            let (detail, done) = update_stage(l, stage.as_ref());
            let y = widgets::progress_card(&mut c, t, f, top, &title, &detail, done);
            let until = until_installed(l, s.source);
            let note = match i.slot.map(|s| s.other().letter()) {
                Some(slot) => tr!(l, "{until}新系统将安装到槽位 {slot}。", "{until}新系統將安裝到槽位 {slot}。", "{until} The new system installs to slot {slot}."),
                None => until.to_string(),
            };
            let y = widgets::footnote(&mut c, t, f, y, &note);
            if *overwriting {
                let note = l.pick([
                    "正在覆盖当前系统，安装完成前重启将无法进入系统。",
                    "正在覆寫目前的系統，安裝完成前重新啟動將無法進入系統。",
                    "The current system is being written over, so a reboot before the install finishes starts no system.",
                ]);
                widgets::footnote(&mut c, t, f, y, note)
            } else {
                y
            }
        }
        Install::Flashing { partition, stage } => {
            let (detail, done) = match stage {
                Some(Stage::Flashing { done, total }) => {
                    let s = share(*done, *total);
                    let p = percent(s);
                    let detail = match done * 2 < *total {
                        true => tr!(l, "正在写入 {p}%", "正在寫入 {p}%", "Writing {p}%"),
                        false => tr!(l, "正在校验 {p}%", "正在驗證 {p}%", "Verifying {p}%"),
                    };
                    (detail, Some(s))
                }
                _ => (l.pick(GETTING_READY).into(), None),
            };
            let title = tr!(l, "正在刷入 {partition}", "正在刷入 {partition}", "Flashing {partition}");
            let y = widgets::progress_card(&mut c, t, f, top, &title, &detail, done);
            let keep = l.pick(match s.source {
                Source::Computer | Source::Rescue => ["刷入完成前，请不要断开数据线。", "刷入完成前，請不要拔除傳輸線。", "Keep the cable in until it's flashed."],
                Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => {
                    ["刷入完成前，请不要关机。", "刷入完成前，請不要關機。", "Don't power off until it's flashed."]
                }
                Source::File(Place::UsbDrive) | Source::Opened { drive: true } => {
                    ["刷入完成前，请不要拔出 U 盘。", "刷入完成前，請不要拔出 USB 隨身碟。", "Keep the USB drive in until it's flashed."]
                }
            });
            widgets::footnote(&mut c, t, f, y, keep)
        }
        Install::SystemChoosing(choice) => {
            let version = &choice.version;
            let detail = tr!(l, "{version}，将安装为第二系统", "{version}，將安裝為第二系統", "{version}, to install as the second system");
            let y = widgets::list(&mut c, t, f, top, &[standing(&choice.name, PURPLE, Icon::Systems, &detail)]).bottom;
            let y = widgets::section(&mut c, t, f, y, l.pick(["分配空间", "分配空間", "Room for it"]));
            let sizes: Vec<String> = choice.sizes.iter().map(|&b| fmt::bytes(b)).collect();
            let least = l.pick(["系统所需的最小空间", "系統所需的最小空間", "The least the system takes"]);
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
            let note = l.pick([
                "所选空间将从 data 分区中划出，供第二系统存放文件，安装后无法更改；删除第二系统后归还。主系统的分区不会被写入。",
                "所選空間將從 data 分割區中劃出，供第二系統存放檔案，安裝後無法變更；刪除第二系統後歸還。主系統的分割區不會被寫入。",
                "The room chosen is set aside in the data partition for the second system's files, can't be changed once it's installed, and comes back when the second system is removed. Nothing of the main system's partitions is written.",
            ]);
            let y = widgets::footnote(&mut c, t, f, list.bottom, note);
            let (label, danger) = confirm_label(Confirm::InstallSystem, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::InstallSystem) });
            let note = l.pick(["返回将取消安装，不会写入任何内容。", "返回將取消安裝，不會寫入任何內容。", "Going back cancels the install without writing anything."]);
            widgets::footnote(&mut c, t, f, track.y + track.h, note)
        }
        Install::Systeming { name, stage } => {
            let title = tr!(l, "正在安装 {name}", "正在安裝 {name}", "Installing {name}");
            let (detail, done) = system_stage(l, stage.as_ref());
            let y = widgets::progress_card(&mut c, t, f, top, &title, &detail, done);
            widgets::footnote(&mut c, t, f, y, until_installed(l, s.source))
        }
        Install::ConfirmingScript => {
            let name = s.package_name.as_deref().unwrap_or(l.pick(["刷机包", "刷機包", "Flashable zip"]));
            let detail = l.pick([
                "该刷机包将运行其中的安装脚本，安装脚本可以改写本机的任何分区。",
                "這個刷機包將執行其中的安裝指令碼，安裝指令碼可以改寫本機的任何分割區。",
                "This zip runs the install script it holds, which can rewrite any partition of this phone.",
            ]);
            let mut y = widgets::list(&mut c, t, f, top, &[standing(name, ORANGE, Icon::Install, detail)]).bottom;
            if let Some(note) = fmt::snapshot_script_note(l, &i.snapshot) {
                y = widgets::footnote(&mut c, t, f, y, note);
            }
            let (label, danger) = confirm_label(Confirm::RunScript, cx.state);
            let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
            out.push(Hit { rect: track, action: Action::Confirm(Confirm::RunScript) });
            let note = l.pick(["返回将取消安装，不会运行安装脚本。", "返回將取消安裝，不會執行安裝指令碼。", "Going back cancels the install without running the script."]);
            widgets::footnote(&mut c, t, f, track.y + track.h, note)
        }
        Install::Scripting { stage, .. } => {
            let (detail, done) = script_stage(l, stage.as_ref(), &s.script_output);
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(["正在安装", "正在安裝", "Installing"]), detail, done);
            let y = widgets::footnote(&mut c, t, f, y, until_installed(l, s.source));
            script_output(&mut c, cx, y, &mut rows)
        }
        Install::Wiping => {
            let y = widgets::progress_card(&mut c, t, f, top, l.pick(WIPING), &capitalized(l, &factory_reset(s)), None);
            widgets::footnote(&mut c, t, f, y, l.pick(KEEP_POWER))
        }
        Install::Wiped(Ok(())) => {
            let made_anew = l.pick(["下次开机时将重新创建 data 分区", "下次開機時將重新建立 data 分割區", "The data partition is made anew when the phone next starts"]);
            let y = widgets::list(&mut c, t, f, top, &[standing(l.pick(["已清除", "已清除", "Wiped"]), GREEN, Icon::Wipe, made_anew)]).bottom;
            widgets::footnote(&mut c, t, f, y, l.pick(MORE_COMMANDS))
        }
        Install::Wiped(Err(stop)) => {
            let (what, detail) = fmt::erase_stop(l, stop);
            let mut y = widgets::list(&mut c, t, f, top, &[standing(l.pick(WIPE_INCOMPLETE), RED, Icon::Wipe, &what)]).bottom;
            if let Some(d) = detail {
                y = widgets::footnote(&mut c, t, f, y, &d);
            }
            widgets::footnote(&mut c, t, f, y, l.pick(MORE_COMMANDS))
        }
        Install::Finished { outcome, damage } => finished(&mut c, cx, top, outcome, *damage, &mut out, &mut rows),
    };
    // What the system asked for says why, where it said.
    let bottom = match (s.source, &s.asked.reason) {
        (Source::Asked { .. }, Some(reason)) => widgets::info_card(&mut c, t, f, bottom + t.m.dp(12.0), &[(l.pick(REASON), reason.clone())]),
        _ => bottom,
    };
    Frame { rows, ..Frame::new(t, fit(c, t, bottom), out, install_title(l, s.source), top) }
}

/// What a request the system made gives as why.
const REASON: Text = ["原因", "原因", "Reason"];

const WIPING: Text = ["正在清除", "正在清除", "Wiping"];
const WIPE_INCOMPLETE: Text = ["清除未完成", "清除未完成", "Wipe incomplete"];
const KEEP_POWER: Text = ["清除完成前，请不要关机。", "清除完成前，請不要關機。", "Don't power off until it's wiped."];

/// What rescue says once a command is done with: the computer may send
/// another.
const MORE_COMMANDS: Text = ["你可以继续在电脑上发送命令。", "你可以繼續在電腦上傳送指令。", "You can send more commands from the computer."];

/// The package the system asked for, by the path it gave, at `y`.
fn asked_package(c: &mut Canvas, cx: &Ctx, y: f32) -> f32 {
    let name = cx.state.package_name.clone().unwrap_or_default();
    widgets::info_card(c, cx.t, cx.f, y + cx.t.m.dp(12.0), &[(cx.state.lang.pick(["更新包", "更新檔", "Package"]), name)])
}

/// What not to do while a package from `source` is installed.
fn until_installed(l: Lang, source: Source) -> &'static str {
    l.pick(match source {
        Source::Computer | Source::Rescue => {
            ["安装完成前，请不要断开数据线或关机。", "安裝完成前，請不要拔除傳輸線或關機。", "Keep the cable in and don't power off until it's installed."]
        }
        Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. } => {
            ["安装完成前，请不要关机。", "安裝完成前，請不要關機。", "Don't power off until it's installed."]
        }
        Source::File(Place::UsbDrive) | Source::Opened { drive: true } => {
            ["安装完成前，请不要拔出 U 盘或关机。", "安裝完成前，請不要拔出 USB 隨身碟或關機。", "Keep the USB drive in and don't power off until it's installed."]
        }
    })
}

/// A phrase that opens a line on its own: in English with its first letter
/// a capital.
fn capitalized(l: Lang, phrase: &str) -> String {
    let mut chars = phrase.chars();
    match (l, chars.next()) {
        (Lang::En, Some(first)) => first.to_uppercase().chain(chars).collect(),
        _ => phrase.to_string(),
    }
}

/// How an install ended, and where to go from there: another package from
/// the computer, or back to choose another file.
fn finished(
    c: &mut Canvas,
    cx: &Ctx,
    top: f32,
    outcome: &Outcome,
    damage: Option<Damage>,
    out: &mut Vec<Hit>,
    rows: &mut Option<Rows>,
) -> f32 {
    let (t, f, source, l) = (cx.t, cx.f, cx.state.source, cx.state.lang);
    let gap = t.m.card_gap;
    let choose_another = ["选择其他文件", "選擇其他檔案", "Choose another file"];
    let (again, another) = match source {
        Source::Computer => (Row::new(l.pick(["再次发送", "再次傳送", "Send again"])).icon(GREEN, Icon::Usb), Action::Sideload),
        Source::File(_) | Source::Opened { .. } => (Row::new(l.pick(choose_another)).icon(GREEN, Icon::Folder), Action::Back),
        Source::Asked { .. } => (Row::new(l.pick(choose_another)).icon(GREEN, Icon::Folder), Action::Open(Screen::Install)),
        Source::Rescue => (Row::new(l.pick(["退出救援模式", "結束救援模式", "Leave rescue mode"])).icon(GREEN, Icon::Usb), Action::Back),
    };
    let restart = Row::new(l.pick(["重启到系统", "重新啟動到系統", "Reboot to system"])).icon(ORANGE, Icon::Power);
    let installed = l.pick(["安装成功", "安裝成功", "Installed"]);
    let nothing_written = l.pick(["未写入任何内容。", "未寫入任何內容。", "Nothing was written."]);
    match outcome {
        Outcome::Installed { slot, .. } => {
            let slot = slot.letter();
            let detail = tr!(
                l,
                "新系统已安装到槽位 {slot}，重启后将从该槽位启动。",
                "新系統已安裝到槽位 {slot}，重新啟動後將從該槽位啟動。",
                "The new system is installed to slot {slot}, which starts after a reboot."
            );
            let y = widgets::list(c, t, f, top, &[standing(installed, GREEN, Icon::Install, &detail)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[restart]);
            out.extend(hits(&next.rows, &[Action::RebootTo(Target::System)]));
            let y = reflashed(c, cx, next.bottom);
            afterwards_of(c, cx, y, true)
        }
        Outcome::Flashed { partition, slots } => {
            let slots = fmt::slots(l, slots);
            let detail = tr!(l, "{partition} 已刷入{slots}。", "{partition} 已刷入{slots}。", "Flashed to {partition} in {slots}.");
            let y = widgets::list(c, t, f, top, &[standing(l.pick(["刷入成功", "刷入成功", "Flashed"]), GREEN, Icon::Install, &detail)]).bottom;
            let mut rows = vec![restart];
            let mut acts = vec![Action::RebootTo(Target::System)];
            if *partition == "recovery" {
                rows.push(Row::new(l.pick(["重启到恢复模式", "重新啟動到復原模式", "Reboot to recovery"])).icon(ORANGE, Icon::Power));
                acts.push(Action::RebootTo(Target::Recovery));
            }
            rows.push(again);
            acts.push(another);
            let next = widgets::list(c, t, f, y + gap, &rows);
            out.extend(hits(&next.rows, &acts));
            afterwards_of(c, cx, next.bottom, true)
        }
        Outcome::Ran { .. } => {
            let ran = l.pick(["安装脚本已运行完成。", "安裝指令碼已執行完成。", "The install script ran to its end."]);
            let y = widgets::list(c, t, f, top, &[standing(installed, GREEN, Icon::Install, ran)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[restart, again]);
            out.extend(hits(&next.rows, &[Action::RebootTo(Target::System), another]));
            let y = reflashed(c, cx, next.bottom);
            let y = afterwards_of(c, cx, y, true);
            script_output(c, cx, y, rows)
        }
        Outcome::SystemInstalled { name } => {
            let detail = tr!(
                l,
                "{name} 已安装为第二系统，可在「双系统」中切换。",
                "{name} 已安裝為第二系統，可在「雙系統」中切換。",
                "{name} is installed as the second system. Switch to it from Dual boot."
            );
            let y = widgets::list(c, t, f, top, &[standing(installed, GREEN, Icon::Systems, &detail)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(DUAL_BOOT)).icon(PURPLE, Icon::Systems)]);
            out.extend(hits(&next.rows, &[Action::Open(Screen::Systems)]));
            afterwards_of(c, cx, next.bottom, true)
        }
        Outcome::Cancelled => {
            let y = widgets::list(c, t, f, top, &[standing(l.pick(["已取消", "已取消", "Cancelled"]), SLATE, Icon::Install, nothing_written)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[again]);
            out.extend(hits(&next.rows, &[another]));
            next.bottom
        }
        Outcome::Refused(r) => {
            let reason = fmt::refusal(l, r, &cx.info.codename, source, cx.state.time_zone);
            let y = widgets::list(c, t, f, top, &[standing(l.pick(["无法安装", "無法安裝", "Can't install"]), ORANGE, Icon::Install, &reason)]).bottom;
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
            let y = widgets::list(c, t, f, top, &[standing(l.pick(["安装失败", "安裝失敗", "Install failed"]), RED, Icon::Install, &what)]).bottom;
            let next = widgets::list(c, t, f, y + gap, &[again, restart]);
            out.extend(hits(&next.rows, &[another, Action::RebootTo(Target::System)]));
            // The package is sent again, or installed or flashed again.
            let (install, flash) = match source {
                Source::Computer | Source::Rescue => (["重新发送", "重新傳送", "Send"], ["重新发送", "重新傳送", "Send"]),
                Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => (["重新安装", "重新安裝", "Install"], ["重新刷入", "重新刷入", "Flash"]),
            };
            let (install, flash) = (l.pick(install), l.pick(flash));
            let warning = match damage {
                Some(Damage::System) => tr!(
                    l,
                    "当前系统已被部分覆盖，重启后将无法进入系统。请{install}完整更新包，安装完成后再重启。",
                    "目前的系統已被部分覆寫，重新啟動後將無法進入系統。請{install}完整更新檔，安裝完成後再重新啟動。",
                    "The current system was partly written over, so a reboot starts no system. {install} the full OTA package again, and reboot once it's installed."
                ),
                Some(Damage::Partition(p)) => tr!(
                    l,
                    "{p} 仅写入了部分内容，重启后可能无法正常启动。请{flash}该镜像，刷入完成后再重启。",
                    "{p} 僅寫入了部分內容，重新啟動後可能無法正常啟動。請{flash}這個映像檔，刷入完成後再重新啟動。",
                    "{p} was only partly written, so the phone may not start properly after a reboot. {flash} the image again, and reboot once it's flashed."
                ),
                Some(Damage::Script) => tr!(
                    l,
                    "本机的分区可能已被部分改写，重启后可能无法正常启动。请{install}该刷机包，安装完成后再重启。",
                    "本機的分割區可能已被部分改寫，重新啟動後可能無法正常啟動。請{install}這個刷機包，安裝完成後再重新啟動。",
                    "This phone's partitions may have been partly rewritten, so it may not start properly after a reboot. {install} the zip again, and reboot once it's installed."
                ),
                None => l.pick(["当前系统未受影响。", "目前的系統未受影響。", "The current system is untouched."]).into(),
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
        true => ["即将自动关机。", "即將自動關機。", "Powering off shortly."],
        false => ["即将自动重启。", "即將自動重新啟動。", "Rebooting shortly."],
    };
    match cx.state.source {
        Source::Rescue => widgets::footnote(c, cx.t, cx.f, y, l.pick(MORE_COMMANDS)),
        Source::Asked { .. } if installed => widgets::footnote(c, cx.t, cx.f, y, l.pick(leaving)),
        _ => y,
    }
}

/// What the wipe page lists of [`WIPES`] beyond everything, under the
/// partition each is in.
const WIPE_GROUPS: [(Text, &[Wipe]); 2] = [
    (DATA_PART, &[Wipe::AppData, Wipe::Media, Wipe::AppCaches, Wipe::SystemCaches, Wipe::Art, Wipe::Logs, Wipe::Updates]),
    (["cache 分区", "cache 分割區", "cache partition"], &[Wipe::Cache]),
];

/// What a factory reset erases, as the wipe page and an update that asks
/// for one tell it; in English as it goes on from what leads to it, its
/// first letter small.
fn factory_reset(s: &State) -> String {
    let also = s.lang.pick(match s.cache_partition {
        true => ["metadata 和 cache 分区", "metadata 和 cache 分割區", "the metadata and cache partitions"],
        false => ["metadata 分区", "metadata 分割區", "the metadata partition"],
    });
    tr!(
        s.lang,
        "清空 data 分区，包括应用、账户、设置和内部存储，并格式化 {also}",
        "清空 data 分割區，包括應用程式、帳戶、設定和內部儲存空間，並格式化 {also}",
        "empties the data partition, apps, accounts, settings and internal storage included, and formats {also}"
    )
}

/// A choice of the wipe page as it lists it: its title, and exactly what it
/// erases.
fn wipe_row(w: Wipe, s: &State) -> (&'static str, String) {
    let l = s.lang;
    let (title, what): (Text, String) = match w {
        Wipe::All => (["全部数据", "全部資料", "All data"], capitalized(l, &factory_reset(s))),
        Wipe::AppData => (
            ["应用数据", "應用程式資料", "App data"],
            l.pick([
                "/data/data、/data/user、/data/user_de，以及内部存储中的 Android/data、Android/media，应用将恢复到初始状态",
                "/data/data、/data/user、/data/user_de，以及內部儲存空間中的 Android/data、Android/media，應用程式將恢復到初始狀態",
                "/data/data, /data/user, /data/user_de, and Android/data and Android/media in internal storage. Apps go back to how they were first installed",
            ])
            .into(),
        ),
        Wipe::Media => (
            ["内部存储", "內部儲存空間", "Internal storage"],
            l.pick(["/data/media，即 /sdcard 中的照片、视频和文件", "/data/media，即 /sdcard 中的相片、影片和檔案", "/data/media: the photos, videos and files in /sdcard"]).into(),
        ),
        Wipe::AppCaches => (
            ["应用缓存", "應用程式快取", "App caches"],
            l.pick([
                "各应用数据中的 cache、code_cache，以及 Android/data 中的 cache，应用将按需重新生成",
                "各應用程式資料中的 cache、code_cache，以及 Android/data 中的 cache，應用程式將視需要重新產生",
                "cache and code_cache in each app's data, and cache in Android/data. Apps make them again as they need them",
            ])
            .into(),
        ),
        Wipe::SystemCaches => (
            ["系统缓存", "系統快取", "System caches"],
            l.pick([
                "/data/system/package_cache、/data/resource-cache、/data/cache，下次开机时重新生成",
                "/data/system/package_cache、/data/resource-cache、/data/cache，下次開機時重新產生",
                "/data/system/package_cache, /data/resource-cache and /data/cache, made again at the next start",
            ])
            .into(),
        ),
        Wipe::Art => (
            ["ART 缓存", "ART 快取", "ART cache"],
            l.pick([
                "/data/dalvik-cache、/data/misc/apexdata/com.android.art/dalvik-cache、/data/ota，下次开机时重新编译，启动时间较长",
                "/data/dalvik-cache、/data/misc/apexdata/com.android.art/dalvik-cache、/data/ota，下次開機時重新編譯，啟動時間較長",
                "/data/dalvik-cache, /data/misc/apexdata/com.android.art/dalvik-cache and /data/ota, compiled again at the next start, which then takes longer",
            ])
            .into(),
        ),
        Wipe::Logs => {
            let own = match (l, s.phone_logs) {
                (_, 0) => String::new(),
                (Lang::En, 1) => ", and this phone's own log folder".into(),
                (_, n) => tr!(l, "，以及本机的 {n} 个日志文件夹", "，以及本機的 {n} 個記錄資料夾", ", and this phone's own {n} log folders"),
            };
            let what = tr!(
                l,
                "/data/tombstones、/data/anr、/data/system/dropbox、/data/misc/logd 等系统日志和崩溃记录，各用户的错误报告{own}",
                "/data/tombstones、/data/anr、/data/system/dropbox、/data/misc/logd 等系統記錄和當機記錄，各使用者的錯誤報告{own}",
                "System logs and crash records, such as /data/tombstones, /data/anr, /data/system/dropbox and /data/misc/logd, each user's bug reports{own}"
            );
            (["日志", "記錄", "Logs"], what)
        }
        Wipe::Updates => (
            ["下载的更新包", "下載的更新檔", "Downloaded updates"],
            l.pick(["/data/ota_package 中的系统更新包", "/data/ota_package 中的系統更新檔", "System update packages in /data/ota_package"]).into(),
        ),
        Wipe::Cache => (
            ["全部内容", "全部內容", "Everything"],
            l.pick(["格式化 cache 分区，保留 recovery 文件夹中的日志", "格式化 cache 分割區，保留 recovery 資料夾中的記錄", "Formats the cache partition, keeping the logs in its recovery folder"]).into(),
        ),
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
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["恢复出厂设置", "恢復原廠設定", "Factory reset"]));
    let (name, sub) = wipe_row(Wipe::All, s);
    let all = widgets::list(&mut c, t, f, y, &[Row::new(name).subtitle(&sub).trail(Trail::Check(chosen(Wipe::All)))]);
    out.extend(hits(&all.rows, &[Action::WipeItem(Wipe::All)]));
    let note = l.pick([
        "系统和已安装的更新不受影响，清除后需重新设置手机。",
        "系統和已安裝的更新不受影響，清除後需重新設定手機。",
        "The system and installed updates stay. The phone has to be set up again afterwards.",
    ]);
    let mut y = widgets::footnote(&mut c, t, f, all.bottom, note);
    for (title, items) in WIPE_GROUPS {
        let shown: Vec<Wipe> = items.iter().copied().filter(|&w| w != Wipe::Cache || s.cache_partition).collect();
        if shown.is_empty() {
            continue;
        }
        let below = widgets::section(&mut c, t, f, y, l.pick(title));
        let texts: Vec<(&str, String)> = shown.iter().map(|&w| wipe_row(w, s)).collect();
        let rows: Vec<Row> =
            shown.iter().zip(&texts).map(|(&w, (name, sub))| Row::new(name).subtitle(sub).trail(Trail::Check(chosen(w)))).collect();
        let list = widgets::list(&mut c, t, f, below, &rows);
        out.extend(hits(&list.rows, &shown.iter().map(|&w| Action::WipeItem(w)).collect::<Vec<_>>()));
        y = list.bottom;
    }
    let note = l.pick([
        "清除应用数据和应用缓存前，需先完成「解密」。清除后无法恢复。",
        "清除應用程式資料和應用程式快取前，需先完成「解密」。清除後無法復原。",
        "Wiping app data or app caches needs the data partition decrypted first. What is wiped can't be brought back.",
    ]);
    let y = widgets::footnote(&mut c, t, f, y, note);
    let (label, danger) = confirm_label(Confirm::Wipe, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Wipe) });
    Frame::new(t, fit(c, t, track.y + track.h), out, l.pick(["清除数据", "清除資料", "Wipe data"]), top)
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
    let field = widgets::search_field(&mut c, t, f, at.x, at.w, at.y + at.h / 2.0, &s.search, l.pick(SEARCH_HINT));
    widgets::cancel_button(&mut c, t, cancel);
    out.push(Hit { rect: cancel, action: Action::CloseSearch });
    let top = field.field.y + field.field.h + m.dp(12.0);
    let kb_top = m.h as f32 - widgets::keyboard_height(t);
    if s.search.trim().is_empty() {
        let how = l.pick([
            "输入功能或分区名称，支持拼音全拼和首字母，例如输入 bf 可找到「备份」。",
            "輸入功能或分割區名稱，支援英文和漢語拼音的全拼與首字母，例如輸入 bf 可找到「備份」。",
            "Type the name of a feature or partition, such as backup to find Backup.",
        ]);
        widgets::footnote(&mut c, t, f, top, how);
    } else {
        let found = search::find_on(&s.search, l, |screen| s.offers(screen));
        if found.is_empty() {
            let typed = s.search.trim();
            let none = tr!(l, "未找到\"{typed}\"", "找不到「{typed}」", "Nothing found for “{typed}”");
            widgets::list(&mut c, t, f, top, &[Row::new(&none).trail(Trail::None)]);
        } else {
            let rows: Vec<Row> = found.iter().map(|e| Row::new(l.pick(e.title)).subtitle(l.pick(e.place))).collect();
            let list = widgets::list(&mut c, t, f, top, &rows);
            // The keyboard covers what does not fit above it.
            let shown = list.rows.iter().zip(&found).filter(|(r, _)| r.y + r.h <= kb_top);
            out.extend(shown.map(|(&rect, e)| Hit { rect, action: Action::Open(e.screen) }));
        }
    }
    let find = l.pick(["搜索", "搜尋", "Search"]);
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
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["备份内容", "備份內容", "Contents"]));
    let system = fmt::snapshot_backup_note(l, &i.snapshot).unwrap_or(l.pick(["当前槽位在 super 中的分区", "目前槽位在 super 中的分割區", "The current slot's partitions in super"]));
    let data = l.pick(match s.data == Data::Open {
        true => ["应用、账户和设置，不包括内部存储、锁屏密码和指纹", "應用程式、帳戶和設定，不包括內部儲存空間、螢幕鎖定密碼和指紋", "Apps, accounts and settings, without internal storage, the screen lock or fingerprints"],
        false => [
            "应用、账户和设置，不包括内部存储、锁屏密码和指纹，需先完成「解密」",
            "應用程式、帳戶和設定，不包括內部儲存空間、螢幕鎖定密碼和指紋，需先完成「解密」",
            "Apps, accounts and settings, without internal storage, the screen lock or fingerprints. Decrypt first",
        ],
    });
    let boot = l.pick(["boot、init_boot、vendor_boot、dtbo、vbmeta 和 recovery", "boot、init_boot、vendor_boot、dtbo、vbmeta 和 recovery", "boot, init_boot, vendor_boot, dtbo, vbmeta and recovery"]);
    let modem = l.pick(["IMEI、基带设置和传感器校准数据", "IMEI、基頻設定和感應器校正資料", "IMEI, modem settings and sensor calibration"]);
    let subs = [boot, system, data, modem];
    let rows: Vec<Row> = BACKUP_PARTS
        .iter()
        .zip(subs)
        .enumerate()
        .map(|(k, (name, sub))| Row::new(l.pick(*name)).subtitle(sub).trail(Trail::Check(s.backup[k])))
        .collect();
    let parts = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&parts.rows, &[0, 1, 2, 3].map(Action::BackupPart)));
    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(["存放位置", "存放位置", "Save to"]));
    let storage = match (s.data.storage_open(), i.storage.free.map(fmt::gb)) {
        (true, Some(free)) => tr!(l, "剩余 {free}", "剩餘 {free}", "{free} free"),
        (true, None) => l.pick(["无法读取剩余空间", "無法讀取剩餘空間", "Free space unknown"]).to_string(),
        (false, _) if s.data == Data::Erased => l.pick(DATA_ERASED).to_string(),
        (false, _) => l.pick(DECRYPT_NEEDED).to_string(),
    };
    let usb = l.pick(match &s.usb_drive {
        Some(UsbDrive::Shown) => ["已连接", "已連接", "Connected"],
        Some(UsbDrive::Unreadable(_)) => USB_DRIVE_UNREADABLE,
        None => USB_DRIVE_WANTED,
    });
    let places = widgets::list(&mut c, t, f, y, &[
        Row::new(place_name(l, Place::Storage)).subtitle(&storage).trail(Trail::Check(s.backup_to == Place::Storage)),
        Row::new(place_name(l, Place::UsbDrive)).subtitle(usb).trail(Trail::Check(s.backup_to == Place::UsbDrive)),
    ]);
    out.extend(hits(&places.rows, &[Action::BackupTo(Place::Storage), Action::BackupTo(Place::UsbDrive)]));
    let y = widgets::section(&mut c, t, f, places.bottom, l.pick(["选项", "選項", "Options"]));
    let options = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(["压缩", "壓縮", "Compress"])).subtitle(l.pick(["备份更小，耗时更长", "備份更小，耗時更長", "Smaller, and slower to make"])).trail(Trail::Switch(s.compress)),
        Row::new(l.pick(["完成后校验", "完成後驗證", "Verify when done"])).subtitle(l.pick(["确认备份文件完整", "確認備份檔案完整", "Checks the backup's files are whole"])).trail(Trail::Switch(s.verify)),
    ]);
    out.extend(hits(&options.rows, &[Action::Toggle(Switch::Compress), Action::Toggle(Switch::Verify)]));
    let note = l.pick([
        "备份将保存在 Skyward/backups 文件夹中，以开始备份的时间命名。",
        "備份將儲存在 Skyward/backups 資料夾中，以開始備份的時間命名。",
        "Backups go in the Skyward/backups folder, named by when they began.",
    ]);
    let y = widgets::footnote(&mut c, t, f, options.bottom, note);
    let (label, danger) = confirm_label(Confirm::Backup, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Backup) });
    Frame::new(t, fit(c, t, track.y + track.h), out, l.pick(["备份", "備份", "Backup"]), top)
}

/// The backups found, newest first, to restore one; those that cannot be
/// read are listed after them, to be deleted.
pub fn restore(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["备份", "備份", "Backups"]));
    let bottom = if s.backups.is_empty() {
        widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["暂无备份", "沒有備份", "No backups"])).trail(Trail::None)]).bottom
    } else {
        let texts: Vec<(String, String)> = s
            .backups
            .iter()
            .map(|b| match &b.held {
                Ok(h) => (fmt::datetime(l, h.made, s.time_zone), format!("{} · {} · {}", fmt::parts(l, &h.parts), fmt::bytes(h.size), place_name(l, b.place))),
                Err(_) => (b.name.clone(), format!("{} · {}", l.pick(UNREADABLE), place_name(l, b.place))),
            })
            .collect();
        let rows: Vec<Row> = texts.iter().map(|(title, sub)| Row::new(title).subtitle(sub)).collect();
        let list = widgets::list(&mut c, t, f, y, &rows);
        out.extend(list.rows.iter().enumerate().map(|(k, &rect)| Hit { rect, action: Action::OpenBackup(k) }));
        list.bottom
    };
    let note = l.pick([
        "将在内部存储和 U 盘中的 Skyward/backups 文件夹查找备份，查找内部存储需先完成「解密」。",
        "將在內部儲存空間和 USB 隨身碟中的 Skyward/backups 資料夾尋找備份，尋找內部儲存空間需先完成「解密」。",
        "Backups are looked for in the Skyward/backups folders of internal storage and the USB drive. Internal storage needs the data partition decrypted first.",
    ]);
    let bottom = widgets::footnote(&mut c, t, f, bottom, note);
    Frame::new(t, fit(c, t, bottom), out, l.pick(["恢复", "還原", "Restore"]), top)
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
    let location = l.pick(["位置", "位置", "Location"]);
    let y = match &entry.held {
        Err(why) => {
            let facts = [(l.pick(["文件夹", "資料夾", "Folder"]), entry.name.clone()), (location, place_name(l, entry.place).to_string())];
            let y = widgets::info_card(&mut c, t, f, top, &facts);
            let note = tr!(
                l,
                "无法读取该备份，备份可能未完成或已损坏。{why}",
                "無法讀取這份備份，備份可能未完成或已損壞。{why}",
                "This backup can't be read. It may not have finished, or it may be damaged. {why}"
            );
            widgets::footnote(&mut c, t, f, y, &note)
        }
        Ok(held) => {
            let mut facts = vec![
                (l.pick(["备份时间", "備份時間", "Made"]), fmt::datetime(l, held.made, s.time_zone)),
                (l.pick(["大小", "大小", "Size"]), fmt::bytes(held.size)),
                (location, place_name(l, entry.place).to_string()),
            ];
            if let Some(system) = &held.system {
                facts.push((l.pick(["系统版本", "系統版本", "System version"]), system.clone()));
            }
            if let Some(slot) = held.slot {
                facts.push((l.pick(["槽位", "槽位", "Slot"]), slot.letter().to_string()));
            }
            let y = widgets::info_card(&mut c, t, f, top, &facts);
            if held.device != i.codename {
                let device = &held.device;
                let note = tr!(
                    l,
                    "该备份来自其他设备（{device}），无法恢复到本机。",
                    "這份備份來自其他裝置（{device}），無法還原到本機。",
                    "This backup is from another device ({device}) and can't be restored to this phone."
                );
                widgets::footnote(&mut c, t, f, y, &note)
            } else {
                let y = widgets::section(&mut c, t, f, y, l.pick(["恢复内容", "還原內容", "Contents"]));
                let kept: Vec<usize> = (0..BACKUP_PARTS.len()).filter(|&k| held.parts[k]).collect();
                let rows: Vec<Row> = kept.iter().map(|&k| Row::new(l.pick(BACKUP_PARTS[k])).trail(Trail::Check(chosen[k]))).collect();
                let list = widgets::list(&mut c, t, f, y, &rows);
                out.extend(list.rows.iter().zip(&kept).map(|(&rect, &k)| Hit { rect, action: Action::RestorePart(k) }));
                let note = l.pick(["恢复后，所选内容的现有数据将被备份覆盖。", "還原後，所選內容的現有資料將被備份覆寫。", "Restoring writes the backup over what the parts chosen hold now."]);
                let mut y = widgets::footnote(&mut c, t, f, list.bottom, note);
                if chosen[2] {
                    let note = l.pick([
                        "data 分区将恢复应用、账户和设置，内部存储、锁屏密码和指纹保持不变。恢复出厂设置后，无法再恢复此前的 data 分区备份。",
                        "data 分割區將還原應用程式、帳戶和設定，內部儲存空間、螢幕鎖定密碼和指紋保持不變。恢復原廠設定後，無法再還原此前的 data 分割區備份。",
                        "The data partition gets its apps, accounts and settings back, and internal storage, the screen lock and fingerprints stay as they are. After a factory reset, an earlier backup of the data partition can't be restored.",
                    ]);
                    y = widgets::footnote(&mut c, t, f, y, note);
                }
                // The boot partitions' vbmeta checks the system's: one of
                // another version without the other may not start.
                if let (Some(was), Some(now)) = (&held.system, &i.os_version) {
                    if was != now && chosen[0] != chosen[1] {
                        let warn = tr!(
                            l,
                            "该备份来自 {was}，本机当前为 {now}。仅恢复启动分区或系统分区中的一项，可能导致无法开机。",
                            "這份備份來自 {was}，本機目前為 {now}。僅還原啟動分割區或系統分割區中的一項，可能導致無法開機。",
                            "This backup is from {was}, and this phone runs {now}. Restoring only one of the boot and system partitions may keep the phone from starting."
                        );
                        y = widgets::footnote(&mut c, t, f, y, &warn);
                    }
                }
                let (label, danger) = confirm_label(Confirm::Restore, cx.state);
                let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
                out.push(Hit { rect: track, action: Action::Confirm(Confirm::Restore) });
                track.y + track.h
            }
        }
    };
    let delete = widgets::list(&mut c, t, f, y + t.m.dp(28.0), &[Row::new(l.pick(["删除备份", "刪除備份", "Delete backup"])).tint(t.pal.danger).trail(Trail::None)]);
    out.extend(hits(&delete.rows, &[Action::AskDelete]));
    let mut bottom = delete.bottom;
    if s.asking_delete {
        let ask = l.pick(["删除该备份？删除后无法恢复。", "刪除這份備份？刪除後無法復原。", "Delete this backup? It can't be brought back."]);
        let y = widgets::footnote(&mut c, t, f, delete.bottom, ask);
        let (label, danger) = confirm_label(Confirm::DeleteBackup, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::DeleteBackup) });
        bottom = track.y + track.h;
    }
    Frame::new(t, fit(c, t, bottom), out, l.pick(["恢复备份", "還原備份", "Restore backup"]), top)
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

/// What a backup, or a restore, has done to a part, and does to it, in
/// each language.
fn job_words(restore: bool) -> (Text, Text) {
    match restore {
        true => (["已恢复", "已還原", "Restored"], ["正在恢复", "正在還原", "Restoring"]),
        false => (["已备份", "已備份", "Backed up"], ["正在备份", "正在備份", "Backing up"]),
    }
}

const VERIFIED: Text = ["已校验", "已驗證", "Verified"];
const VERIFYING: Text = ["正在校验", "正在驗證", "Verifying"];
const WAITING_TURN: Text = ["等待中", "等待中", "Waiting"];
const DONE: Text = ["完成", "完成", "Done"];

/// Where part `k` of a backup or restore stands, as its row says it.
fn part_state(l: Lang, job: &BackupJob, k: usize) -> String {
    use std::cmp::Ordering;
    let (did, doing) = job_words(job.restore);
    let ended = job.outcome.is_some();
    let a = match (&job.outcome, job.at) {
        (Some(BackupEnd::Saved(_)), _) => return l.pick(if job.checks { VERIFIED } else { did }).to_string(),
        (Some(BackupEnd::Restored { .. }), _) => return l.pick(did).to_string(),
        (_, None) => return l.pick(if ended { ["未开始", "未開始", "Not started"] } else { WAITING_TURN }).to_string(),
        (_, Some(a)) => a,
    };
    let state = match k.cmp(&a.item) {
        Ordering::Less if a.checking => VERIFIED,
        Ordering::Less => did,
        Ordering::Equal if ended => ["未完成", "未完成", "Unfinished"],
        Ordering::Equal => return format!("{} {}%", l.pick(if a.checking { VERIFYING } else { doing }), percent(part_share(&a))),
        // Ahead of the pass under way: a backup checks its parts once all
        // are written, and a restore writes them once all are checked.
        Ordering::Greater => match (a.checking, job.restore, ended) {
            (true, false, false) => did,
            (true, false, true) => ["未校验", "未驗證", "Not verified"],
            (false, false, false) | (true, true, false) => WAITING_TURN,
            (false, false, true) => ["未备份", "未備份", "Not backed up"],
            (false, true, false) => VERIFIED,
            (_, true, true) => ["未恢复", "未還原", "Not restored"],
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
    let (did, doing) = job_words(job.restore);
    let icon = if job.restore { Icon::Restore } else { Icon::Backup };
    let writing = job.at.is_some_and(|a| !a.checking);
    let mut y = match &job.outcome {
        None => {
            let (title, detail, done) = match job.at {
                Some(a) => {
                    let (over, passed) = job_progress(job, &a);
                    let total = fmt::bytes(job.plan.iter().map(|(_, n)| n).sum());
                    let (verb, past) = if a.checking { (l.pick(VERIFYING), l.pick(VERIFIED)) } else { (l.pick(doing), l.pick(did)) };
                    let (name, passed) = (&job.plan[a.item].0, fmt::bytes(passed));
                    (format!("{verb} {name}"), tr!(l, "{past} {passed}，共 {total}", "{past} {passed}，共 {total}", "{past} {passed} of {total}"), over)
                }
                None if job.restore => (
                    l.pick(["正在读取备份", "正在讀取備份", "Reading the backup"]).to_string(),
                    l.pick(["正在核对备份与本机", "正在核對備份與本機", "Checking the backup against this phone"]).to_string(),
                    0.0,
                ),
                None => (l.pick(GETTING_READY).to_string(), l.pick(["正在计算备份大小", "正在計算備份大小", "Working out the backup's size"]).to_string(), 0.0),
            };
            widgets::progress_card(&mut c, t, f, top, &title, &detail, Some(done))
        }
        Some(end) => {
            let gone = l.pick(if job.restore { ["未写入任何内容。", "未寫入任何內容。", "Nothing was written."] } else { ["未保存任何备份。", "未儲存任何備份。", "No backup was saved."] });
            let failed = l.pick(if job.restore { RESTORE_FAILED } else { ["备份失败", "備份失敗", "Backup failed"] });
            let (title, color, detail) = match end {
                BackupEnd::Saved(name) => {
                    let saved = match job.place {
                        Place::Storage => tr!(l, "已保存到内部存储的 Skyward/backups/{name}", "已儲存到內部儲存空間的 Skyward/backups/{name}", "Saved to Skyward/backups/{name} in internal storage"),
                        Place::UsbDrive => tr!(l, "已保存到 U 盘的 Skyward/backups/{name}", "已儲存到 USB 隨身碟的 Skyward/backups/{name}", "Saved to Skyward/backups/{name} on the USB drive"),
                    };
                    (l.pick(BACKUP_COMPLETE), GREEN, saved)
                }
                BackupEnd::Restored { .. } => {
                    (l.pick(RESTORE_COMPLETE), GREEN, l.pick(["重启手机后，将使用恢复的内容。", "重新啟動手機後，將使用還原的內容。", "The phone uses what was restored once it reboots."]).to_string())
                }
                BackupEnd::Stopped => (l.pick(["已停止", "已停止", "Stopped"]), SLATE, gone.to_string()),
                BackupEnd::Refused(r) => {
                    let title = l.pick(if job.restore { ["无法恢复", "無法還原", "Can't restore"] } else { ["无法备份", "無法備份", "Can't back up"] });
                    (title, ORANGE, fmt::backup_refusal(l, r, job.restore))
                }
                BackupEnd::Failed(_) if job.restore && writing => {
                    let detail = l.pick([
                        "部分内容已写入，重启后可能无法正常启动。请重新恢复，完成后再重启。",
                        "部分內容已寫入，重新啟動後可能無法正常啟動。請重新還原，完成後再重新啟動。",
                        "Part of it was written, so the phone may not start properly after a reboot. Restore again, and reboot once it's done.",
                    ]);
                    (failed, RED, detail.to_string())
                }
                BackupEnd::Failed(_) => (failed, RED, gone.to_string()),
            };
            widgets::list(&mut c, t, f, top, &[standing(title, color, icon, &detail)]).bottom
        }
    };
    let keep = l.pick(match job.place {
        Place::Storage => ["请不要关机。", "請不要關機。", "Don't power off"],
        Place::UsbDrive => ["请不要拔出 U 盘或关机。", "請不要拔出 USB 隨身碟或關機。", "Keep the USB drive in and don't power off"],
    });
    match &job.outcome {
        None => {
            let note = match (job.restore, job.at) {
                (false, _) => tr!(l, "备份完成前，{keep}停止后将删除未完成的备份。", "備份完成前，{keep}停止後將刪除未完成的備份。", "{keep} until the backup is done. Stopping deletes the unfinished backup."),
                (true, Some(a)) if !a.checking => tr!(l, "恢复完成前，{keep}", "還原完成前，{keep}", "{keep} until the restore is done."),
                (true, _) => l.pick(["校验备份完成前，不会写入任何内容。", "驗證備份完成前，不會寫入任何內容。", "Nothing is written until the backup is verified."]).to_string(),
            };
            y = widgets::footnote(&mut c, t, f, y, &note);
            // A restore that has begun writing goes on to its end.
            if !(job.restore && writing) {
                let label = l.pick(if job.stopping { ["正在停止", "正在停止", "Stopping"] } else { STOP });
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
                    let folders = unmade.join(l.pick(["、", "、", ", "]));
                    let note = tr!(
                        l,
                        "以下文件夹所属的用户已不存在，其中的内容未恢复：{folders}",
                        "以下資料夾所屬的使用者已不存在，其中的內容未還原：{folders}",
                        "The users these folders belonged to no longer exist, so what they hold wasn't restored: {folders}"
                    );
                    y = widgets::footnote(&mut c, t, f, y, &note);
                }
                BackupEnd::Failed(why) => y = widgets::footnote(&mut c, t, f, y, why),
                _ => {}
            }
            let (rows, acts): (Vec<Row>, Vec<Action>) = match end {
                BackupEnd::Restored { .. } => (
                    vec![Row::new(l.pick(["重启到系统", "重新啟動到系統", "Reboot to system"])).icon(ORANGE, Icon::Power), Row::new(l.pick(DONE)).trail(Trail::None)],
                    vec![Action::RebootTo(Target::System), Action::Back],
                ),
                _ => (vec![Row::new(l.pick(DONE)).trail(Trail::None)], vec![Action::Back]),
            };
            let next = widgets::list(&mut c, t, f, y + t.m.card_gap, &rows);
            out.extend(hits(&next.rows, &acts));
            y = next.bottom;
        }
    }
    // The parts come after what can be done, which a long list would push
    // off the screen.
    if !job.plan.is_empty() {
        let heading = l.pick(if job.restore { ["恢复内容", "還原內容", "Contents"] } else { ["备份内容", "備份內容", "Contents"] });
        let y0 = widgets::section(&mut c, t, f, y, heading);
        let states: Vec<String> = (0..job.plan.len()).map(|k| part_state(l, job, k)).collect();
        let rows: Vec<Row> = job.plan.iter().zip(&states).map(|((name, _), state)| Row::new(name).trail(Trail::Text(state))).collect();
        y = widgets::list(&mut c, t, f, y0, &rows).bottom;
    }
    let title = l.pick(match (&job.outcome, job.restore) {
        (None, restore) => job_words(restore).1,
        (Some(BackupEnd::Saved(_)), _) => BACKUP_COMPLETE,
        (Some(BackupEnd::Restored { .. }), _) => RESTORE_COMPLETE,
        (Some(_), false) => ["备份未完成", "備份未完成", "Backup incomplete"],
        (Some(_), true) => ["恢复未完成", "還原未完成", "Restore incomplete"],
    });
    Frame { back: job.outcome.is_some(), ..Frame::new(t, fit(c, t, y), out, title, top) }
}

const BACKUP_COMPLETE: Text = ["备份完成", "備份完成", "Backup complete"];
const RESTORE_COMPLETE: Text = ["恢复完成", "還原完成", "Restore complete"];
const RESTORE_FAILED: Text = ["恢复失败", "還原失敗", "Restore failed"];
const STOP: Text = ["停止", "停止", "Stop"];

/// Tools for the experienced.
pub fn advanced(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = page(cx);
    let top = t.m.content_top;
    let tools = widgets::list(&mut c, t, f, top, &[
        Row::new(l.pick(FILES)).icon(BLUE, Icon::Folder),
        Row::new(l.pick(TERMINAL)).icon(SLATE, Icon::Terminal),
        Row::new(l.pick(RELABEL)).icon(PURPLE, Icon::Shield),
        Row::new(l.pick(FILESYSTEMS)).icon(RED, Icon::Mount),
        Row::new(l.pick(LOGS)).icon(ORANGE, Icon::Log),
        Row::new(l.pick(UPDATE)).icon(GREEN, Icon::Layers).trail(Trail::Value(update_value(cx))),
    ]);
    let out = hits(&tools.rows, &[
        Action::Open(Screen::Files),
        Action::Open(Screen::Terminal),
        Action::Open(Screen::Relabel),
        Action::Open(Screen::Filesystems),
        Action::Open(Screen::Logs),
        Action::Open(Screen::Update),
    ]);
    Frame::new(t, c, out, l.pick(ADVANCED), top)
}

const RELABEL: Text = ["修复 SELinux 上下文", "修復 SELinux 上下文", "Fix SELinux contexts"];
const FILESYSTEMS: Text = ["文件系统", "檔案系統", "Filesystems"];
const STOPPED: Text = ["已停止", "已停止", "Stopped"];
const STOPPING: Text = ["正在停止", "正在停止", "Stopping"];

/// Giving the data partition's files the SELinux labels the installed
/// system's rules give them: what it does, and a swipe to start it; then
/// how far it has come, a way to stop it, and how it ended.
pub fn relabel(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let title = l.pick(RELABEL);
    let Some(job) = &s.relabel else {
        let what = l.pick([
            "按已安装系统的规则，重新设置 data 分区中文件的 SELinux 上下文，应用数据保持不变。",
            "按已安裝系統的規則，重新設定 data 分割區中檔案的 SELinux 上下文，應用程式資料保持不變。",
            "Gives the files in the data partition the SELinux contexts the installed system's rules give them. App data stays as it is.",
        ]);
        let mut y = widgets::paragraph(&mut c, t, f, top, what);
        let note = match s.data {
            Data::Open | Data::Locked { .. } => None,
            Data::Erased => Some(["data 分区已清除。", "data 分割區已清除。", "The data partition was wiped."]),
            Data::Closed | Data::Opening | Data::Failed(_) => {
                Some(["暂无法修复，需先在「解密」中打开 data 分区。", "暫時無法修復，需先在「解密」中開啟 data 分割區。", "Open the data partition in Decrypt first."])
            }
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
        Some(JobEnd::Stopped) => l.pick(STOPPED).into(),
        Some(JobEnd::Failed(_)) => repair.failed(l),
    };
    let detail = match (job.total, job.checked) {
        (0, _) if job.outcome.is_none() => l.pick(GETTING_READY).to_string(),
        (total, checked) => tr!(l, "已检查 {checked} 项，共 {total} 项", "已檢查 {checked} 項，共 {total} 項", "{checked} of {total} checked"),
    };
    let y = widgets::progress_card(&mut c, t, f, top, &heading, &detail, Some(share(job.checked, job.total)));
    match &job.outcome {
        None => {
            let label = l.pick(if job.stopping { STOPPING } else { STOP });
            let stop = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopRelabel]));
            }
        }
        Some(end) => {
            let changed = job.changed;
            let note = match end {
                JobEnd::Done if changed == 0 => l
                    .pick(["全部文件的 SELinux 上下文都正确，无需修复。", "全部檔案的 SELinux 上下文都正確，無需修復。", "Every file's SELinux context is right, so nothing needed fixing."])
                    .to_string(),
                JobEnd::Done => tr!(
                    l,
                    "已按规则重新设置 {changed} 项的 SELinux 上下文。",
                    "已按規則重新設定 {changed} 項的 SELinux 上下文。",
                    "{changed} entries got the SELinux context the rules give them."
                ),
                JobEnd::Stopped => tr!(l, "已停止，已重新设置 {changed} 项。", "已停止，已重新設定 {changed} 項。", "Stopped, with {changed} entries set again."),
                JobEnd::Failed(why) => tr!(l, "修复失败：{why}", "修復失敗：{why}", "Repair failed: {why}"),
            };
            let y = widgets::footnote(&mut c, t, f, y, &note);
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(DONE)).trail(Trail::None)]);
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
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["分区", "分割區", "Partitions"]));
    let bottom = match &s.filesystems {
        None => widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["正在读取分区", "正在讀取分割區", "Reading the partitions"])).trail(Trail::None)]).bottom,
        Some(Err(why)) => {
            let y = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["无法读取分区", "無法讀取分割區", "Couldn't read the partitions"])).trail(Trail::None)]).bottom;
            widgets::footnote(&mut c, t, f, y, why)
        }
        Some(Ok(rows)) if rows.is_empty() => {
            widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["暂无可修复的分区", "沒有可修復的分割區", "No partitions to repair"])).trail(Trail::None)]).bottom
        }
        Some(Ok(rows)) => {
            let subs: Vec<String> = rows.iter().map(|r| format!("{} · {}", fs_held(l, r, &s.data), fmt::bytes(r.size))).collect();
            let shown: Vec<Row> = rows.iter().zip(&subs).map(|(r, sub)| Row::new(&r.name).subtitle(sub)).collect();
            let list = widgets::list(&mut c, t, f, y, &shown);
            out.extend(hits(&list.rows, &(0..rows.len()).map(Action::OpenFilesystem).collect::<Vec<_>>()));
            let note = l.pick(["仅显示本机分区表中可格式化的分区。", "僅顯示本機分割區表中可格式化的分割區。", "Only the partitions in this phone's partition table that can be formatted show here."]);
            widgets::footnote(&mut c, t, f, list.bottom, note)
        }
    };
    Frame::new(t, fit(c, t, bottom), out, l.pick(FILESYSTEMS), top)
}

/// What a partition holds, as its row and page name it: the data
/// partition's is read only once it is open, and until then the row says
/// where it stands, as the home page does.
fn fs_held(l: Lang, row: &FsRow, data: &Data) -> String {
    match (&row.filesystem, row.name == DATA_PARTITION) {
        (Some(filesystem), _) => filesystem.clone(),
        (None, true) => fmt::data(l, data).into(),
        (None, false) => l.pick(["无法识别", "無法辨識", "Unrecognized"]).into(),
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
        (l.pick(["挂载点", "掛載點", "Mount point"]), row.mount_point.clone()),
        (l.pick(["文件系统", "檔案系統", "Filesystem"]), fs_held(l, row, &s.data)),
        (l.pick(["大小", "大小", "Size"]), fmt::bytes(row.size)),
    ];
    let y = widgets::info_card(&mut c, t, f, top, &facts);
    let y = widgets::section(&mut c, t, f, y, l.pick(ACTIONS));
    let chosen = |action: &FsAction| s.fs_action == *action;
    let remakes: Vec<String> = row.others.iter().map(|fs| tr!(l, "更换为 {fs}", "更換為 {fs}", "Change to {fs}")).collect();
    let made: Vec<String> = row.others.iter().map(|_| remade(row, s)).collect();
    let repair = Row::new(l.pick(["修复文件系统", "修復檔案系統", "Repair the filesystem"]))
        .subtitle(l.pick(["检查整个文件系统，并修复发现的错误", "檢查整個檔案系統，並修復發現的錯誤", "Checks the whole filesystem and fixes the errors found"]))
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
        let note = l.pick(["暂不支持更换该分区的文件系统。", "尚不支援更換這個分割區的檔案系統。", "This partition's filesystem can't be changed."]);
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if data {
        let note = l.pick([
            "修复前将关闭 data 分区，完成后需在「解密」中重新打开。",
            "修復前將關閉 data 分割區，完成後需在「解密」中重新開啟。",
            "The data partition closes for the repair, and opens again from Decrypt once it's done.",
        ]);
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if !row.others.is_empty() {
        y = widgets::footnote(&mut c, t, f, y, l.pick(["更换后，分区中的内容无法恢复。", "更換後，分割區中的內容無法復原。", "Once changed, what the partition held can't be brought back."]));
    }
    let (label, danger) = confirm_label(Confirm::Filesystem, s);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Filesystem) });
    Frame::new(t, fit(c, t, track.y + track.h), out, partition_title(l, &row.name), top)
}

/// The title of the page of a partition the filesystem page lists.
fn partition_title(l: Lang, name: &str) -> &'static str {
    l.pick(match name {
        DATA_PARTITION => DATA_PART,
        "metadata" => ["metadata 分区", "metadata 分割區", "metadata partition"],
        "cache" => ["cache 分区", "cache 分割區", "cache partition"],
        _ => ["分区", "分割區", "Partition"],
    })
}

/// What making `row` afresh as another filesystem erases: the data
/// partition goes as a factory reset goes, with /metadata and the cache
/// partition beside it.
fn remade(row: &FsRow, s: &State) -> String {
    match row.name == DATA_PARTITION {
        true => {
            let parts = s.lang.pick(match s.cache_partition {
                true => ["data、cache、metadata 分区", "data、cache、metadata 分割區", "the data, cache and metadata partitions"],
                false => ["data、metadata 分区", "data、metadata 分割區", "the data and metadata partitions"],
            });
            tr!(s.lang, "格式化 {parts}，清空本机的全部数据", "格式化 {parts}，清空本機的全部資料", "Formats {parts}, emptying all of this phone's data")
        }
        false => {
            let name = &row.name;
            tr!(s.lang, "格式化 {name} 分区，清空其中的全部内容", "格式化 {name} 分割區，清空其中的全部內容", "Formats the {name} partition, emptying all it holds")
        }
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
        PartKind::Disable => (["清除分区", "清除分割區", "Wipe partitions"], true),
        PartKind::Unshare => (["取消数据块共享", "取消資料區塊共用", "Unshare blocks"], true),
        PartKind::Repair => (["修复文件系统", "修復檔案系統", "Repair filesystem"], false),
        PartKind::Remake(_) => (["更换文件系统", "更換檔案系統", "Change filesystem"], false),
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
                ItemState::Waiting => l.pick(WAITING_TURN).into(),
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
        true => ["完成后将自动关机。", "完成後將自動關機。", "Powers off on its own once done."],
        false => ["完成后将自动重启。", "完成後將自動重新啟動。", "Reboots on its own once done."],
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
            let failed = verb.failed_on(l, name);
            notes.push(tr!(l, "{failed}：{why}", "{failed}：{why}", "{failed}: {why}"));
        }
    }
    for note in &notes {
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if let (true, Some(reason)) = (work.asked, &s.asked.reason) {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(REASON), reason.clone())]);
    }
    match (&work.outcome, work.asked) {
        (None, _) => {}
        (Some(_), true) if !short => {}
        // The system's work ran without the slot's partitions, which going
        // on in the recovery brings up first, as leaving fastboot does.
        (Some(_), true) => {
            let below = widgets::section(&mut c, t, f, y, l.pick(ACTIONS));
            let rows: Vec<Row> = AFTERWARDS.iter().map(|&(title, _)| Row::new(l.pick(title)).trail(Trail::None)).collect();
            let actions = AFTERWARDS.map(|(_, action)| if action == Action::Dismiss { Action::EnterRecovery } else { action });
            let choices = widgets::list(&mut c, t, f, below, &rows);
            out.extend(hits(&choices.rows, &actions));
            y = choices.bottom;
        }
        (Some(_), false) => {
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(DONE)).trail(Trail::None)]);
            out.extend(hits(&finish.rows, &[Action::Back]));
            y = finish.bottom;
        }
    }
    if !work.output.is_empty() {
        let below = widgets::section(&mut c, t, f, y, l.pick(["程序输出", "程式輸出", "Program output"]));
        rows = Some(Rows { edges: widgets::output_edges(&c, t, f, below, &work.output), action: None });
        y = below;
    }
    let back = work.outcome.is_some() && !work.asked;
    Frame { rows, back, ..Frame::new(t, fit(c, t, y), out, l.pick(title), top) }
}

const ACTIONS: Text = ["操作", "操作", "Actions"];

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
        let p = percent(share);
        let detail = tr!(l, "已完成 {p}%", "已完成 {p}%", "{p}% done");
        let y = widgets::progress_card(&mut c, t, f, top, l.pick(["正在合并更新", "正在合併更新", "Merging the update"]), &detail, Some(share));
        let note = l.pick([
            "合并完成前，暂不刷写 super 中的分区。返回后将继续合并。",
            "合併完成前，暫不刷寫 super 中的分割區。返回後將繼續合併。",
            "Partitions in super aren't flashed until it merges. Merging goes on after going back.",
        ]);
        let y = widgets::footnote(&mut c, t, f, y, note);
        return Frame::new(t, fit(c, t, y), out, l.pick(UPDATE), top);
    }
    let mut y = top;
    if let Some(v) = &i.os_version {
        y = widgets::section_first(&mut c, t, f, y, l.pick(["本机", "本機", "This phone"]));
        y = widgets::info_card(&mut c, t, f, y, &[(l.pick(["当前版本", "目前版本", "Current version"]), v.clone())]);
    }
    if s.update_server {
        let online_update = l.pick(["在线更新", "線上更新", "Online update"]);
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
    let merging = l.pick(["更新合并", "更新合併", "Merging"]);
    y = if y == top { widgets::section_first(&mut c, t, f, y, merging) } else { widgets::section(&mut c, t, f, y, merging) };
    y = widgets::list(&mut c, t, f, y, &[row]).bottom;
    if s.update == Standing::Waiting {
        let (label, danger) = confirm_label(Confirm::Merge, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(28.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::Merge) });
        let note = l.pick(["合并后无法回到更新前的系统。", "合併後無法回到更新前的系統。", "Once merged, the system from before the update can't come back."]);
        y = widgets::footnote(&mut c, t, f, track.y + track.h, note);
    }
    Frame::new(t, fit(c, t, y), out, l.pick(UPDATE), top)
}

/// What downloading or checking for updates needs first, while the phone
/// has joined no network that reaches the internet.
const WLAN_FIRST: Text = ["需先连接 WLAN", "需先連接 Wi-Fi", "Connect to Wi-Fi first"];
const CHECKING: Text = ["正在检查更新", "正在檢查更新", "Checking for updates"];
const CHECK_FOR_UPDATES: Text = ["检查更新", "檢查更新", "Check for updates"];
const DOWNLOADING: Text = ["正在下载", "正在下載", "Downloading"];
const CHECKING_PART: Text = ["正在校验已下载的部分", "正在驗證已下載的部分", "Verifying what was downloaded"];

/// Asking the update server for the newest system, then its full package:
/// downloading it, how far that has come, and installing it once it is
/// whole; then what to note before installing it.
fn online(c: &mut Canvas, cx: &Ctx, y: f32, out: &mut Vec<Hit>) -> f32 {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let release = match &s.check {
        Check::Asking => return widgets::list(c, t, f, y, &[Row::new(l.pick(CHECKING)).trail(Trail::None)]).bottom,
        Check::Answered(Some(r)) => r,
        other => {
            let mut row = Row::new(l.pick(CHECK_FOR_UPDATES));
            if !s.online() {
                row = row.subtitle(l.pick(WLAN_FIRST));
            }
            let check = widgets::list(c, t, f, y, &[row]);
            out.extend(hits(&check.rows, &[Action::CheckUpdate]));
            return match other {
                Check::Failed(why) => widgets::footnote(c, t, f, check.bottom, &fmt::unanswered(l, why)),
                Check::Answered(None) => {
                    let none = l.pick(["暂无本机的系统更新信息。", "目前沒有本機的系統更新資訊。", "The server knows of no system update for this phone."]);
                    widgets::footnote(c, t, f, check.bottom, none)
                }
                _ => check.bottom,
            };
        }
    };
    let newer = i.os_version.as_deref() != Some(release.version.as_str());
    let version = &release.version;
    let title = if newer { tr!(l, "新版本 {version}", "新版本 {version}", "New version {version}") } else { l.pick(["已是最新版本", "已是最新版本", "Up to date"]).to_string() };
    let size = match release.length {
        Some(length) => fmt::file_size(length),
        None => fmt::package_size(&release.size),
    };
    let package = tr!(l, "完整更新包 {size}", "完整更新檔 {size}", "Full OTA package {size}");
    let mut y = widgets::list(c, t, f, y, &[Row::new(&title).icon(GREEN, Icon::Layers).subtitle(&package).trail(Trail::None)]).bottom;
    match (&s.download, s.have) {
        (Some(job), _) if job.outcome.is_none() => {
            let (heading, detail, share) = match job.progress {
                None => (l.pick(GETTING_READY), String::new(), None),
                Some(p) => downloaded(l, p, job.rate),
            };
            y = widgets::progress_card(c, t, f, y + t.m.card_gap, heading, &detail, share);
            let label = l.pick(if job.stopping { ["正在暂停", "正在暫停", "Pausing"] } else { ["暂停", "暫停", "Pause"] });
            let stop = widgets::list(c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopDownload]));
            }
            y = stop.bottom;
        }
        (_, Have::Whole(_)) => {
            let row = Row::new(l.pick(["安装", "安裝", "Install"])).subtitle(l.pick(["已下载到内部存储", "已下載到內部儲存空間", "Downloaded to internal storage"]));
            let install = widgets::list(c, t, f, y + t.m.card_gap, &[row]);
            out.extend(hits(&install.rows, &[Action::InstallDownloaded]));
            y = install.bottom;
        }
        (job, have) => {
            match job.as_ref().and_then(|j| j.outcome.as_ref()) {
                Some(DownloadEnd::Failed(why)) => y = widgets::footnote(c, t, f, y, &fmt::download_failure(l, why)),
                Some(DownloadEnd::Stopped) => y = widgets::footnote(c, t, f, y, l.pick(["已暂停下载。", "已暫停下載。", "Download paused."])),
                _ => {}
            }
            let title = l.pick(if matches!(have, Have::Part(_)) { ["继续下载", "繼續下載", "Resume download"] } else { ["下载", "下載", "Download"] });
            let had = match have {
                Have::Part(done) => {
                    let done = fmt::file_size(done);
                    Some(tr!(l, "已下载 {done}", "已下載 {done}", "{done} downloaded"))
                }
                _ => None,
            };
            let sub = if !s.data.storage_open() {
                Some(l.pick(DECRYPT_NEEDED))
            } else if !s.online() {
                Some(l.pick(WLAN_FIRST))
            } else {
                had.as_deref()
            };
            let row = Row::new(title);
            let download = widgets::list(c, t, f, y + t.m.card_gap, &[sub.map_or(row, |sub| row.subtitle(sub))]);
            out.extend(hits(&download.rows, &[Action::Download]));
            y = download.bottom;
        }
    }
    let saved = tr!(
        l,
        "完整更新包将保存到\"/sdcard/{DOWNLOADS}\"中，安装后不会自动删除。",
        "完整更新檔將儲存到「/sdcard/{DOWNLOADS}」中，安裝後不會自動刪除。",
        "The full OTA package is saved in “/sdcard/{DOWNLOADS}”, and stays there after it's installed."
    );
    let points = [
        l.pick(["安装系统更新不会删除用户数据，建议在安装前进行备份。", "安裝系統更新不會刪除使用者資料，建議在安裝前進行備份。", "Installing a system update deletes no user data. Backing up first is still wise."]),
        saved.as_str(),
        l.pick(["下载中断后可继续下载，已下载的部分会保留。", "下載中斷後可繼續下載，已下載的部分會保留。", "A download cut short can be resumed, keeping what was downloaded."]),
        l.pick([
            "更新后，系统会在后台进行一系列优化，期间可能出现发热、耗电增加等情况，1 ~ 3 天内将自行恢复。",
            "更新後，系統會在背景進行一系列最佳化，期間可能出現發熱、耗電增加等情況，1 ~ 3 天內將自行恢復。",
            "After the update the system optimizes itself in the background, and the phone may run warm or use more battery for 1 to 3 days.",
        ]),
    ];
    widgets::notes(c, t, f, y + t.m.card_gap, l.pick(NOTES), &points)
}

const NOTES: Text = ["注意事项", "注意事項", "Notes"];

/// What a progress card says of a download, and the share of it done:
/// what an earlier download left being checked, or the rest coming at
/// `rate` bytes a second.
fn downloaded(l: Lang, progress: Progress, rate: u64) -> (&'static str, String, Option<f32>) {
    match progress {
        Progress::Checking { done, total } => (l.pick(CHECKING_PART), format!("{} / {}", fmt::file_size(done), fmt::file_size(total)), Some(share(done, total))),
        Progress::Downloading { done, total } => (
            l.pick(DOWNLOADING),
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
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["本机", "本機", "This phone"]));
    let y = widgets::info_card(&mut c, t, f, y, &[(l.pick(["当前版本", "目前版本", "Current version"]), VERSION.to_string())]);
    let y = widgets::section(&mut c, t, f, y, l.pick(["在线更新", "線上更新", "Online update"]));
    let y = skyward_online(&mut c, cx, y, &mut out);
    let points = [
        l.pick([
            "新版本从 GitHub 下载，校验 SHA-256 后写入 recovery 分区，不会改动系统和用户数据。",
            "新版本從 GitHub 下載，驗證 SHA-256 後寫入 recovery 分割區，不會改動系統和使用者資料。",
            "The new version is downloaded from GitHub and goes into the recovery partitions once its SHA-256 checks out. The system and user data are left as they are.",
        ]),
        l.pick([
            "新版本将写入下次启动所用槽位的 recovery 分区；另一个槽位装有 Skyward 时，也会一并更新。",
            "新版本將寫入下次啟動所用槽位的 recovery 分割區；另一個槽位裝有 Skyward 時，也會一併更新。",
            "The new version goes into the recovery partition of the slot that starts next, and into the other slot's where it holds Skyward.",
        ]),
        l.pick(["更新完成后，重启到恢复模式即可使用新版本。", "更新完成後，重新啟動到復原模式即可使用新版本。", "Once updated, reboot to recovery to use the new version."]),
    ];
    let y = widgets::notes(&mut c, t, f, y + t.m.card_gap, l.pick(NOTES), &points);
    Frame::new(t, fit(c, t, y), out, l.pick(SKYWARD_UPDATE), top)
}

/// Asking GitHub for Skyward's newest release, then updating to it: its
/// build for the phone downloaded and written into the recovery partitions,
/// how far that has come, and how it ended.
fn skyward_online(c: &mut Canvas, cx: &Ctx, y: f32, out: &mut Vec<Hit>) -> f32 {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let gap = t.m.card_gap;
    let newest = match &s.skyward_check {
        SkywardCheck::Asking => return widgets::list(c, t, f, y, &[Row::new(l.pick(CHECKING)).trail(Trail::None)]).bottom,
        SkywardCheck::Answered(n) => n,
        other => {
            let mut row = Row::new(l.pick(CHECK_FOR_UPDATES));
            if !s.online() {
                row = row.subtitle(l.pick(WLAN_FIRST));
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
        (true, Some(build)) => {
            let size = fmt::file_size(build.size);
            (tr!(l, "新版本 {version}", "新版本 {version}", "New version {version}"), tr!(l, "安装包 {size}", "安裝包 {size}", "Package {size}"))
        }
        (true, None) => (
            tr!(l, "新版本 {version}", "新版本 {version}", "New version {version}"),
            l.pick(["没有本机的安装包", "沒有本機的安裝包", "No package for this phone"]).to_string(),
        ),
        (false, _) => (
            l.pick(["已是最新版本", "已是最新版本", "Up to date"]).to_string(),
            tr!(l, "最新发布的版本为 {version}", "最新發佈的版本為 {version}", "The newest release is {version}"),
        ),
    };
    let y = widgets::list(c, t, f, y, &[Row::new(&title).icon(GREEN, Icon::Install).subtitle(&detail).trail(Trail::None)]).bottom;
    if !newest.newer || newest.build.is_none() {
        return y;
    }
    match &s.skyward_job {
        Some(job) if job.outcome.is_none() => {
            let (heading, detail, share) = match job.stage {
                None => (l.pick(GETTING_READY), String::new(), None),
                Some(SkywardStage::Download(p)) => downloaded(l, p, job.rate),
                Some(SkywardStage::Write { done, total }) => {
                    let part = share(done, total);
                    let p = percent(part);
                    (l.pick(["正在写入 recovery 分区", "正在寫入 recovery 分割區", "Writing the recovery partitions"]), tr!(l, "已完成 {p}%", "已完成 {p}%", "{p}% done"), Some(part))
                }
            };
            let y = widgets::progress_card(c, t, f, y + gap, heading, &detail, share);
            if matches!(job.stage, Some(SkywardStage::Write { .. })) {
                return widgets::footnote(c, t, f, y, l.pick(["写入完成前，请不要关机。", "寫入完成前，請不要關機。", "Don't power off until it's written."]));
            }
            let stop = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(if job.stopping { STOPPING } else { STOP })).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopSkywardUpdate]));
            }
            stop.bottom
        }
        Some(SkywardJob { outcome: Some(SkywardEnd::Updated(slots)), .. }) => {
            let names = fmt::slots(l, slots);
            let detail = match (l, slots.len()) {
                (Lang::En, 1) => format!("Skyward {version} is in {names}'s recovery partition."),
                _ => tr!(
                    l,
                    "已将 Skyward {version} 刷入{names} 的 recovery 分区。",
                    "已將 Skyward {version} 刷入{names} 的 recovery 分割區。",
                    "Skyward {version} is in the recovery partitions of {names}."
                ),
            };
            let y = widgets::list(c, t, f, y + gap, &[standing(l.pick(["更新成功", "更新成功", "Updated"]), GREEN, Icon::Install, &detail)]).bottom;
            let restart = widgets::list(c, t, f, y + gap, &[Row::new(l.pick(["重启到恢复模式", "重新啟動到復原模式", "Reboot to recovery"])).icon(ORANGE, Icon::Power)]);
            out.extend(hits(&restart.rows, &[Action::RebootTo(Target::Recovery)]));
            restart.bottom
        }
        job => {
            let mut y = y;
            match job.as_ref().and_then(|j| j.outcome.as_ref()) {
                Some(SkywardEnd::Stopped) => y = widgets::footnote(c, t, f, y, l.pick(["已停止下载。", "已停止下載。", "Download stopped."])),
                Some(SkywardEnd::Download(why)) => y = widgets::footnote(c, t, f, y, &fmt::skyward_download_failure(l, why)),
                Some(SkywardEnd::NotWritten(why)) => {
                    y = widgets::footnote(c, t, f, y, l.pick(["更新失败，未写入任何内容。", "更新失敗，未寫入任何內容。", "Couldn't update. Nothing was written."]));
                    y = widgets::footnote(c, t, f, y, why);
                }
                Some(SkywardEnd::WriteFailed(slots, why)) => {
                    let names = fmt::slots(l, slots);
                    let warning = match (l, slots.len()) {
                        (Lang::En, 1) => format!(
                            "{}'s recovery partition was only partly written, so rebooting to recovery may not start Skyward. Update again, and reboot once it's done.",
                            capitalized(l, &names)
                        ),
                        _ => tr!(
                            l,
                            "{names} 的 recovery 分区仅写入了部分内容，重启到恢复模式可能无法进入 Skyward。请重新更新，完成后再重启。",
                            "{names} 的 recovery 分割區僅寫入了部分內容，重新啟動到復原模式可能無法進入 Skyward。請重新更新，完成後再重新啟動。",
                            "The recovery partitions of {names} were only partly written, so rebooting to recovery may not start Skyward. Update again, and reboot once it's done."
                        ),
                    };
                    y = widgets::footnote(c, t, f, y, &warning);
                    y = widgets::footnote(c, t, f, y, why);
                }
                Some(SkywardEnd::Updated(_)) => unreachable!("an update that went in shows how it did"),
                None => {}
            }
            let mut row = Row::new(l.pick(["下载并更新", "下載並更新", "Download and update"]));
            if !s.online() {
                row = row.subtitle(l.pick(WLAN_FIRST));
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
    let switch = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(WLAN)).trail(Trail::Switch(on))]);
    out.extend(hits(&switch.rows, &[Action::Toggle(Switch::Wlan)]));
    let mut y = switch.bottom;
    match &w.power {
        Power::Off => {
            let note = l.pick([
                "开启后，可连接 WLAN 下载系统更新和 Skyward 新版本。",
                "開啟後，可連接 Wi-Fi 下載系統更新和 Skyward 新版本。",
                "When on, system updates and new versions of Skyward can download over Wi-Fi.",
            ]);
            y = widgets::footnote(&mut c, t, f, y, note)
        }
        Power::Failed(why) => y = widgets::footnote(&mut c, t, f, y, &tr!(l, "无法开启 WLAN：{why}", "無法開啟 Wi-Fi：{why}", "Couldn't turn on Wi-Fi: {why}")),
        Power::Starting => y = widgets::footnote(&mut c, t, f, y, l.pick(["正在开启 WLAN", "正在開啟 Wi-Fi", "Turning on Wi-Fi"])),
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
                if let Joining::Connected(link) = &j.stage {
                    if let Some(note) = link.reach.as_ref().and_then(|r| fmt::reach(l, r)) {
                        y = widgets::footnote(&mut c, t, f, y, &note);
                    }
                }
            }
            let others: Vec<(usize, &Network)> =
                w.networks.iter().enumerate().filter(|(_, n)| w.joined.as_ref().is_none_or(|j| j.ssid != n.ssid)).collect();
            let nearby = l.pick(["选取附近的 WLAN", "選擇附近的 Wi-Fi 網路", "Choose a nearby network"]);
            let (below, rescan) = widgets::section_action(&mut c, t, f, y, nearby, l.pick(["刷新", "重新整理", "Refresh"]));
            out.push(Hit { rect: rescan, action: Action::Rescan });
            y = below;
            if others.is_empty() {
                let looking = l.pick(["正在搜索附近的 WLAN", "正在搜尋附近的 Wi-Fi 網路", "Looking for nearby networks"]);
                y = widgets::footnote(&mut c, t, f, y - t.m.dp(10.0), looking) + t.m.card_gap;
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
            rows.push(Row::new(l.pick(ADD_NETWORK)).tint(t.pal.accent).trail(Trail::None));
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
    Frame::new(t, fit(c, t, y), out, l.pick(WLAN), top)
}

const ADD_NETWORK: Text = ["添加网络", "新增網路", "Add network"];
const SHOW_PASSWORD: Text = ["显示密码", "顯示密碼", "Show password"];
const CONNECT_KEY: Text = ["连接", "連線", "Connect"];
const SECURITY: Text = ["安全性", "安全性", "Security"];

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
    let show = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(SHOW_PASSWORD)).trail(Trail::Switch(s.show_password))]);
    out.extend(hits(&show.rows, &[Action::Toggle(Switch::ShowPassword)]));
    keyboard_hits(&mut c, t, f, s, l.pick(CONNECT_KEY), &mut out);
    Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(["输入密码", "輸入密碼", "Enter password"]), top) }
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
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["网络名称", "網路名稱", "Network name"]));
    let (below, mut caret) = widgets::text_field(&mut c, t, f, y, &a.name);
    out.push(Hit { rect: field(y), action: Action::TypeInto(Field::Name) });
    // Held down while its list is open, as the stock row stays.
    let tone = if a.choosing { Tone::page(&t.pressed()) } else { Tone::page(t) };
    let protection = [Row::new(l.pick(SECURITY)).trail(Trail::Choice(fmt::security(l, a.security)))];
    let security = widgets::list_toned(&mut c, t, f, below + m.card_gap, &protection, &tone);
    out.extend(hits(&security.rows, &[Action::ChooseSecurity]));
    if a.security.password() {
        let y = widgets::section(&mut c, t, f, security.bottom, l.pick(["密码", "密碼", "Password"]));
        let shown = if s.show_password { s.password.clone() } else { "•".repeat(s.password.chars().count()) };
        let (below, typed) = widgets::text_field(&mut c, t, f, y, &shown);
        out.push(Hit { rect: field(y), action: Action::TypeInto(Field::Password) });
        if a.field == Field::Password {
            caret = typed;
        }
        let show = widgets::list(&mut c, t, f, below + m.card_gap, &[Row::new(l.pick(SHOW_PASSWORD)).trail(Trail::Switch(s.show_password))]);
        out.extend(hits(&show.rows, &[Action::Toggle(Switch::ShowPassword)]));
    }
    keyboard_hits(&mut c, t, f, s, l.pick(CONNECT_KEY), &mut out);
    if !a.choosing {
        return Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(ADD_NETWORK), top) };
    }
    // A touch anywhere off the list closes it.
    out.clear();
    out.push(Hit { rect: Rect::new(0.0, 0.0, m.w as f32, m.h as f32), action: Action::CloseChoices });
    let names = ADDABLE.map(|p| fmt::security(l, p));
    let chosen = ADDABLE.iter().position(|&p| p == a.security);
    let held = ADDABLE.iter().position(|&p| s.held == Some(Action::PickSecurity(p)));
    let list = widgets::choices(&mut c, t, f, security.rows[0], &names, chosen, held);
    out.extend(hits(&list, &ADDABLE.map(Action::PickSecurity)));
    Frame::new(t, c, out, l.pick(ADD_NETWORK), top)
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
    let security = detail(Outline::Shield, l.pick(SECURITY), fmt::security(l, network.security).to_string());
    let signal = l.pick(["信号强度", "訊號強度", "Signal strength"]);
    let title = l.pick(["网络详情", "網路詳細資料", "Network details"]);
    let Some(j) = joined else {
        let items = [
            detail(Outline::Wifi, l.pick(["频段", "頻段", "Bands"]), fmt::bands(l, network.bands)),
            detail(Outline::Signal, signal, fmt::signal(l, network.signal).to_string()),
            security,
        ];
        let y = widgets::detail_grid(&mut c, t, f, y, &items);
        return Frame::new(t, fit(c, t, y), out, title, top);
    };
    let mut items = vec![detail(Outline::Info, l.pick(["状态信息", "狀態資訊", "Status"]), fmt::joining(l, &j.stage).to_string())];
    match &j.stage {
        Joining::Connected(link) => {
            if let Some(n) = link.generation {
                items.push(detail(Outline::Wifi, l.pick(["技术标准", "技術標準", "Wi-Fi standard"]), fmt::generation(l, n)));
            }
            if let Some(speed) = link.speed {
                items.push(detail(Outline::Speed, l.pick(["连接速度", "連線速度", "Link speed"]), format!("{speed} Mbps")));
            }
            items.push(detail(Outline::Signal, signal, fmt::signal(l, link.signal).to_string()));
            items.push(security);
            items.push(detail(Outline::Address, l.pick(["IP 地址", "IP 位址", "IP address"]), link.address.to_string()));
            items.push(detail(Outline::Mask, l.pick(["子网掩码", "子網路遮罩", "Subnet mask"]), link.mask.to_string()));
            if let Some(router) = link.router {
                items.push(detail(Outline::Router, l.pick(["路由器", "路由器", "Router"]), router.to_string()));
            }
        }
        _ => {
            items.push(detail(Outline::Signal, signal, fmt::signal(l, network.signal).to_string()));
            items.push(security);
        }
    }
    let y = widgets::detail_grid(&mut c, t, f, y, &items);
    let forget = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(["删除网络", "刪除網路", "Forget network"])).tint(t.pal.danger).trail(Trail::None)]);
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
            FileOp::Copy => ["复制到此处", "複製到這裡", "Copy here"],
            FileOp::Move => ["移动到此处", "移動到這裡", "Move here"],
        });
        let (canvas, rects) = widgets::dock_bar(t, f, files::page(t), &[l.pick(CANCEL), go]);
        Dock { canvas, hits: hits(&rects, &[Action::Unpick, Action::Paste]) }
    });
    let button = Some((widgets::Glyph::Terminal, Action::TerminalHere));
    let root = l.pick(["根目录", "根目錄", "Root"]);
    Frame { dock, button, ..listing(cx, &s.files, root, None, l.pick(["暂无文件", "沒有檔案", "No files"]), Action::OpenEntry, l.pick(FILES)) }
}

const CANCEL: Text = ["取消", "取消", "Cancel"];

/// A place's folders and the files in them that can be installed, to
/// choose one. A USB drive's page waits for one to be plugged in, and the
/// drive's files show as soon as it is.
pub fn choose(cx: &Ctx, place: Place) -> Frame {
    let (s, l) = (cx.state, cx.state.lang);
    let unreadable;
    let absent = match (place, &s.usb_drive) {
        (Place::UsbDrive, None) => Some((
            l.pick(["未连接 U 盘", "未連接 USB 隨身碟", "No USB drive"]),
            l.pick(["通过 OTG 连接 U 盘后，将在此显示其中的文件。", "透過 OTG 連接 USB 隨身碟後，將在此顯示其中的檔案。", "Connect a USB drive through OTG to see its files here."]),
        )),
        (Place::UsbDrive, Some(UsbDrive::Unreadable(why))) => {
            unreadable = tr!(l, "请使用 FAT32 或 exFAT 格式的 U 盘。{why}", "請使用 FAT32 或 exFAT 格式的 USB 隨身碟。{why}", "Use a USB drive formatted as FAT32 or exFAT. {why}");
            Some((l.pick(USB_DRIVE_UNREADABLE), unreadable.as_str()))
        }
        _ => None,
    };
    let empty = l.pick(["暂无可安装的文件", "沒有可安裝的檔案", "Nothing here to install"]);
    listing(cx, s.folder(Screen::Choose(place)), place_name(l, place), absent, empty, Action::ChooseEntry, l.pick(["选择文件", "選擇檔案", "Choose a file"]))
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
            files::note(&mut c, t, f, cx.state.lang.pick(["无法读取该文件夹", "無法讀取這個資料夾", "This folder can't be read"]), Some(why));
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
            files::row(c, cx.t, cx.f, cx.state.lang, &entries[i], folder.listed, cx.state.time_zone);
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
        (l.pick(NAME), e.name.clone()),
        (l.pick(["位置", "位置", "Location"]), Path::new(&s.files.path).join(&e.name).to_string_lossy().into_owned()),
    ];
    let holds = l.pick(["内容", "內容", "Holds"]);
    match &e.kind {
        FileKind::Link { target, .. } => facts.push((l.pick(["指向", "指向", "Points to"]), target.clone())),
        FileKind::Folder { items: Some(n) } => {
            let items = match (l, n) {
                (Lang::En, 1) => "1 item".to_string(),
                (_, n) => tr!(l, "{n} 项", "{n} 項", "{n} items"),
            };
            facts.push((holds, items))
        }
        FileKind::Folder { items: None } => facts.push((holds, l.pick(UNREADABLE).to_string())),
        FileKind::File | FileKind::Special => {
            let (size, bytes) = (fmt::file_size(e.size), e.size);
            facts.push((l.pick(["大小", "大小", "Size"]), tr!(l, "{size}（{bytes} 字节）", "{size}（{bytes} 位元組）", "{size} ({bytes} bytes)")))
        }
    }
    facts.push((l.pick(["修改时间", "修改時間", "Modified"]), fmt::datetime(l, e.modified, s.time_zone)));
    let (mode, uid, gid) = (fmt::mode(e.mode), e.uid, e.gid);
    facts.push((l.pick(PERMISSIONS), tr!(l, "{mode}，所有者 {uid}，用户组 {gid}", "{mode}，擁有者 {uid}，群組 {gid}", "{mode}, owner {uid}, group {gid}")));
    let y = widgets::info_card(&mut c, t, f, top, &facts);
    let mut rows = Vec::new();
    let mut acts = Vec::new();
    // Images are flashed; packages and zips that install themselves are
    // installed.
    if installs(e) {
        let image = Path::new(&e.name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("img"));
        rows.push(Row::new(l.pick(if image { ["刷入", "刷入", "Flash"] } else { ["安装", "安裝", "Install"] })));
        acts.push(Action::InstallEntry);
    }
    if e.kind == FileKind::File {
        rows.push(Row::new(l.pick(["编辑", "編輯", "Edit"])));
        acts.push(Action::EditEntry);
    }
    rows.extend([Row::new(l.pick(["复制", "複製", "Copy"])), Row::new(l.pick(["移动", "移動", "Move"])), Row::new(l.pick(RENAME))]);
    acts.extend([Action::PickEntry(FileOp::Copy), Action::PickEntry(FileOp::Move), Action::RenameEntry]);
    // A link's own permissions mean nothing; its target is changed where
    // it is.
    if !matches!(e.kind, FileKind::Link { .. }) {
        rows.push(Row::new(l.pick(CHANGE_PERMISSIONS)));
        acts.push(Action::OpenPermissions);
    }
    rows.push(Row::new(l.pick(["删除", "刪除", "Delete"])).tint(t.pal.danger).trail(Trail::None));
    acts.push(Action::AskDelete);
    let actions = widgets::list(&mut c, t, f, y + t.m.card_gap, &rows);
    let mut out = hits(&actions.rows, &acts);
    let mut bottom = actions.bottom;
    if s.asking_delete {
        let name = &e.name;
        let what = match folder {
            true => tr!(
                l,
                "删除\"{name}\"及其中的全部内容？删除后无法恢复。",
                "刪除「{name}」及其中的全部內容？刪除後無法復原。",
                "Delete “{name}” and everything in it? It can't be brought back."
            ),
            false => tr!(l, "删除\"{name}\"？删除后无法恢复。", "刪除「{name}」？刪除後無法復原。", "Delete “{name}”? It can't be brought back."),
        };
        let y = widgets::footnote(&mut c, t, f, actions.bottom, &what);
        let (label, danger) = confirm_label(Confirm::DeleteEntry, cx.state);
        let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
        out.push(Hit { rect: track, action: Action::Confirm(Confirm::DeleteEntry) });
        bottom = track.y + track.h;
    }
    let title = l.pick(if folder { ["文件夹信息", "資料夾資訊", "Folder info"] } else { ["文件信息", "檔案資訊", "File info"] });
    Frame::new(t, fit(c, t, bottom), out, title, top)
}

const NAME: Text = ["名称", "名稱", "Name"];
const PERMISSIONS: Text = ["权限", "權限", "Permissions"];
const CHANGE_PERMISSIONS: Text = ["修改权限", "修改權限", "Change permissions"];
const RENAME: Text = ["重命名", "重新命名", "Rename"];

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
    let facts = [(l.pick(NAME), e.name.clone()), (l.pick(PERMISSIONS), tr!(l, "{bits}，{mode:04o}", "{bits}，{mode:04o}", "{bits}, {mode:04o}"))];
    let mut y = widgets::info_card(&mut c, t, f, top, &facts);
    let mut out = Vec::new();
    let classes = [(["所有者", "擁有者", "Owner"], 6), (["用户组", "群組", "Group"], 3), (["其他用户", "其他使用者", "Others"], 0)];
    let (read, write, run) = (l.pick(["读取", "讀取", "Read"]), l.pick(["写入", "寫入", "Write"]), l.pick(["执行", "執行", "Execute"]));
    let groups = classes.map(|(title, shift)| (l.pick(title), [(read, 4 << shift), (write, 2 << shift), (run, 1 << shift)]));
    let special = (l.pick(["特殊权限", "特殊權限", "Special"]), [("SUID", 0o4000), ("SGID", 0o2000), (l.pick(["粘滞位", "黏滯位", "Sticky"]), 0o1000)]);
    for (title, bits) in groups.into_iter().chain([special]) {
        y = widgets::section(&mut c, t, f, y, title);
        let rows = bits.map(|(name, bit)| Row::new(name).trail(Trail::Switch(mode & bit != 0)));
        let list = widgets::list(&mut c, t, f, y, &rows);
        out.extend(hits(&list.rows, &bits.map(|(_, bit)| Action::ModeBit(bit))));
        y = list.bottom;
    }
    Frame::new(t, fit(c, t, y), out, l.pick(CHANGE_PERMISSIONS), top)
}

/// A new name for the entry the file page shows, typed on the keyboard.
pub fn rename(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let (y, caret) = widgets::text_field(&mut c, t, f, top, &s.new_name);
    widgets::footnote(&mut c, t, f, y, l.pick(["名称不能包含\"/\"。点击「完成」保存。", "名稱不能包含「/」。點選「完成」儲存。", "A name can't hold “/”. Tap Done to save it."]));
    keyboard_hits(&mut c, t, f, s, l.pick(DONE), &mut out);
    Frame { caret: Some(caret), ..Frame::new(t, c, out, l.pick(RENAME), top) }
}

/// The folder an entry goes to already holds its name: what to do.
pub fn clash(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let (clip, into, _) = s.clash.as_ref().expect("the clash page shows a clash");
    let name = Path::new(&clip.path).file_name().map_or_else(|| clip.path.clone(), |n| n.to_string_lossy().into_owned());
    let verb = l.pick(match clip.op {
        FileOp::Copy => ["复制", "複製", "copy"],
        FileOp::Move => ["移动", "移動", "move"],
    });
    let mut c = page(cx);
    let top = t.m.content_top;
    let there = tr!(l, "\"{into}\"中已有同名的\"{name}\"。", "「{into}」中已有同名的「{name}」。", "“{into}” already holds an entry named “{name}”.");
    let y = widgets::paragraph(&mut c, t, f, top, &there);
    let skip = tr!(l, "保留已有的同名项，不{verb}该项", "保留已有的同名項目，不{verb}這個項目", "Keeps the entry there, and doesn't {verb} this one");
    let choices = widgets::list(&mut c, t, f, y + t.m.dp(12.0), &[
        Row::new(l.pick(["替换", "取代", "Replace"])).subtitle(l.pick(["覆盖已有的同名项", "覆寫已有的同名項目", "Writes over the entry there"])),
        Row::new(l.pick(["跳过", "略過", "Skip"])).subtitle(&skip),
        Row::new(l.pick(["保留两者", "保留兩者", "Keep both"])).subtitle(l.pick(["自动为该项重新命名", "自動為這個項目重新命名", "Gives this one a new name"])),
    ]);
    let out = hits(&choices.rows, &[
        Action::Resolve(ClashChoice::Replace),
        Action::Resolve(ClashChoice::Skip),
        Action::Resolve(ClashChoice::KeepBoth),
    ]);
    Frame::new(t, c, out, l.pick(["名称已存在", "名稱已存在", "Name taken"]), top)
}

/// How work on files is told of: the Chinese verb its sentences build on,
/// in simplified and in traditional characters, then English's own words
/// for what is done and for doing it; the page's titles while it goes on
/// and once it ended short; and what stopping it may have left.
struct FileWords {
    verb: [&'static str; 2],
    done: &'static str,
    base: &'static str,
    titles: (Text, Text),
    stopped: Text,
}

fn file_words(task: &FileTask) -> FileWords {
    match task {
        FileTask::Copy { .. } => FileWords {
            verb: ["复制", "複製"],
            done: "copied",
            base: "copy",
            titles: (["正在复制", "正在複製", "Copying"], ["复制未完成", "複製未完成", "Copy incomplete"]),
            stopped: ["已停止，部分内容可能已复制到目标文件夹。", "已停止，部分內容可能已複製到目標資料夾。", "Stopped. Part of it may have been copied into the folder."],
        },
        FileTask::Move { .. } => FileWords {
            verb: ["移动", "移動"],
            done: "moved",
            base: "move",
            titles: (["正在移动", "正在移動", "Moving"], ["移动未完成", "移動未完成", "Move incomplete"]),
            stopped: ["已停止，部分内容可能已移动到目标文件夹。", "已停止，部分內容可能已移動到目標資料夾。", "Stopped. Part of it may have been moved into the folder."],
        },
        FileTask::Delete { .. } => FileWords {
            verb: ["删除", "刪除"],
            done: "deleted",
            base: "delete",
            titles: (["正在删除", "正在刪除", "Deleting"], ["删除未完成", "刪除未完成", "Delete incomplete"]),
            stopped: ["已停止，部分内容可能已删除。", "已停止，部分內容可能已刪除。", "Stopped. Part of it may have been deleted."],
        },
    }
}

/// Copying, moving or deleting: how far it has come and a way to stop it,
/// then, if it did not finish, why.
pub fn file_job(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let job = s.job.as_ref().expect("the job page shows work on files");
    let path = match &job.task {
        FileTask::Copy { path, .. } | FileTask::Move { path, .. } | FileTask::Delete { path } => path,
    };
    let words = file_words(&job.task);
    let ([hans, hant], done_en, base) = (words.verb, words.done, words.base);
    let name = Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    let share = |(done, total): (u64, u64)| if total == 0 { 0.0 } else { done as f32 / total as f32 };
    // The bar stays while the work measures, so that the button under it
    // does not move. Deleting, and copying entries that hold no bytes, is
    // counted in entries.
    let (detail, done) = match (job.progress, &job.task) {
        (None, _) => (l.pick(GETTING_READY).to_string(), 0.0),
        (Some((bytes, _)), FileTask::Copy { .. } | FileTask::Move { .. }) if bytes.1 > 0 => {
            let (did, total) = (fmt::file_size(bytes.0), fmt::file_size(bytes.1));
            (tr!(l, "已{hans} {did}，共 {total}", "已{hant} {did}，共 {total}", "{did} of {total} {done_en}"), share(bytes))
        }
        (Some((_, entries)), _) => {
            let (did, total) = entries;
            (tr!(l, "已{hans} {did} 项，共 {total} 项", "已{hant} {did} 項，共 {total} 項", "{did} of {total} items {done_en}"), share(entries))
        }
    };
    let mut c = page(cx);
    let top = t.m.content_top;
    let y = widgets::progress_card(&mut c, t, f, top, &name, &detail, Some(done));
    let mut out = Vec::new();
    match &job.outcome {
        None => {
            let label = l.pick(if job.stopping { STOPPING } else { STOP });
            let stop = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(label).trail(Trail::None)]);
            if !job.stopping {
                out.extend(hits(&stop.rows, &[Action::StopJob]));
            }
        }
        Some(end) => {
            let why = match end {
                JobEnd::Stopped => l.pick(words.stopped).to_string(),
                JobEnd::Failed(why) => tr!(l, "{hans}失败：{why}", "{hant}失敗：{why}", "Couldn't {base} it: {why}"),
                JobEnd::Done => unreachable!("work done leaves its page"),
            };
            let y = widgets::footnote(&mut c, t, f, y, &why);
            let finish = widgets::list(&mut c, t, f, y + t.m.card_gap, &[Row::new(l.pick(DONE)).trail(Trail::None)]);
            out.extend(hits(&finish.rows, &[Action::Back]));
        }
    }
    let title = l.pick(if job.outcome.is_none() { words.titles.0 } else { words.titles.1 });
    Frame { back: job.outcome.is_some(), ..Frame::new(t, c, out, title, top) }
}

/// A shell on a terminal: the screen it writes, the keys a phone keyboard
/// lacks, and the keyboard, every key of which goes to the shell.
pub fn terminal(cx: &Ctx) -> Frame {
    let (t, f, s) = (cx.t, cx.f, cx.state);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    keyboard_hits(&mut c, t, f, s, s.lang.pick(["回车", "Enter", "Enter"]), &mut out);
    let l = term::layout(t, f, top);
    term::draw(&mut c, t, f, &l, cx.term);
    let held = cx.term.is_some_and(|term| term.ctrl);
    let keys = term::keys(&mut c, t, f, &l, &term::TERM_KEYS, held);
    out.extend(keys.into_iter().map(|(rect, key)| Hit { rect, action: Action::TermKey(key) }));
    Frame { scrollback: Some((l.card, l.pitch)), ..Frame::new(t, c, out, s.lang.pick(TERMINAL), top) }
}

/// A text file being edited: where it is, its lines wrapped on a card, the
/// keys a phone keyboard lacks for moving the cursor, and the keyboard.
/// Once it is changed, the round button saves it.
pub fn editor(cx: &Ctx) -> Frame {
    let (t, f, s) = (cx.t, cx.f, cx.state);
    let ed = s.editor.as_ref().expect("the editor page shows a file");
    let mut c = page(cx);
    let mut out = Vec::new();
    keyboard_hits(&mut c, t, f, s, s.lang.pick(["换行", "換行", "Return"]), &mut out);
    editor::caption(&mut c, t, f, &ed.path);
    let l = editor::layout(t, f);
    let caret = editor::draw(&mut c, t, f, &l, ed);
    let keys = term::keys(&mut c, t, f, &l, &editor::EDITOR_KEYS, false);
    out.extend(keys.into_iter().map(|(rect, key)| Hit { rect, action: Action::TermKey(key) }));
    let button = ed.changed.then_some((widgets::Glyph::Done, Action::SaveText));
    Frame { scrollback: Some((l.card, l.pitch)), caret, button, ..Frame::new(t, c, out, s.lang.pick(["编辑", "編輯", "Edit"]), t.m.content_top) }
}

/// The languages the pages speak, each named in itself, to speak another.
pub fn language(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = page(cx);
    let top = t.m.content_top;
    let rows: Vec<Row> = Lang::ALL.iter().map(|&k| Row::new(k.name()).trail(Trail::Check(k == l))).collect();
    let list = widgets::list(&mut c, t, f, top, &rows);
    let out = hits(&list.rows, &Lang::ALL.map(Action::Language));
    Frame::new(t, c, out, l.pick(LANGUAGE), top)
}

/// The time zone the status bar's clock and the times on the pages keep.
pub fn time_zone(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let rows: Vec<Row> = TIME_ZONES
        .iter()
        .enumerate()
        .map(|(k, &(name, cities, _))| Row::new(l.pick(name)).subtitle(l.pick(cities)).trail(Trail::Check(k == s.time_zone)))
        .collect();
    let list = widgets::list(&mut c, t, f, top, &rows);
    let picks: Vec<Action> = (0..TIME_ZONES.len()).map(Action::TimeZone).collect();
    let out = hits(&list.rows, &picks);
    let note = l.pick(["状态栏和各页面中的时间按所选时区显示。", "狀態列和各頁面中的時間按所選時區顯示。", "The status bar and the pages tell times in the zone chosen."]);
    let bottom = widgets::footnote(&mut c, t, f, list.bottom, note);
    Frame::new(t, fit(c, t, bottom), out, l.pick(TIME_ZONE), top)
}

/// Saving the logs, as a bug report asks for them.
pub fn logs(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["日志内容", "記錄內容", "Logs"]));
    let parts = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(["Recovery 日志", "Recovery 記錄", "Recovery log"]))
            .subtitle(l.pick(["界面、安装和挂载的记录", "介面、安裝和掛載的記錄", "What the pages, installs and mounts did"]))
            .trail(Trail::Check(s.logs[0])),
        Row::new(l.pick(["内核日志", "核心記錄", "Kernel log"]))
            .subtitle(l.pick(["本次开机以来的内核消息", "本次開機以來的核心訊息", "The kernel's messages since this start"]))
            .trail(Trail::Check(s.logs[1])),
    ]);
    out.extend(hits(&parts.rows, &[Action::LogPart(0), Action::LogPart(1)]));
    let y = widgets::section(&mut c, t, f, parts.bottom, l.pick(["保存到", "儲存到", "Save to"]));
    let storage = l.pick(match cx.state.data.storage_open() {
        true => ["Skyward/logs 文件夹", "Skyward/logs 資料夾", "The Skyward/logs folder"],
        false => DECRYPT_NEEDED,
    });
    let places = widgets::list(&mut c, t, f, y, &[
        Row::new(l.pick(["临时文件夹", "暫存資料夾", "Temporary folder"]))
            .subtitle(l.pick(["/skyward/run/logs，可通过 adb pull 导出到电脑", "/skyward/run/logs，可透過 adb pull 匯出到電腦", "/skyward/run/logs, which adb pull copies to a computer"]))
            .trail(Trail::Check(s.logs_to == 0)),
        Row::new(place_name(l, Place::Storage)).subtitle(storage).trail(Trail::Check(s.logs_to == 1)),
    ]);
    out.extend(hits(&places.rows, &[Action::LogsTo(0), Action::LogsTo(1)]));
    let note = l.pick([
        "每次导出保存在以导出时间命名的文件夹中。重启后将清空临时文件夹。",
        "每次匯出儲存在以匯出時間命名的資料夾中。重新啟動後將清空暫存資料夾。",
        "Each export goes in a folder named by when it was made. A reboot empties the temporary folder.",
    ]);
    let y = widgets::footnote(&mut c, t, f, places.bottom, note);
    let export = widgets::button(&mut c, t, f, y + t.m.dp(24.0), l.pick(["导出", "匯出", "Export"]));
    out.push(Hit { rect: export, action: Action::ExportLogs });
    Frame::new(t, c, out, l.pick(LOGS), top)
}

/// Skyward's licence, in Chinese, ahead of the words the licence gives for
/// a program's notice; English reads those words alone.
const LICENSE_NOTICE: [&str; 2] = [
    "Skyward 是自由软件：你可以依据自由软件基金会发布的 GNU 通用公共许可证第 3 版，或（由你选择）任何更新的版本，重新发布和修改它。\n\n\
发布 Skyward 是希望它有用，但不附带任何担保，也不担保其适销性或适用于特定用途。详见 GNU 通用公共许可证。",
    "Skyward 是自由軟體：你可以依據自由軟體基金會發布的 GNU 通用公眾授權條款第 3 版，或（由你選擇）任何更新的版本，重新發布和修改它。\n\n\
發布 Skyward 是希望它有用，但不附帶任何擔保，也不擔保其適銷性或適用於特定用途。詳見 GNU 通用公眾授權條款。",
];

const LICENSE_ORIGINAL: &str = "Skyward\nCopyright (C) 2026 Kirk Lin\n\n\
This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.\n\n\
This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.\n\n\
You should have received a copy of the GNU General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.";

const LICENSE_TERMS: Text = ["许可证", "授權條款", "License"];

/// Skyward's own licence.
pub fn license(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let terms = l.pick([
        "GNU 通用公共许可证第 3 版或更新的版本（GPL-3.0-or-later）",
        "GNU 通用公眾授權條款第 3 版或更新的版本（GPL-3.0-or-later）",
        "GNU General Public License, version 3 or later (GPL-3.0-or-later)",
    ]);
    let y = widgets::info_card(&mut c, t, f, top, &[(l.pick(LICENSE_TERMS), terms.to_string())]);
    let bottom = match l {
        Lang::Hans | Lang::Hant => {
            let notice = if l == Lang::Hans { LICENSE_NOTICE[0] } else { LICENSE_NOTICE[1] };
            let y = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), notice);
            let y = widgets::section(&mut c, t, f, y, "原文");
            widgets::paragraph(&mut c, t, f, y - t.m.dp(16.0), LICENSE_ORIGINAL)
        }
        Lang::En => widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), LICENSE_ORIGINAL),
    };
    Frame::new(t, fit(c, t, bottom), Vec::new(), l.pick(LICENSE), top)
}

/// The monospaced typeface's licence, which names the copyright holders on
/// its first lines; the image carries the same file beside the font.
const MONO_LICENSE: &str = include_str!("../../../resources/fonts/KirkMono-OFL.txt");

/// The typefaces, as they name themselves, and the terms each is used under.
pub fn font_license(cx: &Ctx) -> Frame {
    let (t, f, l) = (cx.t, cx.f, cx.state.lang);
    let mut c = long_page(cx);
    let top = t.m.content_top;
    let (typeface, version, copyright) = (l.pick(["字体", "字型", "Typeface"]), l.pick(["版本", "版本", "Version"]), l.pick(["版权", "版權", "Copyright"]));
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["界面文字", "介面文字", "Interface text"]));
    let y = widgets::info_card(&mut c, t, f, y, &[
        (typeface, f.info.family.clone()),
        (version, f.info.version.clone()),
        (copyright, f.info.copyright.clone().expect("MiSans names its copyright holder")),
    ]);
    let misans = l.pick([
        "界面文字使用小米 MiSans 字体，依据《MiSans 字体知识产权许可协议》免费使用。字体文件未经修改，仅随 Skyward 提供，不单独分发。\n\n协议全文可在 MiSans 官方网站查看。",
        "介面文字使用小米 MiSans 字型，依據《MiSans 字體知識產權許可協議》免費使用。字型檔案未經修改，僅隨 Skyward 提供，不單獨散布。\n\n協議全文可在 MiSans 官方網站查看。",
        "The interface text is set in Xiaomi's MiSans, used free of charge under the MiSans Font Intellectual Property License Agreement. The font files are unchanged, and come with Skyward alone, never on their own.\n\nThe agreement in full is on the MiSans website.",
    ]);
    let y = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), misans);
    let y = widgets::section(&mut c, t, f, y, l.pick(["终端文字", "終端機文字", "Terminal text"]));
    let holders: Vec<&str> = MONO_LICENSE.lines().take_while(|l| l.starts_with("Copyright")).collect();
    let y = widgets::info_card(&mut c, t, f, y, &[
        (typeface, f.mono_info.family.clone()),
        (version, f.mono_info.version.clone()),
        (copyright, holders.join("\n")),
        (l.pick(LICENSE_TERMS), l.pick(["SIL 开源字体许可证 1.1 版（OFL-1.1）", "SIL 開放字型授權條款 1.1 版（OFL-1.1）", "SIL Open Font License, version 1.1 (OFL-1.1)"]).to_string()),
    ]);
    let mono = l.pick([
        "终端文字使用 Kirk Mono 等宽字体。该许可证允许自由使用、修改和再发布字体，但不允许单独出售字体。\n\n许可证全文位于 /skyward/KirkMono-OFL.txt，也可在 openfontlicense.org 查看。",
        "終端機文字使用 Kirk Mono 等寬字型。這份授權條款允許自由使用、修改和再散布字型，但不允許單獨出售字型。\n\n授權條款全文位於 /skyward/KirkMono-OFL.txt，也可在 openfontlicense.org 查看。",
        "The terminal text is set in the monospaced Kirk Mono. Its license lets the font be used, changed and passed on freely, but not sold on its own.\n\nThe license in full is at /skyward/KirkMono-OFL.txt, and on openfontlicense.org.",
    ]);
    let bottom = widgets::paragraph(&mut c, t, f, y + t.m.dp(4.0), mono);
    Frame::new(t, fit(c, t, bottom), Vec::new(), l.pick(FONTS), top)
}

const BOOT_SLOT: Text = ["启动槽位", "啟動槽位", "Boot slot"];

/// Which slot the phone starts from next, as the boot control holds it,
/// and what it holds of each; picking the other and swiping switches.
/// While an update is in flight the slot stays as the update set it.
pub fn slot(cx: &Ctx) -> Frame {
    let (t, f, s, i, l) = (cx.t, cx.f, cx.state, cx.info, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let title = l.pick(BOOT_SLOT);
    let Some(current) = i.slot else {
        widgets::list(&mut c, t, f, top, &[Row::new(l.pick(["本机没有 A/B 槽位", "本機沒有 A/B 槽位", "This phone has no A/B slots"])).trail(Trail::None)]);
        return Frame::new(t, c, out, title, top);
    };
    let slots = match &s.boot_slots {
        Some(Ok(slots)) => *slots,
        Some(Err(why)) => {
            let list = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(["无法读取启动槽位", "無法讀取啟動槽位", "Couldn't read the boot slots"])).trail(Trail::None)]);
            widgets::footnote(&mut c, t, f, list.bottom, why);
            return Frame::new(t, c, out, title, top);
        }
        None => return Frame::new(t, c, out, title, top),
    };
    let chosen = s.next_slot.unwrap_or(slots.active);
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["下次启动", "下次啟動", "Next boot"]));
    let titles = [Slot::A, Slot::B].map(|slot| {
        let slot = slot.letter();
        tr!(l, "槽位 {slot}", "槽位 {slot}", "Slot {slot}")
    });
    let subs = [Slot::A, Slot::B].map(|slot| {
        let k = slot.number() as usize;
        let which = l.pick(if slot == current { ["当前运行的系统", "目前執行的系統", "The system running now"] } else { ["另一个槽位", "另一個槽位", "The other slot"] });
        match (slots.bootable[k], slots.successful[k]) {
            (false, _) => tr!(l, "{which}，已标记为无法启动", "{which}，已標記為無法啟動", "{which}, marked unbootable"),
            (true, false) => tr!(l, "{which}，尚未完整启动过", "{which}，尚未完整啟動過", "{which}, not yet fully started"),
            (true, true) => which.to_string(),
        }
    });
    let rows: Vec<Row> =
        [Slot::A, Slot::B].iter().enumerate().map(|(k, &slot)| Row::new(&titles[k]).subtitle(&subs[k]).trail(Trail::Check(slot == chosen))).collect();
    let list = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&list.rows, &[Slot::A, Slot::B].map(Action::PickSlot)));
    if let Some(note) = fmt::snapshot_slot_note(l, &i.snapshot) {
        widgets::footnote(&mut c, t, f, list.bottom, note);
        return Frame::new(t, c, out, title, top);
    }
    let note = l.pick([
        "另一个槽位中没有完整的系统时，切换后将无法开机。",
        "另一個槽位中沒有完整的系統時，切換後將無法開機。",
        "If the other slot holds no whole system, the phone won't start after switching.",
    ]);
    let y = widgets::footnote(&mut c, t, f, list.bottom, note);
    let (label, danger) = confirm_label(Confirm::Slot, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::Slot) });
    Frame::new(t, c, out, title, top)
}

const MAIN_SYSTEM: Text = ["主系统", "主系統", "Main system"];
const SECOND_SYSTEM: Text = ["第二系统", "第二系統", "Second system"];

/// What the home page and the systems page say once the second system's
/// start stopped.
const SECOND_FAILED: Text = ["第二系统未能启动", "第二系統未能啟動", "The second system couldn't start"];

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
    let title = l.pick(DUAL_BOOT);
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
                    Some(name) => tr!(l, "正在切换到 {name}", "正在切換到 {name}", "Switching to {name}"),
                    None => l.pick(["正在切换系统", "正在切換系統", "Switching systems"]).to_string(),
                };
                (heading, l.pick(["正在写入启动镜像", "正在寫入啟動映像檔", "Writing its boot images"]))
            }
            SystemsWork::Removing => (
                l.pick(["正在删除第二系统", "正在刪除第二系統", "Removing the second system"]).to_string(),
                l.pick(["正在归还 data 分区中的空间", "正在歸還 data 分割區中的空間", "Giving its room back to the data partition"]),
            ),
        };
        let y = widgets::progress_card(&mut c, t, f, top, &heading, detail, None);
        let y = widgets::footnote(&mut c, t, f, y, l.pick(["完成前，请不要关机。", "完成前，請不要關機。", "Don't power off until it's done."]));
        return Frame::new(t, fit(c, t, y), out, title, top);
    }
    let systems = match &s.systems {
        Some(Ok(systems)) => systems,
        Some(Err(why)) => {
            let list = widgets::list(&mut c, t, f, top, &[Row::new(l.pick(["无法读取双系统", "無法讀取雙系統", "Couldn't read the systems"])).trail(Trail::None)]);
            let y = widgets::footnote(&mut c, t, f, list.bottom, why);
            return Frame::new(t, fit(c, t, y), out, title, top);
        }
        None => return Frame::new(t, fit(c, t, top), out, title, top),
    };
    let starts = starting(systems);
    let chosen = s.system_pick.unwrap_or(starts);
    let second_role = l.pick(SECOND_SYSTEM);
    let main = SystemCard {
        role: l.pick(MAIN_SYSTEM),
        name: &systems.installed.name,
        detail: systems.installed.version.as_deref().unwrap_or_default(),
        mark: SystemMark::Check(chosen == Who::Installed),
    };
    let second = match &systems.other {
        Some(other) => SystemCard { role: second_role, name: &other.name, detail: &other.version, mark: SystemMark::Check(chosen == Who::Other) },
        None => SystemCard {
            role: second_role,
            name: l.pick(["安装系统", "安裝系統", "Install a system"]),
            detail: l.pick(["选择第二系统的安装包", "選擇第二系統的安裝檔", "Choose the second system's package"]),
            mark: SystemMark::Opens,
        },
    };
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["下次启动", "下次啟動", "Next boot"]));
    let shown = widgets::system_cards(&mut c, t, f, y, &[main, second]);
    let second_opens = match systems.other {
        Some(_) => Action::PickSystem(Who::Other),
        None => Action::Open(Screen::Install),
    };
    out.extend(hits(&shown.rows, &[Action::PickSystem(Who::Installed), second_opens]));
    let mut y = shown.bottom + t.m.dp(12.0);
    if let Some(why) = &s.system_notice {
        y = widgets::info_card(&mut c, t, f, y, &[(l.pick(SECOND_FAILED), why.clone())]);
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
    let what = l.pick([
        "点按卡片选择系统。切换系统只改写下次启动槽位的 boot 和 init_boot 分区，主系统的其他分区保持不变。第二系统的文件保存在 data 分区中。",
        "點按卡片選擇系統。切換系統只改寫下次啟動槽位的 boot 和 init_boot 分割區，主系統的其他分割區保持不變。第二系統的檔案儲存在 data 分割區中。",
        "Tap a card to pick a system. Switching rewrites only the boot and init_boot partitions of the slot that starts next, and leaves the main system's other partitions as they are. The second system's files are kept in the data partition.",
    ]);
    y = widgets::footnote(&mut c, t, f, y, what);
    if systems.other.is_some() && systems.changed {
        let note = l.pick([
            "安装第二系统后，主系统的 vendor_boot 或 dtbo 分区已改变，第二系统可能无法正常启动。",
            "安裝第二系統後，主系統的 vendor_boot 或 dtbo 分割區已改變，第二系統可能無法正常啟動。",
            "The main system's vendor_boot or dtbo partition has changed since the second system was installed, so the second system may not start properly.",
        ]);
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if let Some(other) = &systems.other {
        y = widgets::section(&mut c, t, f, y, second_role);
        let size = fmt::bytes(other.size);
        let facts = widgets::list(&mut c, t, f, y, &[Row::new(l.pick(["占用空间", "佔用空間", "Space used"])).trail(Trail::Text(&size))]);
        let mounted = matches!(s.data, Data::Open | Data::Locked { .. });
        let remove = Row::new(l.pick(["删除第二系统", "刪除第二系統", "Remove the second system"])).tint(t.pal.danger);
        let (remove, act) = match mounted {
            true => (remove.trail(Trail::None), Action::AskDelete),
            false => (remove.subtitle(l.pick(DECRYPT_NEEDED)), Action::Open(Screen::Decrypt)),
        };
        let removing = widgets::list(&mut c, t, f, facts.bottom + t.m.card_gap, &[remove]);
        out.extend(hits(&removing.rows, &[act]));
        y = removing.bottom;
        if s.asking_delete && mounted {
            let ask = match starts {
                Who::Other => l.pick([
                    "将先切换回主系统，再删除第二系统及其全部数据，删除后无法恢复。",
                    "將先切換回主系統，再刪除第二系統及其全部資料，刪除後無法復原。",
                    "The phone goes back to the main system first, then the second system and all its data are removed for good.",
                ]),
                Who::Installed => l.pick([
                    "删除第二系统及其全部数据？删除后无法恢复。",
                    "刪除第二系統及其全部資料？刪除後無法復原。",
                    "Remove the second system and all its data? It can't be brought back.",
                ]),
            };
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
    let running = l.pick(if s.fastboot.running { ["正在运行", "正在執行", "Running"] } else { ["未运行", "未執行", "Not running"] });
    let link = l.pick(match s.fastboot.link {
        UsbLink::Connected => ["已连接电脑", "已連接電腦", "Computer connected"],
        UsbLink::Connecting => ["正在连接", "正在連線", "Connecting"],
        UsbLink::Unplugged => ["未连接电脑", "未連接電腦", "No computer"],
    });
    let slot = i.slot.map(|s| s.letter());
    let mut facts = vec![
        Row::new("fastbootd").trail(Trail::Text(running)),
        Row::new("USB").trail(Trail::Text(link)),
        Row::new(l.pick(["设备代号", "裝置代號", "Codename"])).trail(Trail::Text(&i.codename)),
    ];
    if let Some(letter) = slot {
        facts.push(Row::new(l.pick(CURRENT_SLOT)).trail(Trail::Text(letter)));
    }
    let y = widgets::section_first(&mut c, t, f, top, l.pick(["状态", "狀態", "Status"]));
    let state = widgets::list(&mut c, t, f, y, &facts);
    let note = l.pick([
        "你可以在电脑上使用 fastboot 命令刷写分区，包括 super 中的逻辑分区。",
        "你可以在電腦上使用 fastboot 指令刷寫分割區，包括 super 中的邏輯分割區。",
        "fastboot commands on the computer can flash partitions, the logical ones in super among them.",
    ]);
    let y = widgets::footnote(&mut c, t, f, state.bottom, note);
    let y = widgets::section(&mut c, t, f, y, l.pick(["退出 fastboot", "結束 fastboot", "Leave fastboot"]));
    let rows: Vec<Row> = FASTBOOT_CHOICES.iter().map(|&(title, _)| Row::new(l.pick(title)).trail(Trail::None)).collect();
    let choices = widgets::list(&mut c, t, f, y, &rows);
    let actions: Vec<Action> = FASTBOOT_CHOICES.iter().map(|&(_, action)| action).collect();
    let out = hits(&choices.rows, &actions);
    widgets::footnote(&mut c, t, f, choices.bottom, l.pick(["按音量键选择，按电源键确认。", "按音量鍵選擇，按電源鍵確認。", "Choose with the volume keys, and press the power key to confirm."]));
    Frame { back: false, ..Frame::new(t, c, out, "Fastboot", top) }
}

/// Where to go once what the system asked for has stopped or cannot be
/// done, in the order the pages list them.
const AFTERWARDS: [(Text, Action); 3] = [
    (["重启到系统", "重新啟動到系統", "Reboot to system"], Action::RebootTo(Target::System)),
    (["进入恢复模式", "進入復原模式", "Enter recovery"], Action::Dismiss),
    (["关机", "關機", "Power off"], Action::RebootTo(Target::PowerOff)),
];

/// What work the system asked for says while it goes on of how the phone
/// leaves once it is done.
const POWERS_OFF_WHEN_DONE: Text = ["完成后将自动关机。", "完成後將自動關機。", "Powers off on its own once done."];
const REBOOTS_WHEN_DONE: Text = ["完成后将自动重启。", "完成後將自動重新啟動。", "Reboots on its own once done."];

/// The list of [`AFTERWARDS`] at `y`, and where it ends. Work the user
/// chose in the recovery goes back to its home page, rather than into it.
fn afterwards(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, y: f32, asked: bool, out: &mut Vec<Hit>) -> f32 {
    let rows: Vec<Row> = AFTERWARDS
        .iter()
        .map(|&(title, action)| Row::new(l.pick(if action == Action::Dismiss && !asked { ["返回主页", "返回主頁", "Back to home"] } else { title })).trail(Trail::None))
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
    let wiped = ["已清除", "已清除", "Wiped"];
    let heading = l.pick(match &e.outcome {
        None => WIPING,
        Some(Ok(())) => wiped,
        Some(Err(_)) => WIPE_INCOMPLETE,
    });
    let mut y = widgets::section_first(&mut c, t, f, top, heading);
    if !e.parts.is_empty() {
        let states: Vec<String> = (0..e.parts.len())
            .map(|k| match (k.cmp(&e.done), failed) {
                (Ordering::Less, _) => l.pick(wiped).into(),
                (Ordering::Equal, false) => match e.progress {
                    Some((done, total)) if total > 0 => format!("{} {}%", l.pick(WIPING), percent(share(done, total))),
                    _ => l.pick(WIPING).into(),
                },
                (Ordering::Equal, true) if begun => l.pick(["未完成", "未完成", "Unfinished"]).into(),
                (Ordering::Greater, false) => l.pick(WAITING_TURN).into(),
                (Ordering::Equal | Ordering::Greater, true) => l.pick(["未清除", "未清除", "Not wiped"]).into(),
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
                true => ["本机的全部数据将被清除，完成后将自动关机。", "本機的全部資料將被清除，完成後將自動關機。", "All of this phone's data is being wiped, and it powers off on its own once done."],
                false => ["本机的全部数据将被清除，完成后将自动重启。", "本機的全部資料將被清除，完成後將自動重新啟動。", "All of this phone's data is being wiped, and it reboots on its own once done."],
            };
            vec![l.pick(leaving).into()]
        }
        (_, true, false) => vec![l.pick(if s.asked.shutdown_after { POWERS_OFF_WHEN_DONE } else { REBOOTS_WHEN_DONE }).into()],
        (None, false, _) => vec![l.pick(KEEP_POWER).into()],
        (Some(Ok(())), false, true) => {
            vec![l.pick(["下次开机时将重新创建 data 分区，启动时间较长。", "下次開機時將重新建立 data 分割區，啟動時間較長。", "The data partition is made anew at the next start, which then takes longer."]).into()]
        }
        (Some(Ok(())), false, false) => Vec::new(),
    };
    for note in &notes {
        y = widgets::footnote(&mut c, t, f, y, note);
    }
    if let (true, Some(reason)) = (e.asked, &s.asked.reason) {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(REASON), reason.clone())]);
    }
    if failed || (!e.asked && e.outcome.is_some()) {
        let below = widgets::section(&mut c, t, f, y, l.pick(ACTIONS));
        y = afterwards(&mut c, t, f, l, below, e.asked, &mut out);
    }
    let title = l.pick(match (e.reset, e.asked) {
        (true, _) => FACTORY_RESET,
        (false, true) => ["清除缓存", "清除快取", "Wipe cache"],
        (false, false) => ["清除数据", "清除資料", "Wipe data"],
    });
    Frame { back: false, ..Frame::new(t, fit(c, t, y), out, title, top) }
}

const FACTORY_RESET: Text = ["恢复出厂设置", "恢復原廠設定", "Factory reset"];

/// The system could not start Android and asks whether to erase all user
/// data: start it again, look around the recovery first, or erase.
pub fn cant_start(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let what = l.pick([
        "无法启动系统，本机数据可能已损坏。如果每次启动都停留在此页面，可恢复出厂设置，这将清除本机的全部数据。",
        "無法啟動系統，本機資料可能已損壞。如果每次啟動都停留在此頁面，可恢復原廠設定，這將清除本機的全部資料。",
        "The system couldn't start, and this phone's data may be damaged. If every start ends on this page, a factory reset may help, though it wipes all of this phone's data.",
    ]);
    let mut y = widgets::paragraph(&mut c, t, f, top, what);
    if let Some(reason) = &s.asked.reason {
        y = widgets::info_card(&mut c, t, f, y + t.m.dp(12.0), &[(l.pick(REASON), reason.clone())]);
    }
    y = widgets::section(&mut c, t, f, y, l.pick(ACTIONS));
    let rows = [
        Row::new(l.pick(["重试", "重試", "Try again"])).subtitle(l.pick(["重新启动系统", "重新啟動系統", "Starts the system again"])).trail(Trail::None),
        Row::new(l.pick(["进入恢复模式", "進入復原模式", "Enter recovery"])).subtitle(l.pick(["不清除任何内容", "不清除任何內容", "Wipes nothing"])).trail(Trail::None),
    ];
    let choices = widgets::list(&mut c, t, f, y, &rows);
    out.extend(hits(&choices.rows, &[Action::RebootTo(Target::System), Action::Dismiss]));
    let y = widgets::footnote(&mut c, t, f, choices.bottom, l.pick(["恢复出厂设置后，数据无法恢复。", "恢復原廠設定後，資料無法復原。", "Data a factory reset wipes can't be brought back."]));
    let (label, danger) = confirm_label(Confirm::FactoryReset, cx.state);
    let track = widgets::confirm(&mut c, t, f, y + t.m.dp(24.0), label, danger, s.confirm);
    out.push(Hit { rect: track, action: Action::Confirm(Confirm::FactoryReset) });
    Frame { back: false, ..Frame::new(t, c, out, l.pick(["无法启动系统", "無法啟動系統", "Can't start the system"]), top) }
}

/// Something the system asked of the recovery that it does not do: what
/// was asked, and where to go instead.
pub fn unhandled(cx: &Ctx) -> Frame {
    let (t, f, s, l) = (cx.t, cx.f, cx.state, cx.state.lang);
    let mut c = page(cx);
    let mut out = Vec::new();
    let top = t.m.content_top;
    let mut items = vec![(l.pick(["请求", "要求", "Request"]), s.asked.unhandled.clone().unwrap_or_default())];
    if let Some(reason) = &s.asked.reason {
        items.push((l.pick(REASON), reason.clone()));
    }
    let y = widgets::info_card(&mut c, t, f, top, &items);
    let y = widgets::footnote(&mut c, t, f, y, l.pick(["Skyward 暂不支持该请求，本机未做任何改动。", "Skyward 尚不支援這項要求，本機未做任何變更。", "Skyward doesn't support this request, and changed nothing on this phone."]));
    let y = widgets::section(&mut c, t, f, y, l.pick(ACTIONS));
    afterwards(&mut c, t, f, l, y, true, &mut out);
    Frame { back: false, ..Frame::new(t, c, out, l.pick(["无法处理该请求", "無法處理這項要求", "Can't handle the request"]), top) }
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
        r.outcome = Some(BackupEnd::Refused(BackupRefusal::DataReset));
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

    #[test]
    fn a_phrase_that_opens_a_line_starts_with_a_capital_in_english() {
        assert_eq!(capitalized(Lang::En, "empties the data partition"), "Empties the data partition");
        assert_eq!(capitalized(Lang::Hans, "清空 data 分区"), "清空 data 分区");
    }
}
