use crate::app::{
    BackupRefusal, Data, EraseStop, FileKind, FileRow, ItemEnd, MountFailure, Part, PartKind, PartStop, Place, Source, TermEnd, BACKUP_PARTS,
    TIME_ZONES,
};
use crate::lang::{Lang, Text};
use crate::tr;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::Timestamp;
use skyward_info::{Battery, Charge, Cpu, PatchLevel, Storage};
use skyward_install::ota::Trouble;
use skyward_install::{Failure, ImageKind, Ramdisk, Refusal};
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

/// A count in Chinese numerals, a pair as 双, or 雙 in traditional
/// characters.
fn numeral(n: u32, traditional: bool) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match n {
        2 => (if traditional { "雙" } else { "双" }).into(),
        0..=9 => D[n as usize].into(),
        10 => "十".into(),
        11..=19 => format!("十{}", D[(n % 10) as usize]),
        20..=99 if n.is_multiple_of(10) => format!("{}十", D[(n / 10) as usize]),
        20..=99 => format!("{}十{}", D[(n / 10) as usize], D[(n % 10) as usize]),
        _ => n.to_string(),
    }
}

/// `八核 最高 3.19GHz`, `Octa-core, up to 3.19GHz`
pub fn cpu(l: Lang, c: &Cpu) -> String {
    let cores = match (l, c.cores) {
        (Lang::Hans, n) => format!("{}核", numeral(n, false)),
        (Lang::Hant, n) => format!("{}核", numeral(n, true)),
        (Lang::En, 1) => "Single-core".into(),
        (Lang::En, 2) => "Dual-core".into(),
        (Lang::En, 4) => "Quad-core".into(),
        (Lang::En, 6) => "Hexa-core".into(),
        (Lang::En, 8) => "Octa-core".into(),
        (Lang::En, n) => format!("{n}-core"),
    };
    match c.max_khz {
        Some(khz) => {
            let ghz = format!("{:.2}", khz as f64 / 1e6);
            tr!(l, "{cores} 最高 {ghz}GHz", "{cores} 最高 {ghz}GHz", "{cores}, up to {ghz}GHz")
        }
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
        Charge::Charging => ["充电中", "充電中", "Charging"],
        Charge::Discharging | Charge::NotCharging => ["未充电", "未充電", "Not charging"],
        Charge::Full => ["已充满", "已充滿", "Full"],
        Charge::Unknown => ["状态未知", "狀態不明", "Unknown"],
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
        "Good" => ["良好", "良好", "Good"],
        "Overheat" | "Hot" => ["过热", "過熱", "Overheated"],
        "Warm" => ["偏热", "偏熱", "Warm"],
        "Cool" => ["偏冷", "偏冷", "Cool"],
        "Cold" => ["过冷", "過冷", "Cold"],
        "Dead" => ["损坏", "損壞", "Dead"],
        "Over voltage" => ["电压过高", "電壓過高", "Over voltage"],
        "Over current" => ["电流过大", "電流過大", "Over current"],
        "Unspecified failure" => ["故障", "故障", "Failure"],
        "Calibration required" => ["需要校准", "需要校正", "Needs calibration"],
        "No battery" => ["未检测到电池", "未偵測到電池", "No battery"],
        "Unknown" => ["未知", "不明", "Unknown"],
        other => return other.to_string(),
    };
    l.pick(text).to_string()
}

/// Health and remaining capacity against design, e.g. `良好 · 剩余容量 93%`.
pub fn wear(l: Lang, b: &Battery) -> Option<String> {
    let state = b.health.as_deref().map(|h| health(l, h));
    let left = match (b.full, b.design) {
        (Some(full), Some(design)) if design > 0 => {
            let share = (full * 100 + design / 2) / design;
            Some(tr!(l, "剩余容量 {share}%", "剩餘容量 {share}%", "Maximum capacity {share}%"))
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
        Some(p) => {
            let date = patch(l, p);
            tr!(l, "{release} | Android 安全更新：{date}", "{release} | Android 安全性更新：{date}", "{release} | Android security update: {date}")
        }
        None => release.to_string(),
    }
}

pub fn verified(l: Lang, state: &str) -> String {
    let text = match state {
        "green" => ["绿色 · 已通过官方密钥验证", "綠色 · 已通過官方金鑰驗證", "Green · Verified with the official key"],
        "yellow" => ["黄色 · 已通过自定义密钥验证", "黃色 · 已通過自訂金鑰驗證", "Yellow · Verified with a custom key"],
        "orange" => ["橙色 · 设备已解锁，不验证系统", "橙色 · 裝置已解鎖，不驗證系統", "Orange · Unlocked, so the system isn't verified"],
        "red" => ["红色 · 验证失败", "紅色 · 驗證失敗", "Red · Verification failed"],
        other => return other.to_string(),
    };
    l.pick(text).to_string()
}

pub fn snapshot(l: Lang, s: &SnapshotState) -> &'static str {
    l.pick(match s {
        SnapshotState::None => ["暂无待合并的更新", "沒有待合併的更新", "No update waiting to merge"],
        SnapshotState::Snapshotted => ["更新已写入，等待合并", "更新已寫入，等待合併", "Update written, waiting to merge"],
        SnapshotState::Merging => ["正在合并更新", "正在合併更新", "Merging the update"],
        SnapshotState::MergeFailed => ["更新合并失败", "更新合併失敗", "The update failed to merge"],
        SnapshotState::Unknown => ["无法读取更新状态", "無法讀取更新狀態", "Couldn't read the update's status"],
    })
}

/// Why super is off limits while an update is in flight; `None` when it is not.
pub fn snapshot_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => {
            ["更新合并完成前，暂不刷写 super 中的分区", "更新合併完成前，暫不刷寫 super 中的分割區", "Partitions in super aren't flashed until the update merges"]
        }
        SnapshotState::MergeFailed => [
            "在系统中处理完成前，暂不刷写 super 中的分区",
            "在系統中處理完成前，暫不刷寫 super 中的分割區",
            "Partitions in super aren't flashed until the system deals with the update",
        ],
        SnapshotState::Unknown => [
            "读取到更新状态前，暂不刷写 super 中的分区",
            "讀取到更新狀態前，暫不刷寫 super 中的分割區",
            "Partitions in super aren't flashed until the update's status can be read",
        ],
    };
    Some(l.pick(text))
}

/// The home page's warning of a battery too low to install on: how low it
/// is, and what installing waits for.
pub fn battery_low(l: Lang, level: u8, charging: bool) -> (String, String) {
    let least = skyward_install::least_charge(charging);
    (
        tr!(l, "电量仅剩 {level}%", "電量僅剩 {level}%", "Battery at {level}%"),
        tr!(l, "充电至 {least}% 以上才能安装", "充電至 {least}% 以上才能安裝", "Charge it past {least}% to install"),
    )
}

/// The home page's word that the phone restarts from the slot it does not
/// run from now.
pub fn next_boot(l: Lang, next: Slot, now: Slot) -> (String, String) {
    let (next, now) = (next.letter(), now.letter());
    (
        tr!(l, "重启后从 {next} 槽位启动", "重新啟動後從 {next} 槽位啟動", "Restarts from slot {next}"),
        tr!(l, "当前运行的是 {now} 槽位", "目前執行的是 {now} 槽位", "Running from slot {now} now"),
    )
}

/// Why the slot to start next stays as an update in flight set it; `None`
/// when none is.
pub fn snapshot_slot_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => {
            ["更新合并完成前，暂不切换启动槽位。", "更新合併完成前，暫不切換啟動槽位。", "The boot slot isn't switched until the update merges."]
        }
        SnapshotState::MergeFailed => {
            ["在系统中处理完成前，暂不切换启动槽位。", "在系統中處理完成前，暫不切換啟動槽位。", "The boot slot isn't switched until the system deals with the update."]
        }
        SnapshotState::Unknown => {
            ["读取到更新状态前，暂不切换启动槽位。", "讀取到更新狀態前，暫不切換啟動槽位。", "The boot slot isn't switched until the update's status can be read."]
        }
    };
    Some(l.pick(text))
}

/// What the recovery says as it starts, of a switch between the systems
/// that a restart or a loss of power cut off: finished, or why not.
pub fn switch_resumed(l: Lang) -> &'static str {
    l.pick(["已完成上次中断的系统切换", "已完成上次中斷的系統切換", "The switch of systems cut off last time is finished"])
}

pub fn switch_unfinished(l: Lang, why: &str) -> String {
    tr!(l, "上次中断的系统切换未能完成：{why}", "上次中斷的系統切換未能完成：{why}", "The switch of systems cut off last time couldn't be finished: {why}")
}

/// Why the second system does not start yet while an update is in flight,
/// which the main system has to start once more to merge; `None` when none
/// is.
pub fn snapshot_system_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => [
            "系统更新合并完成前，暂不切换到第二系统。请先启动主系统，等待更新合并完成。",
            "系統更新合併完成前，暫不切換到第二系統。請先啟動主系統，等待更新合併完成。",
            "The second system doesn't start until the system update merges. Start the main system first, and let the update merge.",
        ],
        SnapshotState::MergeFailed => [
            "在主系统中处理完成更新前，暂不切换到第二系统。",
            "在主系統中處理完成更新前，暫不切換到第二系統。",
            "The second system doesn't start until the main system deals with the update.",
        ],
        SnapshotState::Unknown => [
            "读取到更新状态前，暂不切换到第二系统。",
            "讀取到更新狀態前，暫不切換到第二系統。",
            "The second system doesn't start until the update's status can be read.",
        ],
    };
    Some(l.pick(text))
}

/// What an install script that writes to super does to an update in
/// flight; `None` when none is known to be.
pub fn snapshot_script_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None | SnapshotState::Unknown => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => [
            "系统更新尚未完成合并。刷机包改写 super 中的分区，会导致这次更新无法完成。",
            "系統更新尚未完成合併。刷機包改寫 super 中的分割區，會導致這次更新無法完成。",
            "A system update hasn't finished merging. A flashable zip that rewrites partitions in super keeps it from ever finishing.",
        ],
        SnapshotState::MergeFailed => [
            "系统更新合并失败，需在系统中处理。刷机包改写 super 中的分区，会导致这次更新无法完成。",
            "系統更新合併失敗，需在系統中處理。刷機包改寫 super 中的分割區，會導致這次更新無法完成。",
            "A system update failed to merge, and the system has to deal with it. A flashable zip that rewrites partitions in super keeps it from ever finishing.",
        ],
    };
    Some(l.pick(text))
}

/// Why the slot's partitions in super are not backed up while an update is
/// in flight; `None` when none is.
pub fn snapshot_backup_note(l: Lang, s: &SnapshotState) -> Option<&'static str> {
    let text = match s {
        SnapshotState::None => return None,
        SnapshotState::Snapshotted | SnapshotState::Merging => {
            ["更新合并完成前，暂不备份系统分区", "更新合併完成前，暫不備份系統分割區", "System partitions aren't backed up until the update merges"]
        }
        SnapshotState::MergeFailed => {
            ["在系统中处理完成前，暂不备份系统分区", "在系統中處理完成前，暫不備份系統分割區", "System partitions aren't backed up until the system deals with the update"]
        }
        SnapshotState::Unknown => {
            ["读取到更新状态前，暂不备份系统分区", "讀取到更新狀態前，暫不備份系統分割區", "System partitions aren't backed up until the update's status can be read"]
        }
    };
    Some(l.pick(text))
}

/// Why backing up, or restoring, did not start.
pub fn backup_refusal(l: Lang, r: &BackupRefusal, restore: bool) -> String {
    match r {
        BackupRefusal::Room { needed, free } => {
            let (needed, free) = (gb(*needed), gb(*free));
            tr!(
                l,
                "存储空间不足。备份需要 {needed}，可用空间为 {free}。",
                "儲存空間不足。備份需要 {needed}，可用空間為 {free}。",
                "Not enough storage. The backup needs {needed}, and {free} is free."
            )
        }
        BackupRefusal::Update(state) => {
            let what = l.pick(match restore {
                true => ["恢复启动分区和系统分区", "還原啟動分割區和系統分割區", "restored"],
                false => ["备份系统分区", "備份系統分割區", "backed up"],
            });
            let parts = if restore { "The boot and system partitions" } else { "System partitions" };
            match state {
                SnapshotState::Snapshotted | SnapshotState::Merging => tr!(
                    l,
                    "更新合并完成前，暂不{what}。你可以在「高级 > 系统更新」中查看。",
                    "更新合併完成前，暫不{what}。你可以在「進階 > 系統更新」中查看。",
                    "{parts} aren't {what} until the update merges. You can see it in Advanced > System update."
                ),
                SnapshotState::MergeFailed => tr!(
                    l,
                    "更新合并失败。在系统中处理完成前，暂不{what}。",
                    "更新合併失敗。在系統中處理完成前，暫不{what}。",
                    "The update failed to merge. {parts} aren't {what} until the system deals with it."
                ),
                SnapshotState::Unknown => tr!(
                    l,
                    "无法读取更新状态，暂不{what}。",
                    "無法讀取更新狀態，暫不{what}。",
                    "{parts} aren't {what} while the update's status can't be read."
                ),
                SnapshotState::None => unreachable!("no update in flight holds anything back"),
            }
        }
        BackupRefusal::OtherPhone(device) => tr!(
            l,
            "该备份来自其他设备（{device}），无法恢复到本机。",
            "這份備份來自其他裝置（{device}），無法還原到本機。",
            "This backup is from another device ({device}) and can't be restored to this phone."
        ),
        BackupRefusal::Resized(name) => tr!(
            l,
            "{name} 的大小与备份时不同，系统可能已更新。该备份中的系统分区无法恢复到当前系统。",
            "{name} 的大小與備份時不同，系統可能已更新。這份備份中的系統分割區無法還原到目前的系統。",
            "{name} isn't the size it was at the backup, so the system may have been updated. The backup's system partitions can't be restored to the current system."
        ),
        BackupRefusal::DataReset => l
            .pick([
                "备份后 data 分区已重新创建，例如恢复过出厂设置，该备份中的 data 分区无法恢复。",
                "備份後 data 分割區已重新建立，例如曾恢復原廠設定，這份備份中的 data 分割區無法還原。",
                "The data partition was made anew after the backup, as a factory reset does, so the backup's data partition can't be restored.",
            ])
            .into(),
    }
}

/// What the update page says of an update standing so, and the line under
/// it on what comes next; `next` is the slot the bootloader starts next,
/// where the boot control could tell.
pub fn update(l: Lang, s: &Standing, next: Option<Slot>) -> (&'static str, Option<String>) {
    let line = |text: Text| Some(l.pick(text).to_string());
    match s {
        Standing::None => (l.pick(["暂无待合并的更新", "沒有待合併的更新", "No update waiting to merge"]), None),
        Standing::Unknown => (
            l.pick(["无法读取更新状态", "無法讀取更新狀態", "Couldn't read the update's status"]),
            line(["无法读取 /metadata 中的更新记录", "無法讀取 /metadata 中的更新記錄", "Couldn't read the update's records in /metadata"]),
        ),
        Standing::Writing => (
            l.pick(["更新未写入完成", "更新尚未寫入完成", "The update isn't fully written"]),
            line(["启动系统后将继续安装", "啟動系統後將繼續安裝", "Installing goes on once the system starts"]),
        ),
        Standing::Installed(installed) => {
            let slot = installed.letter();
            let detail = match next {
                Some(next) if next == *installed => tr!(
                    l,
                    "重启后将从槽位 {slot} 启动新系统，启动后自动合并",
                    "重新啟動後將從槽位 {slot} 啟動新系統，啟動後自動合併",
                    "After a reboot the new system starts from slot {slot}, and merges once it has started"
                ),
                Some(_) => tr!(
                    l,
                    "需在系统的「系统更新」中重启，才会从槽位 {slot} 启动新系统",
                    "需在系統的「系統更新」中重新啟動，才會從槽位 {slot} 啟動新系統",
                    "Reboot from the system's System update to start slot {slot}'s new system"
                ),
                None => tr!(
                    l,
                    "从槽位 {slot} 启动新系统后将自动合并",
                    "從槽位 {slot} 啟動新系統後將自動合併",
                    "Merges once slot {slot}'s new system starts"
                ),
            };
            (l.pick(["更新已安装", "更新已安裝", "Update installed"]), Some(detail))
        }
        Standing::Waiting => (
            l.pick(["更新等待合并", "更新等待合併", "Update waiting to merge"]),
            line(["启动新系统后将自动合并，也可以在此合并", "啟動新系統後將自動合併，也可以在此合併", "Merges once the new system starts, or can be merged here"]),
        ),
        Standing::Compressed => (
            l.pick(["更新等待合并", "更新等待合併", "Update waiting to merge"]),
            line(["此类更新需在系统中合并，启动系统后将自动合并", "這類更新需在系統中合併，啟動系統後將自動合併", "An update of this kind merges in the system, once it starts"]),
        ),
        Standing::Merging(_) => (l.pick(["正在合并更新", "正在合併更新", "Merging the update"]), None),
        Standing::Failed(code) => (
            l.pick(["更新合并失败", "更新合併失敗", "The update failed to merge"]),
            Some(tr!(l, "重启后将继续合并（{code:?}）", "重新啟動後將繼續合併（{code:?}）", "Merging goes on after a reboot ({code:?})")),
        ),
        Standing::Merged => (
            l.pick(["更新已合并", "更新已合併", "Update merged"]),
            line(["下次启动系统时将清理更新留下的文件", "下次啟動系統時將清除更新留下的檔案", "The files the update left are cleared when the system next starts"]),
        ),
    }
}

/// Where an update standing so is, beside the entry of its page.
pub fn update_value(l: Lang, s: &Standing) -> &'static str {
    l.pick(match s {
        Standing::None => ["暂无", "無", "None"],
        Standing::Unknown => ["无法读取", "無法讀取", "Unreadable"],
        Standing::Writing => ["未写入完成", "尚未寫入完成", "Not fully written"],
        Standing::Installed(_) => ["已安装", "已安裝", "Installed"],
        Standing::Waiting | Standing::Compressed => ["等待合并", "等待合併", "Waiting to merge"],
        Standing::Merging(_) => ["正在合并", "正在合併", "Merging"],
        Standing::Failed(_) => ["合并失败", "合併失敗", "Merge failed"],
        Standing::Merged => ["已合并", "已合併", "Merged"],
    })
}

pub fn data(l: Lang, d: &Data) -> &'static str {
    l.pick(match d {
        Data::Closed | Data::Opening | Data::Failed(_) => ["未挂载", "未掛載", "Not mounted"],
        Data::Locked { .. } => ["未解密", "未解密", "Locked"],
        Data::Open => ["已解密", "已解密", "Decrypted"],
        Data::Erased => ["已清除", "已清除", "Wiped"],
    })
}

/// A part of the phone erasing goes through, as its page names it.
pub fn part(l: Lang, p: Part) -> &'static str {
    l.pick(match p {
        Part::Data => ["data 分区", "data 分割區", "data partition"],
        Part::Cache => ["cache 分区", "cache 分割區", "cache partition"],
        Part::Metadata => ["metadata 分区", "metadata 分割區", "metadata partition"],
        Part::AppData => ["应用数据", "應用程式資料", "App data"],
        Part::Media => ["内部存储", "內部儲存空間", "Internal storage"],
        Part::AppCaches => ["应用缓存", "應用程式快取", "App caches"],
        Part::SystemCaches => ["系统缓存", "系統快取", "System caches"],
        Part::Art => ["ART 缓存", "ART 快取", "ART cache"],
        Part::Logs => ["日志", "記錄", "Logs"],
        Part::Updates => ["下载的更新包", "下載的更新檔", "Downloaded updates"],
    })
}

/// A part of the phone erasing goes through, as an English sentence names
/// it.
fn part_in_sentence(p: Part) -> &'static str {
    match p {
        Part::Data => "the data partition",
        Part::Cache => "the cache partition",
        Part::Metadata => "the metadata partition",
        Part::AppData => "app data",
        Part::Media => "internal storage",
        Part::AppCaches => "app caches",
        Part::SystemCaches => "system caches",
        Part::Art => "the ART cache",
        Part::Logs => "the logs",
        Part::Updates => "the downloaded updates",
    }
}

/// `before`, `inner` and `after` joined, with a space between Chinese and
/// the Latin letters or digits at either end of `inner`, as the pages write
/// them: 清除 ART 缓存失败.
pub(crate) fn spaced(before: &str, inner: &str, after: &str) -> String {
    let open = if inner.starts_with(|c: char| c.is_ascii()) && !before.ends_with(|c: char| c.is_ascii()) { " " } else { "" };
    let close = if inner.ends_with(|c: char| c.is_ascii()) && !after.starts_with(|c: char| c.is_ascii()) { " " } else { "" };
    format!("{before}{open}{inner}{close}{after}")
}

/// A name that opens an English title, as it reads within a sentence: its
/// first letter small, unless the word is written all in capitals.
fn in_sentence(name: &str) -> String {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && !second.is_uppercase() => {
            first.to_lowercase().chain(name[first.len_utf8()..].chars()).collect()
        }
        _ => name.to_string(),
    }
}

/// Things named in a row: in Chinese with 、 between them, in English with
/// commas, each after the first as a sentence names it.
pub fn list<'a>(l: Lang, names: impl IntoIterator<Item = &'a str>) -> String {
    let names: Vec<String> = names.into_iter().enumerate().map(|(k, n)| if l == Lang::En && k > 0 { in_sentence(n) } else { n.to_string() }).collect();
    names.join(l.pick(["、", "、", ", "]))
}

/// Users by how the pages call them: the owner, then the others by number,
/// as「机主、用户 999」, or as an English sentence names them, as "the owner
/// and user 999".
pub fn users(l: Lang, users: &[u32]) -> String {
    let names: Vec<String> = users
        .iter()
        .map(|&u| match u {
            0 => l.pick(["机主", "擁有者", "the owner"]).to_string(),
            u => tr!(l, "用户 {u}", "使用者 {u}", "user {u}"),
        })
        .collect();
    match (l, names.as_slice()) {
        (Lang::En, [.., last]) if names.len() > 1 => format!("{} and {last}", names[..names.len() - 1].join(", ")),
        (Lang::En, _) => names.join(""),
        _ => names.join("、"),
    }
}

/// Why erasing did not start or stopped short, and what the system said of
/// it when there is that.
pub fn erase_stop(l: Lang, s: &EraseStop) -> (String, Option<String>) {
    let said = |text: Text| l.pick(text).to_string();
    match s {
        EraseStop::Update(Hold::Unconfirmed) => (
            said([
                "当前槽位的系统更新尚未完成，暂无法清除。你可以在「高级 > 系统更新」中合并后重试。",
                "目前槽位的系統更新尚未完成，暫時無法清除。你可以在「進階 > 系統更新」中合併後重試。",
                "The current slot's system update hasn't finished, so nothing can be wiped yet. Merge it in Advanced > System update, then try again.",
            ]),
            None,
        ),
        EraseStop::Update(Hold::Merging) => (
            said([
                "系统更新正在合并，暂无法清除。你可以在「高级 > 系统更新」中查看，合并完成后重试。",
                "系統更新正在合併，暫時無法清除。你可以在「進階 > 系統更新」中查看，合併完成後重試。",
                "A system update is merging, so nothing can be wiped yet. You can see it in Advanced > System update; try again once it has merged.",
            ]),
            None,
        ),
        EraseStop::Update(Hold::MergeFailed) => (
            said([
                "系统更新合并失败，暂无法清除。请先启动系统处理后重试。",
                "系統更新合併失敗，暫時無法清除。請先啟動系統處理後重試。",
                "A system update failed to merge, so nothing can be wiped yet. Start the system to deal with it, then try again.",
            ]),
            None,
        ),
        EraseStop::NotRebuilt => (
            said([
                "本机系统开机时不会重新创建已清空的 data 分区，无法清除。",
                "本機系統開機時不會重新建立已清空的 data 分割區，無法清除。",
                "This phone's system doesn't make an emptied data partition anew when it starts, so it can't be wiped.",
            ]),
            None,
        ),
        EraseStop::NoCache => (said(["本机没有 cache 分区。", "本機沒有 cache 分割區。", "This phone has no cache partition."]), None),
        EraseStop::Erased => (
            said([
                "data 分区已清空，下次开机时将重新创建，暂无可清除的内容。",
                "data 分割區已清空，下次開機時將重新建立，目前沒有可清除的內容。",
                "The data partition is empty and will be made anew when the phone next starts. There's nothing to wipe.",
            ]),
            None,
        ),
        EraseStop::Closed(why) => (
            said(["无法打开 data 分区，未清除任何内容。", "無法開啟 data 分割區，未清除任何內容。", "Couldn't open the data partition, so nothing was wiped."]),
            Some(why.clone()),
        ),
        EraseStop::Locked(locked) => {
            let who = users(l, locked);
            let what = match l {
                Lang::Hans => spaced("", &who, "的数据尚未解密，未清除任何内容。请先在「解密」中输入锁屏密码后重试。"),
                Lang::Hant => spaced("", &who, "的資料尚未解密，未清除任何內容。請先在「解密」中輸入螢幕鎖定密碼後重試。"),
                Lang::En => format!("The data of {who} isn't decrypted, so nothing was wiped. Enter the screen lock in Decrypt, then try again."),
            };
            (what, None)
        }
        EraseStop::Failed(p, why) => {
            let what = match l {
                Lang::Hans => spaced("清除", part(l, *p), "失败。"),
                Lang::Hant => spaced("清除", part(l, *p), "失敗。"),
                Lang::En => format!("Couldn't wipe {}.", part_in_sentence(*p)),
            };
            (what, Some(why.clone()))
        }
    }
}

/// How work on whole partitions of a kind is told of: the Chinese verb its
/// sentences are built on, in simplified and in traditional characters,
/// and English's own words for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verb {
    hans: &'static str,
    hant: &'static str,
    /// Under way: Repairing.
    going: &'static str,
    /// Done to a partition: repaired.
    done: &'static str,
    /// The work as a thing: Repair, as in Repair failed.
    noun: &'static str,
    /// The verb itself: repair, as in Couldn't repair vendor.
    base: &'static str,
}

impl Verb {
    pub fn of(kind: &PartKind) -> Verb {
        let (hans, hant, going, done, noun, base) = match kind {
            PartKind::Disable => ("清除", "清除", "Wiping", "wiped", "Wipe", "wipe"),
            PartKind::Unshare => ("处理", "處理", "Unsharing", "unshared", "Unsharing", "unshare"),
            PartKind::Repair => ("修复", "修復", "Repairing", "repaired", "Repair", "repair"),
            PartKind::Remake(_) => ("格式化", "格式化", "Formatting", "formatted", "Format", "format"),
        };
        Verb { hans, hant, going, done, noun, base }
    }

    /// Under way: 正在修复, Repairing.
    pub fn going(self, l: Lang) -> String {
        let (hans, hant, going) = (self.hans, self.hant, self.going);
        tr!(l, "正在{hans}", "正在{hant}", "{going}")
    }

    /// Ended, with every partition done: 修复完成, Repair complete.
    pub fn complete(self, l: Lang) -> String {
        let (hans, hant, noun) = (self.hans, self.hant, self.noun);
        tr!(l, "{hans}完成", "{hant}完成", "{noun} complete")
    }

    /// Ended partway through the partitions: 清除未完成, Wipe incomplete.
    pub fn incomplete(self, l: Lang) -> String {
        let (hans, hant, noun) = (self.hans, self.hant, self.noun);
        tr!(l, "{hans}未完成", "{hant}未完成", "{noun} incomplete")
    }

    /// Gone wrong: 修复失败, Repair failed.
    pub fn failed(self, l: Lang) -> String {
        let (hans, hant, noun) = (self.hans, self.hant, self.noun);
        tr!(l, "{hans}失败", "{hant}失敗", "{noun} failed")
    }

    /// A partition the work ended before: 未修复, Not repaired.
    pub fn not_done(self, l: Lang) -> String {
        let (hans, hant, done) = (self.hans, self.hant, self.done);
        tr!(l, "未{hans}", "未{hant}", "Not {done}")
    }

    /// What the page asks while the work goes on: 修复完成前，请不要关机。
    pub fn hold_on(self, l: Lang) -> String {
        let (hans, hant) = (self.hans, self.hant);
        tr!(l, "{hans}完成前，请不要关机。", "{hant}完成前，請不要關機。", "Don't power off until it's done.")
    }

    /// What could not be done to the partition `name`: 修复 vendor 失败,
    /// Couldn't repair vendor.
    pub fn failed_on(self, l: Lang, name: &str) -> String {
        match l {
            Lang::Hans => spaced(self.hans, name, "失败"),
            Lang::Hant => spaced(self.hant, name, "失敗"),
            Lang::En => format!("Couldn't {} {name}", self.base),
        }
    }
}

/// How a partition's work ended, as its row says it, `verb` telling of the
/// work: 已修复, 修复失败.
pub fn item_end(l: Lang, end: &ItemEnd, verb: Verb) -> String {
    let text = match end {
        ItemEnd::Erased => ["已清除", "已清除", "Wiped"],
        ItemEnd::Unshared => ["已处理", "已處理", "Unshared"],
        ItemEnd::NotShared(_) => ["无需处理", "無需處理", "Nothing to do"],
        ItemEnd::Clean => ["未发现错误", "未發現錯誤", "No errors found"],
        ItemEnd::Fixed => ["已修复", "已修復", "Repaired"],
        ItemEnd::Made => ["已格式化", "已格式化", "Formatted"],
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
        PartStop::Unmounted(base) => (
            tr!(l, "请先在「挂载」中挂载 {base} 分区后重试。", "請先在「掛載」中掛載 {base} 分割區後重試。", "Mount the {base} partition in Mount, then try again."),
            None,
        ),
        PartStop::Failed(why) => (l.pick(UNEXPECTED).into(), Some(why.clone())),
    }
}

/// What the pages say of an error they have no words of their own for,
/// above what went wrong.
const UNEXPECTED: Text = ["发生意外错误。", "發生未預期的錯誤。", "Something went wrong."];

/// A file size, `7.1GB` or `96.0MB`.
pub fn bytes(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}GB", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}MB", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}KB", n as f64 / 1e3),
        n => format!("{n}B"),
    }
}

/// The months, as English names them.
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A day of the calendar as the language writes it: `2026年5月8日`,
/// `May 8, 2026`.
fn day(l: Lang, year: i32, month: i32, d: i32) -> String {
    let name = MONTHS[(month - 1) as usize];
    tr!(l, "{year}年{month}月{d}日", "{year}年{month}月{d}日", "{name} {d}, {year}")
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
        TIME_ZONES.iter().map(|&(_, _, name)| TimeZone::get(name).expect("the recovery carries the rules of every zone listed")).collect()
    });
    &all[zone]
}

/// A Unix time on the calendar and clock of the zone at `zone` in
/// [`TIME_ZONES`]. A time past either end of the calendar, as a damaged
/// file's can be, is taken as that end.
fn local(unix: i64, zone: usize) -> DateTime {
    let unix = unix.clamp(Timestamp::MIN.as_second(), Timestamp::MAX.as_second());
    let at = Timestamp::from_second(unix).expect("a time within the calendar");
    at.to_zoned(rules(zone).clone()).datetime()
}

/// The day a Unix time falls on in the zone at `zone` in [`TIME_ZONES`],
/// `2026年5月8日`.
pub fn date(l: Lang, unix: i64, zone: usize) -> String {
    let d = local(unix, zone);
    day(l, d.year().into(), d.month().into(), d.day().into())
}

/// When a list entry last changed, as the stock file manager tells it
/// against `now`, when the folder was read: the time for today, month and
/// day for a file from this year, and the whole date for a folder from this
/// year or anything older; all in the zone at `zone` in [`TIME_ZONES`].
pub fn file_time(unix: i64, folder: bool, now: i64, zone: usize) -> String {
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

/// A Unix time in the zone at `zone` in [`TIME_ZONES`], to the minute:
/// `2026年10月19日 02:31`, `October 19, 2026, 02:31`.
pub fn datetime(l: Lang, unix: i64, zone: usize) -> String {
    let (date, clock) = (date(l, unix, zone), clock(unix, zone));
    tr!(l, "{date} {clock}", "{date} {clock}", "{date}, {clock}")
}

/// The time of day in the zone at `zone` in [`TIME_ZONES`], `HH:MM`, as
/// the status bar shows it.
pub fn clock(unix: i64, zone: usize) -> String {
    let t = local(unix, zone);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// A Unix time in the zone at `zone` in [`TIME_ZONES`], to the second, as
/// the name of what is saved then, which sorts by it: `20261021-133000`.
pub fn stamp(unix: i64, zone: usize) -> String {
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
/// was read, in the zone at `zone` in [`TIME_ZONES`].
pub fn file_line(l: Lang, f: &FileRow, now: i64, zone: usize) -> String {
    const DIVIDER: &str = "  |  ";
    match &f.kind {
        FileKind::Folder { items: Some(n) } => {
            let items = match (l, n) {
                (Lang::En, 1) => "1 item".to_string(),
                _ => tr!(l, "{n} 项", "{n} 項", "{n} items"),
            };
            format!("{}{DIVIDER}{items}", file_time(f.modified, true, now, zone))
        }
        FileKind::Folder { items: None } => l.pick(["无法读取", "無法讀取", "Unreadable"]).to_string(),
        FileKind::File => format!("{}{DIVIDER}{}", file_time(f.modified, false, now, zone), file_size(f.size)),
        FileKind::Link { target, .. } => tr!(l, "链接到 {target}", "連結到 {target}", "Link to {target}"),
        FileKind::Special => {
            let what = l.pick(match f.mode & 0o170_000 {
                0o020_000 => ["字符设备", "字元裝置", "Character device"],
                0o060_000 => ["块设备", "區塊裝置", "Block device"],
                0o010_000 => ["管道", "管道", "Pipe"],
                _ => ["套接字", "通訊端", "Socket"],
            });
            format!("{}{DIVIDER}{what}", file_time(f.modified, false, now, zone))
        }
    }
}

/// The parts a backup holds: `启动分区、系统分区`.
pub fn parts(l: Lang, held: &[bool; 4]) -> String {
    list(l, BACKUP_PARTS.iter().zip(held).filter(|(_, &on)| on).map(|(name, _)| l.pick(*name)))
}

/// What an image is, and what it holds.
pub fn image_kind(l: Lang, kind: ImageKind) -> (&'static str, &'static str) {
    const RECOVERY: Text = ["恢复模式镜像", "復原模式映像檔", "Recovery image"];
    const BOOT: Text = ["启动镜像", "啟動映像檔", "Boot image"];
    let (name, holds) = match kind {
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Recovery } => {
            (RECOVERY, ["包含内核和恢复模式 ramdisk", "包含核心和復原模式 ramdisk", "Holds a kernel and a recovery ramdisk"])
        }
        ImageKind::Boot { kernel: false, ramdisk: Ramdisk::Recovery } => {
            (RECOVERY, ["仅包含恢复模式 ramdisk", "僅包含復原模式 ramdisk", "Holds a recovery ramdisk alone"])
        }
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::Generic } => {
            (BOOT, ["包含内核和通用 ramdisk", "包含核心和通用 ramdisk", "Holds a kernel and a generic ramdisk"])
        }
        ImageKind::Boot { kernel: true, ramdisk: Ramdisk::None } => (BOOT, ["仅包含内核", "僅包含核心", "Holds a kernel alone"]),
        ImageKind::Boot { kernel: false, .. } => (BOOT, ["仅包含通用 ramdisk", "僅包含通用 ramdisk", "Holds a generic ramdisk alone"]),
        ImageKind::VendorBoot => {
            (["vendor_boot 镜像", "vendor_boot 映像檔", "vendor_boot image"], ["包含厂商 ramdisk 和设备树", "包含廠商 ramdisk 和裝置樹", "Holds the vendor ramdisk and device tree"])
        }
        ImageKind::Dtbo => (["dtbo 镜像", "dtbo 映像檔", "dtbo image"], ["包含设备树叠加层", "包含裝置樹疊加層", "Holds device tree overlays"]),
        ImageKind::Vbmeta => (["vbmeta 镜像", "vbmeta 映像檔", "vbmeta image"], ["包含启动验证数据", "包含啟動驗證資料", "Holds verified boot data"]),
    };
    (l.pick(name), l.pick(holds))
}

/// What a partition an image can go to is for.
pub fn partition_role(l: Lang, name: &str) -> &'static str {
    l.pick(match name {
        "boot" => ["内核", "核心", "Kernel"],
        "init_boot" => ["通用 ramdisk", "通用 ramdisk", "Generic ramdisk"],
        "recovery" => ["恢复模式", "復原模式", "Recovery"],
        "vendor_boot" => ["厂商 ramdisk 和设备树", "廠商 ramdisk 和裝置樹", "Vendor ramdisk and device tree"],
        "dtbo" => ["设备树叠加层", "裝置樹疊加層", "Device tree overlays"],
        "vbmeta" => ["启动验证数据", "啟動驗證資料", "Verified boot data"],
        "vbmeta_system" => ["系统分区的验证数据", "系統分割區的驗證資料", "Verification data of the system partitions"],
        "vbmeta_vendor" => ["厂商分区的验证数据", "廠商分割區的驗證資料", "Verification data of the vendor partitions"],
        _ => ["分区", "分割區", "Partition"],
    })
}

/// `槽位 B`, `槽位 A 和 B`, `slot B`, `slots A and B`
pub fn slots(l: Lang, slots: &[Slot]) -> String {
    let letters: Vec<&str> = slots.iter().map(|s| s.letter()).collect();
    let joined = letters.join(l.pick([" 和 ", " 和 ", " and "]));
    match (l, slots.len()) {
        (Lang::En, 1) => format!("slot {joined}"),
        (Lang::En, _) => format!("slots {joined}"),
        _ => format!("槽位 {joined}"),
    }
}

/// How long the screen stays on with nothing touched: `30 秒`, `2 分钟`,
/// or `永不`.
pub fn screen_off(l: Lang, after_ms: Option<u64>) -> String {
    match after_ms {
        None => l.pick(["永不", "永不", "Never"]).into(),
        Some(ms) if ms < 60_000 => match (l, ms / 1000) {
            (Lang::En, 1) => "1 second".into(),
            (_, n) => tr!(l, "{n} 秒", "{n} 秒", "{n} seconds"),
        },
        Some(ms) => match (l, ms / 60_000) {
            (Lang::En, 1) => "1 minute".into(),
            (_, n) => tr!(l, "{n} 分钟", "{n} 分鐘", "{n} minutes"),
        },
    }
}

/// A mount or unmount that failed, with why: `挂载 vendor 失败：…`
pub fn mount_failure(l: Lang, m: &MountFailure) -> String {
    let (base, detail) = (&m.base, &m.detail);
    match m.mounting {
        true => tr!(l, "挂载 {base} 失败：{detail}", "掛載 {base} 失敗：{detail}", "Couldn't mount {base}: {detail}"),
        false => tr!(l, "卸载 {base} 失败：{detail}", "卸載 {base} 失敗：{detail}", "Couldn't unmount {base}: {detail}"),
    }
}

/// How another package is given to the installer from where this one came
/// from: the computer sends it, or another file is chosen, as one is in
/// place of a package the system named. English starts its sentence with it.
fn give(l: Lang, source: Source) -> &'static str {
    l.pick(match source {
        Source::Computer | Source::Rescue => ["发送", "傳送", "Send"],
        Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => ["选择", "選擇", "Choose"],
    })
}

/// Why a package from `source` was not installed, for a phone with this
/// codename, its dates in the zone at `zone` in [`TIME_ZONES`].
pub fn refusal(l: Lang, r: &Refusal, codename: &str, source: Source, zone: usize) -> String {
    let give = give(l, source);
    match r {
        Refusal::Unknown => tr!(
            l,
            "无法识别该文件，请{give} A/B 完整更新包、刷机包或分区镜像。",
            "無法辨識這個檔案，請{give} A/B 完整更新檔、刷機包或分割區映像檔。",
            "This file isn't recognized. {give} an A/B full OTA package, a flashable zip or a partition image."
        ),
        Refusal::Image => tr!(
            l,
            "无法安装分区镜像，请{give} A/B 完整更新包或刷机包。",
            "無法安裝分割區映像檔，請{give} A/B 完整更新檔或刷機包。",
            "A partition image can't be installed this way. {give} an A/B full OTA package or a flashable zip."
        ),
        Refusal::NotAb(kind) if kind.is_empty() => tr!(
            l,
            "该更新包未注明类型，请{give} A/B 完整更新包。",
            "這個更新檔未註明類型，請{give} A/B 完整更新檔。",
            "This package doesn't say its type. {give} an A/B full OTA package."
        ),
        Refusal::NotAb(kind) => tr!(
            l,
            "该更新包的类型为 {kind}，请{give} A/B 完整更新包。",
            "這個更新檔的類型為 {kind}，請{give} A/B 完整更新檔。",
            "This package's type is {kind}. {give} an A/B full OTA package."
        ),
        Refusal::OtherDevice(devices) => tr!(
            l,
            "该更新包适用于 {devices}，本机为 {codename}。",
            "這個更新檔適用於 {devices}，本機為 {codename}。",
            "This package is for {devices}, and this phone is {codename}."
        ),
        Refusal::OtherSerial => l
            .pick([
                "该更新包仅适用于指定序列号的设备，不包括本机。",
                "這個更新檔僅適用於指定序號的裝置，不包括本機。",
                "This package is only for devices with certain serial numbers, and this phone isn't one of them.",
            ])
            .into(),
        Refusal::Incremental => tr!(
            l,
            "无法安装增量更新包，请{give}完整更新包。",
            "無法安裝增量更新檔，請{give}完整更新檔。",
            "An incremental OTA package can't be installed. {give} a full OTA package."
        ),
        Refusal::Older { package, installed } => {
            let (package, installed) = (date(l, *package, zone), date(l, *installed, zone));
            tr!(
                l,
                "该更新包的构建日期为 {package}，早于当前系统的 {installed}。",
                "這個更新檔的建置日期為 {package}，早於目前系統的 {installed}。",
                "This package was built on {package}, before the current system, built on {installed}."
            )
        }
        Refusal::OlderPatch { package, installed } => tr!(
            l,
            "该更新包的安全补丁为 {package}，早于当前系统的 {installed}。",
            "這個更新檔的安全性修補程式為 {package}，早於目前系統的 {installed}。",
            "This package's security patch is {package}, older than the current system's {installed}."
        ),
        Refusal::NoPartition => l.pick(["本机没有可刷入该镜像的分区。", "本機沒有可刷入這個映像檔的分割區。", "This phone has no partition this image can be flashed to."]).into(),
        Refusal::Battery { level, charging } => {
            let least = skyward_install::least_charge(*charging);
            tr!(
                l,
                "电量仅剩 {level}%，请充电至 {least}% 以上后重试。",
                "電量僅剩 {level}%，請充電至 {least}% 以上後重試。",
                "The battery is at {level}%. Charge it past {least}%, then try again."
            )
        }
        Refusal::Pending(state) => {
            let standing = snapshot(l, state);
            tr!(
                l,
                "{standing}。请重启到系统，完成上一次更新后重试。",
                "{standing}。請重新啟動到系統，完成上一次更新後重試。",
                "{standing}. Reboot to the system to finish the last update, then try again."
            )
        }
        Refusal::OldHost => l
            .pick([
                "电脑上的 adb 版本过低，请更新 Android SDK Platform-Tools 后重试。",
                "電腦上的 adb 版本過舊，請更新 Android SDK Platform-Tools 後重試。",
                "The computer's adb is too old. Update Android SDK Platform-Tools, then try again.",
            ])
            .into(),
        Refusal::Unsigned => l
            .pick([
                "该刷机包未签名，请关闭「验证刷机包签名」后重试。",
                "這個刷機包未簽署，請關閉「驗證刷機包簽章」後重試。",
                "This zip isn't signed. Turn off “Verify zip signatures”, then try again.",
            ])
            .into(),
        Refusal::Unverified => l
            .pick([
                "该刷机包未通过签名验证，请关闭「验证刷机包签名」后重试。",
                "這個刷機包未通過簽章驗證，請關閉「驗證刷機包簽章」後重試。",
                "This zip's signature didn't verify. Turn off “Verify zip signatures”, then try again.",
            ])
            .into(),
        Refusal::Panicked => l
            .pick(["暂无法安装该更新，请重启到系统后重试。", "暫時無法安裝這個更新，請重新啟動到系統後重試。", "This update can't be installed now. Reboot to the system, then try again."])
            .into(),
        Refusal::SystemAsked => l
            .pick([
                "系统请求安装的是另一个系统，只能在本机上手动安装。",
                "系統要求安裝的是另一個系統，只能在本機上手動安裝。",
                "The system asked for another system to be installed, which only goes in by hand on this phone.",
            ])
            .into(),
        Refusal::DataClosed => l
            .pick([
                "另一个系统将安装在 data 分区中，请先完成「解密」。",
                "另一個系統將安裝在 data 分割區中，請先完成「解密」。",
                "The other system goes into the data partition. Decrypt it first.",
            ])
            .into(),
        Refusal::SystemInstalled(name) => tr!(
            l,
            "已安装 {name}，请先在「双系统」中删除后重试。",
            "已安裝 {name}，請先在「雙系統」中刪除後重試。",
            "{name} is installed already. Remove it in Dual boot, then try again."
        ),
        Refusal::NoRoom { least, room } => {
            let (least, room) = (bytes(*least), bytes(*room));
            tr!(
                l,
                "该系统至少需要 {least}，data 分区仅剩 {room} 可用。",
                "這個系統至少需要 {least}，data 分割區僅剩 {room} 可用。",
                "This system needs at least {least}, and the data partition has {room} to spare."
            )
        }
        Refusal::NotBrick(_) | Refusal::NoSerial => unreachable!("only a request to erase partitions is refused as {r:?}"),
    }
}

/// Why the system's request to erase the partitions it lists was not
/// carried out, on a phone whose codename is `codename`.
pub fn erase_request_refusal(l: Lang, r: &Refusal, codename: &str) -> String {
    match r {
        Refusal::Unsigned => l.pick(["该请求未签名。", "這項要求未簽署。", "The request isn't signed."]).into(),
        Refusal::Unverified => l.pick(["该请求未通过签名验证。", "這項要求未通過簽章驗證。", "The request's signature didn't verify."]).into(),
        Refusal::NotBrick(kind) if kind.is_empty() => l.pick(["该请求未注明类型。", "這項要求未註明類型。", "The request doesn't say its type."]).into(),
        Refusal::NotBrick(kind) => tr!(l, "该请求的类型为 {kind}。", "這項要求的類型為 {kind}。", "The request's type is {kind}."),
        Refusal::OtherDevice(devices) => {
            tr!(l, "该请求适用于 {devices}，本机为 {codename}。", "這項要求適用於 {devices}，本機為 {codename}。", "The request is for {devices}, and this phone is {codename}.")
        }
        Refusal::OtherSerial => l
            .pick([
                "该请求仅适用于指定序列号的设备，不包括本机。",
                "這項要求僅適用於指定序號的裝置，不包括本機。",
                "The request is only for devices with certain serial numbers, and this phone isn't one of them.",
            ])
            .into(),
        Refusal::NoSerial => l.pick(["该请求未注明设备序列号。", "這項要求未註明裝置序號。", "The request names no serial number."]).into(),
        other => unreachable!("a request to erase partitions is not refused as {other:?}"),
    }
}

/// What went wrong installing a package from `source`, and the details
/// behind it when there are any.
pub fn failure(l: Lang, f: &Failure, source: Source) -> (String, Option<String>) {
    let said = |text: Text| l.pick(text).to_string();
    match f {
        Failure::Engine(code) => {
            let what = match (Trouble::of(*code), source) {
                (Trouble::Signature, _) => {
                    let give = give(l, source);
                    tr!(
                        l,
                        "更新包签名验证失败，请{give}官方更新包。",
                        "更新檔簽章驗證失敗，請{give}官方更新檔。",
                        "The package's signature didn't verify. {give} an official package."
                    )
                }
                (Trouble::Damaged, _) => said(["更新包已损坏，请重新下载。", "更新檔已損壞，請重新下載。", "The package is damaged. Download it again."]),
                (Trouble::Older, _) => said(["更新包的版本过旧。", "更新檔的版本過舊。", "The package is too old."]),
                (Trouble::NoSpace, _) => said(["super 分区空间不足。", "super 分割區空間不足。", "Not enough space in the super partition."]),
                (Trouble::Transfer, Source::Computer | Source::Rescue) => said([
                    "读取更新包失败，请检查数据线连接后重新发送。",
                    "讀取更新檔失敗，請檢查傳輸線連接後重新傳送。",
                    "Couldn't read the package. Check the cable, then send it again.",
                ]),
                (Trouble::Transfer, Source::File(Place::Storage) | Source::Opened { drive: false } | Source::Asked { .. }) => {
                    said(["读取更新包失败。", "讀取更新檔失敗。", "Couldn't read the package."])
                }
                (Trouble::Transfer, Source::File(Place::UsbDrive) | Source::Opened { drive: true }) => said([
                    "读取更新包失败，请检查 U 盘连接后重试。",
                    "讀取更新檔失敗，請檢查 USB 隨身碟連接後重試。",
                    "Couldn't read the package. Check the USB drive, then try again.",
                ]),
                (Trouble::Write, _) => said(["写入分区失败。", "寫入分割區失敗。", "Couldn't write a partition."]),
                (Trouble::Verify, _) => said(["分区校验失败。", "分割區驗證失敗。", "A partition didn't verify."]),
                (Trouble::PostInstall, _) => said(["完成安装失败。", "完成安裝失敗。", "Couldn't finish installing."]),
                (Trouble::Unsupported, _) => said(["暂不支持该更新包的格式。", "尚不支援這個更新檔的格式。", "This package's format isn't supported."]),
                (Trouble::Other, _) => said(["更新程序出错。", "更新程式發生錯誤。", "The updater ran into an error."]),
            };
            (what, Some(tr!(l, "错误代码：{code}", "錯誤代碼：{code}", "Error code: {code}")))
        }
        Failure::Script(code) => {
            (tr!(l, "安装脚本运行失败，退出码为 {code}。", "安裝指令碼執行失敗，結束代碼為 {code}。", "The install script failed with exit code {code}."), None)
        }
        Failure::Disconnected => (
            said(["电脑已停止发送，请检查数据线连接后重新发送。", "電腦已停止傳送，請檢查傳輸線連接後重新傳送。", "The computer stopped sending. Check the cable, then send it again."]),
            None,
        ),
        Failure::Unreadable(why) => match source {
            Source::File(Place::UsbDrive) | Source::Opened { drive: true } => (
                said(["读取文件失败，请检查 U 盘连接后重试。", "讀取檔案失敗，請檢查 USB 隨身碟連接後重試。", "Couldn't read the file. Check the USB drive, then try again."]),
                Some(why.clone()),
            ),
            _ => (said(["读取文件失败。", "讀取檔案失敗。", "Couldn't read the file."]), Some(why.clone())),
        },
        Failure::Changed => match source {
            Source::Computer | Source::Rescue => {
                (said(["文件在发送过程中被修改，请重新发送。", "檔案在傳送過程中被修改，請重新傳送。", "The file changed while it was sent. Send it again."]), None)
            }
            Source::File(_) | Source::Opened { .. } | Source::Asked { .. } => {
                (said(["文件在读取过程中被修改，请重新选择。", "檔案在讀取過程中被修改，請重新選擇。", "The file changed while it was read. Choose it again."]), None)
            }
        },
        Failure::Error(detail) => (said(UNEXPECTED), Some(detail.clone())),
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
        3 => ["极佳", "極佳", "Excellent"],
        2 => ["良好", "良好", "Good"],
        1 => ["一般", "普通", "Fair"],
        _ => ["微弱", "微弱", "Weak"],
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
    tr!(l, "第{n}代", "第{n}代", "Wi-Fi {n}")
}

/// How a network is protected, as the system's network details name it.
pub fn security(l: Lang, s: Security) -> &'static str {
    match s {
        Security::Open => l.pick(["无", "無", "None"]),
        Security::Owe => l.pick(["增强型开放式", "增強型開放式", "Enhanced Open"]),
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
        (Power::Off | Power::Failed(_), _) => l.pick(["已关闭", "已關閉", "Off"]),
        (Power::Starting, _) => l.pick(["正在开启", "正在開啟", "Turning on"]),
        (Power::On, Some(Joined { stage: WlanStage::Connected(_), name, .. })) => name,
        (Power::On, Some(_)) => l.pick(["正在连接", "正在連線", "Connecting"]),
        (Power::On, None) => l.pick(["未连接", "未連線", "Not connected"]),
    }
}

/// How joining the network chosen goes, under its name; joined, how far it
/// reaches, as the system's WLAN page says it.
pub fn joining(l: Lang, stage: &WlanStage) -> &'static str {
    l.pick(match stage {
        WlanStage::Joining => ["正在连接", "正在連線", "Connecting"],
        WlanStage::Addressing => ["正在获取 IP 地址", "正在取得 IP 位址", "Getting an IP address"],
        WlanStage::Connected(link) => match link.reach {
            Some(Reach::Login(_)) => ["需要登录", "需要登入", "Sign-in required"],
            Some(Reach::Offline(_)) => ["已连接，但无法访问互联网", "已連線，但無法連上網際網路", "Connected, no internet"],
            Some(Reach::Online) | None => ["已连接", "已連線", "Connected"],
        },
        WlanStage::WrongPassword => ["密码错误", "密碼錯誤", "Wrong password"],
        WlanStage::NotFound => ["未找到该网络", "找不到這個網路", "Network not found"],
        WlanStage::Failed => ["连接失败", "連線失敗", "Couldn't connect"],
        WlanStage::NoAddress(_) => ["无法获取 IP 地址", "無法取得 IP 位址", "Couldn't get an IP address"],
    })
}

/// What a network that wants a login in a browser first leaves to do,
/// with a recovery that has no browser to log in with.
const LOGIN: Text = [
    "需要先在网页上登录才能访问互联网，Skyward 无法打开网页登录。可以改用无需登录的网络，例如手机热点。",
    "需要先在網頁上登入才能連上網際網路，Skyward 無法開啟網頁登入。可以改用無需登入的網路，例如手機熱點。",
    "needs a sign-in in a browser before it reaches the internet, and Skyward has no browser to sign in with. Use a network that needs no sign-in instead, such as a phone's hotspot.",
];

/// What the WLAN page says under the network joined of how far it reaches,
/// where it reaches short of the internet.
pub fn reach(l: Lang, reach: &Reach) -> Option<String> {
    let login = l.pick(LOGIN);
    match reach {
        Reach::Online => None,
        Reach::Login(_) => Some(tr!(l, "该网络{login}", "這個網路{login}", "This network {login}")),
        Reach::Offline(why) => Some(tr!(l, "该网络无法访问互联网：{why}", "這個網路無法連上網際網路：{why}", "This network doesn't reach the internet: {why}")),
    }
}

/// Why the update server gave no answer, as the update page says it.
pub fn unanswered(l: Lang, why: &Unanswered) -> String {
    let login = l.pick(LOGIN);
    match why {
        Unanswered::Login => tr!(
            l,
            "检查更新失败，当前 WLAN {login}",
            "檢查更新失敗，目前連線的 Wi-Fi 網路{login}",
            "Couldn't check for updates. The Wi-Fi network joined {login}"
        ),
        Unanswered::Offline(why) => tr!(
            l,
            "检查更新失败，当前 WLAN 无法访问互联网：{why}",
            "檢查更新失敗，目前連線的 Wi-Fi 網路無法連上網際網路：{why}",
            "Couldn't check for updates. The Wi-Fi network joined doesn't reach the internet: {why}"
        ),
        Unanswered::Failed(why) => tr!(l, "检查更新失败：{why}", "檢查更新失敗：{why}", "Couldn't check for updates: {why}"),
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
        DownloadFailure::NoRoom { needed, free } => {
            let (needed, free) = (file_size(*needed), file_size(*free));
            tr!(
                l,
                "内部存储空间不足，需 {needed} 可用空间，当前可用 {free}。",
                "內部儲存空間不足，需要 {needed} 的可用空間，目前可用 {free}。",
                "Not enough internal storage: {needed} has to be free, and {free} is."
            )
        }
        DownloadFailure::Damaged => l
            .pick(["完整更新包校验失败，已删除，请重新下载。", "完整更新檔驗證失敗，已刪除，請重新下載。", "The full OTA package didn't verify and was deleted. Download it again."])
            .into(),
        DownloadFailure::Changed => {
            l.pick(["服务器上的版本已更新，请重新检查更新。", "伺服器上的版本已更新，請重新檢查更新。", "The server has a newer version now. Check for updates again."]).into()
        }
        DownloadFailure::Other(why) => tr!(l, "下载失败：{why}", "下載失敗：{why}", "Download failed: {why}"),
    }
}

/// Why downloading Skyward's build did not end with it whole. It goes into
/// the recovery's own folder, which is in memory.
pub fn skyward_download_failure(l: Lang, f: &DownloadFailure) -> String {
    match f {
        DownloadFailure::NoRoom { needed, free } => {
            let (needed, free) = (file_size(*needed), file_size(*free));
            tr!(
                l,
                "运行内存不足，需 {needed} 可用空间，当前可用 {free}。",
                "記憶體不足，需要 {needed} 的可用空間，目前可用 {free}。",
                "Not enough memory: {needed} has to be free, and {free} is."
            )
        }
        DownloadFailure::Damaged => {
            l.pick(["安装包校验失败，已删除，请重新更新。", "安裝包驗證失敗，已刪除，請重新更新。", "The package didn't verify and was deleted. Update again."]).into()
        }
        DownloadFailure::Changed => unreachable!("GitHub is not asked again where a build is"),
        DownloadFailure::Other(why) => tr!(l, "下载失败：{why}", "下載失敗：{why}", "Download failed: {why}"),
    }
}

/// How the terminal page's shell ended.
pub fn term_end(l: Lang, end: &TermEnd) -> String {
    match end {
        TermEnd::Exited(code) => tr!(l, "shell 已退出，退出码 {code}", "shell 已結束，結束代碼 {code}", "The shell exited with code {code}"),
        TermEnd::Signalled(signal) => tr!(l, "shell 已被信号 {signal} 终止", "shell 已被訊號 {signal} 終止", "Signal {signal} stopped the shell"),
        TermEnd::Other(status) => tr!(l, "shell 已结束：{status}", "shell 已結束：{status}", "The shell ended: {status}"),
    }
}

/// What a computer is told internal storage is called, as the stock system
/// names it.
pub fn storage_name(l: Lang) -> &'static str {
    l.pick(["内部存储设备", "內部儲存裝置", "Internal shared storage"])
}

/// File transfer kept internal storage from work that needed it back, as
/// the system says why.
pub fn transfer_kept(l: Lang, why: &str) -> String {
    tr!(l, "未能停止文件传输：{why}", "無法停止檔案傳輸：{why}", "Couldn't stop file transfer: {why}")
}

/// File transfer did not turn on, as the system says why.
pub fn transfer_failed(l: Lang, why: &str) -> String {
    tr!(l, "无法开启文件传输：{why}", "無法開啟檔案傳輸：{why}", "Couldn't turn on file transfer: {why}")
}

/// File transfer did not turn off, as the system says why.
pub fn transfer_left_on(l: Lang, why: &str) -> String {
    tr!(l, "关闭文件传输失败：{why}", "關閉檔案傳輸失敗：{why}", "Couldn't turn off file transfer: {why}")
}

/// File transfer stopped on its own, as the system says why.
pub fn transfer_stopped(l: Lang, why: &str) -> String {
    tr!(l, "文件传输已停止：{why}", "檔案傳輸已停止：{why}", "File transfer stopped: {why}")
}

/// Merging the update failed, as the system says why.
pub fn merge_failed(l: Lang, why: &str) -> String {
    tr!(l, "未能合并更新：{why}", "無法合併更新：{why}", "Couldn't merge the update: {why}")
}

/// What work that takes a program of the installed system's `base`
/// partition asks while that partition is not mounted.
pub fn mount_first(l: Lang, base: &str) -> String {
    tr!(l, "请先在「挂载」中挂载 {base} 分区", "請先在「掛載」中掛載 {base} 分割區", "Mount the {base} partition in Mount first")
}

/// The installed system's SELinux rules could not be read, as the system
/// says why.
pub fn rules_unread(l: Lang, why: &str) -> String {
    tr!(l, "未能读取系统的 SELinux 规则：{why}", "無法讀取系統的 SELinux 規則：{why}", "Couldn't read the system's SELinux rules: {why}")
}

/// The logs were saved in `folder`.
pub fn exported(l: Lang, folder: &str) -> String {
    tr!(l, "已导出到 {folder}", "已匯出到 {folder}", "Exported to {folder}")
}

/// The logs were not saved, as the system says why.
pub fn export_failed(l: Lang, why: &str) -> String {
    tr!(l, "导出失败：{why}", "匯出失敗：{why}", "Couldn't export: {why}")
}

/// A screenshot went into the gallery's screenshots.
pub const SCREENSHOT_SAVED: Text = ["截图已保存到相册", "截圖已儲存到相簿", "Screenshot saved to the gallery"];

/// A screenshot is held until the owner's storage opens, and then goes into
/// the gallery's.
pub const SCREENSHOT_HELD: Text = ["截图已暂存，解密后存入相册", "截圖已暫存，解密後存入相簿", "Screenshot kept. It goes to the gallery once decrypted"];

/// A screenshot was not saved, as the system says why.
pub fn screenshot_failed(l: Lang, why: &str) -> String {
    tr!(l, "截图保存失败：{why}", "截圖儲存失敗：{why}", "Couldn't save the screenshot: {why}")
}

/// The screenshots held while the owner's storage was closed did not go into
/// the gallery's, as the system says why.
pub fn screenshots_not_kept(l: Lang, why: &str) -> String {
    tr!(l, "暂存的截图未能存入相册：{why}", "暫存的截圖未能存入相簿：{why}", "Kept screenshots couldn't go to the gallery: {why}")
}

/// The name a screenshot taken `unix_ms` milliseconds into Unix time is saved
/// under, in the zone at `zone` in [`TIME_ZONES`]: as the system names its
/// own, the time to the millisecond and then where it was taken,
/// `Screenshot_2026-10-26-01-45-00-123_Skyward.png`.
pub fn screenshot_name(unix_ms: i64, zone: usize) -> String {
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
pub const TERMINAL_BUSY: Text = ["终端正在运行命令，请结束后重试", "終端機正在執行指令，請結束後重試", "The terminal is running a command. End it, then try again"];

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(numeral(12, false), "十二");
        assert_eq!(numeral(20, false), "二十");
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
        assert_eq!(date(HANS, 1_778_243_655, 0), "2026年5月8日");
        assert_eq!(date(Lang::En, 1_778_243_655, 0), "May 8, 2026");
        assert_eq!(date(HANS, 0, 0), "1970年1月1日");
        // Midnight on a leap day in UTC is eight in the morning in China.
        assert_eq!(date(HANS, 1_709_164_800, 0), "2024年2月29日");
        assert_eq!(date(HANS, 1_709_164_800 - 8 * 3600 - 1, 0), "2024年2月28日");
        assert_eq!(patch(HANS, &PatchLevel { year: 2026, month: 4, day: 1 }), "2026年4月1日");
        assert_eq!(slots(HANS, &[Slot::B]), "槽位 B");
        assert_eq!(slots(HANS, &[Slot::A, Slot::B]), "槽位 A 和 B");
        assert_eq!(slots(Lang::En, &[Slot::B]), "slot B");
        assert_eq!(slots(Lang::En, &[Slot::A, Slot::B]), "slots A and B");
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
        assert_eq!(datetime(HANS, 1_792_348_260, 0), "2026年10月19日 02:31");
        assert_eq!(datetime(Lang::En, 1_792_348_260, 0), "October 19, 2026, 02:31");
        assert_eq!(clock(1_792_348_260, 0), "02:31");
        // Midnight in Beijing is 16:00 UTC the day before.
        assert_eq!(clock(1_792_339_200, 0), "00:00");
        assert_eq!(stamp(1_792_348_265, 0), "20261019-023105");
    }

    #[test]
    fn a_screenshot_is_named_as_the_system_names_its_own() {
        assert_eq!(screenshot_name(1_792_348_265_007, 0), "Screenshot_2026-10-19-02-31-05-007_Skyward.png");
        assert_eq!(screenshot_name(1_792_348_265_999, zone("UTC")), "Screenshot_2026-10-18-18-31-05-999_Skyward.png", "in the zone chosen");
    }

    /// Where a zone named in the tz database is in [`TIME_ZONES`].
    fn zone(name: &str) -> usize {
        TIME_ZONES.iter().position(|&(_, _, n)| n == name).expect("a zone listed")
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
        assert_eq!(date(HANS, i64::MAX, 0), "9999年12月31日");
        assert_eq!(date(HANS, i64::MIN, zone("UTC")), "-9999年1月2日");
        assert_eq!(date(Lang::En, i64::MAX, 0), "December 31, 9999");
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
        assert_eq!(file_line(HANS, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, 0), "2026/10/19  |  6 项");
        assert_eq!(file_line(Lang::Hant, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, 0), "2026/10/19  |  6 項");
        assert_eq!(file_line(Lang::En, &row(FileKind::Folder { items: Some(6) }, 4096, 0o040_755), later, 0), "2026/10/19  |  6 items");
        assert_eq!(file_line(Lang::En, &row(FileKind::Folder { items: Some(1) }, 4096, 0o040_755), later, 0), "2026/10/19  |  1 item");
        assert_eq!(file_line(HANS, &row(FileKind::Folder { items: None }, 4096, 0o040_700), later, 0), "无法读取");
        assert_eq!(file_line(HANS, &row(FileKind::File, 26_950, 0o100_644), later, 0), "10/19  |  26.95 KB");
        let link = FileKind::Link { target: "/storage/self/primary".into(), folder: true };
        assert_eq!(file_line(HANS, &row(link, 21, 0o120_777), later, 0), "链接到 /storage/self/primary");
        assert_eq!(file_line(HANS, &row(FileKind::Special, 0, 0o060_600), later, 0), "10/19  |  块设备");
        assert_eq!(file_line(Lang::En, &row(FileKind::Special, 0, 0o060_600), later, 0), "10/19  |  Block device");
    }

    #[test]
    fn list_dates_are_told_against_the_day_the_folder_was_read() {
        // 2026-10-19 02:31 in Beijing.
        let at = 1_792_348_260;
        assert_eq!(file_time(at, false, at + 3600, 0), "02:31", "today: the time");
        assert_eq!(file_time(at, true, at + 3600, 0), "02:31", "a folder today too");
        assert_eq!(file_time(at, false, at + 86_400, 0), "10/19", "this year: a file's month and day");
        assert_eq!(file_time(at, true, at + 86_400, 0), "2026/10/19", "this year: a folder's whole date");
        assert_eq!(file_time(at, false, at + 90 * 86_400, 0), "2026/10/19", "another year: the whole date");
        // 2026-10-18 23:59 UTC is already the 19th in Beijing.
        assert_eq!(file_time(1_792_367_940, false, at, 0), "07:59");
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
        assert_eq!(refusal(HANS, &Refusal::OtherDevice("nezha".into()), "ishtar", pc, 0), "该更新包适用于 nezha，本机为 ishtar。");
        assert_eq!(
            refusal(HANS, &Refusal::Older { package: 1_770_000_000, installed: 1_778_243_655 }, "ishtar", pc, 0),
            "该更新包的构建日期为 2026年2月2日，早于当前系统的 2026年5月8日。"
        );
        assert_eq!(
            refusal(Lang::En, &Refusal::Older { package: 1_770_000_000, installed: 1_778_243_655 }, "ishtar", pc, 0),
            "This package was built on February 2, 2026, before the current system, built on May 8, 2026."
        );
        assert_eq!(refusal(HANS, &Refusal::Battery { level: 12, charging: true }, "ishtar", pc, 0), "电量仅剩 12%，请充电至 15% 以上后重试。");
        assert_eq!(
            refusal(HANS, &Refusal::Pending(SnapshotState::Snapshotted), "ishtar", pc, 0),
            "更新已写入，等待合并。请重启到系统，完成上一次更新后重试。"
        );
        assert_eq!(
            refusal(Lang::En, &Refusal::Pending(SnapshotState::Snapshotted), "ishtar", pc, 0),
            "Update written, waiting to merge. Reboot to the system to finish the last update, then try again."
        );
        assert_eq!(failure(HANS, &Failure::Engine(26), pc), ("更新包签名验证失败，请发送官方更新包。".into(), Some("错误代码：26".into())));
        assert_eq!(failure(Lang::Hant, &Failure::Engine(26), pc), ("更新檔簽章驗證失敗，請傳送官方更新檔。".into(), Some("錯誤代碼：26".into())));
        assert_eq!(failure(HANS, &Failure::Error("open ep0: busy".into()), pc).1.as_deref(), Some("open ep0: busy"));
        assert_eq!(failure(HANS, &Failure::Script(1), pc), ("安装脚本运行失败，退出码为 1。".into(), None));
        assert_eq!(refusal(HANS, &Refusal::Unsigned, "ishtar", pc, 0), "该刷机包未签名，请关闭「验证刷机包签名」后重试。");
        assert_eq!(refusal(HANS, &Refusal::Unverified, "ishtar", pc, 0), "该刷机包未通过签名验证，请关闭「验证刷机包签名」后重试。");
    }

    #[test]
    fn a_file_chosen_is_answered_with_choosing_another() {
        let file = Source::File(Place::Storage);
        assert_eq!(refusal(HANS, &Refusal::Unknown, "ishtar", file, 0), "无法识别该文件，请选择 A/B 完整更新包、刷机包或分区镜像。");
        assert_eq!(
            refusal(Lang::En, &Refusal::Unknown, "ishtar", file, 0),
            "This file isn't recognized. Choose an A/B full OTA package, a flashable zip or a partition image."
        );
        assert_eq!(refusal(HANS, &Refusal::Incremental, "ishtar", file, 0), "无法安装增量更新包，请选择完整更新包。");
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
}
