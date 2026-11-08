use crate::app::Screen;
use crate::lang::{Lang, Text};

/// A feature or partition the search finds, and the page it opens.
pub struct Entry {
    pub title: Text,
    /// Where it lives, shown under the title.
    pub place: Text,
    pub screen: Screen,
    /// What it answers to besides its title: pinyin in full and by
    /// initials, and English, since the keyboard types Latin letters.
    pub keys: &'static [&'static str],
}

const fn entry(title: Text, place: Text, screen: Screen, keys: &'static [&'static str]) -> Entry {
    Entry { title, place, screen, keys }
}

/// A partition, named the same in every language.
const fn partition(name: &'static str, place: Text, screen: Screen) -> Entry {
    Entry { title: [name; 3], place, screen, keys: &[] }
}

const HOME: Text = ["主页", "主頁", "Home"];
const INSTALL: Text = ["安装", "安裝", "Install"];
const ADVANCED: Text = ["高级", "進階", "Advanced"];
const SETTINGS: Text = ["设置", "設定", "Settings"];
const MOUNT: Text = ["挂载", "掛載", "Mount"];

pub const ENTRIES: &[Entry] = &[
    entry(["WLAN", "Wi-Fi", "Wi-Fi"], HOME, Screen::Wlan, &["wuxianwangluo", "wxwl", "wangluo", "wl", "lianwang", "wifi", "wlan"]),
    entry(
        ["双系统", "雙系統", "Dual boot"],
        HOME,
        Screen::Systems,
        &["shuangxitong", "sxt", "qiehuanxitong", "qhxt", "duoxitong", "dxt", "dualboot", "multiboot", "linux"],
    ),
    entry(INSTALL, HOME, Screen::Install, &["anzhuang", "az", "install", "shuaji", "sj"]),
    entry(["从内部存储安装", "從內部儲存空間安裝", "Install from internal storage"], INSTALL, Screen::Install, &["congneibucunchuanzhuang", "cnbccaz", "neicun", "sdcard", "zip"]),
    entry(["从 U 盘安装", "從 USB 隨身碟安裝", "Install from a USB drive"], INSTALL, Screen::Install, &["congupananzhuang", "cupaz", "upan", "usb", "otg"]),
    entry(["通过电脑安装", "透過電腦安裝", "Install from a computer"], INSTALL, Screen::Install, &["tongguodiannaoanzhuang", "tgdnaz", "sideload", "adb"]),
    entry(["备份", "備份", "Backup"], HOME, Screen::Backup, &["beifen", "bf", "backup"]),
    entry(["恢复", "還原", "Restore"], HOME, Screen::Restore, &["huifu", "hf", "restore"]),
    entry(MOUNT, HOME, Screen::Mount, &["guazai", "gz", "mount", "umount"]),
    entry(["清除数据", "清除資料", "Wipe data"], HOME, Screen::Wipe, &["qingchushuju", "qcsj", "wipe", "format", "geshihua", "gsh"]),
    entry(ADVANCED, HOME, Screen::Advanced, &["gaoji", "gj", "advanced"]),
    entry(["文件管理", "檔案管理", "File manager"], ADVANCED, Screen::Files, &["wenjianguanli", "wjgl", "files", "file"]),
    entry(["终端", "終端機", "Terminal"], ADVANCED, Screen::Terminal, &["zhongduan", "zd", "terminal", "shell"]),
    entry(
        ["修复 SELinux 上下文", "修復 SELinux 上下文", "Fix SELinux contexts"],
        ADVANCED,
        Screen::Relabel,
        &["xiufuselinuxshangxiawen", "xfsxw", "xiufu", "xf", "shangxiawen", "context", "restorecon", "label"],
    ),
    entry(
        ["文件系统", "檔案系統", "Filesystems"],
        ADVANCED,
        Screen::Filesystems,
        &["wenjianxitong", "wjxt", "genghuan", "gh", "fsck", "filesystem", "f2fs", "ext4"],
    ),
    entry(["导出日志", "匯出記錄", "Export logs"], ADVANCED, Screen::Logs, &["daochurizhi", "dcrz", "rizhi", "rz", "logs", "dmesg"]),
    entry(
        ["系统更新", "系統更新", "System update"],
        ADVANCED,
        Screen::Update,
        &["xitonggengxin", "xtgx", "hebing", "hb", "xiazai", "xz", "wanzhenggengxinbao", "wzgxb", "update", "ota", "merge", "snapshot", "download"],
    ),
    entry(SETTINGS, HOME, Screen::Settings, &["shezhi", "sz", "settings"]),
    entry(["深色模式", "深色模式", "Dark mode"], SETTINGS, Screen::Settings, &["shensemoshi", "ssms", "dark", "theme"]),
    entry(["屏幕亮度", "螢幕亮度", "Screen brightness"], SETTINGS, Screen::Settings, &["pingmuliangdu", "pmld", "liangdu", "ld", "brightness"]),
    entry(["息屏时间", "螢幕自動關閉", "Screen timeout"], SETTINGS, Screen::ScreenOff, &["xipingshijian", "xpsj", "timeout"]),
    entry(["振动强度", "震動強度", "Vibration intensity"], SETTINGS, Screen::Settings, &["zhendongqiangdu", "zdqd", "qiangdu", "qd", "intensity", "strength"]),
    entry(["触摸振动", "觸控震動", "Touch vibration"], SETTINGS, Screen::Settings, &["chumozhendong", "cmzd", "zhendong", "vibrate", "haptic"]),
    entry(["文件传输", "檔案傳輸", "File transfer"], SETTINGS, Screen::Settings, &["wenjianchuanshu", "wjcs", "chuanshu", "mtp", "usb"]),
    entry(["语言", "語言", "Language"], SETTINGS, Screen::Language, &["yuyan", "yy", "language", "fanti", "jianti", "english"]),
    entry(["时区", "時區", "Time zone"], SETTINGS, Screen::TimeZone, &["shiqu", "sq", "timezone"]),
    entry(["重启", "重新啟動", "Reboot"], HOME, Screen::Reboot, &["chongqi", "cq", "reboot", "fastboot", "bootloader", "edl", "guanji"]),
    entry(["启动槽位", "啟動槽位", "Boot slot"], ["重启", "重新啟動", "Reboot"], Screen::Slot, &["qidongcaowei", "qdcw", "caowei", "cw", "slot"]),
    entry(["关于本机", "關於手機", "About phone"], SETTINGS, Screen::About, &["guanyubenji", "gybj", "about", "banben", "version"]),
    entry(
        ["Skyward 更新", "Skyward 更新", "Skyward update"],
        SETTINGS,
        Screen::SkywardUpdate,
        &["skywardgengxin", "skywardgx", "shengji", "xiazai", "update", "upgrade", "release", "github"],
    ),
    entry(
        ["全部参数与信息", "全部規格與資訊", "All specs"],
        ["设置 > 关于本机", "設定 > 關於手機", "Settings > About phone"],
        Screen::Specs,
        &["quanbucanshuyuxinxi", "qbcsyxx", "canshu", "specs"],
    ),
    entry(
        ["开源许可", "開放原始碼授權", "Open source licenses"],
        ["设置 > 关于本机 > 全部参数与信息", "設定 > 關於手機 > 全部規格與資訊", "Settings > About phone > All specs"],
        Screen::License,
        &["kaiyuanxuke", "kyxk", "license", "gpl"],
    ),
    entry(
        ["界面字体", "介面字型", "UI fonts"],
        ["设置 > 关于本机 > 全部参数与信息", "設定 > 關於手機 > 全部規格與資訊", "Settings > About phone > All specs"],
        Screen::FontLicense,
        &["jiemianziti", "jmzt", "ziti", "font", "misans", "kirkmono", "dengkuan"],
    ),
    partition("boot", INSTALL, Screen::Install),
    partition("init_boot", INSTALL, Screen::Install),
    partition("vendor_boot", INSTALL, Screen::Install),
    partition("recovery", INSTALL, Screen::Install),
    partition("dtbo", INSTALL, Screen::Install),
    partition("vbmeta", INSTALL, Screen::Install),
    partition("system", MOUNT, Screen::Mount),
    partition("system_ext", MOUNT, Screen::Mount),
    partition("product", MOUNT, Screen::Mount),
    partition("vendor", MOUNT, Screen::Mount),
    partition("odm", MOUNT, Screen::Mount),
    partition("vendor_dlkm", MOUNT, Screen::Mount),
    partition("system_dlkm", MOUNT, Screen::Mount),
    partition("odm_dlkm", MOUNT, Screen::Mount),
];

/// The entries `query` finds, best first: a title in `l` holding it, then a
/// key starting with it, then a key holding it.
pub fn find(query: &str, l: Lang) -> Vec<&'static Entry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let rank = |e: &Entry| {
        if l.pick(e.title).to_lowercase().contains(&q) {
            Some(0)
        } else if e.keys.iter().any(|k| k.starts_with(&q)) {
            Some(1)
        } else if e.keys.iter().any(|k| k.contains(&q)) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, usize, &Entry)> = ENTRIES.iter().enumerate().filter_map(|(k, e)| rank(e).map(|r| (r, k, e))).collect();
    found.sort_by_key(|&(r, k, _)| (r, k));
    found.into_iter().map(|(_, _, e)| e).collect()
}

/// What [`find`] finds that the phone has, as `offers` tells of each page:
/// a phone whose WLAN the recovery does not bring up has no WLAN page.
pub fn find_on(query: &str, l: Lang, offers: impl Fn(Screen) -> bool) -> Vec<&'static Entry> {
    find(query, l).into_iter().filter(|e| offers(e.screen)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(query: &str) -> Vec<&'static str> {
        find(query, Lang::Hans).iter().map(|e| Lang::Hans.pick(e.title)).collect()
    }

    #[test]
    fn pinyin_initials_and_english_all_find_a_feature() {
        assert_eq!(titles("bf"), ["备份"]);
        assert_eq!(titles("beifen"), ["备份"]);
        assert_eq!(titles("Backup"), ["备份"]);
        assert_eq!(titles("xpsj"), ["息屏时间"]);
    }

    #[test]
    fn a_title_holding_the_query_comes_before_keys_that_do() {
        // vendor_boot holds "boot" in its title; the reboot page only in a key.
        let found = titles("boot");
        assert_eq!(&found[..3], ["boot", "init_boot", "vendor_boot"]);
        assert!(found.contains(&"重启"), "{found:?}");
    }

    #[test]
    fn chinese_titles_match_as_typed() {
        assert_eq!(titles("挂载"), ["挂载"]);
    }

    #[test]
    fn the_filesystem_page_is_found_by_what_it_does() {
        assert_eq!(titles("wjxt"), ["文件系统"]);
        assert_eq!(titles("fsck"), ["文件系统"]);
        assert_eq!(titles("genghuan"), ["文件系统"]);
    }

    #[test]
    fn the_wlan_is_found_where_the_phone_has_it() {
        assert_eq!(titles("wifi"), ["WLAN"]);
        assert_eq!(find_on("wlan", Lang::Hans, |_| true).iter().map(|e| e.title[0]).collect::<Vec<_>>(), ["WLAN"]);
        assert!(find_on("wlan", Lang::Hans, |s| s != Screen::Wlan).is_empty());
        assert_eq!(titles("xiazai"), ["系统更新", "Skyward 更新"]);
    }

    #[test]
    fn the_second_system_is_found_where_the_phone_can_hold_one() {
        assert_eq!(titles("sxt"), ["双系统"]);
        assert_eq!(titles("qiehuan"), ["双系统"]);
        assert_eq!(find("dual", Lang::En).iter().map(|e| e.title[2]).collect::<Vec<_>>(), ["Dual boot"]);
        assert!(find_on("shuangxitong", Lang::Hans, |s| s != Screen::Systems).is_empty());
    }

    #[test]
    fn skyward_is_updated_from_its_own_page() {
        assert_eq!(titles("skyward"), ["Skyward 更新"]);
        assert_eq!(titles("gengxin"), ["系统更新", "Skyward 更新"]);
        assert!(find_on("skyward", Lang::Hans, |s| s != Screen::SkywardUpdate).is_empty());
    }

    #[test]
    fn a_title_is_found_in_the_language_the_pages_speak() {
        let english = |query| find(query, Lang::En).iter().map(|e| e.title[2]).collect::<Vec<_>>();
        assert_eq!(english("brightness"), ["Screen brightness"]);
        assert_eq!(english("about"), ["About phone"]);
        assert_eq!(english("wi-fi"), ["Wi-Fi"]);
        let traditional = find("設定", Lang::Hant);
        assert_eq!(traditional.first().map(|e| e.title[1]), Some("設定"));
        assert!(find("设置", Lang::Hant).is_empty(), "the simplified title belongs to the other script");
    }

    #[test]
    fn nothing_typed_finds_nothing() {
        assert!(find("", Lang::Hans).is_empty());
        assert!(find("   ", Lang::En).is_empty());
        assert!(find("zzzz", Lang::Hant).is_empty());
    }
}
