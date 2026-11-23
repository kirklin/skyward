//! The settings page and the pages under it: its sliders and switches,
//! the language, the time zone, and how long the screen stays on.

use super::*;
use anyhow::{Context, Result};
use jiff::tz::TimeZone;

/// The settings' sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Brightness,
    /// How strongly the motor taps.
    Vibration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Vibrate,
    /// Offer internal storage to a computer over USB.
    FileTransfer,
    /// Compress a backup.
    Compress,
    /// Read a backup back once it is written.
    Verify,
    /// Run a zip's install script only if the zip was signed with one of
    /// the phone's update keys.
    Signature,
    /// Write Skyward back into the recovery partitions an install writes
    /// over.
    KeepRecovery,
    Wlan,
    /// Show the password typed rather than a dot for each character.
    ShowPassword,
    /// Keep the time zone the installed system is set to.
    SystemTimeZone,
}

/// How long the screen stays on with nothing touched, in milliseconds, in
/// the order the page lists them; `None` keeps it on.
pub const SCREEN_OFF: [Option<u64>; 7] = [Some(15_000), Some(30_000), Some(60_000), Some(120_000), Some(300_000), Some(600_000), None];

/// A part of the world the time zone page lists its zones under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Americas,
    EuropeAfrica,
    /// Eleven zones of one country, which its people look for under it.
    Russia,
    Asia,
    Oceania,
}

impl Region {
    /// The regions, in the order the time zone page lists them.
    pub const ALL: [Region; 5] = [Region::Americas, Region::EuropeAfrica, Region::Russia, Region::Asia, Region::Oceania];

    pub fn name(self) -> Text {
        match self {
            Region::Americas => texts::time_zone::region::AMERICAS,
            Region::EuropeAfrica => texts::time_zone::region::EUROPE_AFRICA,
            Region::Russia => texts::time_zone::region::RUSSIA,
            Region::Asia => texts::time_zone::region::ASIA,
            Region::Oceania => texts::time_zone::region::OCEANIA,
        }
    }
}

/// A time zone the clock and the times on the pages can keep.
#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub name: Text,
    /// The cities it is known by.
    pub cities: Text,
    /// Its name in the tz database, whose rules it keeps.
    pub tz: &'static str,
    /// The other names in the tz database whose places keep the same time
    /// under the zone's name: a system set to one of them shows as the
    /// zone, and keeps the rules of the name it is set to.
    pub also: &'static [&'static str],
    /// The part of the world its page lists it under; none for the one
    /// zone that is no place's own.
    pub region: Option<Region>,
}

const fn zone(region: Region, name: Text, cities: Text, tz: &'static str, also: &'static [&'static str]) -> Zone {
    Zone { name, cities, tz, also, region: Some(region) }
}

/// The time zones the clock and the times on the pages can keep, each
/// region's from west to east.
pub static TIME_ZONES: [Zone; 62] = {
    use texts::time_zone::{cities as c, name as n};
    use Region::{Americas, Asia, EuropeAfrica, Oceania, Russia};
    [
        zone(Americas, n::HAWAII, c::HAWAII, "Pacific/Honolulu", &[]),
        zone(Americas, n::ALASKA, c::ALASKA, "America/Anchorage", &["America/Juneau", "America/Sitka", "America/Nome"]),
        zone(Americas, n::US_PACIFIC, c::US_PACIFIC, "America/Los_Angeles", &["America/Tijuana"]),
        zone(Americas, n::US_MOUNTAIN, c::US_MOUNTAIN, "America/Denver", &["America/Boise"]),
        zone(Americas, n::US_CENTRAL, c::US_CENTRAL, "America/Chicago", &["America/Menominee"]),
        zone(Americas, n::MEXICO, c::MEXICO, "America/Mexico_City", &["America/Monterrey", "America/Merida"]),
        zone(Americas, n::US_EASTERN, c::US_EASTERN, "America/New_York", &["America/Toronto", "America/Detroit"]),
        zone(Americas, n::COLOMBIA, c::COLOMBIA, "America/Bogota", &[]),
        zone(Americas, n::PERU, c::PERU, "America/Lima", &[]),
        zone(Americas, n::VENEZUELA, c::VENEZUELA, "America/Caracas", &[]),
        zone(Americas, n::AMAZON, c::AMAZON, "America/Manaus", &["America/Porto_Velho", "America/Boa_Vista", "America/Cuiaba", "America/Campo_Grande"]),
        zone(Americas, n::CHILE, c::CHILE, "America/Santiago", &[]),
        zone(
            Americas,
            n::BRASILIA,
            c::BRASILIA,
            "America/Sao_Paulo",
            &["America/Bahia", "America/Fortaleza", "America/Recife", "America/Belem", "America/Maceio", "America/Araguaina", "America/Santarem"],
        ),
        zone(
            Americas,
            n::ARGENTINA,
            c::ARGENTINA,
            "America/Argentina/Buenos_Aires",
            &["America/Buenos_Aires", "America/Argentina/Cordoba", "America/Cordoba", "America/Argentina/Mendoza", "America/Mendoza"],
        ),
        zone(EuropeAfrica, n::BRITAIN, c::BRITAIN, "Europe/London", &["Europe/Belfast", "Europe/Jersey", "Europe/Guernsey", "Europe/Isle_of_Man"]),
        zone(
            EuropeAfrica,
            n::CENTRAL_EUROPE,
            c::CENTRAL_EUROPE,
            "Europe/Berlin",
            &[
                "Europe/Paris",
                "Europe/Rome",
                "Europe/Madrid",
                "Europe/Warsaw",
                "Europe/Amsterdam",
                "Europe/Brussels",
                "Europe/Vienna",
                "Europe/Zurich",
                "Europe/Prague",
                "Europe/Budapest",
                "Europe/Stockholm",
                "Europe/Oslo",
                "Europe/Copenhagen",
                "Europe/Belgrade",
                "Europe/Zagreb",
                "Europe/Ljubljana",
                "Europe/Bratislava",
                "Europe/Luxembourg",
                "Europe/Malta",
                "Europe/Monaco",
                "Europe/Tirane",
                "Europe/Sarajevo",
                "Europe/Skopje",
                "Europe/Podgorica",
                "Europe/Vaduz",
                "Europe/Andorra",
                "Europe/San_Marino",
                "Europe/Vatican",
                "Europe/Busingen",
                "Europe/Gibraltar",
            ],
        ),
        zone(
            EuropeAfrica,
            n::WEST_AFRICA,
            c::WEST_AFRICA,
            "Africa/Lagos",
            &[
                "Africa/Kinshasa",
                "Africa/Luanda",
                "Africa/Douala",
                "Africa/Libreville",
                "Africa/Niamey",
                "Africa/Ndjamena",
                "Africa/Brazzaville",
                "Africa/Bangui",
                "Africa/Malabo",
                "Africa/Porto-Novo",
            ],
        ),
        zone(
            EuropeAfrica,
            n::EASTERN_EUROPE,
            c::EASTERN_EUROPE,
            "Europe/Kyiv",
            &[
                "Europe/Kiev",
                "Europe/Athens",
                "Europe/Helsinki",
                "Europe/Bucharest",
                "Europe/Sofia",
                "Europe/Riga",
                "Europe/Vilnius",
                "Europe/Tallinn",
                "Europe/Mariehamn",
                "Asia/Nicosia",
                "Europe/Nicosia",
                "Europe/Uzhgorod",
                "Europe/Zaporozhye",
            ],
        ),
        zone(EuropeAfrica, n::EGYPT, c::EGYPT, "Africa/Cairo", &[]),
        zone(EuropeAfrica, n::SOUTH_AFRICA, c::SOUTH_AFRICA, "Africa/Johannesburg", &["Africa/Maseru", "Africa/Mbabane"]),
        zone(EuropeAfrica, n::TURKEY, c::TURKEY, "Europe/Istanbul", &["Asia/Istanbul", "Turkey"]),
        zone(
            EuropeAfrica,
            n::EAST_AFRICA,
            c::EAST_AFRICA,
            "Africa/Nairobi",
            &["Africa/Addis_Ababa", "Africa/Dar_es_Salaam", "Africa/Kampala", "Africa/Mogadishu", "Africa/Djibouti", "Africa/Asmara"],
        ),
        zone(Russia, n::KALININGRAD, c::KALININGRAD, "Europe/Kaliningrad", &[]),
        zone(Russia, n::MOSCOW, c::MOSCOW, "Europe/Moscow", &["Europe/Kirov", "Europe/Volgograd", "W-SU"]),
        zone(Russia, n::SAMARA, c::SAMARA, "Europe/Samara", &["Europe/Astrakhan", "Europe/Saratov", "Europe/Ulyanovsk"]),
        zone(Russia, n::YEKATERINBURG, c::YEKATERINBURG, "Asia/Yekaterinburg", &[]),
        zone(Russia, n::OMSK, c::OMSK, "Asia/Omsk", &[]),
        zone(Russia, n::KRASNOYARSK, c::KRASNOYARSK, "Asia/Krasnoyarsk", &["Asia/Novosibirsk", "Asia/Novokuznetsk", "Asia/Barnaul", "Asia/Tomsk"]),
        zone(Russia, n::IRKUTSK, c::IRKUTSK, "Asia/Irkutsk", &[]),
        zone(Russia, n::YAKUTSK, c::YAKUTSK, "Asia/Yakutsk", &["Asia/Chita", "Asia/Khandyga"]),
        zone(Russia, n::VLADIVOSTOK, c::VLADIVOSTOK, "Asia/Vladivostok", &["Asia/Ust-Nera"]),
        zone(Russia, n::MAGADAN, c::MAGADAN, "Asia/Magadan", &["Asia/Sakhalin", "Asia/Srednekolymsk"]),
        zone(Russia, n::KAMCHATKA, c::KAMCHATKA, "Asia/Kamchatka", &["Asia/Anadyr"]),
        zone(Asia, n::ISRAEL, c::ISRAEL, "Asia/Jerusalem", &["Asia/Tel_Aviv", "Israel"]),
        zone(Asia, n::ARABIA, c::ARABIA, "Asia/Riyadh", &["Asia/Kuwait", "Asia/Qatar", "Asia/Bahrain", "Asia/Baghdad", "Asia/Aden"]),
        zone(Asia, n::IRAN, c::IRAN, "Asia/Tehran", &["Iran"]),
        zone(Asia, n::GULF, c::GULF, "Asia/Dubai", &["Asia/Muscat"]),
        zone(Asia, n::PAKISTAN, c::PAKISTAN, "Asia/Karachi", &[]),
        zone(Asia, n::UZBEKISTAN, c::UZBEKISTAN, "Asia/Tashkent", &["Asia/Samarkand"]),
        zone(
            Asia,
            n::KAZAKHSTAN,
            c::KAZAKHSTAN,
            "Asia/Almaty",
            &["Asia/Qyzylorda", "Asia/Aqtobe", "Asia/Aqtau", "Asia/Atyrau", "Asia/Oral", "Asia/Qostanay"],
        ),
        zone(Asia, n::INDIA, c::INDIA, "Asia/Kolkata", &["Asia/Calcutta"]),
        zone(Asia, n::NEPAL, c::NEPAL, "Asia/Kathmandu", &["Asia/Katmandu"]),
        zone(Asia, n::BANGLADESH, c::BANGLADESH, "Asia/Dhaka", &["Asia/Dacca"]),
        zone(Asia, n::MYANMAR, c::MYANMAR, "Asia/Yangon", &["Asia/Rangoon"]),
        zone(Asia, n::INDOCHINA, c::INDOCHINA, "Asia/Bangkok", &["Asia/Ho_Chi_Minh", "Asia/Saigon", "Asia/Phnom_Penh", "Asia/Vientiane"]),
        zone(Asia, n::INDONESIA_WESTERN, c::INDONESIA_WESTERN, "Asia/Jakarta", &["Asia/Pontianak"]),
        zone(Asia, n::CHINA, c::CHINA, "Asia/Shanghai", &["Asia/Chongqing", "Asia/Chungking", "Asia/Harbin", "PRC"]),
        zone(Asia, n::HONG_KONG, c::HONG_KONG, "Asia/Hong_Kong", &["Hongkong"]),
        zone(Asia, n::TAIPEI, c::TAIPEI, "Asia/Taipei", &["ROC"]),
        zone(Asia, n::SINGAPORE, c::SINGAPORE, "Asia/Singapore", &["Asia/Kuala_Lumpur", "Asia/Kuching", "Singapore"]),
        zone(Asia, n::PHILIPPINES, c::PHILIPPINES, "Asia/Manila", &[]),
        zone(Asia, n::INDONESIA_CENTRAL, c::INDONESIA_CENTRAL, "Asia/Makassar", &["Asia/Ujung_Pandang"]),
        zone(Asia, n::JAPAN, c::JAPAN, "Asia/Tokyo", &["Japan"]),
        zone(Asia, n::KOREA, c::KOREA, "Asia/Seoul", &["ROK"]),
        zone(Asia, n::INDONESIA_EASTERN, c::INDONESIA_EASTERN, "Asia/Jayapura", &[]),
        zone(Oceania, n::AUSTRALIA_WESTERN, c::AUSTRALIA_WESTERN, "Australia/Perth", &["Australia/West"]),
        zone(Oceania, n::AUSTRALIA_CENTRAL_STANDARD, c::AUSTRALIA_CENTRAL_STANDARD, "Australia/Darwin", &["Australia/North"]),
        zone(Oceania, n::AUSTRALIA_CENTRAL, c::AUSTRALIA_CENTRAL, "Australia/Adelaide", &["Australia/South", "Australia/Broken_Hill"]),
        zone(Oceania, n::AUSTRALIA_EASTERN_STANDARD, c::AUSTRALIA_EASTERN_STANDARD, "Australia/Brisbane", &["Australia/Queensland", "Australia/Lindeman"]),
        zone(
            Oceania,
            n::AUSTRALIA_EASTERN,
            c::AUSTRALIA_EASTERN,
            "Australia/Sydney",
            &["Australia/Melbourne", "Australia/Canberra", "Australia/Hobart", "Australia/ACT", "Australia/NSW", "Australia/Victoria", "Australia/Tasmania"],
        ),
        zone(Oceania, n::NEW_ZEALAND, c::NEW_ZEALAND, "Pacific/Auckland", &["NZ"]),
        Zone {
            name: n::UTC,
            cities: same("UTC"),
            tz: UTC,
            also: &["Etc/UTC", "Etc/UCT", "UCT", "Etc/Universal", "Universal", "Etc/Zulu", "Zulu", "Etc/GMT", "GMT", "Etc/Greenwich", "Greenwich"],
            region: None,
        },
    ]
};

/// The name of Coordinated Universal Time in the tz database.
pub const UTC: &str = "UTC";

/// The zone the clock keeps until the installed system's is known or
/// another is chosen, by its name in the tz database.
pub const FIRST_ZONE: &str = "Asia/Shanghai";

/// Where the zone the tz database names `tz` is in [`TIME_ZONES`], by its
/// own name or one of those that keep its time under its name.
pub fn listed(tz: &str) -> Option<usize> {
    TIME_ZONES.iter().position(|zone| zone.tz == tz || zone.also.contains(&tz))
}

/// The time zone the installed system is set to.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemZone {
    /// Its name in the tz database.
    pub name: String,
    pub rules: TimeZone,
}

impl SystemZone {
    /// The zone the tz database names `name`, with the rules the recovery
    /// carries for it.
    pub fn named(name: &str) -> Result<SystemZone> {
        let rules = TimeZone::get(name).with_context(|| format!("the recovery carries no rules for the time zone {name:?}"))?;
        Ok(SystemZone { name: name.to_string(), rules })
    }
}

impl State {
    /// The installed system's time zone, while the clock keeps it.
    pub fn followed(&self) -> Option<&SystemZone> {
        self.system_zone.as_ref().filter(|_| self.follow_system_zone)
    }

    /// The rules of the zone the clock and the times on the pages keep:
    /// the installed system's while it is followed and known, and the one
    /// chosen otherwise.
    pub fn zone(&self) -> &TimeZone {
        match self.followed() {
            Some(system) => &system.rules,
            None => fmt::rules(self.time_zone),
        }
    }

    /// Where the zone the clock keeps is in [`TIME_ZONES`]; none for a
    /// zone of the installed system's that is not listed.
    pub fn zone_listed(&self) -> Option<usize> {
        match self.followed() {
            Some(system) => listed(&system.name),
            None => Some(self.time_zone),
        }
    }

    /// What the zone the clock keeps is called: its name where it is
    /// listed, and otherwise the name the tz database gives it.
    pub fn zone_name(&self) -> &str {
        match (self.zone_listed(), self.followed()) {
            (Some(at), _) => self.lang.pick(TIME_ZONES[at].name),
            (None, Some(system)) => &system.name,
            (None, None) => unreachable!("a zone chosen is one listed"),
        }
    }
}

impl App {
    /// The installed system is set to the time zone the tz database names
    /// `name`: the clock keeps it while the system's zone is followed.
    pub fn set_system_zone(&mut self, name: &str) -> Result<()> {
        let zone = SystemZone::named(name)?;
        if self.state.system_zone.as_ref() != Some(&zone) {
            self.state.system_zone = Some(zone);
            self.refresh();
        }
        Ok(())
    }

    /// The time is `unix`, in Unix seconds. The time zone pages tell how
    /// far each zone is from UTC now, to the minute, and are drawn again
    /// when a zone's clocks go forward or back.
    pub fn set_now(&mut self, unix: i64) {
        let minute = unix - unix.rem_euclid(60);
        if minute == self.state.now {
            return;
        }
        let before = self.offsets();
        self.state.now = minute;
        if matches!(self.screen(), Screen::TimeZone | Screen::TimeZones(_)) && self.offsets() != before {
            self.refresh();
        }
    }

    /// How far from UTC each zone the time zone pages tell of is now.
    fn offsets(&self) -> Vec<i32> {
        let now = self.state.now;
        let listed = (0..TIME_ZONES.len()).map(|at| fmt::utc_offset_seconds(now, fmt::rules(at)));
        listed.chain(self.state.system_zone.iter().map(|system| fmt::utc_offset_seconds(now, &system.rules))).collect()
    }

    pub(super) fn set_slider(&mut self, t: &Theme, which: Slider, track: Rect, x: f32) {
        let travel = (track.w - track.h).max(1.0);
        self.set_level(t, which, track, (x - track.x - track.h / 2.0) / travel);
    }

    /// Where a slider stands, as a share of its span.
    pub(super) fn level(&self, which: Slider) -> f32 {
        match which {
            Slider::Brightness => self.state.brightness,
            Slider::Vibration => self.state.vibration,
        }
    }

    /// Set a slider to `level` of its span, the brightness no lower than
    /// its floor, and draw the slider at `track` on the page again.
    pub(super) fn set_level(&mut self, t: &Theme, which: Slider, track: Rect, level: f32) {
        let value = match which {
            Slider::Brightness => &mut self.state.brightness,
            Slider::Vibration => &mut self.state.vibration,
        };
        *value = level.clamp(if which == Slider::Brightness { MIN_BRIGHTNESS } else { 0.0 }, 1.0);
        let value = *value;
        if let Some(fr) = self.frame.as_mut() {
            fr.canvas.fill_rect(track.x, track.y, track.w, track.h, t.pal.card);
            widgets::slider(&mut fr.canvas, t, track.x, track.y, track.w, value);
        }
        self.layer = None;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn the_file_transfer_switch_has_the_runtime_follow_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        assert!(app.state.file_transfer, "on until turned off");
        let (_, y) = spot(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer));
        if y > H as f32 * 0.9 {
            scroll_to(&mut app, &f, y - H as f32 * 0.8, 0);
        }
        assert_eq!(tap_at(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer), 2000), [Effect::FileTransfer(false)]);
        assert!(!app.state.file_transfer);
        assert_eq!(tap_at(&mut app, &f, &i, Action::Toggle(Switch::FileTransfer), 3000), [Effect::FileTransfer(true)]);
        app.file_transfer_failed(false, "无法开启文件传输", 4000);
        assert!(!app.state.file_transfer, "the switch shows how it stands");
    }

    #[test]
    fn theme_previews_switch_the_theme() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        tap(&mut app, &f, &i, Action::Theme { dark: true });
        assert!(app.state.dark);
        assert!(app.theme().dark);
        tap(&mut app, &f, &i, Action::Theme { dark: false });
        assert!(!app.state.dark);
    }

    #[test]
    fn dragging_the_track_sets_brightness() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Slider(Slider::Brightness)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        let travel = track.w - track.h;
        assert!(app.touch(Touch::Down(track.x + track.h / 2.0 + travel * 0.25, y), &f, 0).is_empty());
        assert!((app.panel(0).unwrap() - 0.25).abs() < 1e-5);
        assert!(app.touch(Touch::Move(track.x + track.h / 2.0 + travel * 0.75, y + 40.0), &f, 0).is_empty());
        assert!((app.panel(0).unwrap() - 0.75).abs() < 1e-5);
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "the panel shows the brightness");
        assert_eq!(app.state.brightness, 0.75);
    }

    #[test]
    fn the_strength_slider_taps_the_motor_as_strongly_as_it_is_let_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        let track = app.frame(&f, &i).hits.iter().find(|h| h.action == Action::Slider(Slider::Vibration)).unwrap().rect;
        let y = track.centre().1 - app.top().1;
        assert!(app.touch(Touch::Down(track.x + track.w * 0.25, y), &f, 0).is_empty());
        let x = track.x + track.h / 2.0 + (track.w - track.h) * 0.8;
        assert!(app.touch(Touch::Move(x, y + 40.0), &f, 0).is_empty(), "nothing taps while the finger moves");
        assert_eq!(app.touch(Touch::Up, &f, 0), vec![Effect::Haptic], "let go, the motor taps");
        assert!((app.state.vibration - 0.8).abs() < 1e-5);
        assert!(app.touch(Touch::Down(track.x - 50.0, y), &f, 0).is_empty());
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "touched beside the track, the slider stays");
        app.state.vibrate = false;
        app.touch(Touch::Down(track.x, y), &f, 0);
        assert!(app.touch(Touch::Up, &f, 0).is_empty(), "with touch vibration off, the motor stays still");
        assert_eq!(app.state.vibration, 0.0, "the strength still moves, down to the slider's left end");
    }

    #[test]
    fn the_language_chosen_is_spoken_on_every_page() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        tap(&mut app, &f, &i, Action::Open(Screen::Language));
        assert_eq!(app.frame(&f, &i).title, "语言");
        tap(&mut app, &f, &i, Action::Language(Lang::En));
        assert_eq!((app.state.lang, app.screen()), (Lang::En, Screen::Language), "the page stays");
        assert_eq!(app.frame(&f, &i).title, "Language");
        tap(&mut app, &f, &i, Action::Language(Lang::Hant));
        assert_eq!(app.frame(&f, &i).title, "語言");
        tap_back(&mut app, &f, &i, 0);
        app.tick(1000);
        assert_eq!((app.screen(), app.frame(&f, &i).title), (Screen::Settings, "設定"), "and so does the page gone back to");
        app.set_mounts(Ok(vec![MountRow { base: "odm".into(), point: "/odm".into(), mounted: true, holders: vec!["qseecomd".into()] }]));
        tap(&mut app, &f, &i, Action::Open(Screen::Language));
        tap(&mut app, &f, &i, Action::Language(Lang::En));
        tap_back(&mut app, &f, &i, 0);
        app.tick(2000);
        tap_back(&mut app, &f, &i, 2000);
        app.tick(3000);
        tap(&mut app, &f, &i, Action::Open(Screen::Mount));
        tap(&mut app, &f, &i, Action::Unavailable("A partition in use can't be unmounted"));
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("A partition in use can't be unmounted"), "as do its messages");
    }

    #[test]
    fn a_language_chosen_is_kept_and_spoken_over_the_one_the_system_names() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        app.take_settings(None, Some(Lang::Hant));
        assert_eq!((app.state.lang, app.settings().language), (Lang::Hant, None), "the system's, until one is chosen");
        tap(&mut app, &f, &i, Action::Open(Screen::Language));
        tap(&mut app, &f, &i, Action::Language(Lang::En));
        let kept = app.settings();
        assert_eq!(kept.language.as_deref(), Some("en"));

        let mut next = App::new(Screen::Home, W, H, PPD);
        next.take_settings(Some(&kept), Some(Lang::Hans));
        assert_eq!(next.state.lang, Lang::En, "the next start speaks the language chosen");
        assert_eq!(next.settings(), kept, "and keeps it chosen");
    }

    #[test]
    fn with_no_language_chosen_a_start_speaks_the_system_s_or_else_the_one_last_spoken() {
        let spoke_english = Settings { spoken: Some("en".into()), ..Settings::default() };
        let mut app = App::new(Screen::Home, W, H, PPD);
        app.take_settings(Some(&spoke_english), Some(Lang::Hant));
        assert_eq!(app.state.lang, Lang::Hant, "the one the system names");
        let mut fastboot = App::new(Screen::Fastboot, W, H, PPD);
        fastboot.take_settings(Some(&spoke_english), None);
        assert_eq!(fastboot.state.lang, Lang::En, "a start that names none, as fastboot's");
        let mut first = App::new(Screen::Fastboot, W, H, PPD);
        first.take_settings(None, None);
        assert_eq!(first.state.lang, State::default().lang, "and with nothing kept, the pages' own");
    }

    #[test]
    fn the_other_pages_choices_are_kept_too() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        tap(&mut app, &f, &i, Action::Toggle(Switch::Vibrate));
        app.state.check_signature = true;
        app.state.backup_to = Place::UsbDrive;
        app.state.logs_to = 1;
        let kept = app.settings();
        let mut next = App::new(Screen::Home, W, H, PPD);
        next.take_settings(Some(&kept), None);
        assert!(!next.state.vibrate && next.state.check_signature);
        assert_eq!((next.state.backup_to, next.state.logs_to), (Place::UsbDrive, 1));
    }

    #[test]
    fn logs_are_exported_as_ticked_where_they_can_go() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Logs, W, H, PPD);
        let both = Effect::ExportLogs { recovery: true, kernel: true, storage: false };
        assert_eq!(tap(&mut app, &f, &i, Action::ExportLogs), vec![both], "both, to the temporary folder, at first");
        tap(&mut app, &f, &i, Action::LogPart(0));
        tap(&mut app, &f, &i, Action::LogsTo(1));
        assert!(tap(&mut app, &f, &i, Action::ExportLogs).is_empty(), "internal storage waits for the data partition");
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(TOAST_MS), "and says so");
        app.data_waiting(None);
        assert_eq!(tap(&mut app, &f, &i, Action::ExportLogs), vec![Effect::ExportLogs { recovery: false, kernel: true, storage: true }]);
        tap(&mut app, &f, &i, Action::LogPart(1));
        assert!(tap(&mut app, &f, &i, Action::ExportLogs).is_empty(), "nothing ticked, nothing saved");
    }

    /// Where a zone is in [`TIME_ZONES`], by its name in the tz database.
    fn zone(tz: &str) -> usize {
        listed(tz).expect("a zone listed")
    }

    /// 2026-01-15 12:00 UTC, and 2026-07-01 12:00 UTC.
    const WINTER: i64 = 1_768_478_400;
    const SUMMER: i64 = 1_782_907_200;

    /// Whether the page shown has a control that does `action`.
    fn offers(app: &mut App, f: &Fonts, i: &DeviceInfo, action: Action) -> bool {
        app.frame(f, i).hits.iter().any(|h| h.action == action)
    }

    #[test]
    fn a_time_zone_chosen_from_its_part_of_the_world_is_the_one_kept() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        app.set_now(WINTER);
        tap(&mut app, &f, &i, Action::Open(Screen::TimeZone));
        assert_eq!(app.state.time_zone, zone("Asia/Shanghai"), "China until another is chosen");
        assert!(!offers(&mut app, &f, &i, Action::Toggle(Switch::SystemTimeZone)), "a phone with no system has no zone of its own to follow");
        tap(&mut app, &f, &i, Action::Open(Screen::TimeZones(Region::EuropeAfrica)));
        tap(&mut app, &f, &i, Action::TimeZone(zone("Europe/London")));
        assert_eq!((app.state.time_zone, app.screen()), (zone("Europe/London"), Screen::TimeZones(Region::EuropeAfrica)), "the page stays");
        assert_eq!(fmt::clock(WINTER, app.state.zone()), "12:00");
        tap_back(&mut app, &f, &i, 1000);
        tap_at(&mut app, &f, &i, Action::TimeZone(zone(UTC)), 2000);
        assert_eq!(app.state.zone_name(), "协调世界时");
    }

    #[test]
    fn the_system_s_time_zone_is_followed_until_another_is_chosen() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::TimeZone, W, H, PPD);
        app.set_now(SUMMER);
        app.set_system_zone("Europe/Rome").unwrap();
        assert_eq!(app.state.zone_listed(), Some(zone("Europe/Berlin")), "Rome keeps Central European Time");
        assert_eq!(app.state.zone_name(), "中欧时间");
        assert_eq!(fmt::clock(SUMMER, app.state.zone()), "14:00");
        // Turned off, the switch leaves the clock in the zone it kept.
        tap(&mut app, &f, &i, Action::Toggle(Switch::SystemTimeZone));
        assert!(!app.state.follow_system_zone);
        assert_eq!(app.state.time_zone, zone("Europe/Berlin"));
        assert_eq!(fmt::clock(SUMMER, app.state.zone()), "14:00");
        tap_at(&mut app, &f, &i, Action::TimeZone(zone(UTC)), 1000);
        assert_eq!(fmt::clock(SUMMER, app.state.zone()), "12:00");
        tap_at(&mut app, &f, &i, Action::Toggle(Switch::SystemTimeZone), 2000);
        assert!(app.state.follow_system_zone);
        assert_eq!(fmt::clock(SUMMER, app.state.zone()), "14:00", "Rome's again");
        tap_at(&mut app, &f, &i, Action::TimeZone(zone(UTC)), 3000);
        assert!(!app.state.follow_system_zone, "a zone chosen is kept in place of the system's");
    }

    #[test]
    fn a_zone_of_the_system_s_that_is_not_listed_is_kept_under_its_own_name() {
        let mut app = App::new(Screen::TimeZone, W, H, PPD);
        app.set_system_zone("Asia/Kabul").unwrap();
        assert_eq!((app.state.zone_listed(), app.state.zone_name()), (None, "Asia/Kabul"));
        assert_eq!(fmt::utc_offset(SUMMER, app.state.zone()), "UTC+04:30");
        assert!(app.set_system_zone("Mars/Olympus").is_err(), "a name the recovery carries no rules for");
        assert_eq!(app.state.zone_name(), "Asia/Kabul", "the zone kept stays");
        app.state.follow_system_zone = false;
        assert_eq!(app.state.zone_name(), "中国标准时间");
    }

    #[test]
    fn a_part_of_the_world_lists_its_zones_by_how_far_they_are_from_utc_now() {
        let (f, i) = (fonts(), info());
        for now in [WINTER, SUMMER] {
            for region in Region::ALL {
                let mut app = App::new(Screen::TimeZones(region), W, H, PPD);
                app.set_now(now);
                let zones: Vec<usize> = app.frame(&f, &i).hits.iter().map(|h| match h.action {
                    Action::TimeZone(at) => at,
                    other => panic!("{region:?} lists only zones, not {other:?}"),
                }).collect();
                let of_region = TIME_ZONES.iter().filter(|zone| zone.region == Some(region)).count();
                assert_eq!(zones.len(), of_region, "{region:?} lists each of its zones");
                let offsets: Vec<i32> = zones.iter().map(|&at| fmt::utc_offset_seconds(now, fmt::rules(at))).collect();
                assert!(offsets.is_sorted(), "{region:?} at {now}: {offsets:?}");
            }
        }
    }

    #[test]
    fn the_time_zone_pages_are_drawn_again_when_a_zone_s_clocks_change() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::TimeZones(Region::EuropeAfrica), W, H, PPD);
        // 2026-03-29 00:59 UTC, a minute before Europe's clocks go forward.
        let before = 1_774_745_940;
        app.set_now(before);
        app.frame(&f, &i);
        app.set_now(before + 30);
        assert!(app.frame.is_some(), "the same minute");
        app.set_now(before + 60);
        assert!(app.frame.is_none(), "London is an hour ahead of UTC now");
        app.frame(&f, &i);
        app.set_now(before + 120);
        assert!(app.frame.is_some(), "no zone's clocks changed in this minute");
    }

    #[test]
    fn choosing_how_long_the_screen_stays_on() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Settings, &f, &i);
        tap(&mut app, &f, &i, Action::Open(Screen::ScreenOff));
        assert_eq!(app.screen(), Screen::ScreenOff);
        let frame = app.frame(&f, &i);
        assert_eq!(frame.title, "息屏时间");
        assert!(frame.back);
        // The choices sit in order, top to bottom.
        let actions: Vec<Action> = frame.hits.iter().map(|h| h.action).collect();
        assert_eq!(actions, (0..SCREEN_OFF.len()).map(Action::ScreenOff).collect::<Vec<_>>());
        tap(&mut app, &f, &i, Action::ScreenOff(0));
        assert_eq!(app.state.screen_off, 0);
        assert_eq!(app.screen(), Screen::ScreenOff, "the page stays");
        // Fifteen seconds, dim for the last three, once the row tapped has
        // faded back.
        app.tick(1000);
        assert_eq!(app.deadline(1000), Some(12_000));
        app.tick(15_000);
        assert!(!app.display_on());
    }
}
