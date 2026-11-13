use anyhow::{anyhow, bail, Context, Result};
use skyward_config::DeviceConfig;
use skyward_sys::{prop, ToldOnce};
use skyward_vab::{Slot, SnapshotState};
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

/// Version of this build, shown on the About page.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const CPU: &str = "/sys/devices/system/cpu";
const POWER_SUPPLY: &str = "/sys/class/power_supply";
const SYS_BLOCK: &str = "/sys/class/block";

/// What the recovery shows about the device. A source that may legitimately
/// be absent (no battery driver loaded, build properties missing from the
/// recovery ramdisk) is an `Option`; a source that exists but cannot be read
/// or parsed is an error. The installed system's partitions are the one
/// source the recovery has to do without: where their build properties
/// cannot be read, the details are as where nothing is mounted, and the
/// log says why.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub model: String,
    pub codename: String,
    pub soc: Option<skyward_config::Names>,
    pub cpu: Cpu,
    /// `MemTotal` in KiB.
    pub ram_kib: u64,
    /// Panel resolution as width × height in pixels.
    pub panel: Option<(u32, u32)>,
    /// Marketed screen diagonal in inches.
    pub screen: Option<f32>,
    pub os_version: Option<String>,
    pub android: Option<String>,
    /// The locale the installed system is built to start in, such as
    /// `zh-CN`.
    pub locale: Option<String>,
    pub security_patch: Option<PatchLevel>,
    pub kernel: String,
    pub storage: Storage,
    pub battery: Option<Battery>,
    pub slot: Option<Slot>,
    pub snapshot: SnapshotState,
    pub boot: Boot,
}

/// `ro.build.version.security_patch`, a calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchLevel {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl FromStr for PatchLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        let [y, m, d] = parts.as_slice() else { bail!("security patch {s:?} is not YYYY-MM-DD") };
        let level = PatchLevel { year: parse(y, "patch year")?, month: parse(m, "patch month")?, day: parse(d, "patch day")? };
        if !(1..=12).contains(&level.month) || !(1..=31).contains(&level.day) {
            bail!("security patch {s:?} is not a date");
        }
        Ok(level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cpu {
    pub cores: u32,
    /// Highest `cpuinfo_max_freq` across cores in kHz; absent until the
    /// cpufreq driver is loaded.
    pub max_khz: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Storage {
    /// Size of the userdata partition in bytes.
    pub total: u64,
    /// Free bytes on /data, counted like `File.getFreeSpace()`, while /data
    /// is mounted.
    pub free: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    NotCharging,
    Full,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub level: u8,
    pub charge: Charge,
    /// Tenths of a degree Celsius.
    pub temp: Option<i32>,
    /// Kernel `POWER_SUPPLY_HEALTH` text such as `Good`.
    pub health: Option<String>,
    pub cycles: Option<u32>,
    /// Design capacity in µAh.
    pub design: Option<u64>,
    /// Present full-charge capacity in µAh.
    pub full: Option<u64>,
    /// Voltage in µV.
    pub voltage: Option<u64>,
    /// A charger charging it quickly, as Xiaomi's Qualcomm phones tell
    /// beside their power supplies.
    pub quick: bool,
    /// A charger or a computer powers the phone, by cable or wirelessly: a
    /// power supply besides the battery is online.
    pub plugged: bool,
}

impl Battery {
    /// Whether it charges, or is full and topped up.
    pub fn charging(&self) -> bool {
        matches!(self.charge, Charge::Charging | Charge::Full)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boot {
    /// From `androidboot.vbmeta.device_state`.
    pub unlocked: Option<bool>,
    /// `androidboot.verifiedbootstate`: green, yellow, orange or red.
    pub verified: Option<String>,
}

pub fn collect(cfg: &DeviceConfig) -> Result<DeviceInfo> {
    collect_with(cfg, battery_now()?)
}

/// The phone's details with `battery` for the battery, read apart: by one
/// who reads it oftener than the rest, or who goes on without it where it
/// cannot be read, as the pages do when they come up.
pub fn collect_with(cfg: &DeviceConfig, battery: Option<Battery>) -> Result<DeviceInfo> {
    let d = &cfg.display;
    let unlocked = match skyward_block::boot_param("androidboot.vbmeta.device_state")?.as_deref() {
        None => None,
        Some("locked") => Some(false),
        Some("unlocked") => Some(true),
        Some(other) => bail!("androidboot.vbmeta.device_state {other:?}"),
    };
    let system = SystemProps::current()?;
    Ok(DeviceInfo {
        // The system names the exact edition; the configured model covers a
        // system whose partitions are not mounted.
        model: system.get("ro.product.marketname").unwrap_or_else(|| cfg.device.model.clone()),
        codename: cfg.device.codename.clone(),
        soc: cfg.device.soc.clone(),
        cpu: cpu(Path::new(CPU))?,
        ram_kib: mem_total(&read("/proc/meminfo")?)?,
        panel: (d.width > 0 && d.height > 0).then_some((d.width, d.height)),
        screen: d.diagonal,
        os_version: system.get(&cfg.device.os_version_prop),
        android: system.get("ro.build.version.release"),
        locale: system.get("ro.product.locale"),
        security_patch: system.get("ro.build.version.security_patch").map(|s| s.parse()).transpose()?,
        kernel: kernel_release()?,
        storage: storage()?,
        battery,
        slot: skyward_vab::current_slot()?,
        snapshot: skyward_vab::snapshot_state()?,
        boot: Boot { unlocked, verified: skyward_block::boot_param("androidboot.verifiedbootstate")? },
    })
}

/// Build properties of the system the About page describes.
enum SystemProps {
    /// Android is running and keeps them in its property area.
    Running,
    /// Recovery is running; they come from the build.prop files of the
    /// installed system's mounted partitions, none when nothing is mounted.
    Installed(HashMap<String, String>),
}

/// Why the installed system's properties could last not be read, as the
/// log was told: the details are collected again and again.
static PROPS_UNREAD: Mutex<ToldOnce> = Mutex::new(ToldOnce::new());

impl SystemProps {
    /// A recovery has to come up whatever state the installed system's
    /// partitions are in, so properties that cannot be read are none, as
    /// where nothing is mounted, and the log is told why.
    fn current() -> Result<SystemProps> {
        if !skyward_config::ramdisk::in_recovery() {
            return Ok(SystemProps::Running);
        }
        let read = match skyward_vab::current_slot()? {
            Some(slot) => installed_props(slot),
            None => Ok(HashMap::new()),
        };
        let why = read.as_ref().err().map(|e| format!("{e:#}"));
        if PROPS_UNREAD.lock().expect("nothing panics while it is held").untold(why.as_deref()) {
            skyward_sys::kmsg(&format!("the installed system's properties could not be read, and the phone's details go without them: {}", why.unwrap_or_default()))?;
        }
        Ok(SystemProps::Installed(read.unwrap_or_default()))
    }

    fn get(&self, name: &str) -> Option<String> {
        match self {
            SystemProps::Running => prop(name),
            SystemProps::Installed(props) => props.get(name).cloned(),
        }
    }
}

/// The order init reads build.prop files in at boot; later files override
/// earlier ones.
const PROP_ORDER: [&str; 8] = ["system", "system_ext", "system_dlkm", "vendor", "vendor_dlkm", "odm_dlkm", "odm", "product"];

/// What init names the properties the bootloader passes.
const BOOT_PROPS: &str = "ro.boot.";

/// The installed system's build properties from its mounted partitions, in
/// init's order. Partitions init does not list, such as Xiaomi's mi_ext,
/// come after the ones it does. Init has the properties the bootloader
/// passes before it reads any of these, and a file may import another named
/// after them, as Xiaomi's odm imports what each hardware version sets; so
/// they come first here too, the same in a recovery as in the system.
pub fn installed_props(slot: Slot) -> Result<HashMap<String, String>> {
    let base = |m: &skyward_vab::Mounted| -> String {
        m.partition.strip_suffix(slot.suffix()).expect("mounted partitions carry the slot suffix").to_string()
    };
    let mut parts = skyward_vab::mounted_partitions(slot)?;
    parts.sort_by_key(|m| PROP_ORDER.iter().position(|b| *b == base(m)).unwrap_or(PROP_ORDER.len()));
    let mut props: HashMap<String, String> = skyward_sys::props(BOOT_PROPS).into_iter().collect();
    for m in &parts {
        let files: &[&str] = match base(m).as_str() {
            "system" => &["system/build.prop"],
            "vendor" => &["default.prop", "build.prop"],
            _ => &["etc/build.prop"],
        };
        for f in files {
            let path = m.mount_point.join(f);
            match fs::metadata(&path) {
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("stat {}", path.display())),
                Ok(_) => {}
            }
            rsproperties::load_properties_from_file(&path, None, "u:r:init:s0", &mut props)
                .with_context(|| format!("read {}", path.display()))?;
        }
    }
    Ok(props)
}

fn read(path: impl AsRef<Path>) -> Result<String> {
    let p = path.as_ref();
    Ok(fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?.trim().to_string())
}

/// `None` when the file does not exist; any other failure is an error.
fn read_opt(path: impl AsRef<Path>) -> Result<Option<String>> {
    let p = path.as_ref();
    match fs::read_to_string(p) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
    }
}

fn parse<T: FromStr>(s: &str, what: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(|e| anyhow!("{what}: {s:?}: {e}"))
}

fn parse_opt<T: FromStr>(path: impl AsRef<Path>, what: &str) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    read_opt(path)?.map(|s| parse(&s, what)).transpose()
}

fn cpu(root: &Path) -> Result<Cpu> {
    let ids = cpulist(&read(root.join("possible"))?)?;
    let mut max_khz: Option<u32> = None;
    for id in &ids {
        let path = root.join(format!("cpu{id}/cpufreq/cpuinfo_max_freq"));
        if let Some(khz) = parse_opt::<u32>(&path, "cpuinfo_max_freq")? {
            max_khz = Some(max_khz.map_or(khz, |m| m.max(khz)));
        }
    }
    Ok(Cpu { cores: ids.len() as u32, max_khz })
}

/// Kernel cpulist syntax: comma-separated ids and inclusive ranges (`0-3,6`).
fn cpulist(s: &str) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    for part in s.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u32, u32) = (parse(a, "cpulist")?, parse(b, "cpulist")?);
                if a > b {
                    bail!("cpulist range {part:?} runs backwards");
                }
                ids.extend(a..=b);
            }
            None => ids.push(parse(part, "cpulist")?),
        }
    }
    Ok(ids)
}

fn mem_total(meminfo: &str) -> Result<u64> {
    let line = meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .ok_or_else(|| anyhow!("MemTotal missing from /proc/meminfo"))?;
    let kib = line["MemTotal:".len()..]
        .trim()
        .strip_suffix("kB")
        .ok_or_else(|| anyhow!("MemTotal is not in kB: {line:?}"))?;
    parse(kib.trim(), "MemTotal")
}

fn kernel_release() -> Result<String> {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } != 0 {
        return Err(std::io::Error::last_os_error()).context("uname");
    }
    let release = unsafe { CStr::from_ptr(u.release.as_ptr()) };
    Ok(release.to_str().context("kernel release is not UTF-8")?.to_string())
}

fn storage() -> Result<Storage> {
    let link = Path::new(skyward_block::BY_NAME).join("userdata");
    let dev = fs::canonicalize(&link).with_context(|| format!("resolve {}", link.display()))?;
    let name = dev.file_name().ok_or_else(|| anyhow!("{} has no device name", dev.display()))?;
    let sectors: u64 = parse(&read(Path::new(SYS_BLOCK).join(name).join("size"))?, "block size")?;
    let data = Path::new("/data");
    let free = if is_mountpoint(data)? { Some(skyward_block::mount::space(data).context("read the room in /data")?.free) } else { None };
    Ok(Storage { total: sectors * 512, free })
}

/// A directory is a mountpoint when it lives on a different filesystem from
/// its parent.
fn is_mountpoint(path: &Path) -> Result<bool> {
    let parent = path.parent().ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    let here = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let up = fs::metadata(parent).with_context(|| format!("stat {}", parent.display()))?;
    Ok(here.dev() != up.dev())
}

/// The battery as it is now.
pub fn battery_now() -> Result<Option<Battery>> {
    battery(Path::new(POWER_SUPPLY))
}

/// The first power supply whose type is `Battery`; `None` while no fuel-gauge
/// driver is loaded.
fn battery(root: &Path) -> Result<Option<Battery>> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", root.display())),
    };
    let mut dirs: Vec<_> = entries.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    dirs.sort();
    let mut first = None;
    let mut plugged = false;
    for dir in dirs {
        match read_opt(dir.join("type"))?.as_deref() {
            Some("Battery") => {
                first.get_or_insert(dir);
            }
            Some(_) => plugged |= online(&dir)?,
            None => {}
        }
    }
    let Some(dir) = first else { return Ok(None) };
    let charge = match read(dir.join("status"))?.as_str() {
        "Charging" => Charge::Charging,
        "Discharging" => Charge::Discharging,
        "Not charging" => Charge::NotCharging,
        "Full" => Charge::Full,
        "Unknown" => Charge::Unknown,
        other => bail!("{}: status {other:?}", dir.display()),
    };
    Ok(Some(Battery {
        level: parse(&read(dir.join("capacity"))?, "capacity")?,
        charge,
        temp: parse_opt(dir.join("temp"), "temp")?,
        health: read_opt(dir.join("health"))?,
        cycles: parse_opt(dir.join("cycle_count"), "cycle_count")?,
        design: parse_opt(dir.join("charge_full_design"), "charge_full_design")?,
        full: parse_opt(dir.join("charge_full"), "charge_full")?,
        voltage: parse_opt(dir.join("voltage_now"), "voltage_now")?,
        // How quickly the charger charges: 0 normally, then fast, flash,
        // turbo and super, which the stock SystemUI all counts quick.
        quick: parse_opt::<u8>(root.with_file_name("qcom-battery").join("quick_charge_type"), "quick_charge_type")?.is_some_and(|k| (1..=4).contains(&k)),
        plugged,
    }))
}

/// Whether the power supply in `dir` says a charger or a computer is
/// attached to it.
fn online(dir: &Path) -> Result<bool> {
    let path = dir.join("online");
    said_online(fs::read_to_string(&path)).with_context(|| format!("read {}", path.display()))
}

/// What a power supply's `online` file says, as reading it went. A supply
/// without one has no say, and one whose driver cannot tell yet, as
/// Qualcomm's USB-C connector cannot early in a boot, answers "try again":
/// either is taken as offline, and the next reading asks again.
fn said_online(read: std::io::Result<String>) -> std::io::Result<bool> {
    match read {
        Ok(s) => Ok(s.trim() == "1"),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::WouldBlock) => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpulist_ranges_and_singles() {
        assert_eq!(cpulist("0-7").unwrap(), (0..8).collect::<Vec<_>>());
        assert_eq!(cpulist("0-3,6,8-9").unwrap(), vec![0, 1, 2, 3, 6, 8, 9]);
        assert_eq!(cpulist("0").unwrap(), vec![0]);
        assert!(cpulist("3-1").is_err());
        assert!(cpulist("").is_err());
    }

    #[test]
    fn mem_total_from_meminfo() {
        // Values reported by a Xiaomi 17 Ultra.
        let text = "MemTotal:       15472464 kB\nMemFree:         2381608 kB\n";
        assert_eq!(mem_total(text).unwrap(), 15_472_464);
        assert!(mem_total("MemFree: 1 kB\n").is_err());
    }

    #[test]
    fn patch_level_parses_dates_only() {
        let p: PatchLevel = "2026-10-01".parse().unwrap();
        assert_eq!(p, PatchLevel { year: 2026, month: 10, day: 1 });
        assert!("2026-13-01".parse::<PatchLevel>().is_err());
        assert!("2026-10".parse::<PatchLevel>().is_err());
        assert!("October 2026".parse::<PatchLevel>().is_err());
    }

    #[test]
    fn kernel_release_is_readable() {
        assert!(!kernel_release().unwrap().is_empty());
    }

    #[test]
    fn a_battery_charged_quickly_says_so_beside_the_power_supplies() {
        // The 17 Ultra's in the recovery, on a charger it charges slowly on.
        let class = tempfile::tempdir().unwrap();
        let supplies = class.path().join("power_supply");
        let bat = supplies.join("battery");
        fs::create_dir_all(&bat).unwrap();
        for (name, value) in [("type", "Battery"), ("status", "Charging"), ("capacity", "96"), ("temp", "377")] {
            fs::write(bat.join(name), format!("{value}\n")).unwrap();
        }
        let b = battery(&supplies).unwrap().expect("a battery");
        assert_eq!((b.level, b.temp, b.quick), (96, Some(377), false), "a phone that does not tell charges as it may");
        let qcom = class.path().join("qcom-battery");
        fs::create_dir_all(&qcom).unwrap();
        fs::write(qcom.join("quick_charge_type"), "0\n").unwrap();
        assert!(!battery(&supplies).unwrap().expect("a battery").quick);
        fs::write(qcom.join("quick_charge_type"), "3\n").unwrap();
        assert!(battery(&supplies).unwrap().expect("a battery").quick, "turbo charging is quick");
    }

    #[test]
    fn the_phone_is_plugged_in_while_a_power_supply_besides_the_battery_is_online() {
        // The 13 Ultra's power supplies in the recovery.
        let class = tempfile::tempdir().unwrap();
        let supplies = class.path().join("power_supply");
        let supply = |name: &str, files: &[(&str, &str)]| {
            let dir = supplies.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, value) in files {
                fs::write(dir.join(file), format!("{value}\n")).unwrap();
            }
        };
        supply("battery", &[("type", "Battery"), ("status", "Discharging"), ("capacity", "80")]);
        supply("usb", &[("type", "USB"), ("online", "0")]);
        supply("wireless", &[("type", "Wireless"), ("online", "0")]);
        supply("ucsi-source-psy-soc:qcom,pmic_glink:qcom,ucsi1", &[("type", "USB"), ("online", "0")]);
        assert!(!battery(&supplies).unwrap().expect("a battery").plugged, "on its battery alone");
        supply("usb", &[("type", "USB_PD"), ("online", "1")]);
        supply("ucsi-source-psy-soc:qcom,pmic_glink:qcom,ucsi1", &[("online", "1")]);
        assert!(battery(&supplies).unwrap().expect("a battery").plugged, "on a charger's cable");
        supply("usb", &[("type", "USB"), ("online", "0")]);
        supply("ucsi-source-psy-soc:qcom,pmic_glink:qcom,ucsi1", &[("online", "0")]);
        supply("wireless", &[("online", "1")]);
        assert!(battery(&supplies).unwrap().expect("a battery").plugged, "on a wireless charger");
    }

    #[test]
    fn a_power_supply_that_cannot_tell_yet_is_taken_as_offline() {
        use std::io::Error;
        assert!(said_online(Ok("1\n".into())).unwrap());
        assert!(!said_online(Ok("0\n".into())).unwrap());
        assert!(!said_online(Err(Error::from(ErrorKind::WouldBlock))).unwrap(), "Qualcomm's USB-C connector early in a boot");
        assert!(!said_online(Err(Error::from(ErrorKind::NotFound))).unwrap(), "a supply without the file");
        assert!(said_online(Err(Error::from(ErrorKind::PermissionDenied))).is_err(), "anything else is an error");
    }
}
