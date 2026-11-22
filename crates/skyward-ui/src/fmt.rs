use crate::app::{
    BackupRefusal, Data, EraseStop, FileKind, FileRow, ItemEnd, MountFailure, Part, PartKind, PartStop, Place, Source, TermEnd, BACKUP_PARTS,
    TIME_ZONES,
};
use crate::lang::{Lang, Text};
use crate::texts;
use jiff::civil::DateTime;
pub use jiff::tz::TimeZone;
use jiff::Timestamp;
use skyward_info::{Battery, Charge, Cpu, PatchLevel, Storage};
use skyward_install::ota::Trouble;
use skyward_install::{Failure, ImageKind, Place as FlashPlace, Ramdisk, Refusal, Target as FlashTarget};
use skyward_net::download::Failure as DownloadFailure;
use skyward_net::reach::Reach;
use skyward_net::release::Unanswered;
use skyward_net::supplicant::Security;
use skyward_net::wlan::{Bands, Joined, Power, Stage as WlanStage, Wlan};
use skyward_vab::{Hold, Slot, SnapshotState, Standing};
use std::sync::OnceLock;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Marketed RAM size: the smallest standard module size that holds `MemTotal`.
pub fn ram(kib: u64) -> String {
    const SIZES: [f64; 16] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 18.0, 20.0, 24.0, 32.0, 48.0, 64.0];
    let gib = kib as f64 / (1024.0 * 1024.0);
    let size = SIZES.iter().copied().find(|&s| s >= gib).unwrap_or(gib.ceil());
    format!("{size:.1} GB")
}

/// Marketed storage size in GB: the smallest power of two (in GiB) that holds
/// the userdata partition.
pub fn capacity_gb(total: u64) -> u64 {
    let gib = total as f64 / GIB;
    let mut gb = 8;
    while (gb as f64) < gib {
        gb *= 2;
    }
    gb
}

pub fn capacity(total: u64) -> String {
    match capacity_gb(total) {
        gb if gb >= 1024 => format!("{}TB", gb / 1024),
        gb => format!("{gb}GB"),
    }
}

pub fn gb(bytes: u64) -> String {
    format!("{:.1}GB", bytes as f64 / 1e9)
}

/// `已用/总量` as the stock settings count it: everything that is not free
/// is used, against the marketed size.
pub fn storage_used(s: &Storage) -> Option<String> {
    let free = s.free?;
    let used = capacity_gb(s.total) as f64 - free as f64 / 1e9;
    Some(format!("{used:.1}GB/{}", capacity(s.total)))
}

/// A count as the language counts cores: in Chinese numerals where it is
/// written in Chinese characters, a pair as 双, or 雙 in traditional ones,
/// and in figures in every other language.
fn numeral(l: Lang, n: u32) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match (l, n) {
        (Lang::Hans, 2) => "双".into(),
        (Lang::Hant, 2) => "雙".into(),
        (Lang::Hans | Lang::Hant, 0..=9) => D[n as usize].into(),
        (Lang::Hans | Lang::Hant, 10) => "十".into(),
        (Lang::Hans | Lang::Hant, 11..=19) => format!("十{}", D[(n % 10) as usize]),
        (Lang::Hans | Lang::Hant, 20..=99) if n.is_multiple_of(10) => format!("{}十", D[(n / 10) as usize]),
        (Lang::Hans | Lang::Hant, 20..=99) => format!("{}十{}", D[(n / 10) as usize], D[(n % 10) as usize]),
        _ => n.to_string(),
    }
}

/// `八核 最高 3.19GHz`, `Octa-core, up to 3.19GHz`
pub fn cpu(l: Lang, c: &Cpu) -> String {
    let cores = match c.cores {
        1 => l.pick(texts::specs::cpu::CORES_1).to_string(),
        2 => l.pick(texts::specs::cpu::CORES_2).to_string(),
        4 => l.pick(texts::specs::cpu::CORES_4).to_string(),
        6 => l.pick(texts::specs::cpu::CORES_6).to_string(),
        8 => l.pick(texts::specs::cpu::CORES_8).to_string(),
        n => texts::specs::cpu::cores(l, numeral(l, n)),
    };
    match c.max_khz {
        Some(khz) => texts::specs::cpu::up_to(l, &cores, format!("{:.2}", khz as f64 / 1e6)),
        None => cores,
    }
}

/// A marketing name laid out the way the stock settings stack it in large
/// type: an edition credit ("by Leica") starts its own line.
pub fn model_lines(name: &str) -> String {
    name.replacen(" by ", "\nby ", 1)
}

pub fn panel((w, h): (u32, u32)) -> String {
    format!("{} x {}", w.max(h), w.min(h))
}

pub fn inches(d: f32) -> String {
    format!("{d}″")
}

pub fn mah(uah: u64) -> String {
    format!("{}mAh", uah / 1000)
}

pub fn charge(l: Lang, c: Charge) -> &'static str {
    l.pick(match c {
        Charge::Charging => texts::battery::CHARGING,
        Charge::Discharging | Charge::NotCharging => texts::battery::NOT_CHARGING,
        Charge::Full => texts::battery::FULL,
        Charge::Unknown => texts::battery::UNKNOWN,
    })
}

/// `74% · 充电中`
pub fn level(l: Lang, b: &Battery) -> String {
    format!("{}% · {}", b.level, charge(l, b.charge))
}

pub fn celsius(tenths: i32) -> String {
    format!("{}°C", degrees(tenths))
}

/// A temperature given in tenths of a degree, to the tenth: `35.2`.
pub fn degrees(tenths: i32) -> String {
    format!("{:.1}", tenths as f64 / 10.0)
}

pub fn volts(uv: u64) -> String {
    format!("{:.2}V", uv as f64 / 1e6)
}

/// The kernel's battery health in the language; a state it does not list
/// shows as reported.
pub fn health(l: Lang, h: &str) -> String {
    let text = match h {
        "Good" => texts::battery::health::GOOD,
        "Overheat" | "Hot" => texts::battery::health::OVERHEATED,
        "Warm" => texts::battery::health::WARM,
        "Cool" => texts::battery::health::COOL,
        "Cold" => texts::battery::health::COLD,
        "Dead" => texts::battery::health::DEAD,
        "Over voltage" => texts::battery::health::OVER_VOLTAGE,
        "Over current" => texts::battery::health::OVER_CURRENT,
        "Unspecified failure" => texts::battery::health::FAILURE,
        "Calibration required" => texts::battery::health::NEEDS_CALIBRATION,
        "No battery" => texts::battery::health::NO_BATTERY,
        "Unknown" => texts::battery::health::UNKNOWN,
        other => return other.to_string(),
    };
    l.pick(text).to_string()
}

/// Health and remaining capacity against design, e.g. `良好 · 剩余容量 93%`.
pub fn wear(l: Lang, b: &Battery) -> Option<String> {
    let state = b.health.as_deref().map(|h| health(l, h));
    let left = match (b.full, b.design) {
        (Some(full), Some(design)) if design > 0 => {
            Some(texts::battery::capacity_left(l, (full * 100 + design / 2) / design))
        }
        _ => None,
    };
    match (state, left) {
        (Some(s), Some(l)) => Some(format!("{s} · {l}")),
        (s, l) => s.or(l),
    }
}

pub fn android(l: Lang, release: &str, patch_level: Option<&PatchLevel>) -> String {
    match patch_level {
        Some(p) => texts::android::security_update(l, release, patch(l, p)),
        None => release.to_string(),
    }
}

pub fn verified(l: Lang, state: &str) -> String {
    let text = match state {
        "green" => texts::verified_boot::GREEN,
        "yellow" => texts::verified_boot::YELLOW,
        "orange" => texts::verified_boot::ORANGE,
        "red" => texts::verified_boot::RED,
        other => return other.to_string(),
    };
    l.pick(text).to_string()
}

pub fn snapshot(l: Lang, s: &SnapshotState) -> &'static str {
    l.pick(match s {
        SnapshotState::None => texts::snapshot::NONE,
        SnapshotState::Snapshotted => texts::snapshot::WRITTEN,
        SnapshotState::Merging => texts::snapshot::MERGING,
        SnapshotState::MergeFailed => texts::snapshot::FAILED,
        SnapshotState::Unknown => texts::snapshot::UNREADABLE,
    })
}

/// Why super is off limits while an update is in flight; `None` when it is not.
pub fn snapshot_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => texts::snapshot::flash::UNTIL_MERGED,
        SnapshotState::MergeFailed => texts::snapshot::flash::UNTIL_DEALT_WITH,
        SnapshotState::Unknown => texts::snapshot::flash::UNTIL_READ,
    };
    Some(l.pick(text))
}

/// The home page's warning of a battery too low to install on: how low it
/// is, and what installing waits for.
pub fn battery_low(l: Lang, level: u8, charging: bool) -> (String, String) {
    (texts::battery::low(l, level), texts::battery::charge_to_install(l, skyward_install::least_charge(charging)))
}

/// The home page's word that the phone restarts from the slot it does not
/// run from now.
pub fn next_boot(l: Lang, next: Slot, now: Slot) -> (String, String) {
    (texts::slot::restarts_from(l, next.letter()), texts::slot::running_from(l, now.letter()))
}

/// Why the slot to start next stays as an update in flight set it; `None`
/// when none is.
pub fn snapshot_slot_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => texts::snapshot::slot::UNTIL_MERGED,
        SnapshotState::MergeFailed => texts::snapshot::slot::UNTIL_DEALT_WITH,
        SnapshotState::Unknown => texts::snapshot::slot::UNTIL_READ,
    };
    Some(l.pick(text))
}

/// What the recovery says as it starts, of a switch between the systems
/// that a restart or a loss of power cut off: finished, or why not.
pub fn switch_resumed(l: Lang) -> &'static str {
    l.pick(texts::systems::switch::RESUMED)
}

pub fn switch_unfinished(l: Lang, why: &str) -> String {
    texts::systems::switch::unfinished(l, why)
}

/// Why the second system does not start yet while an update is in flight,
/// which the main system has to start once more to merge; `None` when none
/// is.
pub fn snapshot_system_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => texts::snapshot::second_system::UNTIL_MERGED,
        SnapshotState::MergeFailed => texts::snapshot::second_system::UNTIL_DEALT_WITH,
        SnapshotState::Unknown => texts::snapshot::second_system::UNTIL_READ,
    };
    Some(l.pick(text))
}

/// What an install script that writes to super does to an update in
/// flight; `None` when none is known to be.
pub fn snapshot_script_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None | SnapshotState::Unknown => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => texts::snapshot::script::UNMERGED,
        SnapshotState::MergeFailed => texts::snapshot::script::FAILED,
    };
    Some(l.pick(text))
}

/// Why the slot's partitions in super are not backed up while an update is
/// in flight; `None` when none is.
pub fn snapshot_backup_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => texts::snapshot::backup::UNTIL_MERGED,
        SnapshotState::MergeFailed => texts::snapshot::backup::UNTIL_DEALT_WITH,
        SnapshotState::Unknown => texts::snapshot::backup::UNTIL_READ,
    };
    Some(l.pick(text))
}

/// Why backing up, or restoring, did not start.
pub fn backup_refusal(l: Lang, r: &BackupRefusal, restore: bool) -> String {
    use texts::backup::refused;
    match r {
        BackupRefusal::Room { needed, free } => refused::room(l, gb(*needed), gb(*free)),
        BackupRefusal::Update(state) => l
            .pick(match (state, restore) {
                (SnapshotState::Snapshotted | SnapshotState::Merging, false) => refused::BACKUP_UNTIL_MERGED,
                (SnapshotState::Snapshotted | SnapshotState::Merging, true) => refused::RESTORE_UNTIL_MERGED,
                (SnapshotState::MergeFailed, false) => refused::BACKUP_UNTIL_DEALT_WITH,
                (SnapshotState::MergeFailed, true) => refused::RESTORE_UNTIL_DEALT_WITH,
                (SnapshotState::Unknown, false) => refused::BACKUP_UNTIL_READ,
                (SnapshotState::Unknown, true) => refused::RESTORE_UNTIL_READ,
                (SnapshotState::None, _) => unreachable!("no update in flight holds anything back"),
            })
            .into(),
        BackupRefusal::OtherPhone(device) => refused::other_phone(l, device),
        BackupRefusal::Resized(name) => refused::resized(l, name),
        BackupRefusal::DataReset => l.pick(refused::DATA_RESET).into(),
    }
}

/// What the update page says of an update standing so, and the line under
/// it on what comes next; `next` is the slot the bootloader starts next,
/// where the boot control could tell.
pub fn update(l: Lang, s: &Standing, next: Option<Slot>) -> (&'static str, Option<String>) {
    use texts::update::standing;
    let line = |text: Text| Some(l.pick(text).to_string());
    match s {
        Standing::None => (l.pick(texts::snapshot::NONE), None),
        Standing::Unknown => (l.pick(texts::snapshot::UNREADABLE), line(standing::RECORDS_UNREAD)),
        Standing::Writing => (l.pick(standing::NOT_WRITTEN), line(standing::GOES_ON_IN_SYSTEM)),
        Standing::Installed(installed) => {
            let slot = installed.letter();
            let detail = match next {
                Some(next) if next == *installed => standing::starts_and_merges(l, slot),
                Some(_) => standing::reboot_from_system(l, slot),
                None => standing::merges_once_started(l, slot),
            };
            (l.pick(standing::INSTALLED), Some(detail))
        }
        Standing::Waiting => (l.pick(standing::WAITING), line(standing::MERGES_OR_HERE)),
        Standing::Compressed => (l.pick(standing::WAITING), line(standing::MERGES_IN_SYSTEM)),
        Standing::Merging(_) => (l.pick(texts::snapshot::MERGING), None),
        Standing::Failed(code) => (l.pick(texts::snapshot::FAILED), Some(standing::goes_on_after_reboot(l, format!("{code:?}")))),
        Standing::Merged => (l.pick(standing::MERGED), line(standing::CLEARED_NEXT_START)),
    }
}

/// Where an update standing so is, beside the entry of its page.
pub fn update_value(l: Lang, s: &Standing) -> &'static str {
    l.pick(match s {
        Standing::None => texts::update::value::NONE,
        Standing::Unknown => texts::common::UNREADABLE,
        Standing::Writing => texts::update::value::NOT_WRITTEN,
        Standing::Installed(_) => texts::update::value::INSTALLED,
        Standing::Waiting | Standing::Compressed => texts::update::value::WAITING,
        Standing::Merging(_) => texts::update::value::MERGING,
        Standing::Failed(_) => texts::update::value::FAILED,
        Standing::Merged => texts::update::value::MERGED,
    })
}

pub fn data(l: Lang, d: &Data) -> &'static str {
    l.pick(match d {
        Data::Closed | Data::Opening | Data::Failed(_) => texts::data::NOT_MOUNTED,
        Data::Locked { .. } => texts::data::LOCKED,
        Data::Open => texts::data::DECRYPTED,
        Data::Erased => texts::data::WIPED,
    })
}

/// A part of the phone erasing goes through, as its page names it.
pub fn part(l: Lang, p: Part) -> &'static str {
    l.pick(match p {
        Part::Data => texts::wipe::part::DATA,
        Part::Cache => texts::wipe::part::CACHE,
        Part::Metadata => texts::wipe::part::METADATA,
        Part::AppData => texts::wipe::part::APP_DATA,
        Part::Media => texts::wipe::part::MEDIA,
        Part::AppCaches => texts::wipe::part::APP_CACHES,
        Part::SystemCaches => texts::wipe::part::SYSTEM_CACHES,
        Part::Art => texts::wipe::part::ART,
        Part::Logs => texts::wipe::part::LOGS,
        Part::Updates => texts::wipe::part::UPDATES,
    })
}

/// `name` as a sentence written in Chinese characters carries it ahead of
/// them: with a space after the Latin letters or digits it ends in, as the
/// pages write 用户 999 的数据. Every other language writes its own spaces.
fn before_characters(l: Lang, name: &str) -> String {
    let apart = matches!(l, Lang::Hans | Lang::Hant) && name.ends_with(|c: char| c.is_ascii());
    format!("{name}{}", if apart { " " } else { "" })
}

/// A name that opens a title, as it reads within a sentence of the
/// language: its first letter small, unless the word is written all in
/// capitals.
fn in_sentence(l: Lang, name: &str) -> String {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && !second.is_uppercase() => {
            let (first, rest) = name.split_at(first.len_utf8());
            format!("{}{rest}", l.lowercase(first))
        }
        _ => name.to_string(),
    }
}

/// Things named in a row, with what the language sets between them, and
/// each after the first as a sentence names it where the language writes
/// a row so.
pub fn list<'a>(l: Lang, names: impl IntoIterator<Item = &'a str>) -> String {
    let small = l.pick(texts::list::LOWERCASE_AFTER_FIRST);
    let names: Vec<String> = names.into_iter().enumerate().map(|(k, n)| if small && k > 0 { in_sentence(l, n) } else { n.to_string() }).collect();
    names.join(l.pick(texts::list::SEPARATOR))
}

/// Users by how the pages call them: the owner, then the others by number,
/// as「机主、用户 999」, or as an English sentence names them, as "the owner
/// and user 999".
pub fn users(l: Lang, users: &[u32]) -> String {
    let names: Vec<String> = users
        .iter()
        .map(|&u| match u {
            0 => l.pick(texts::users::OWNER).to_string(),
            u => texts::users::user(l, u),
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [a, b] => texts::list::two(l, a, b),
        [rest @ .., last] => texts::list::end(l, rest.join(l.pick(texts::list::SEPARATOR)), last),
    }
}

/// Why erasing did not start or stopped short, and what the system said of
/// it when there is that.
pub fn erase_stop(l: Lang, s: &EraseStop) -> (String, Option<String>) {
    use texts::wipe::{failed, stopped};
    let said = |text: Text| l.pick(text).to_string();
    match s {
        EraseStop::Update(Hold::Unconfirmed) => (said(stopped::UPDATE_UNFINISHED), None),
        EraseStop::Update(Hold::Merging) => (said(stopped::UPDATE_MERGING), None),
        EraseStop::Update(Hold::MergeFailed) => (said(stopped::UPDATE_FAILED), None),
        EraseStop::NotRebuilt => (said(stopped::NOT_REBUILT), None),
        EraseStop::NoCache => (said(stopped::NO_CACHE), None),
        EraseStop::Erased => (said(stopped::ERASED), None),
        EraseStop::Closed(why) => (said(stopped::CLOSED), Some(why.clone())),
        EraseStop::Locked(locked) => (stopped::locked(l, before_characters(l, &users(l, locked))), None),
        EraseStop::Failed(p, why) => {
            let what = said(match p {
                Part::Data => failed::DATA,
                Part::Cache => failed::CACHE,
                Part::Metadata => failed::METADATA,
                Part::AppData => failed::APP_DATA,
                Part::Media => failed::MEDIA,
                Part::AppCaches => failed::APP_CACHES,
                Part::SystemCaches => failed::SYSTEM_CACHES,
                Part::Art => failed::ART,
                Part::Logs => failed::LOGS,
                Part::Updates => failed::UPDATES,
            });
            (what, Some(why.clone()))
        }
    }
}

/// Work on whole partitions of a kind, which each language tells of in
/// its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Wipe,
    Unshare,
    Repair,
    Format,
}

impl Verb {
    pub fn of(kind: &PartKind) -> Verb {
        match kind {
            PartKind::Disable => Verb::Wipe,
            PartKind::Unshare => Verb::Unshare,
            PartKind::Repair => Verb::Repair,
            PartKind::Remake(_) => Verb::Format,
        }
    }

    /// Under way: 正在修复, Repairing.
    pub fn going(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::GOING,
            Verb::Unshare => texts::parts::unshare::GOING,
            Verb::Repair => texts::parts::repair::GOING,
            Verb::Format => texts::parts::format::GOING,
        })
        .into()
    }

    /// Ended, with every partition done: 修复完成, Repair complete.
    pub fn complete(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::COMPLETE,
            Verb::Unshare => texts::parts::unshare::COMPLETE,
            Verb::Repair => texts::parts::repair::COMPLETE,
            Verb::Format => texts::parts::format::COMPLETE,
        })
        .into()
    }

    /// Ended partway through the partitions: 清除未完成, Wipe incomplete.
    pub fn incomplete(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::INCOMPLETE,
            Verb::Unshare => texts::parts::unshare::INCOMPLETE,
            Verb::Repair => texts::parts::repair::INCOMPLETE,
            Verb::Format => texts::parts::format::INCOMPLETE,
        })
        .into()
    }

    /// Gone wrong: 修复失败, Repair failed.
    pub fn failed(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::FAILED,
            Verb::Unshare => texts::parts::unshare::FAILED,
            Verb::Repair => texts::parts::repair::FAILED,
            Verb::Format => texts::parts::format::FAILED,
        })
        .into()
    }

    /// A partition the work ended before: 未修复, Not repaired.
    pub fn not_done(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::NOT_DONE,
            Verb::Unshare => texts::parts::unshare::NOT_DONE,
            Verb::Repair => texts::parts::repair::NOT_DONE,
            Verb::Format => texts::parts::format::NOT_DONE,
        })
        .into()
    }

    /// What the page asks while the work goes on: 修复完成前，请不要关机。
    pub fn hold_on(self, l: Lang) -> String {
        l.pick(match self {
            Verb::Wipe => texts::parts::wipe::HOLD_ON,
            Verb::Unshare => texts::parts::unshare::HOLD_ON,
            Verb::Repair => texts::parts::repair::HOLD_ON,
            Verb::Format => texts::parts::format::HOLD_ON,
        })
        .into()
    }

    /// What could not be done to the partition `name`: 修复 vendor 失败,
    /// Couldn't repair vendor.
    pub fn failed_on(self, l: Lang, name: &str) -> String {
        match self {
            Verb::Wipe => texts::parts::wipe::failed_on(l, name),
            Verb::Unshare => texts::parts::unshare::failed_on(l, name),
            Verb::Repair => texts::parts::repair::failed_on(l, name),
            Verb::Format => texts::parts::format::failed_on(l, name),
        }
    }
}

/// How a partition's work ended, as its row says it, `verb` telling of the
/// work: 已修复, 修复失败.
pub fn item_end(l: Lang, end: &ItemEnd, verb: Verb) -> String {
    let text = match end {
        ItemEnd::Erased => texts::parts::ended::WIPED,
        ItemEnd::Unshared => texts::parts::ended::UNSHARED,
        ItemEnd::NotShared(_) => texts::parts::ended::NOTHING_TO_DO,
        ItemEnd::Clean => texts::parts::ended::CLEAN,
        ItemEnd::Fixed => texts::parts::ended::REPAIRED,
        ItemEnd::Made => texts::parts::ended::FORMATTED,
        ItemEnd::Failed(_) => return verb.failed(l),
    };
    l.pick(text).to_string()
}

/// Why work on whole partitions stopped before going through them, on a
/// phone whose codename is `codename`, and the details behind it when
/// there are any.
pub fn part_stop(l: Lang, stop: &PartStop, codename: &str) -> (String, Option<String>) {
    match stop {
        PartStop::Refused(r) => (erase_request_refusal(l, r, codename), None),
        PartStop::Unmounted(base) => (texts::mount::first_then_retry(l, base), None),
        PartStop::Failed(why) => (l.pick(texts::common::UNEXPECTED).into(), Some(why.clone())),
    }
}

/// A file size, `7.1GB` or `96.0MB`.
pub fn bytes(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}GB", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}MB", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}KB", n as f64 / 1e3),
        n => format!("{n}B"),
    }
}

/// The months, as each language names them in a date.
const MONTHS: [Text; 12] = [
    texts::date::month::JANUARY,
    texts::date::month::FEBRUARY,
    texts::date::month::MARCH,
    texts::date::month::APRIL,
    texts::date::month::MAY,
    texts::date::month::JUNE,
    texts::date::month::JULY,
    texts::date::month::AUGUST,
    texts::date::month::SEPTEMBER,
    texts::date::month::OCTOBER,
    texts::date::month::NOVEMBER,
    texts::date::month::DECEMBER,
];

/// A day of the calendar as the language writes it: `2026年5月8日`,
/// `May 8, 2026`.
fn day(l: Lang, year: i32, month: i32, d: i32) -> String {
    texts::date::day(l, l.pick(MONTHS[(month - 1) as usize]), d, year)
}

/// `2026年4月1日`
pub fn patch(l: Lang, p: &PatchLevel) -> String {
    day(l, p.year.into(), p.month.into(), p.day.into())
}

/// The rules of each of [`TIME_ZONES`], read once from the ones the
/// recovery carries.
pub fn rules(zone: usize) -> &'static TimeZone {
    static RULES: OnceLock<Vec<TimeZone>> = OnceLock::new();
    let all = RULES.get_or_init(|| {
        TIME_ZONES.iter().map(|zone| TimeZone::get(zone.tz).expect("the recovery carries the rules of every zone listed")).collect()
    });
    &all[zone]
}

/// A Unix time as the calendar keeps it. A time past either end of the
/// calendar, as a damaged file's can be, is taken as that end.
fn instant(unix: i64) -> Timestamp {
    let unix = unix.clamp(Timestamp::MIN.as_second(), Timestamp::MAX.as_second());
    Timestamp::from_second(unix).expect("a time within the calendar")
}

/// A Unix time on the calendar and clock of `zone`.
fn local(unix: i64, zone: &TimeZone) -> DateTime {
    instant(unix).to_zoned(zone.clone()).datetime()
}

/// How far ahead of UTC the clocks of `zone` are at a Unix time, in
/// seconds: less than none to the west of it.
pub fn utc_offset_seconds(unix: i64, zone: &TimeZone) -> i32 {
    zone.to_offset(instant(unix)).seconds()
}

/// How far from UTC the clocks of `zone` are at a Unix time, as a list of
/// zones tells it: `UTC+08:00`, `UTC−03:30`.
pub fn utc_offset(unix: i64, zone: &TimeZone) -> String {
    let seconds = utc_offset_seconds(unix, zone);
    let minutes = seconds.unsigned_abs() / 60;
    format!("UTC{}{:02}:{:02}", if seconds < 0 { '−' } else { '+' }, minutes / 60, minutes % 60)
}

/// The day a Unix time falls on in `zone`, `2026年5月8日`.
pub fn date(l: Lang, unix: i64, zone: &TimeZone) -> String {
    let d = local(unix, zone);
    day(l, d.year().into(), d.month().into(), d.day().into())
}

/// When a list entry last changed, as the stock file manager tells it
/// against `now`, when the folder was read: the time for today, month and
/// day for a file from this year, and the whole date for a folder from this
/// year or anything older; all in `zone`.
pub fn file_time(unix: i64, folder: bool, now: i64, zone: &TimeZone) -> String {
    let (day, today) = (local(unix, zone).date(), local(now, zone).date());
    if day == today {
        clock(unix, zone)
    } else if day.year() == today.year() && !folder {
        format!("{:02}/{:02}", day.month(), day.day())
    } else {
        format!("{}/{:02}/{:02}", day.year(), day.month(), day.day())
    }
}

/// A file's size as the stock file manager writes it: in steps of 1000,
/// going up a unit past 900, with at most two decimals and no trailing
/// zeros.
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let (mut v, mut unit) = (bytes as f64, 0);
    while v > 900.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    let hundredths = (v * 100.0).round() as u64;
    let n = match hundredths {
        h if h % 100 == 0 => format!("{v:.0}"),
        h if h % 10 == 0 => format!("{v:.1}"),
        _ => format!("{v:.2}"),
    };
    format!("{n} {}", UNITS[unit])
}

/// A Unix time in `zone`, to the minute: `2026年10月19日 02:31`,
/// `October 19, 2026, 02:31`.
pub fn datetime(l: Lang, unix: i64, zone: &TimeZone) -> String {
    texts::date::at(l, date(l, unix, zone), clock(unix, zone))
}

/// The time of day in `zone`, `HH:MM`, as the status bar shows it.
pub fn clock(unix: i64, zone: &TimeZone) -> String {
    let t = local(unix, zone);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// A Unix time in `zone`, to the second, as the name of what is saved
/// then, which sorts by it: `20261021-133000`.
pub fn stamp(unix: i64, zone: &TimeZone) -> String {
    let t = local(unix, zone);
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second())
}

/// A file's type and permissions as `ls -l` prints them: `drwxr-xr-x`.
pub fn mode(mode: u32) -> String {
    let kind = match mode & 0o170_000 {
        0o040_000 => 'd',
        0o120_000 => 'l',
        0o020_000 => 'c',
        0o060_000 => 'b',
        0o010_000 => 'p',
        0o140_000 => 's',
        _ => '-',
    };
    let mut out = String::from(kind);
    // The set-user, set-group and sticky bits show in each class's x.
    for (shift, special, set, unset) in [(6, 0o4000, 's', 'S'), (3, 0o2000, 's', 'S'), (0, 0o1000, 't', 'T')] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(match (bits & 1 != 0, mode & special != 0) {
            (true, true) => set,
            (false, true) => unset,
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    out
}

/// The line under an entry's name in the file list, laid out as the stock
/// file manager lays it out, with dates told against `now`, when the folder
/// was read, in `zone`.
pub fn file_line(l: Lang, f: &FileRow, now: i64, zone: &TimeZone) -> String {
    const DIVIDER: &str = "  |  ";
    match &f.kind {
        FileKind::Folder { items: Some(n) } => {
            format!("{}{DIVIDER}{}", file_time(f.modified, true, now, zone), texts::files::line::items(l, *n as u64))
        }
        FileKind::Folder { items: None } => l.pick(texts::common::UNREADABLE).to_string(),
        FileKind::File => format!("{}{DIVIDER}{}", file_time(f.modified, false, now, zone), file_size(f.size)),
        FileKind::Link { target, .. } => texts::files::line::link_to(l, target),
        FileKind::Special => {
            let what = l.pick(match f.mode & 0o170_000 {
                0o020_000 => texts::files::line::CHAR_DEVICE,
                0o060_000 => texts::files::line::BLOCK_DEVICE,
                0o010_000 => texts::files::line::PIPE,
                _ => texts::files::line::SOCKET,
            });
            format!("{}{DIVIDER}{what}", file_time(f.modified, false, now, zone))
        }
    }
}

/// The parts a backup holds: `启动分区、系统分区`.
pub fn parts(l: Lang, held: &[bool; 4]) -> String {
    list(l, BACKUP_PARTS.iter().zip(held).filter(|&(_, &on)| on).map(|(name, _)| l.pick(*name)))
}

/// What an image is, and what it holds.
pub fn image_kind(l: Lang, kind: ImageKind) -> (&'static str, String) {
    use texts::image::{self, holds};
    let (name, holds) = match kind {
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery } => (image::RECOVERY, holds::KERNEL_AND_RECOVERY_RAMDISK),
        ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery } => (image::RECOVERY, holds::RECOVERY_RAMDISK),
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Generic } => (image::BOOT, holds::KERNEL_AND_GENERIC_RAMDISK),
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::None } => (image::BOOT, holds::KERNEL),
        ImageKind::Boot { kernel: false, .. } => (image::BOOT, holds::GENERIC_RAMDISK),
        ImageKind::VendorBoot => (image::VENDOR_BOOT, holds::VENDOR_RAMDISK),
        ImageKind::Dtbo => (image::DTBO, holds::OVERLAYS),
        ImageKind::Vbmeta => (image::VBMETA, holds::VERIFIED_BOOT_DATA),
        ImageKind::Super => (image::WHOLE_SUPER, holds::LOGICAL_PARTITIONS),
        ImageKind::Filesystem(filesystem) => return (l.pick(image::PARTITION), holds::filesystem(l, filesystem.name())),
        ImageKind::Other => (image::PARTITION, holds::UNIDENTIFIED),
    };
    (l.pick(name), l.pick(holds).into())
}

/// What a partition an image can go to is for.
pub fn partition_role(l: Lang, partition: &FlashTarget) -> &'static str {
    use texts::image::role;
    l.pick(match (partition.place, partition.name.as_str()) {
        (FlashPlace::Logical, _) => role::LOGICAL,
        (FlashPlace::Single, _) => role::WHOLE_SUPER,
        (FlashPlace::Slotted, "boot") => role::BOOT,
        (FlashPlace::Slotted, "init_boot") => role::INIT_BOOT,
        (FlashPlace::Slotted, "recovery") => role::RECOVERY,
        (FlashPlace::Slotted, "vendor_boot") => role::VENDOR_BOOT,
        (FlashPlace::Slotted, "dtbo") => role::DTBO,
        (FlashPlace::Slotted, "vbmeta") => role::VBMETA,
        (FlashPlace::Slotted, "vbmeta_system") => role::VBMETA_SYSTEM,
        (FlashPlace::Slotted, "vbmeta_vendor") => role::VBMETA_VENDOR,
        (FlashPlace::Slotted, _) => role::PARTITION,
    })
}

/// How long the screen stays on with nothing touched: `30 秒`, `2 分钟`,
/// or `永不`.
pub fn screen_off(l: Lang, after_ms: Option<u64>) -> String {
    match after_ms {
        None => l.pick(texts::screen_off::NEVER).into(),
        Some(ms) if ms < 60_000 => texts::screen_off::seconds(l, ms / 1000),
        Some(ms) => texts::screen_off::minutes(l, ms / 60_000),
    }
}

/// A mount or unmount that failed, with why: `挂载 vendor 失败：…`
pub fn mount_failure(l: Lang, m: &MountFailure) -> String {
    match m.mounting {
        true => texts::mount::failed(l, &m.base, &m.detail),
        false => texts::mount::unmount_failed(l, &m.base, &m.detail),
    }
}

/// Whether another package from where this one came from is sent, as the
/// computer sends one, and not chosen, as a file is in place of one chosen
/// before or of a package the system named.
fn sent(source: Source) -> bool {
    match source {
        Source::Computer | Source::Rescue => true,
        Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => false,
    }
}

/// Why a package from `source` was not installed, for a phone with this
/// codename, its dates in `zone`.
pub fn refusal(l: Lang, r: &Refusal, codename: &str, source: Source, zone: &TimeZone) -> String {
    use texts::install::refused;
    let said = |text: Text| l.pick(text).to_string();
    match (r, sent(source)) {
        (Refusal::Unknown, true) => said(refused::UNKNOWN_SEND),
        (Refusal::Unknown, false) => said(refused::UNKNOWN_CHOOSE),
        (Refusal::Image, true) => said(refused::IMAGE_SEND),
        (Refusal::Image, false) => said(refused::IMAGE_CHOOSE),
        (Refusal::NotAb(kind), true) if kind.is_empty() => said(refused::UNTYPED_SEND),
        (Refusal::NotAb(kind), false) if kind.is_empty() => said(refused::UNTYPED_CHOOSE),
        (Refusal::NotAb(kind), true) => refused::not_ab_send(l, kind),
        (Refusal::NotAb(kind), false) => refused::not_ab_choose(l, kind),
        (Refusal::OtherDevice(devices), _) => refused::other_device(l, devices, codename),
        (Refusal::OtherSerial, _) => said(refused::OTHER_SERIAL),
        (Refusal::Incremental, true) => said(refused::INCREMENTAL_SEND),
        (Refusal::Incremental, false) => said(refused::INCREMENTAL_CHOOSE),
        (Refusal::Older { package, installed }, _) => refused::older(l, date(l, *package, zone), date(l, *installed, zone)),
        (Refusal::OlderPatch { package, installed }, _) => refused::older_patch(l, package, installed),
        (Refusal::NoPartition, _) => said(refused::NO_PARTITION),
        (Refusal::Battery { level, charging }, _) => refused::battery(l, level, skyward_install::least_charge(*charging)),
        (Refusal::Pending(state), _) => refused::pending(l, snapshot(l, state)),
        (Refusal::OldHost, _) => said(refused::OLD_HOST),
        (Refusal::Unsigned, _) => said(refused::UNSIGNED),
        (Refusal::Unverified, _) => said(refused::UNVERIFIED),
        (Refusal::Panicked, _) => said(refused::PANICKED),
        (Refusal::SystemAsked, _) => said(refused::SYSTEM_ASKED),
        (Refusal::DataClosed, _) => said(refused::DATA_CLOSED),
        (Refusal::SystemInstalled(name), _) => refused::system_installed(l, name),
        (Refusal::NoRoom { least, room }, _) => refused::no_room(l, bytes(*least), bytes(*room)),
        (Refusal::NotBrick(_) | Refusal::NoSerial, _) => unreachable!("only a request to erase partitions is refused as {r:?}"),
    }
}

/// Why the system's request to erase the partitions it lists was not
/// carried out, on a phone whose codename is `codename`.
pub fn erase_request_refusal(l: Lang, r: &Refusal, codename: &str) -> String {
    use texts::erase_request::refused;
    match r {
        Refusal::Unsigned => l.pick(refused::UNSIGNED).into(),
        Refusal::Unverified => l.pick(refused::UNVERIFIED).into(),
        Refusal::NotBrick(kind) if kind.is_empty() => l.pick(refused::UNTYPED).into(),
        Refusal::NotBrick(kind) => refused::typed(l, kind),
        Refusal::OtherDevice(devices) => refused::other_device(l, devices, codename),
        Refusal::OtherSerial => l.pick(refused::OTHER_SERIAL).into(),
        Refusal::NoSerial => l.pick(refused::NO_SERIAL).into(),
        other => unreachable!("a request to erase partitions is not refused as {other:?}"),
    }
}

/// What went wrong installing a package from `source`, and the details
/// behind it when there are any.
pub fn failure(l: Lang, f: &Failure, source: Source) -> (String, Option<String>) {
    use texts::install::failed;
    let said = |text: Text| l.pick(text).to_string();
    match f {
        Failure::Engine(code) => {
            let what = said(match (Trouble::of(*code), source) {
                (Trouble::Signature, _) if sent(source) => failed::SIGNATURE_SEND,
                (Trouble::Signature, _) => failed::SIGNATURE_CHOOSE,
                (Trouble::Damaged, _) => failed::DAMAGED,
                (Trouble::Older, _) => failed::OLDER,
                (Trouble::NoSpace, _) => failed::NO_SPACE,
                (Trouble::Transfer, Source::Computer | Source::Rescue) => failed::TRANSFER_CABLE,
                (Trouble::Transfer, Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. }) => failed::TRANSFER,
                (Trouble::Transfer, Source::File(Place::UsbDrive) | Source::Opened { drive: true }) => failed::TRANSFER_DRIVE,
                (Trouble::Write, _) => failed::WRITE,
                (Trouble::Verify, _) => failed::VERIFY,
                (Trouble::PostInstall, _) => failed::POST_INSTALL,
                (Trouble::Unsupported, _) => failed::UNSUPPORTED,
                (Trouble::Other, _) => failed::UPDATER,
            });
            (what, Some(failed::error_code(l, code)))
        }
        Failure::Script(code) => (failed::script(l, code), None),
        Failure::Disconnected => (said(failed::DISCONNECTED), None),
        Failure::Unreadable(why) => match source {
            Source::File(Place::UsbDrive) | Source::Opened { drive: true } => (said(failed::UNREADABLE_DRIVE), Some(why.clone())),
            _ => (said(failed::UNREADABLE), Some(why.clone())),
        },
        Failure::Changed => match sent(source) {
            true => (said(failed::CHANGED_SENT), None),
            false => (said(failed::CHANGED_READ), None),
        },
        Failure::Error(detail) => (said(texts::common::UNEXPECTED), Some(detail.clone())),
    }
}

/// How many of a WLAN fan's three arcs a signal of `dbm` lights.
pub fn signal_level(dbm: i32) -> u8 {
    match dbm {
        d if d >= -60 => 3,
        d if d >= -70 => 2,
        d if d >= -80 => 1,
        _ => 0,
    }
}

/// How fast data goes, `bytes` a second, as the stock status bar writes it
/// over its unit: in KB/s up to 999 and in MB/s past that, to three
/// figures, two of them after the point below 10 and one below 100.
pub fn speed(bytes: u64) -> (String, &'static str) {
    let kb = bytes as f64 / 1024.0;
    let (v, unit) = if kb < 999.5 { (kb, "KB/s") } else { (kb / 1024.0, "MB/s") };
    let figures = if v < 9.995 {
        format!("{v:.2}")
    } else if v < 99.95 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    };
    (figures, unit)
}

/// A signal's strength in words, as the stock network details say it.
pub fn signal(l: Lang, dbm: i32) -> &'static str {
    l.pick(match signal_level(dbm) {
        3 => texts::wlan::signal::EXCELLENT,
        2 => texts::wlan::signal::GOOD,
        1 => texts::wlan::signal::FAIR,
        _ => texts::wlan::signal::WEAK,
    })
}

/// The bands a network is on, as the tag after its name: none for 2.4 GHz
/// alone, as the stock WLAN page shows it.
pub fn band_tag(b: Bands) -> Option<String> {
    let names: Vec<&str> = [(b.ghz2, "2.4G"), (b.ghz5, "5G"), (b.ghz6, "6G")].into_iter().filter_map(|(on, n)| on.then_some(n)).collect();
    (names != ["2.4G"] && !names.is_empty()).then(|| names.join("/"))
}

/// The bands a network is on, in full.
pub fn bands(l: Lang, b: Bands) -> String {
    list(l, [(b.ghz2, "2.4 GHz"), (b.ghz5, "5 GHz"), (b.ghz6, "6 GHz")].into_iter().filter_map(|(on, n)| on.then_some(n)))
}

/// A generation of Wi-Fi, as the stock network details say it.
pub fn generation(l: Lang, n: u8) -> String {
    texts::wlan::generation(l, n)
}

/// How a network is protected, as the system's network details name it.
pub fn security(l: Lang, s: Security) -> &'static str {
    match s {
        Security::Open => l.pick(texts::wlan::security::NONE),
        Security::Owe => l.pick(texts::wlan::security::ENHANCED_OPEN),
        Security::Wep => "WEP",
        Security::Psk => "WPA/WPA2-Personal",
        Security::Sae => "WPA3-Personal",
        Security::PskSae => "WPA2/WPA3-Personal",
        Security::Enterprise => "WPA/WPA2-Enterprise",
    }
}

/// What the home page says beside its WLAN entry: the network joined, or
/// how the WLAN stands.
pub fn wlan_value(l: Lang, w: &Wlan) -> &str {
    match (&w.power, &w.joined) {
        (Power::Off | Power::Failed(_), _) => l.pick(texts::wlan::OFF),
        (Power::Starting, _) => l.pick(texts::wlan::TURNING_ON),
        (Power::On, Some(Joined { stage: WlanStage::Connected(_), name, .. })) => name,
        (Power::On, Some(_)) => l.pick(texts::wlan::CONNECTING),
        (Power::On, None) => l.pick(texts::wlan::NOT_CONNECTED),
    }
}

/// How joining the network chosen goes, under its name; joined, how far it
/// reaches, as the system's WLAN page says it.
pub fn joining(l: Lang, stage: &WlanStage) -> &'static str {
    use texts::wlan::joining;
    l.pick(match stage {
        WlanStage::Joining => texts::wlan::CONNECTING,
        WlanStage::Addressing => joining::GETTING_ADDRESS,
        WlanStage::Connected(link) => match link.reach {
            Some(Reach::Login(_)) => joining::SIGN_IN_REQUIRED,
            Some(Reach::Offline(_)) => joining::NO_INTERNET,
            Some(Reach::Online) | None => joining::CONNECTED,
        },
        WlanStage::WrongPassword => joining::WRONG_PASSWORD,
        WlanStage::NotFound => joining::NOT_FOUND,
        WlanStage::Failed => joining::FAILED,
        WlanStage::NoAddress(_) => joining::NO_ADDRESS,
    })
}

/// What the WLAN page says under the network joined of how far it reaches,
/// where it reaches short of the internet: a network that wants a login in
/// a browser first is left to a recovery that has no browser to log in
/// with.
pub fn reach(l: Lang, reach: &Reach) -> Option<String> {
    match reach {
        Reach::Online => None,
        Reach::Login(_) => Some(l.pick(texts::wlan::NEEDS_LOGIN).into()),
        Reach::Offline(why) => Some(texts::wlan::offline(l, why)),
    }
}

/// Why the update server gave no answer, as the update page says it.
pub fn unanswered(l: Lang, why: &Unanswered) -> String {
    match why {
        Unanswered::Login => l.pick(texts::update::unanswered::LOGIN).into(),
        Unanswered::Offline(why) => texts::update::unanswered::offline(l, why),
        Unanswered::Failed(why) => texts::update::unanswered::failed(l, why),
    }
}

/// A size the update server gives, such as 7.1G, with its unit spelled
/// out; one in another form shows as it came.
pub fn package_size(size: &str) -> String {
    match size.char_indices().last() {
        Some((at, unit @ ('K' | 'M' | 'G' | 'T'))) if size[..at].parse::<f64>().is_ok() => format!("{} {unit}B", &size[..at]),
        _ => size.to_string(),
    }
}

/// Why downloading the full package did not end with it whole.
pub fn download_failure(l: Lang, f: &DownloadFailure) -> String {
    match f {
        DownloadFailure::NoRoom { needed, free } => texts::download::failed::no_room(l, file_size(*needed), file_size(*free)),
        DownloadFailure::Damaged => l.pick(texts::download::failed::DAMAGED).into(),
        DownloadFailure::Changed => l.pick(texts::download::failed::CHANGED).into(),
        DownloadFailure::Other(why) => texts::download::failed::other(l, why),
    }
}

/// Why downloading Skyward's build did not end with it whole. It goes into
/// the recovery's own folder, which is in memory.
pub fn skyward_download_failure(l: Lang, f: &DownloadFailure) -> String {
    match f {
        DownloadFailure::NoRoom { needed, free } => texts::download::failed::no_memory(l, file_size(*needed), file_size(*free)),
        DownloadFailure::Damaged => l.pick(texts::download::failed::SKYWARD_DAMAGED).into(),
        DownloadFailure::Changed => unreachable!("GitHub is not asked again where a build is"),
        DownloadFailure::Other(why) => texts::download::failed::other(l, why),
    }
}

/// How the terminal page's shell ended.
pub fn term_end(l: Lang, end: &TermEnd) -> String {
    match end {
        TermEnd::Exited(code) => texts::terminal::exited(l, code),
        TermEnd::Signalled(signal) => texts::terminal::signalled(l, signal),
        TermEnd::Other(status) => texts::terminal::ended(l, status),
    }
}

/// What a computer is told internal storage is called, as the stock system
/// names it.
pub fn storage_name(l: Lang) -> &'static str {
    l.pick(texts::transfer::STORAGE_NAME)
}

/// File transfer kept internal storage from work that needed it back, as
/// the system says why.
pub fn transfer_kept(l: Lang, why: &str) -> String {
    texts::transfer::kept(l, why)
}

/// File transfer did not turn on, as the system says why.
pub fn transfer_failed(l: Lang, why: &str) -> String {
    texts::transfer::failed(l, why)
}

/// File transfer did not turn off, as the system says why.
pub fn transfer_left_on(l: Lang, why: &str) -> String {
    texts::transfer::left_on(l, why)
}

/// File transfer stopped on its own, as the system says why.
pub fn transfer_stopped(l: Lang, why: &str) -> String {
    texts::transfer::stopped(l, why)
}

/// Merging the update failed, as the system says why.
pub fn merge_failed(l: Lang, why: &str) -> String {
    texts::update::merge_failed(l, why)
}

/// What work that takes a program of the installed system's `base`
/// partition asks while that partition is not mounted.
pub fn mount_first(l: Lang, base: &str) -> String {
    texts::mount::first(l, base)
}

/// The installed system's SELinux rules could not be read, as the system
/// says why.
pub fn rules_unread(l: Lang, why: &str) -> String {
    texts::relabel::rules_unread(l, why)
}

/// The logs were saved in `folder`.
pub fn exported(l: Lang, folder: &str) -> String {
    texts::logs::exported(l, folder)
}

/// The logs were not saved, as the system says why.
pub fn export_failed(l: Lang, why: &str) -> String {
    texts::logs::export_failed(l, why)
}

/// A screenshot went into the gallery's screenshots.
pub const SCREENSHOT_SAVED: Text = texts::screenshot::SAVED;

/// A screenshot is held until the owner's storage opens, and then goes into
/// the gallery's.
pub const SCREENSHOT_HELD: Text = texts::screenshot::HELD;

/// A screenshot was not saved, as the system says why.
pub fn screenshot_failed(l: Lang, why: &str) -> String {
    texts::screenshot::failed(l, why)
}

/// The screenshots held while the owner's storage was closed did not go into
/// the gallery's, as the system says why.
pub fn screenshots_not_kept(l: Lang, why: &str) -> String {
    texts::screenshot::not_kept(l, why)
}

/// The name a screenshot taken `unix_ms` milliseconds into Unix time is saved
/// under, in `zone`: as the system names its own, the time to the
/// millisecond and then where it was taken,
/// `Screenshot_2026-10-26-01-45-00-123_Skyward.png`.
pub fn screenshot_name(unix_ms: i64, zone: &TimeZone) -> String {
    let t = local(unix_ms.div_euclid(1000), zone);
    format!(
        "Screenshot_{:04}-{:02}-{:02}-{:02}-{:02}-{:02}-{:03}_Skyward.png",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        unix_ms.rem_euclid(1000)
    )
}

/// What the terminal is told while a command it runs holds it.
pub const TERMINAL_BUSY: Text = texts::terminal::BUSY;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::listed;

    const HANS: Lang = Lang::Hans;

    // Xiaomi 13 Ultra (ishtar, 16 GB + 512 GB): readings from `skyward info`
    // and the strings its stock About pages print for them.
    // Xiaomi 17 Ultra (nezha, 16 GB + 1 TB): MemTotal from its boot log and
    // the userdata size from `fastboot getvar all`.
    #[test]
    fn ram_rounds_to_module_size() {
        assert_eq!(ram(15_556_972), "16.0 GB");
        assert_eq!(ram(15_472_464), "16.0 GB");
    }

    #[test]
    fn storage_matches_the_stock_settings() {
        let ishtar = Storage { total: 496_550_785_024, free: Some(61_300_000_000) };
        assert_eq!(capacity(ishtar.total), "512GB");
        assert_eq!(storage_used(&ishtar).unwrap(), "450.7GB/512GB");
        assert_eq!(capacity(0xF2_091F_B000), "1TB");
        assert_eq!(storage_used(&Storage { total: 1, free: None }), None);
    }

    #[test]
    fn cpu_summary() {
        let nezha = Cpu { cores: 8, max_khz: Some(4_608_000) };
        assert_eq!(cpu(HANS, &Cpu { cores: 8, max_khz: Some(3_187_200) }), "八核 最高 3.19GHz");
        assert_eq!(cpu(HANS, &nezha), "八核 最高 4.61GHz");
        assert_eq!(cpu(HANS, &Cpu { cores: 2, max_khz: None }), "双核");
        assert_eq!(cpu(Lang::Hant, &Cpu { cores: 2, max_khz: None }), "雙核");
        assert_eq!(cpu(Lang::En, &nezha), "Octa-core, up to 4.61GHz");
        assert_eq!(cpu(Lang::En, &Cpu { cores: 12, max_khz: None }), "12-core");
        assert_eq!(numeral(HANS, 12), "十二");
        assert_eq!(numeral(HANS, 20), "二十");
    }

    // Xiaomi 13 Ultra battery as `skyward info` read it.
    #[test]
    fn battery_lines() {
        let b = Battery {
            level: 100,
            charge: Charge::Full,
            temp: Some(338),
            health: Some("Good".into()),
            cycles: Some(169),
            design: Some(5_500_000),
            full: Some(5_095_000),
            voltage: Some(4_482_000),
            quick: false,
            plugged: true,
        };
        assert_eq!(level(HANS, &b), "100% · 已充满");
        assert_eq!(level(Lang::Hant, &b), "100% · 已充滿");
        assert_eq!(celsius(b.temp.unwrap()), "33.8°C");
        assert_eq!([degrees(352), degrees(400), degrees(-5)], ["35.2", "40.0", "-0.5"], "to the tenth, a whole degree too");
        assert_eq!(wear(HANS, &b).unwrap(), "良好 · 剩余容量 93%");
        assert_eq!(wear(Lang::En, &b).unwrap(), "Good · Maximum capacity 93%");
        assert_eq!(mah(b.design.unwrap()), "5500mAh");
        assert_eq!(volts(b.voltage.unwrap()), "4.48V");
    }

    // Xiaomi 17 Ultra build and panel, as its stock settings print them.
    #[test]
    fn versions_and_states() {
        let patch = PatchLevel { year: 2026, month: 10, day: 1 };
        assert_eq!(android(HANS, "17", Some(&patch)), "17 | Android 安全更新：2026年10月1日");
        assert_eq!(android(Lang::Hant, "17", Some(&patch)), "17 | Android 安全性更新：2026年10月1日");
        assert_eq!(android(Lang::En, "17", Some(&patch)), "17 | Android security update: October 1, 2026");
        assert_eq!(panel((1200, 2608)), "2608 x 1200");
        assert_eq!(model_lines("Xiaomi 17 Ultra by Leica"), "Xiaomi 17 Ultra\nby Leica");
        assert_eq!(model_lines("Xiaomi 17 Ultra"), "Xiaomi 17 Ultra");
        assert_eq!(inches(6.9), "6.9″");
        assert_eq!(verified(HANS, "orange"), "橙色 · 设备已解锁，不验证系统");
        assert_eq!(health(Lang::En, "Watchdog timer expire"), "Watchdog timer expire");
    }

    #[test]
    fn sizes_and_dates() {
        assert_eq!(bytes(7_103_041_280), "7.1GB");
        assert_eq!(bytes(100_663_296), "100.7MB");
        assert_eq!(bytes(131_072), "131KB");
        assert_eq!(bytes(64), "64B");
        // The Xiaomi 13 Ultra's installed system, by its ro.build.date.utc.
        assert_eq!(date(HANS, 1_778_243_655, china()), "2026年5月8日");
        assert_eq!(date(Lang::En, 1_778_243_655, china()), "May 8, 2026");
        assert_eq!(date(HANS, 0, china()), "1970年1月1日");
        // Midnight on a leap day in UTC is eight in the morning in China.
        assert_eq!(date(HANS, 1_709_164_800, china()), "2024年2月29日");
        assert_eq!(date(HANS, 1_709_164_800 - 8 * 3600 - 1, china()), "2024年2月28日");
        assert_eq!(patch(HANS, &PatchLevel { year: 2026, month: 4, day: 1 }), "2026年4月1日");
    }

    #[test]
    fn screen_off_times() {
        let labels = |l| crate::app::SCREEN_OFF.iter().map(|&ms| screen_off(l, ms)).collect::<Vec<String>>();
        assert_eq!(labels(HANS), ["15 秒", "30 秒", "1 分钟", "2 分钟", "5 分钟", "10 分钟", "永不"]);
        assert_eq!(labels(Lang::Hant), ["15 秒", "30 秒", "1 分鐘", "2 分鐘", "5 分鐘", "10 分鐘", "永不"]);
        assert_eq!(labels(Lang::En), ["15 seconds", "30 seconds", "1 minute", "2 minutes", "5 minutes", "10 minutes", "Never"]);
    }

    #[test]
    fn times_to_the_minute_in_china() {
        // 2026-10-18 18:31 UTC is 02:31 the next morning in Beijing.
        assert_eq!(datetime(HANS, 1_792_348_260, china()), "2026年10月19日 02:31");
        assert_eq!(datetime(Lang::En, 1_792_348_260, china()), "October 19, 2026, 02:31");
        assert_eq!(clock(1_792_348_260, china()), "02:31");
        // Midnight in Beijing is 16:00 UTC the day before.
        assert_eq!(clock(1_792_339_200, china()), "00:00");
        assert_eq!(stamp(1_792_348_265, china()), "20261019-023105");
    }

    #[test]
    fn a_screenshot_is_named_as_the_system_names_its_own() {
        assert_eq!(screenshot_name(1_792_348_265_007, china()), "Screenshot_2026-10-19-02-31-05-007_Skyward.png");
        assert_eq!(screenshot_name(1_792_348_265_999, zone("UTC")), "Screenshot_2026-10-18-18-31-05-999_Skyward.png", "in the zone chosen");
    }

    /// The rules of a zone listed, by its name in the tz database.
    fn zone(name: &str) -> &'static TimeZone {
        rules(listed(name).expect("a zone listed"))
    }

    /// China's, which most of the tests tell their times in.
    fn china() -> &'static TimeZone {
        zone("Asia/Shanghai")
    }

    #[test]
    fn a_list_of_zones_tells_how_far_each_is_from_utc() {
        // 2026-07-01 12:00 UTC, and 2026-01-15 12:00 UTC.
        let (summer, winter) = (1_782_907_200, 1_768_478_400);
        assert_eq!(utc_offset(summer, china()), "UTC+08:00");
        assert_eq!(utc_offset(summer, zone("UTC")), "UTC+00:00");
        assert_eq!((utc_offset(summer, zone("Europe/London")), utc_offset(winter, zone("Europe/London"))), ("UTC+01:00".into(), "UTC+00:00".into()));
        assert_eq!((utc_offset(summer, zone("America/New_York")), utc_offset(winter, zone("America/New_York"))), ("UTC−04:00".into(), "UTC−05:00".into()));
        assert_eq!(utc_offset(summer, zone("Asia/Kolkata")), "UTC+05:30");
        assert_eq!(utc_offset(summer, zone("Asia/Kathmandu")), "UTC+05:45");
        assert_eq!((utc_offset(summer, zone("Australia/Adelaide")), utc_offset(winter, zone("Australia/Adelaide"))), ("UTC+09:30".into(), "UTC+10:30".into()));
        assert_eq!(utc_offset_seconds(winter, zone("America/Los_Angeles")), -8 * 3600);
    }

    #[test]
    fn the_recovery_carries_the_rules_of_every_name_a_zone_is_known_by() {
        let mut names: Vec<&str> = TIME_ZONES.iter().flat_map(|zone| std::iter::once(zone.tz).chain(zone.also.iter().copied())).collect();
        for name in &names {
            assert!(TimeZone::get(name).is_ok(), "{name}");
        }
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "each name is one zone's");
    }

    #[test]
    fn the_places_that_share_a_zone_s_name_keep_its_time() {
        // Each month's middle, from 2026-01-15 12:00 UTC on.
        let months: Vec<i64> = (0..24).map(|k| 1_768_478_400 + k * 2_629_800).collect();
        let mut apart = Vec::new();
        for zone in &TIME_ZONES {
            let own = TimeZone::get(zone.tz).unwrap();
            for also in zone.also {
                let other = TimeZone::get(also).unwrap();
                if months.iter().any(|&at| utc_offset_seconds(at, &other) != utc_offset_seconds(at, &own)) {
                    apart.push((zone.tz, *also));
                }
            }
        }
        assert_eq!(apart, [], "names whose clocks part from their zone's");
    }

    #[test]
    fn zones_keep_their_own_summer_time() {
        // 2026-07-01 12:00 UTC, and 2026-01-15 12:00 UTC.
        let (summer, winter) = (1_782_907_200, 1_768_478_400);
        assert_eq!(clock(summer, zone("UTC")), "12:00");
        assert_eq!(clock(summer, zone("Europe/London")), "13:00", "British Summer Time");
        assert_eq!(clock(winter, zone("Europe/London")), "12:00");
        assert_eq!(clock(summer, zone("Europe/Berlin")), "14:00");
        assert_eq!(clock(winter, zone("Europe/Berlin")), "13:00");
        assert_eq!(clock(summer, zone("America/New_York")), "08:00");
        assert_eq!(clock(winter, zone("America/New_York")), "07:00");
        assert_eq!(datetime(HANS, summer, zone("America/Los_Angeles")), "2026年7月1日 05:00");
        assert_eq!(clock(summer, zone("Asia/Tokyo")), "21:00");
        assert_eq!(clock(winter, zone("Asia/Shanghai")), "20:00", "China keeps no summer time");
        // New York goes over to summer time at 2:00 on 2026-03-08.
        let before = 1_772_953_200 - 60;
        assert_eq!((clock(before, zone("America/New_York")), clock(before + 60, zone("America/New_York"))), ("01:59".into(), "03:00".into()));
    }

    #[test]
    fn a_time_past_the_calendar_shows_as_its_end() {
        assert_eq!(date(HANS, i64::MAX, china()), "9999年12月31日");
        assert_eq!(date(HANS, i64::MIN, zone("UTC")), "-9999年1月2日");
        assert_eq!(date(Lang::En, i64::MAX, china()), "December 31, 9999");
    }

    #[test]
    fn modes_read_as_ls_prints_them() {
        assert_eq!(mode(0o040_755), "drwxr-xr-x");
        assert_eq!(mode(0o100_644), "-rw-r--r--");
        assert_eq!(mode(0o120_777), "lrwxrwxrwx");
        assert_eq!(mode(0o041_777), "drwxrwxrwt", "the sticky /tmp");
        assert_eq!(mode(0o104_755), "-rwsr-xr-x", "set-user on an executable");
        assert_eq!(mode(0o020_600), "crw-------");
    }

    #[test]
    fn entries_say_what_they_are() {
        // Changed at 2026-10-19 02:31 in Beijing; listed a week later.
        let row = |kind, size, mode| FileRow { name: "x".into(), kind, size, modified: 1_792_348_260, mode, uid: 0, gid: 0 };
        let later = 1_792_348_260 + 7 * 86_400;
        assert_eq!(file_line(HANS, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, china()), "2026/10/19  |  6 项");
        assert_eq!(file_line(Lang::Hant, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, china()), "2026/10/19  |  6 項");
        assert_eq!(file_line(Lang::En, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, china()), "2026/10/19  |  6 items");
        assert_eq!(file_line(Lang::En, &row(FileKind::Folder { items: Some(1) }, 4096, 0o040_755), later, china()), "2026/10/19  |  1 item");
        assert_eq!(file_line(HANS, &row(FileKind::Folder { items: None }, 4096, 0o040_700), later, china()), "无法读取");
        assert_eq!(file_line(HANS, &row(FileKind::File, 26_950, 0o100_644), later, china()), "10/19  |  26.95 KB");
        let link = FileKind::Link { target: "/storage/self/primary".into(), folder: true };
        assert_eq!(file_line(HANS, &row(link, 21, 0o120_777), later, china()), "链接到 /storage/self/primary");
        assert_eq!(file_line(HANS, &row(FileKind::Special, 0, 0o060_600), later, china()), "10/19  |  块设备");
        assert_eq!(file_line(Lang::En, &row(FileKind::Special, 0, 0o060_600), later, china()), "10/19  |  Block device");
    }

    #[test]
    fn list_dates_are_told_against_the_day_the_folder_was_read() {
        // 2026-10-19 02:31 in Beijing.
        let at = 1_792_348_260;
        assert_eq!(file_time(at, false, at + 3600, china()), "02:31", "today: the time");
        assert_eq!(file_time(at, true, at + 3600, china()), "02:31", "a folder today too");
        assert_eq!(file_time(at, false, at + 86_400, china()), "10/19", "this year: a file's month and day");
        assert_eq!(file_time(at, true, at + 86_400, china()), "2026/10/19", "this year: a folder's whole date");
        assert_eq!(file_time(at, false, at + 90 * 86_400, china()), "2026/10/19", "another year: the whole date");
        // 2026-10-18 23:59 UTC is already the 19th in Beijing.
        assert_eq!(file_time(1_792_367_940, false, at, china()), "07:59");
        // In London the file changed on the evening of the 18th, the day
        // before.
        assert_eq!(file_time(1_792_348_260, false, 1_792_367_940, zone("Europe/London")), "10/18");
    }

    #[test]
    fn sizes_go_up_a_unit_past_900_with_two_decimals_at_most() {
        assert_eq!(file_size(0), "0 B");
        assert_eq!(file_size(900), "900 B");
        assert_eq!(file_size(901), "0.9 KB");
        assert_eq!(file_size(42_633), "42.63 KB");
        assert_eq!(file_size(26_950), "26.95 KB");
        assert_eq!(file_size(1_090_000), "1.09 MB");
        assert_eq!(file_size(634_960), "634.96 KB");
        assert_eq!(file_size(2_000_000_000), "2 GB");
        assert_eq!(file_size(1_500_000), "1.5 MB");
    }

    #[test]
    fn a_backup_lists_its_parts() {
        assert_eq!(parts(HANS, &[true, true, false, true]), "启动分区、系统分区、基带与 EFS");
        assert_eq!(parts(Lang::Hant, &[true, true, false, true]), "啟動分割區、系統分割區、基頻與 EFS");
        assert_eq!(parts(Lang::En, &[true, true, true, true]), "Boot partitions, system partitions, data partition, modem and EFS");
    }

    #[test]
    fn an_english_list_names_what_follows_the_first_as_a_sentence_does() {
        assert_eq!(list(Lang::En, ["ART cache", "App data", "Logs"]), "ART cache, app data, logs", "a word in capitals stays so");
        assert_eq!(list(HANS, ["ART 缓存", "应用数据"]), "ART 缓存、应用数据");
        assert_eq!(bands(Lang::En, Bands { ghz2: true, ghz5: true, ghz6: false }), "2.4 GHz, 5 GHz");
        assert_eq!(bands(HANS, Bands { ghz2: true, ghz5: true, ghz6: true }), "2.4 GHz、5 GHz、6 GHz");
    }

    #[test]
    fn mount_failures_name_the_partition_and_why() {
        let unmounting = MountFailure {
            mounting: false,
            base: "vendor".into(),
            detail: "unmount /vendor: Device or resource busy (os error 16)".into(),
        };
        assert_eq!(mount_failure(HANS, &unmounting), "卸载 vendor 失败：unmount /vendor: Device or resource busy (os error 16)");
        assert_eq!(mount_failure(Lang::En, &unmounting), "Couldn't unmount vendor: unmount /vendor: Device or resource busy (os error 16)");
        let mounting = MountFailure {
            mounting: true,
            base: "odm".into(),
            detail: "mount odm_b: /dev/block/dm-8 holds none of erofs, ext4, f2fs".into(),
        };
        assert_eq!(mount_failure(HANS, &mounting), "挂载 odm 失败：mount odm_b: /dev/block/dm-8 holds none of erofs, ext4, f2fs");
        assert_eq!(mount_failure(Lang::Hant, &mounting), "掛載 odm 失敗：mount odm_b: /dev/block/dm-8 holds none of erofs, ext4, f2fs");
    }

    #[test]
    fn reasons_read_as_sentences() {
        let pc = Source::Computer;
        assert_eq!(refusal(HANS, &Refusal::OtherDevice("nezha".into()), "ishtar", pc, china()), "该更新包适用于 nezha，本机为 ishtar。");
        assert_eq!(
            refusal(HANS, &Refusal::Older { package: 1_770_000_000, installed: 1_778_243_655 }, "ishtar", pc, china()),
            "该更新包的构建日期为 2026年2月2日，早于当前系统的 2026年5月8日。"
        );
        assert_eq!(
            refusal(Lang::En, &Refusal::Older { package: 1_770_000_000, installed: 1_778_243_655 }, "ishtar", pc, china()),
            "This package was built on February 2, 2026, before the current system, built on May 8, 2026."
        );
        assert_eq!(refusal(HANS, &Refusal::Battery { level: 12, charging: true }, "ishtar", pc, china()), "电量仅剩 12%，请充电至 15% 以上后重试。");
        assert_eq!(
            refusal(HANS, &Refusal::Pending(SnapshotState::Snapshotted), "ishtar", pc, china()),
            "更新已写入，等待合并。请重启到系统，完成上一次更新后重试。"
        );
        assert_eq!(
            refusal(Lang::En, &Refusal::Pending(SnapshotState::Snapshotted), "ishtar", pc, china()),
            "Update written, waiting to merge. Reboot to the system to finish the last update, then try again."
        );
        assert_eq!(failure(HANS, &Failure::Engine(26), pc), ("更新包签名验证失败，请发送官方更新包。".into(), Some("错误代码：26".into())));
        assert_eq!(failure(Lang::Hant, &Failure::Engine(26), pc), ("更新檔簽章驗證失敗，請傳送官方更新檔。".into(), Some("錯誤代碼：26".into())));
        assert_eq!(failure(HANS, &Failure::Error("open ep0: busy".into()), pc).1.as_deref(), Some("open ep0: busy"));
        assert_eq!(failure(HANS, &Failure::Script(1), pc), ("安装脚本运行失败，退出码为 1。".into(), None));
        assert_eq!(refusal(HANS, &Refusal::Unsigned, "ishtar", pc, china()), "该刷机包未签名，请关闭「验证刷机包签名」后重试。");
        assert_eq!(refusal(HANS, &Refusal::Unverified, "ishtar", pc, china()), "该刷机包未通过签名验证，请关闭「验证刷机包签名」后重试。");
    }

    #[test]
    fn a_file_chosen_is_answered_with_choosing_another() {
        let file = Source::File(Place::Storage);
        assert_eq!(refusal(HANS, &Refusal::Unknown, "ishtar", file, china()), "无法识别该文件，请选择 A/B 完整更新包、刷机包或分区镜像。");
        assert_eq!(
            refusal(Lang::En, &Refusal::Unknown, "ishtar", file, china()),
            "This file isn't recognized. Choose an A/B full OTA package, a flashable zip or a partition image."
        );
        assert_eq!(refusal(HANS, &Refusal::Incremental, "ishtar", file, china()), "无法安装增量更新包，请选择完整更新包。");
        assert_eq!(failure(HANS, &Failure::Engine(26), file).0, "更新包签名验证失败，请选择官方更新包。");
        assert_eq!(failure(HANS, &Failure::Changed, file).0, "文件在读取过程中被修改，请重新选择。");
        let gone = "read /sdcard/Download/ota.zip: Input/output error (os error 5)";
        assert_eq!(failure(HANS, &Failure::Unreadable(gone.into()), file), ("读取文件失败。".into(), Some(gone.into())));
        let pulled = failure(HANS, &Failure::Unreadable(gone.into()), Source::File(Place::UsbDrive));
        assert_eq!(pulled.0, "读取文件失败，请检查 U 盘连接后重试。", "a drive that came out is checked first");
    }

    #[test]
    fn erasing_that_stops_says_what_was_left_and_why() {
        assert_eq!(users(HANS, &[0]), "机主");
        assert_eq!(users(HANS, &[0, 999]), "机主、用户 999");
        assert_eq!(users(Lang::Hant, &[0, 999]), "擁有者、使用者 999");
        assert_eq!(users(Lang::En, &[0]), "the owner");
        assert_eq!(users(Lang::En, &[0, 10, 999]), "the owner, user 10 and user 999");
        assert!(erase_stop(HANS, &EraseStop::Locked(vec![0, 999])).0.starts_with("机主、用户 999 的数据尚未解密，"));
        assert!(erase_stop(HANS, &EraseStop::Locked(vec![0])).0.starts_with("机主的数据尚未解密，"));
        assert!(erase_stop(Lang::En, &EraseStop::Locked(vec![0, 999])).0.starts_with("The data of the owner and user 999 isn't decrypted,"));
        assert_eq!(erase_stop(HANS, &EraseStop::Failed(Part::Art, "x".into())).0, "清除 ART 缓存失败。", "a Latin name stands apart");
        assert_eq!(erase_stop(HANS, &EraseStop::Failed(Part::Data, "x".into())).0, "清除 data 分区失败。");
        assert_eq!(erase_stop(HANS, &EraseStop::Failed(Part::Logs, "x".into())), ("清除日志失败。".into(), Some("x".into())));
        assert_eq!(erase_stop(Lang::Hant, &EraseStop::Failed(Part::Logs, "x".into())).0, "清除記錄失敗。");
        assert_eq!(erase_stop(Lang::En, &EraseStop::Failed(Part::Data, "x".into())).0, "Couldn't wipe the data partition.");
        assert_eq!(erase_stop(Lang::En, &EraseStop::Failed(Part::AppData, "x".into())).0, "Couldn't wipe app data.");
    }

    #[test]
    fn work_on_partitions_is_told_in_the_language() {
        let repair = Verb::of(&PartKind::Repair);
        assert_eq!([repair.going(HANS), repair.complete(HANS), repair.failed(HANS), repair.not_done(HANS)], ["正在修复", "修复完成", "修复失败", "未修复"]);
        assert_eq!([repair.going(Lang::Hant), repair.failed(Lang::Hant)], ["正在修復", "修復失敗"]);
        assert_eq!(
            [repair.going(Lang::En), repair.complete(Lang::En), repair.failed(Lang::En), repair.not_done(Lang::En)],
            ["Repairing", "Repair complete", "Repair failed", "Not repaired"]
        );
        assert_eq!(repair.failed_on(HANS, "vendor"), "修复 vendor 失败");
        assert_eq!(repair.failed_on(Lang::En, "vendor"), "Couldn't repair vendor");
        assert_eq!(Verb::of(&PartKind::Disable).incomplete(Lang::En), "Wipe incomplete");
        assert_eq!(item_end(Lang::En, &ItemEnd::Failed(None), Verb::of(&PartKind::Remake("f2fs".into()))), "Format failed");
        assert_eq!(item_end(Lang::Hant, &ItemEnd::Fixed, repair), "已修復");
    }

    #[test]
    fn an_installed_update_says_whether_its_slot_starts_next() {
        let installed = Standing::Installed(Slot::A);
        assert_eq!(update(HANS, &installed, Some(Slot::A)).1.unwrap(), "重启后将从槽位 A 启动新系统，启动后自动合并");
        assert_eq!(
            update(HANS, &installed, Some(Slot::B)).1.unwrap(),
            "需在系统的「系统更新」中重启，才会从槽位 A 启动新系统",
            "the system's updater has not had slot A start next yet"
        );
        assert_eq!(update(HANS, &installed, None).1.unwrap(), "从槽位 A 启动新系统后将自动合并", "the boot control could not tell");
        assert_eq!(update(Lang::En, &installed, Some(Slot::A)).1.unwrap(), "After a reboot the new system starts from slot A, and merges once it has started");
    }

    #[test]
    fn a_speed_is_written_to_three_figures_over_its_unit() {
        let s = |bytes| {
            let (figures, unit) = speed(bytes);
            format!("{figures} {unit}")
        };
        // As the 17 Ultra's status bar showed slow ones.
        assert_eq!(s(0), "0.00 KB/s");
        assert_eq!(s(61), "0.06 KB/s");
        assert_eq!(s(7188), "7.02 KB/s");
        assert_eq!(s(12_800), "12.5 KB/s");
        assert_eq!(s(512_000), "500 KB/s");
        assert_eq!(s(1_023_000), "999 KB/s");
        assert_eq!(s(1_024_000), "0.98 MB/s", "past 999 KB/s the unit grows");
        assert_eq!(s(42_772_090), "40.8 MB/s");
    }

    #[test]
    fn a_network_short_of_the_internet_says_how_and_what_is_left_to_do() {
        use std::net::Ipv4Addr;
        let joined = |reach: Option<Reach>| {
            WlanStage::Connected(skyward_net::wlan::Link {
                address: Ipv4Addr::new(192, 168, 72, 76),
                mask: Ipv4Addr::new(255, 255, 240, 0),
                router: Some(Ipv4Addr::new(192, 168, 64, 254)),
                dns: vec![Ipv4Addr::new(119, 29, 29, 29)],
                signal: -52,
                frequency: 5745,
                speed: None,
                generation: None,
                mac: "02:00:00:00:00:05".into(),
                reach,
            })
        };
        assert_eq!(joining(HANS, &joined(None)), "已连接", "until it has been looked at");
        assert_eq!(joining(HANS, &joined(Some(Reach::Online))), "已连接");
        assert_eq!(joining(HANS, &joined(Some(Reach::Login(None)))), "需要登录");
        assert_eq!(joining(Lang::En, &joined(Some(Reach::Login(None)))), "Sign-in required");
        assert_eq!(joining(HANS, &joined(Some(Reach::Offline("no answer in 5 s".into())))), "已连接，但无法访问互联网");
        assert_eq!(reach(HANS, &Reach::Online), None, "the page says nothing of a network that reaches the internet");
        let login = reach(HANS, &Reach::Login(Some("http://192.168.64.254:2060/redirect/".into()))).expect("a login is said");
        assert_eq!(login, "该网络需要先在网页上登录才能访问互联网，Skyward 无法打开网页登录。可以改用无需登录的网络，例如手机热点。");
        assert_eq!(reach(HANS, &Reach::Offline("no answer in 5 s".into())).as_deref(), Some("该网络无法访问互联网：no answer in 5 s"));
        assert!(unanswered(HANS, &Unanswered::Login).starts_with("检查更新失败，当前 WLAN 需要先在网页上登录"));
        assert!(unanswered(Lang::En, &Unanswered::Login).starts_with("Couldn't check for updates. The Wi-Fi network joined needs a sign-in in a browser"));
        assert_eq!(unanswered(HANS, &Unanswered::Offline("could not reach connect.rom.miui.com".into())), "检查更新失败，当前 WLAN 无法访问互联网：could not reach connect.rom.miui.com");
        assert_eq!(unanswered(HANS, &Unanswered::Failed("io: received fatal alert: HandshakeFailure".into())), "检查更新失败：io: received fatal alert: HandshakeFailure", "a network that reaches the internet leaves how asking failed");
    }

    #[test]
    fn a_shell_tells_how_it_ended() {
        assert_eq!(term_end(HANS, &TermEnd::Exited(0)), "shell 已退出，退出码 0");
        assert_eq!(term_end(Lang::Hant, &TermEnd::Signalled(9)), "shell 已被訊號 9 終止");
        assert_eq!(term_end(Lang::En, &TermEnd::Exited(127)), "The shell exited with code 127");
    }

    #[test]
    fn every_language_writes_a_date_in_its_own_order_with_its_own_month() {
        // The Xiaomi 13 Ultra's installed system, by its ro.build.date.utc.
        let built = 1_778_243_655;
        assert_eq!(date(Lang::Hant, built, china()), "2026年5月8日");
        assert_eq!(date(Lang::Tr, built, china()), "8 Mayıs 2026");
        assert_eq!(date(Lang::Ru, built, china()), "8 мая 2026 г.");
        assert_eq!(date(Lang::De, built, china()), "8. Mai 2026");
        assert_eq!(date(Lang::Fr, built, china()), "8 mai 2026");
        assert_eq!(date(Lang::Pl, built, china()), "8 maja 2026");
        assert_eq!(date(Lang::Ja, built, china()), "2026年5月8日");
        for l in Lang::ALL {
            let written = date(l, built, china());
            assert!(written.contains("2026") && written.contains('8'), "{l:?} writes the year and the day: {written}");
            let timed = datetime(l, built, china());
            assert!(timed.contains(&written) && timed.contains(&clock(built, china())), "{l:?} writes the time with the date: {timed}");
        }
    }

    #[test]
    fn a_count_takes_the_form_its_language_gives_the_number() {
        let items = |l, n| texts::files::line::items(l, n);
        assert_eq!([items(Lang::En, 1), items(Lang::En, 2)], ["1 item", "2 items"]);
        assert_eq!([items(Lang::Fr, 0), items(Lang::Fr, 1), items(Lang::Fr, 2)], ["0 élément", "1 élément", "2 éléments"], "none is one in French");
        let russian = [1, 2, 5, 11, 21, 22, 25, 111].map(|n| items(Lang::Ru, n));
        assert_eq!(russian, ["1 объект", "2 объекта", "5 объектов", "11 объектов", "21 объект", "22 объекта", "25 объектов", "111 объектов"]);
        let polish = [1, 2, 5, 12, 21, 22].map(|n| items(Lang::Pl, n));
        assert_eq!(polish, ["1 element", "2 elementy", "5 elementów", "12 elementów", "21 elementów", "22 elementy"]);
        assert_eq!([items(Lang::Ja, 1), items(Lang::Ja, 68)], ["1 項目", "68 項目"], "one form where the language has one");
        assert_eq!([items(HANS, 1), items(HANS, 68)], ["1 项", "68 项"]);
    }

    #[test]
    fn a_row_of_names_is_set_and_ended_as_its_language_does() {
        assert_eq!(list(Lang::De, ["ART-Cache", "App-Daten"]), "ART-Cache, App-Daten", "German keeps its nouns' capitals");
        assert_eq!(list(Lang::Ru, ["Кэш ART", "Данные приложений"]), "Кэш ART, данные приложений");
        assert_eq!(list(Lang::Tr, ["ART önbelleği", "İndirilenler", "Isı kayıtları"]), "ART önbelleği, indirilenler, ısı kayıtları", "by Turkish's own small letters");
        assert_eq!(list(Lang::Ja, ["ART キャッシュ", "アプリのデータ"]), "ART キャッシュ、アプリのデータ");
        assert_eq!(users(Lang::Tr, &[0, 10]), "cihaz sahibi ve kullanıcı 10");
        assert_eq!(users(Lang::Ru, &[0, 10, 999]), "владелец, пользователь 10 и пользователь 999");
        assert_eq!(users(Lang::Fr, &[10]), texts::users::user(Lang::Fr, 10));
    }
}
