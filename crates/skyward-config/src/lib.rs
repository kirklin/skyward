use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Recovery ramdisk files Skyward reads, adds or changes, as archive paths
/// (they sit under `/` once the kernel unpacks the ramdisk).
pub mod ramdisk {
    /// The device configuration, copied from `devices/<codename>.toml`.
    pub const CONFIG: &str = "skyward/device.toml";
    /// The UI typeface.
    pub const FONT: &str = "skyward/MiSansVF.ttf";
    /// The terminal's monospaced typeface.
    pub const MONO: &str = "skyward/KirkMono-Regular.ttf";
    /// The monospaced typeface's licence, which travels with every copy of
    /// it.
    pub const MONO_LICENSE: &str = "skyward/KirkMono-OFL.txt";
    /// The recovery service binary init starts.
    pub const RECOVERY: &str = "system/bin/recovery";
    /// The first property file init loads in recovery mode.
    pub const PROP_DEFAULT: &str = "prop.default";
    /// The scheduling profiles libprocessgroup reads.
    pub const TASK_PROFILES: &str = "system/etc/task_profiles.json";
    /// The init rules that set up file transfer's USB function.
    pub const MTP_RC: &str = "system/etc/init/skyward-mtp.rc";

    /// Whether the running root is Skyward's recovery ramdisk.
    pub fn in_recovery() -> bool {
        std::path::Path::new("/").join(CONFIG).exists()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceConfig {
    pub device: Device,
    #[serde(default)]
    pub display: Display,
    #[serde(default)]
    pub modules: Modules,
    #[serde(default)]
    pub touch: Touch,
    #[serde(default)]
    pub partitions: Partitions,
    /// Where the firmware the phone's drivers ask for is, on phones whose
    /// recovery leaves it out of their reach.
    #[serde(default)]
    pub firmware: Option<FirmwarePartition>,
    #[serde(default, rename = "service")]
    pub services: Vec<Service>,
    /// The vibration motor's click, on phones that have one.
    #[serde(default)]
    pub haptics: Option<Haptics>,
    /// The recovery's own logd, on phones whose recovery leaves out
    /// libraries it needs.
    #[serde(default)]
    pub logd: Option<Logd>,
    /// Files the phone's own init hands to an account as it boots, before
    /// the programs that open them start; a recovery runs none of init's
    /// boot actions.
    #[serde(default)]
    pub owners: Vec<Owners>,
    #[serde(default)]
    pub wipe: Wipe,
    #[serde(default)]
    pub backup: Backup,
    /// The phone's WLAN, on phones whose driver the recovery brings up.
    #[serde(default)]
    pub wlan: Option<Wlan>,
    /// Where the phone maker answers which system is the newest, on phones
    /// whose server the recovery knows.
    #[serde(default)]
    pub update: Option<Update>,
    /// The stock build the recovery is made on.
    #[serde(default)]
    pub stock: Option<Stock>,
}

/// The stock build a recovery is made on: the phone maker's full update
/// package of it, whose recovery partition the recovery is built on top
/// of, and that partition as it should come out of the package.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Stock {
    /// The build's version, such as OS4.0.4.0.XPACNXM.
    pub version: String,
    /// Where the package is downloaded from, over HTTPS.
    pub package: String,
    /// The recovery partition's SHA-256, in lowercase hex.
    pub recovery: String,
}

/// What brings the phone's WLAN up inside a recovery, which loads none of
/// it: the driver, what it reads as it starts, and the phone's own
/// wpa_supplicant, which joins networks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Wlan {
    /// The driver's kernel modules; what they depend on loads first.
    pub modules: Vec<String>,
    /// What is written once they are in, each value to its file, such as
    /// what tells a platform driver it may start the chip.
    #[serde(default)]
    pub writes: Vec<Write>,
    /// Files the driver asks the kernel's firmware loader for that are not
    /// where the loader looks in a recovery.
    #[serde(default)]
    pub firmware: Vec<Firmware>,
    /// The network interface the driver makes.
    pub interface: String,
    /// The phone's own wpa_supplicant.
    pub supplicant: String,
    /// Directories for its LD_LIBRARY_PATH, searched in order and before
    /// the libraries of the vendor interface the phone was built against.
    #[serde(default)]
    pub libraries: Vec<String>,
    /// The service it registers with the service directory as it starts,
    /// which the directory takes only from a program the running system
    /// describes.
    pub service: String,
    /// The page the stock system asks over plain HTTP to tell whether a
    /// network reaches the internet, which answers 204 with nothing.
    pub probe: String,
}

/// A value written to a file, such as a switch in sysfs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Write {
    pub path: String,
    pub value: String,
}

/// The partition holding the firmware images of a phone's processors and
/// radios, which their drivers ask the kernel's firmware loader for, and of
/// the apps its secure world runs. It is mounted where and as the phone's
/// own table mounts it, and shown as well at the folder whose image folder
/// the recovery's own ueventd looks in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FirmwarePartition {
    /// The partition's name, without the slot it is of.
    pub partition: String,
}

/// A file a driver asks the firmware loader for, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Firmware {
    /// The name the driver asks for, from the top of the loader's folders.
    pub name: String,
    /// Where the file is: a path in the running recovery, or from the top
    /// of `partition`.
    pub path: String,
    /// A partition the recovery does not keep mounted, which is mounted
    /// read-only while the file is read.
    #[serde(default)]
    pub partition: Option<String>,
}

/// The phone maker's update server, which the stock updater asks about the
/// installed system.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Update {
    /// Where the questions are sent.
    pub server: String,
}

/// What a backup takes in and leaves out beyond what every Android phone
/// keeps the same.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Backup {
    /// The partitions that keep the baseband's settings, with the phone's
    /// own identity, and the sensors' calibration.
    #[serde(default)]
    pub efs: Vec<String>,
    /// Folders in the data partition, from its top, that the phone's own
    /// secure world keeps in step with itself, as its own storage and the
    /// fingerprint templates: an older copy written back would not open.
    /// A `*` stands for any one name.
    #[serde(default)]
    pub data_skipped: Vec<String>,
}

/// What erasing on the wipe page takes in beyond the places every Android
/// phone keeps the same.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Wipe {
    /// Folders in the data partition where the phone's own programs keep
    /// their logs and dumps: their files go with the logs and crash
    /// records, the folders staying.
    #[serde(default)]
    pub logs: Vec<String>,
}

/// The data partition, which the folders erasing takes in are in.
const DATA: &str = "/data";

/// The vibration motor's click, played through the kernel's force-feedback
/// events: a waveform its driver keeps, or one made from samples at the
/// motor's resonant frequency, which the driver plays as they are given.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Haptics {
    /// The driver's number for a click waveform it keeps, on phones whose
    /// driver plays the waveforms it keeps.
    #[serde(default)]
    pub click: Option<i16>,
    /// The file the driver says the motor's resonant frequency in, in
    /// tenths of a hertz whatever its label, on phones whose click is made
    /// from samples.
    #[serde(default)]
    pub resonance: Option<String>,
    pub strength: Strength,
}

/// How strongly the click plays, each as a share of the most its waveform
/// drives the motor with. The strength slider goes from the lightest to the
/// strongest, each step the same factor stronger.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Strength {
    pub lightest: f32,
    pub strongest: f32,
}

/// What the recovery's own logd, which keeps what programs log for
/// logcat, needs from the installed system before it can start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Logd {
    /// Libraries of the installed system it loads that the recovery's own
    /// library folder lacks, put there before it is started.
    pub libraries: Vec<String>,
}

/// Files the phone's own init hands to one account as it boots, each given
/// the same mode, as a run of its chown and chmod lines does: device nodes,
/// and the kernel's switches that the phone's own programs open.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Owners {
    pub user: String,
    pub group: String,
    /// The mode init's chmod lines give them, written as an octal number.
    pub mode: u32,
    pub paths: Vec<String>,
}

/// A name as each language the pages speak writes it: Chinese in
/// simplified and in traditional characters, and English.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Names {
    pub hans: String,
    pub hant: String,
    pub en: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub codename: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_true")]
    pub ab: bool,
    /// SoC marketing name shown on the About page.
    #[serde(default)]
    pub soc: Option<Names>,
    /// System property that carries the vendor OS version.
    #[serde(default = "default_os_version_prop")]
    pub os_version_prop: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Display {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub density: u32,
    /// Marketed screen diagonal in inches.
    #[serde(default)]
    pub diagonal: Option<f32>,
    /// How round the screen's corners are, as a radius in pixels; a page
    /// sliding over another rounds its leading corners to it.
    #[serde(default)]
    pub corner_radius: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Modules {
    /// Partitions the phone keeps its kernel modules on, searched in order.
    #[serde(default = "default_module_sources")]
    pub sources: Vec<String>,
    /// Modules the recovery loads at startup that first-stage init leaves
    /// out, such as the touchscreen driver. Dependencies come from the
    /// source's modules.dep and load first.
    #[serde(default)]
    pub load: Vec<String>,
}

impl Default for Modules {
    fn default() -> Self {
        Self { sources: default_module_sources(), load: Vec::new() }
    }
}

/// The touchscreen, on phones whose touch controller is to be told
/// something before a recovery can read touches from it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Touch {
    /// What is written once the touchscreen's input device has appeared,
    /// each value to its file: the driver makes that device as the last
    /// step of starting the controller.
    #[serde(default)]
    pub writes: Vec<Write>,
    /// The screen's own axes whose numbers the panel reports the other way
    /// round, which one drawn from a panel mirrored against the screen does.
    /// Which way round a panel lies is what its ranges say; which way its
    /// numbers run is not, so this is said here.
    #[serde(default)]
    pub reversed: Vec<String>,
}

/// Partitions as the phone itself mounts them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Partitions {
    /// Partitions the phone's own programs and drivers use where its own
    /// table mounts them, mounted there as that table says before any of
    /// them starts.
    #[serde(default)]
    pub mounted: Vec<String>,
}

/// A program from the phone's own partitions that the recovery keeps
/// running, such as the touch processing of a panel that hands raw frames
/// to the host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Service {
    pub path: String,
    /// What the program is given after its own path, as init gives a
    /// service the words after its path.
    #[serde(default)]
    pub args: Vec<String>,
    /// Directories for LD_LIBRARY_PATH, searched in order.
    #[serde(default)]
    pub libraries: Vec<String>,
    /// The program runs once, to its end, as a service of init's marked
    /// oneshot does: it is not started again, the programs after it start
    /// once it has ended, and it has to end well.
    #[serde(default)]
    pub oneshot: bool,
    /// A property the program sets once it is ready for the rest. Programs
    /// are started in the order they are written, and one that says it has
    /// something to wait for holds up the ones after it.
    #[serde(default)]
    pub ready_property: Option<String>,
    /// A service the program registers with the phone's service directory
    /// once it is ready.
    #[serde(default)]
    pub ready_service: Option<String>,
    /// Services the program registers besides that one, which the
    /// directory takes only when they are described to it as well.
    #[serde(default)]
    pub registers: Vec<String>,
    /// The file that describes what the program registers, where the phone
    /// keeps that file apart from its other descriptions, as one its init
    /// puts in place of another on some builds of the hardware.
    #[serde(default)]
    pub description: Option<String>,
    /// A service the program registers with the other directory, the one
    /// the phone's hardware services of the older kind use. It is written
    /// as the interface and then which of them it is.
    #[serde(default)]
    pub ready_hardware_service: Option<String>,
    /// Where one model's hardware comes in more than one build, the
    /// installed system's own properties say which build it is: the program
    /// runs only where the property has this value.
    #[serde(default)]
    pub when: Option<Property>,
    /// The user the program runs as, as the phone's own init runs it, by
    /// the name Android gives the account; root where none is given. What
    /// the program makes in the data partition is then that user's, as it
    /// is when the phone runs it, and the phone's own copy of the program
    /// can still read and remove it.
    #[serde(default)]
    pub user: Option<String>,
    /// Its group and then the others it is in, as init's group line names
    /// them.
    #[serde(default)]
    pub groups: Vec<String>,
    /// What it may do of what only root may, as init's capabilities line
    /// names it: it keeps these alone, through its change of user.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// The kernel's capabilities, by the names init's capabilities lines give
/// them, in the order the kernel numbers them.
const CAPABILITIES: [&str; 41] = [
    "CHOWN",
    "DAC_OVERRIDE",
    "DAC_READ_SEARCH",
    "FOWNER",
    "FSETID",
    "KILL",
    "SETGID",
    "SETUID",
    "SETPCAP",
    "LINUX_IMMUTABLE",
    "NET_BIND_SERVICE",
    "NET_BROADCAST",
    "NET_ADMIN",
    "NET_RAW",
    "IPC_LOCK",
    "IPC_OWNER",
    "SYS_MODULE",
    "SYS_RAWIO",
    "SYS_CHROOT",
    "SYS_PTRACE",
    "SYS_PACCT",
    "SYS_ADMIN",
    "SYS_BOOT",
    "SYS_NICE",
    "SYS_RESOURCE",
    "SYS_TIME",
    "SYS_TTY_CONFIG",
    "MKNOD",
    "LEASE",
    "AUDIT_WRITE",
    "AUDIT_CONTROL",
    "SETFCAP",
    "MAC_OVERRIDE",
    "MAC_ADMIN",
    "SYSLOG",
    "WAKE_ALARM",
    "BLOCK_SUSPEND",
    "AUDIT_READ",
    "PERFMON",
    "BPF",
    "CHECKPOINT_RESTORE",
];

/// The kernel's number for a capability, by the name init's capabilities
/// line gives it.
pub fn capability(name: &str) -> Option<u32> {
    CAPABILITIES.iter().position(|c| *c == name).map(|n| n as u32)
}

/// A property of the installed system, and a value it may have.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Property {
    pub property: String,
    pub value: String,
}

impl Service {
    /// Whether the program, or a directory it loads libraries from, is
    /// under `dir`: without what is mounted there it cannot start again.
    pub fn uses(&self, dir: &Path) -> bool {
        Path::new(&self.path).starts_with(dir) || self.libraries.iter().any(|l| Path::new(l).starts_with(dir))
    }

    /// Whether the program is this phone's to run, by the installed
    /// system's own properties.
    pub fn runs_on(&self, props: &HashMap<String, String>) -> bool {
        self.when.as_ref().is_none_or(|p| props.get(&p.property) == Some(&p.value))
    }
}

fn default_true() -> bool {
    true
}

fn default_os_version_prop() -> String {
    "ro.build.display.id".into()
}

fn default_module_sources() -> Vec<String> {
    vec!["vendor_boot".into(), "vendor_dlkm".into(), "vendor".into()]
}

impl DeviceConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        let text = std::fs::read_to_string(p)
            .with_context(|| format!("read config {}", p.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let cfg: DeviceConfig = toml::from_str(text).context("parse device config")?;
        for log in &cfg.wipe.logs {
            let path = Path::new(log);
            let plain = path.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)));
            ensure!(
                path.is_absolute() && plain && path.starts_with(DATA) && path != Path::new(DATA),
                "[wipe] logs: {log} is not a folder inside {DATA}"
            );
        }
        for name in &cfg.backup.efs {
            ensure!(!name.is_empty() && !name.contains('/'), "[backup] efs: {name} is not a partition's name");
        }
        if let Some(firmware) = &cfg.firmware {
            let name = &firmware.partition;
            ensure!(!name.is_empty() && !name.contains('/'), "[firmware] partition: {name} is not a partition's name");
        }
        for name in &cfg.partitions.mounted {
            ensure!(!name.is_empty() && !name.contains('/'), "[partitions] mounted: {name} is not a partition's name");
        }
        for s in cfg.services.iter().filter(|s| s.oneshot) {
            let waits = s.ready_property.is_some() || s.ready_service.is_some() || s.ready_hardware_service.is_some();
            ensure!(
                !waits && s.registers.is_empty() && s.description.is_none(),
                "[[service]] {}: a program run once is ready when it ends, and registers nothing",
                s.path
            );
        }
        for s in &cfg.services {
            ensure!(s.user.is_none() || !s.groups.is_empty(), "[[service]] {}: a program given a user is given its group too", s.path);
            for name in s.user.iter().chain(&s.groups) {
                ensure!(!name.is_empty() && !name.contains(char::is_whitespace), "[[service]] {}: {name:?} is not an account's name", s.path);
            }
            for name in &s.capabilities {
                ensure!(capability(name).is_some(), "[[service]] {}: {name} is not a capability init knows", s.path);
            }
        }
        for folder in &cfg.backup.data_skipped {
            let path = Path::new(folder);
            let plain = path.components().all(|c| matches!(c, std::path::Component::Normal(_)));
            ensure!(plain && !folder.is_empty(), "[backup] data_skipped: {folder} is not a path from the top of {DATA}");
        }
        if let Some(haptics) = &cfg.haptics {
            ensure!(
                haptics.click.is_some() != haptics.resonance.is_some(),
                "[haptics] names either a waveform the driver keeps or the file it says the motor's resonant frequency in"
            );
            if let Some(file) = &haptics.resonance {
                ensure!(Path::new(file).is_absolute(), "[haptics] resonance: {file} is not an absolute path");
            }
            let Strength { lightest, strongest } = haptics.strength;
            ensure!(
                0.0 < lightest && lightest < strongest && strongest <= 1.0,
                "[haptics] strength: {lightest} and {strongest} are not shares rising from above 0 to at most 1"
            );
        }
        if let Some(logd) = &cfg.logd {
            for library in &logd.libraries {
                let path = Path::new(library);
                let plain = path.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)));
                ensure!(path.is_absolute() && plain && path.file_name().is_some(), "[logd] libraries: {library} is not an absolute path to a file");
            }
        }
        for owners in &cfg.owners {
            for name in [&owners.user, &owners.group] {
                ensure!(!name.is_empty() && !name.contains(char::is_whitespace), "[[owners]] {name:?} is not an account's name");
            }
            ensure!(owners.mode <= 0o7777, "[[owners]] mode {:o} is not a file's mode", owners.mode);
            ensure!(!owners.paths.is_empty(), "[[owners]] for {}:{} names no file", owners.user, owners.group);
            for file in &owners.paths {
                let path = Path::new(file);
                let plain = path.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)));
                ensure!(path.is_absolute() && plain && path.file_name().is_some(), "[[owners]] paths: {file} is not an absolute path to a file");
            }
        }
        if let Some(stock) = &cfg.stock {
            let name = stock.package.rsplit('/').next().unwrap_or_default();
            ensure!(
                stock.package.starts_with("https://") && name.ends_with(".zip") && name.contains(&stock.version) && !stock.version.is_empty(),
                "[stock] package: {} is not an HTTPS address of the full package of {}",
                stock.package,
                stock.version
            );
            ensure!(
                stock.recovery.len() == 64 && stock.recovery.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "[stock] recovery: {} is not a SHA-256 in lowercase hex",
                stock.recovery
            );
        }
        if let Some(wlan) = &cfg.wlan {
            ensure!(wlan.probe.starts_with("http://"), "[wlan] probe: {} is not a plain HTTP address", wlan.probe);
            for file in &wlan.firmware {
                let name = Path::new(&file.name);
                let plain = name.components().all(|c| matches!(c, std::path::Component::Normal(_)));
                ensure!(plain && !file.name.is_empty(), "[wlan] firmware: {} is not a name from the top of the loader's folders", file.name);
                let path = Path::new(&file.path);
                match &file.partition {
                    Some(partition) => {
                        let plain = path.components().all(|c| matches!(c, std::path::Component::Normal(_)));
                        ensure!(
                            !partition.is_empty() && !partition.contains('/'),
                            "[wlan] firmware: {partition} is not a partition's name"
                        );
                        ensure!(plain && !file.path.is_empty(), "[wlan] firmware: {} is not a path from the top of {partition}", file.path);
                    }
                    None => ensure!(path.is_absolute(), "[wlan] firmware: {} is not an absolute path", file.path),
                }
            }
        }
        for axis in &cfg.touch.reversed {
            ensure!(axis == "x" || axis == "y", "[touch] reversed: {axis} is not one of the screen's axes");
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal() {
        let cfg = DeviceConfig::parse("[device]\ncodename = \"nezha\"\n").unwrap();
        assert_eq!(cfg.device.codename, "nezha");
        assert!(cfg.device.ab);
        assert_eq!(cfg.device.os_version_prop, "ro.build.display.id");
        assert_eq!(cfg.modules.sources, default_module_sources());
        assert!(cfg.modules.load.is_empty());
        assert_eq!((cfg.firmware, cfg.wlan, cfg.update), (None, None, None), "a phone may have none of them");
        assert_eq!(cfg.logd, None, "a recovery's own logd usually has all it needs");
    }

    #[test]
    fn nezha_gives_its_logd_the_libraries_its_recovery_leaves_out() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/nezha.toml")).unwrap();
        let logd = cfg.logd.expect("the 17 Ultra's recovery leaves out what its logd loads");
        assert_eq!(logd.libraries, ["/mnt/system/system/lib64/libsysutils.so", "/mnt/system/system/lib64/libcap.so"]);
        let ishtar = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        assert_eq!(ishtar.logd, None, "the 13 Ultra's recovery carries both");
    }

    #[test]
    fn ishtar_clicks_with_a_waveform_its_driver_keeps() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        let haptics = cfg.haptics.expect("the 13 Ultra has a motor");
        assert_eq!((haptics.click, haptics.resonance), (Some(0), None));
        assert_eq!(haptics.strength, Strength { lightest: 0.5, strongest: 1.0 });
    }

    #[test]
    fn a_click_is_kept_or_made_and_its_strengths_rise() {
        let with = |body: &str| DeviceConfig::parse(&format!("[device]\ncodename = \"x\"\n[haptics]\n{body}\n"));
        let strength = "strength = { lightest = 0.25, strongest = 1.0 }";
        assert!(with(&format!("click = 0\n{strength}")).is_ok());
        assert!(with(&format!("resonance = \"/sys/class/qcom-haptics/lra_frequency_hz\"\n{strength}")).is_ok());
        assert!(with(strength).is_err(), "a click of neither kind");
        assert!(with(&format!("click = 0\nresonance = \"/sys/f\"\n{strength}")).is_err(), "a click of both kinds");
        assert!(with(&format!("resonance = \"sys/f\"\n{strength}")).is_err(), "a file named from nowhere");
        assert!(with("click = 0").is_err(), "a click without its strengths");
        for (lightest, strongest) in [(0.0, 0.5), (0.5, 0.5), (0.5, 0.25), (0.25, 1.5)] {
            let s = format!("click = 0\nstrength = {{ lightest = {lightest:?}, strongest = {strongest:?} }}");
            assert!(with(&s).is_err(), "{lightest} to {strongest} is refused");
        }
    }

    #[test]
    fn a_library_for_logd_is_an_absolute_path_to_a_file() {
        let with = |library: &str| DeviceConfig::parse(&format!("[device]\ncodename = \"x\"\n[logd]\nlibraries = [\"{library}\"]\n"));
        assert!(with("/mnt/system/system/lib64/libcap.so").is_ok());
        for bad in ["lib64/libcap.so", "/", "/mnt/system/../lib64/libcap.so", ""] {
            assert!(with(bad).is_err(), "{bad} is refused");
        }
    }

    #[test]
    fn files_handed_over_name_their_accounts_a_mode_and_absolute_paths() {
        let with = |body: &str| DeviceConfig::parse(&format!("[device]\ncodename = \"x\"\n[[owners]]\n{body}\n"));
        let ok = with("user = \"system\"\ngroup = \"system\"\nmode = 0o664\npaths = [\"/dev/xiaomi-touch\"]").unwrap();
        assert_eq!(ok.owners, [Owners { user: "system".into(), group: "system".into(), mode: 0o664, paths: vec!["/dev/xiaomi-touch".into()] }]);
        assert!(with("user = \"system\"\ngroup = \"system\"\nmode = 0o664\npaths = []").is_err(), "no file");
        assert!(with("user = \"sys tem\"\ngroup = \"system\"\nmode = 0o664\npaths = [\"/dev/a\"]").is_err());
        assert!(with("user = \"system\"\ngroup = \"system\"\nmode = 0o17777\npaths = [\"/dev/a\"]").is_err(), "not a file's mode");
        for bad in ["dev/a", "/", "/sys/../dev/a"] {
            assert!(with(&format!("user = \"system\"\ngroup = \"system\"\nmode = 0o664\npaths = [\"{bad}\"]")).is_err(), "{bad} is refused");
        }
        assert!(DeviceConfig::parse("[device]\ncodename = \"x\"\n").unwrap().owners.is_empty(), "a phone may need none");
    }

    #[test]
    fn parses_nezha() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/nezha.toml")).unwrap();
        assert_eq!(cfg.device.codename, "nezha");
        assert_eq!(cfg.device.os_version_prop, "ro.mi.os.version.incremental");
        let soc = cfg.device.soc.expect("nezha names its SoC");
        assert_eq!(soc.hans, "第五代骁龙®8至尊版移动平台");
        assert_eq!(soc.en, "Snapdragon® 8 Elite Gen 5 Mobile Platform");
        assert_eq!(cfg.display.width, 1200);
        assert_eq!(cfg.display.diagonal, Some(6.9));
        assert_eq!(cfg.modules.load, ["synaptics_tcm2"]);
        assert_eq!(
            cfg.touch.writes,
            [Write { path: "/sys/class/touch/touch_dev/enable_touch_raw".into(), value: "0".into() }],
            "its controller reports positions itself"
        );
        assert_eq!(cfg.firmware, Some(FirmwarePartition { partition: "modem".into() }));
        assert_eq!(cfg.partitions.mounted, ["persist"], "the secure world keeps its own files there");
        let paths: Vec<&str> = cfg.services.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/vendor/bin/sh",
                "/vendor/bin/qseecomd",
                "/vendor/bin/hlosminkdaemon",
                "/vendor/bin/hw/android.hardware.security.onekeymint-service-qti",
                "/vendor/bin/hw/android.hardware.gatekeeper-rust-service-qti",
                "/odm/bin/hw/android.hardware.secure_element-service-goodix",
                "/odm/bin/hw/android.hardware.weaver-service",
                "/vendor/bin/hw/android.hardware.boot-service.qti",
            ]
        );
        for s in &cfg.services {
            assert_eq!(s.libraries[..2], ["/system/lib64", "/vendor/lib64"], "{} loads the recovery's own libraries first", s.path);
            assert_eq!(s.ready_hardware_service, None, "{} is of the newer kind", s.path);
        }
        let [post_boot, listeners, opener, keymint, gatekeeper, element, weaver, boot] = &cfg.services[..] else { panic!("eight programs") };
        assert!(post_boot.oneshot, "the system's own settings for after start are made once");
        assert_eq!(post_boot.args, ["/vendor/bin/init.kernel.post_boot.sh"]);
        assert!(cfg.services[1..].iter().all(|s| !s.oneshot && s.args.is_empty()));
        assert_eq!(listeners.ready_property.as_deref(), Some("vendor.sys.listeners.registered"));
        assert_eq!((opener.ready_property.as_ref(), opener.ready_service.as_ref()), (None, None));
        assert_eq!(keymint.ready_service.as_deref(), Some("android.hardware.security.keymint.IKeyMintDevice/default"));
        assert!(keymint.registers.contains(&"android.hardware.security.secureclock.ISecureClock/default".to_string()));
        assert!(keymint.registers.contains(&"android.hardware.security.sharedsecret.ISharedSecret/default".to_string()));
        assert_eq!(gatekeeper.ready_service.as_deref(), Some("android.hardware.gatekeeper.IGatekeeper/default"));
        assert_eq!(element.ready_service.as_deref(), Some("android.hardware.secure_element.ISecureElement/eSE2"));
        assert_eq!(element.description.as_deref(), Some("/odm/etc/secure_element-service-goodixsp.xml"));
        assert_eq!(weaver.ready_service.as_deref(), Some("android.hardware.weaver.IWeaver/default"));
        assert_eq!(boot.ready_service.as_deref(), Some("android.hardware.boot.IBootControl/default"));
        let haptics = cfg.haptics.expect("the 17 Ultra has a motor");
        assert_eq!((haptics.click, haptics.resonance.as_deref()), (None, Some("/sys/class/qcom-haptics/lra_frequency_hz")), "its click is made");
        assert_eq!(haptics.strength, Strength { lightest: 0.25, strongest: 0.75 }, "past three quarters the motor rattles");
        assert_eq!(cfg.backup.efs, ["modemst1", "modemst2", "fsg", "fsc", "persist"]);
        assert!(cfg.backup.data_skipped.contains(&"vendor/goodix".to_string()));
        assert!(cfg.wipe.logs.contains(&"/data/vendor/bsplog".to_string()));
    }

    #[test]
    fn the_secure_element_of_each_build_runs_on_that_build_alone() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/nezha.toml")).unwrap();
        let runs = |props: &[(&str, &str)]| -> Vec<String> {
            let props: HashMap<String, String> = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            cfg.services.iter().filter(|s| s.runs_on(&props)).map(|s| s.path.clone()).collect()
        };
        let leica = runs(&[("ro.special_edition", "true")]);
        assert_eq!(leica.len(), 8);
        let plain = runs(&[]);
        assert_eq!(plain.len(), 6);
        assert!(!plain.iter().any(|p| p.starts_with("/odm/")), "the other builds' weaver cannot run in a recovery");
        assert_eq!(runs(&[("ro.special_edition", "false")]), plain);
    }

    #[test]
    fn a_program_runs_only_where_the_property_has_its_value() {
        let base = "[device]\ncodename = \"x\"\n[[service]]\npath = \"/odm/bin/a\"\n";
        let only = DeviceConfig::parse(&format!("{base}when = {{ property = \"ro.sku\", value = \"p\" }}\n")).unwrap();
        let always = DeviceConfig::parse(base).unwrap();
        let props = |v: Option<&str>| -> HashMap<String, String> { v.map(|v| ("ro.sku".to_string(), v.to_string())).into_iter().collect() };
        for (value, runs) in [(Some("p"), true), (Some("q"), false), (None, false)] {
            assert_eq!(only.services[0].runs_on(&props(value)), runs, "with ro.sku {value:?}");
            assert!(always.services[0].runs_on(&props(value)));
        }
    }

    #[test]
    fn a_program_run_once_waits_for_nothing_and_registers_nothing() {
        let base = "[device]\ncodename = \"x\"\n[[service]]\npath = \"/vendor/bin/sh\"\nargs = [\"/vendor/bin/a.sh\"]\noneshot = true\n";
        let once = DeviceConfig::parse(base).unwrap();
        assert!(once.services[0].oneshot);
        assert_eq!(once.services[0].args, ["/vendor/bin/a.sh"]);
        assert!(DeviceConfig::parse(&format!("{base}ready_property = \"vendor.a\"\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}ready_service = \"a.IA/default\"\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}registers = [\"a.IB/default\"]\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}description = \"/vendor/etc/a.xml\"\n")).is_err());
        let kept = DeviceConfig::parse("[device]\ncodename = \"x\"\n[[service]]\npath = \"/vendor/bin/a\"\n").unwrap();
        assert!(!kept.services[0].oneshot && kept.services[0].args.is_empty(), "a program is kept running unless it says otherwise");
    }

    #[test]
    fn a_program_runs_as_its_user_in_its_groups_keeping_named_capabilities() {
        let base = "[device]\ncodename = \"x\"\n[[service]]\npath = \"/odm/bin/a\"\n";
        let root = DeviceConfig::parse(base).unwrap();
        let s = &root.services[0];
        assert_eq!((s.user.as_ref(), s.groups.len(), s.capabilities.len()), (None, 0, 0), "root, as init runs one that names no user");
        let named = DeviceConfig::parse(&format!("{base}user = \"system\"\ngroups = [\"system\", \"drmrpc\"]\ncapabilities = [\"WAKE_ALARM\"]\n")).unwrap();
        let s = &named.services[0];
        assert_eq!((s.user.as_deref(), &s.groups[..], &s.capabilities[..]), (Some("system"), &["system".to_string(), "drmrpc".to_string()][..], &["WAKE_ALARM".to_string()][..]));
        assert!(DeviceConfig::parse(&format!("{base}user = \"system\"\n")).is_err(), "a user without its group");
        assert!(DeviceConfig::parse(&format!("{base}user = \"system\"\ngroups = [\"system\"]\ncapabilities = [\"WAKE\"]\n")).is_err(), "no such capability");
        assert!(DeviceConfig::parse(&format!("{base}user = \"sys tem\"\ngroups = [\"system\"]\n")).is_err());
        assert_eq!(
            [capability("CHOWN"), capability("SETPCAP"), capability("WAKE_ALARM"), capability("BLOCK_SUSPEND"), capability("CHECKPOINT_RESTORE"), capability("CAP_CHOWN")],
            [Some(0), Some(8), Some(35), Some(36), Some(40), None],
            "numbered as the kernel numbers them, named as init names them"
        );
    }

    #[test]
    fn each_phones_programs_run_as_their_init_scripts_run_them() {
        let nezha = DeviceConfig::parse(include_str!("../../../devices/nezha.toml")).unwrap();
        let ishtar = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        let piano = DeviceConfig::parse(include_str!("../../../devices/piano.toml")).unwrap();
        for s in nezha.services.iter().chain(&ishtar.services).chain(&piano.services) {
            assert!(s.user.is_some(), "{} names the user its init script does", s.path);
        }
        let find = |cfg: &DeviceConfig, path: &str| cfg.services.iter().find(|s| s.path == path).cloned().expect("listed");
        let element = find(&nezha, "/odm/bin/hw/android.hardware.secure_element-service-goodix");
        assert_eq!((element.user.as_deref(), &element.groups[..]), (Some("system"), &["system".to_string()][..]), "its logs in the data partition are the system's");
        let weaver = find(&nezha, "/odm/bin/hw/android.hardware.weaver-service");
        assert_eq!(weaver.capabilities, ["WAKE_ALARM"]);
        let listeners = find(&nezha, "/vendor/bin/qseecomd");
        assert_eq!((listeners.user.as_deref(), &listeners.groups[..]), (Some("root"), &["root".to_string(), "drmrpc".to_string()][..]));
        let opener = find(&nezha, "/vendor/bin/hlosminkdaemon");
        assert_eq!(opener.capabilities, ["WAKE_ALARM", "BLOCK_SUSPEND"]);
        let touch = find(&ishtar, "/odm/bin/touch_report");
        assert_eq!((touch.user.as_deref(), &touch.capabilities[..]), (Some("system"), &["SYS_NICE".to_string()][..]));
    }

    #[test]
    fn each_phone_names_the_stock_build_its_recovery_is_made_on() {
        for (text, version) in [
            (include_str!("../../../devices/nezha.toml"), "OS4.0.4.0.XPACNXM"),
            (include_str!("../../../devices/ishtar.toml"), "OS3.0.308.0.WMACNXM"),
            (include_str!("../../../devices/pudding.toml"), "OS4.0.4.0.XPCCNXM"),
            (include_str!("../../../devices/pandora.toml"), "OS4.0.4.0.XBLCNXM"),
            (include_str!("../../../devices/popsicle.toml"), "OS4.0.4.0.XPBCNXM"),
            (include_str!("../../../devices/myron.toml"), "OS4.0.4.0.XPMCNXM"),
            (include_str!("../../../devices/songyuan.toml"), "OS4.0.5.0.XGNCNXM"),
            (include_str!("../../../devices/athens.toml"), "OS4.0.4.0.XPICNXM"),
            (include_str!("../../../devices/fuxi.toml"), "OS3.0.309.0.WMCCNXM"),
            (include_str!("../../../devices/nuwa.toml"), "OS3.0.312.0.WMBCNXM"),
            (include_str!("../../../devices/houji.toml"), "OS3.0.307.0.WNCCNXM"),
            (include_str!("../../../devices/shennong.toml"), "OS3.0.309.0.WNBCNXM"),
            (include_str!("../../../devices/aurora.toml"), "OS3.0.308.0.WNACNXM"),
            (include_str!("../../../devices/dada.toml"), "OS3.0.307.0.WOCCNXM"),
            (include_str!("../../../devices/haotian.toml"), "OS3.0.309.0.WOBCNXM"),
            (include_str!("../../../devices/xuanyuan.toml"), "OS3.0.314.0.WOACNXM"),
            (include_str!("../../../devices/byron.toml"), "OS3.0.310.0.WAFCNXM"),
            (include_str!("../../../devices/zorn.toml"), "OS3.0.309.0.WOKCNXM"),
            (include_str!("../../../devices/miro.toml"), "OS3.0.305.0.WOMCNXM"),
            (include_str!("../../../devices/annibale.toml"), "OS3.0.307.0.WPKCNXM"),
            (include_str!("../../../devices/warsaw.toml"), "OS3.0.309.0.WHPCNXM"),
            (include_str!("../../../devices/piano.toml"), "OS4.0.5.0.XPYCNXM"),
        ] {
            let stock = DeviceConfig::parse(text).unwrap().stock.expect("a stock build to build on");
            assert_eq!(stock.version, version);
            assert!(stock.package.ends_with(".zip") && stock.package.contains(&format!("/{version}/")), "{}", stock.package);
        }
    }

    #[test]
    fn a_stock_build_is_a_full_package_over_https_and_a_sha_256() {
        let ok = "[device]\ncodename = \"x\"\n[stock]\nversion = \"OS1.0.1.0.XXXCNXM\"\npackage = \"https://d.example/OS1.0.1.0.XXXCNXM/x-ota_full-OS1.0.1.0.XXXCNXM-user-16.0-0123456789.zip\"\n";
        let hash = "faa425b10231a0cfe2d74ff34f344f032ee2d891ea5e9420bf4e67cd86d622a7";
        assert!(DeviceConfig::parse(&format!("{ok}recovery = \"{hash}\"\n")).is_ok());
        assert!(DeviceConfig::parse(&format!("{ok}recovery = \"{}\"\n", hash.to_uppercase())).is_err(), "in lowercase");
        assert!(DeviceConfig::parse(&format!("{ok}recovery = \"{}\"\n", &hash[1..])).is_err(), "too short");
        let plain = ok.replace("https://", "http://");
        assert!(DeviceConfig::parse(&format!("{plain}recovery = \"{hash}\"\n")).is_err(), "over plain HTTP");
        let other = ok.replace("version = \"OS1.0.1.0", "version = \"OS1.0.2.0");
        assert!(DeviceConfig::parse(&format!("{other}recovery = \"{hash}\"\n")).is_err(), "the package of another build");
        assert_eq!(DeviceConfig::parse("[device]\ncodename = \"x\"\n").unwrap().stock, None);
    }

    #[test]
    fn the_firmware_partition_is_named_plainly() {
        let base = "[device]\ncodename = \"x\"\n[firmware]\n";
        assert!(DeviceConfig::parse(&format!("{base}partition = \"modem\"\n")).is_ok());
        assert!(DeviceConfig::parse(&format!("{base}partition = \"by-name/modem\"\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}partition = \"\"\n")).is_err());
    }

    #[test]
    fn a_partition_mounted_as_the_phone_mounts_it_is_named_plainly() {
        let with = |name: &str| DeviceConfig::parse(&format!("[device]\ncodename = \"x\"\n[partitions]\nmounted = [\"{name}\"]\n"));
        assert_eq!(with("persist").unwrap().partitions.mounted, ["persist"]);
        assert!(with("by-name/persist").is_err());
        assert!(with("").is_err());
        assert!(DeviceConfig::parse("[device]\ncodename = \"x\"\n").unwrap().partitions.mounted.is_empty(), "a phone may need none");
    }

    #[test]
    fn parses_ishtar() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        assert_eq!(cfg.device.codename, "ishtar");
        assert_eq!((cfg.display.width, cfg.display.height, cfg.display.density), (1440, 3200, 560));
        assert_eq!(cfg.modules.load, ["fts_touch_spi"]);
        assert!(cfg.touch.writes.is_empty(), "its own touch processing runs");
        let [touch_owners] = &cfg.owners[..] else { panic!("one run of the touch layer's files") };
        assert_eq!((touch_owners.user.as_str(), touch_owners.group.as_str(), touch_owners.mode), ("system", "system", 0o664));
        assert!(
            touch_owners.paths.iter().any(|p| p == "/dev/xiaomi-touch"),
            "the touch processing, run as system, opens the touch layer's node"
        );
        assert!(cfg.firmware.is_none(), "its recovery mounts the modem partition itself");
        let paths: Vec<&str> = cfg.services.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/odm/bin/touch_report",
                "/odm/bin/touchsensor",
                "/vendor/bin/qseecomd",
                "/vendor/bin/hw/android.hardware.security.keymint-service-qti",
                "/mnt/system_ext/bin/hwservicemanager",
                "/vendor/bin/hw/android.hardware.gatekeeper@1.0-service-qti",
                "/vendor/bin/hw/android.hardware.boot@1.2-service",
            ]
        );
        assert_eq!(cfg.services[1].libraries, ["/odm/lib64", "/system/lib64"]);
        assert_eq!(cfg.services[0].ready_property, None, "the touch programs are ready as soon as they run");
        assert_eq!(cfg.services[2].ready_property.as_deref(), Some("vendor.sys.listeners.registered"));
        assert_eq!(
            cfg.services[3].ready_service.as_deref(),
            Some("android.hardware.security.keymint.IKeyMintDevice/default")
        );
        assert_eq!(
            cfg.services[4].ready_property.as_deref(),
            Some("hwservicemanager.ready"),
            "the directory for the hardware services says so itself"
        );
        assert_eq!(
            cfg.services[5].ready_hardware_service.as_deref(),
            Some("android.hardware.gatekeeper@1.0::IGatekeeper/default")
        );
        assert_eq!(cfg.services[6].ready_hardware_service.as_deref(), Some("android.hardware.boot@1.2::IBootControl/default"));
        assert_eq!(cfg.backup.efs, ["modemst1", "modemst2", "fsg", "fsc", "persist"]);
        assert!(cfg.backup.data_skipped.contains(&"vendor/tzstorage".to_string()));
    }

    #[test]
    fn parses_songyuan() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/songyuan.toml")).unwrap();
        assert_eq!(cfg.device.codename, "songyuan");
        assert_eq!(cfg.device.brand, "Redmi");
        assert_eq!(cfg.device.model, "REDMI K100 Pro Max");
        assert_eq!(cfg.device.os_version_prop, "ro.mi.os.version.incremental");
        let soc = cfg.device.soc.expect("songyuan names its SoC");
        assert_eq!(soc.en, "Snapdragon® 8 Elite Gen 5 Mobile Platform");
        assert_eq!((cfg.display.width, cfg.display.height, cfg.display.density), (1200, 2608, 480));
        assert_eq!(cfg.display.diagonal, Some(6.9));
        assert_eq!(cfg.display.corner_radius, 170);
        assert!(cfg.modules.sources.iter().any(|s| s == "system_dlkm"), "rfkill is one of the kernel's own modules");
        assert_eq!(cfg.modules.load, ["focaltech_touch_3685g_1"]);
        assert_eq!(
            cfg.touch.writes,
            [Write { path: "/sys/class/touch/touch_dev/enable_touch_raw".into(), value: "0".into() }],
            "its controller reports positions itself"
        );
        assert_eq!(cfg.firmware, Some(FirmwarePartition { partition: "modem".into() }));
        assert_eq!(cfg.partitions.mounted, ["persist"], "the secure world keeps its own files there");
        let paths: Vec<&str> = cfg.services.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/vendor/bin/sh",
                "/vendor/bin/sh",
                "/vendor/bin/qseecomd",
                "/vendor/bin/hlosminkdaemon",
                "/vendor/bin/hw/android.hardware.security.onekeymint-service-qti",
                "/vendor/bin/hw/android.hardware.gatekeeper-rust-service-qti",
                "/vendor/bin/hw/android.hardware.boot-service.qti",
                "/odm/bin/hw/android.hardware.weaver-service.thales",
            ]
        );
        for s in &cfg.services {
            assert_eq!(s.ready_hardware_service, None, "{} is of the newer kind", s.path);
        }
        let [post_boot, gatekeeper_props, listeners, opener, keymint, gatekeeper, boot, weaver] = &cfg.services[..] else {
            panic!("eight programs")
        };
        assert!(post_boot.oneshot, "the system's own settings for after start are made once");
        assert_eq!(post_boot.args, ["/vendor/bin/init.kernel.post_boot.sh"]);
        assert!(gatekeeper_props.oneshot, "the credential service is told before it is started");
        assert_eq!(
            gatekeeper_props.args,
            ["-c", "setprop vendor.gatekeeper.disable_spu true; setprop vendor.gatekeeper.is_security_level_spu 0"],
            "a recovery runs none of the secure processing unit's own service"
        );
        assert_eq!(listeners.ready_property.as_deref(), Some("vendor.sys.listeners.registered"));
        assert_eq!(opener.capabilities, ["WAKE_ALARM", "BLOCK_SUSPEND"]);
        assert_eq!(
            &opener.groups[..],
            &["system".to_string(), "vendor_ssgtzd".to_string(), "wakelock".to_string()][..],
            "its init script's oem_2912 is the name the phone's own group file gives 2912"
        );
        assert_eq!(keymint.ready_service.as_deref(), Some("android.hardware.security.keymint.IKeyMintDevice/default"));
        assert!(keymint.registers.contains(&"android.hardware.security.sharedsecret.ISharedSecret/default".to_string()));
        assert_eq!(gatekeeper.ready_service.as_deref(), Some("android.hardware.gatekeeper.IGatekeeper/default"));
        assert_eq!(boot.ready_service.as_deref(), Some("android.hardware.boot.IBootControl/default"));
        assert_eq!(weaver.ready_service.as_deref(), Some("android.hardware.weaver.IWeaver/default"), "the weaver skyward's crypto asks for");
        assert_eq!(
            (weaver.user.as_deref(), &weaver.groups[..]),
            (Some("root"), &["root".to_string()][..]),
            "run as its own account it ends at once, and as root it stays up"
        );
        let wlan = cfg.wlan.expect("the K100 Pro Max's WLAN is described");
        assert_eq!(wlan.modules, ["rfkill", "qca_cld3_peach_v2"]);
        assert_eq!(wlan.writes, [Write { path: "/sys/kernel/cnss/fs_ready".into(), value: "1".into() }]);
        assert_eq!(wlan.firmware.len(), 1, "the link its address is kept behind points at nothing on the phone itself");
        assert_eq!(wlan.firmware[0].partition, None);
        assert_eq!(wlan.libraries, ["/system/lib64", "/vendor/lib64"], "the recovery's own libraries come first");
        assert_eq!(wlan.service, "android.hardware.wifi.supplicant.ISupplicant/default");
        assert_eq!(cfg.update.expect("its update server is known").server, "https://update.miui.com/updates/miotaV3.php");
        let logd = cfg.logd.expect("the K100 Pro Max's recovery leaves out what its logd loads");
        assert_eq!(logd.libraries, ["/mnt/system/system/lib64/libsysutils.so", "/mnt/system/system/lib64/libcap.so"]);
        let haptics = cfg.haptics.expect("the K100 Pro Max has a motor");
        assert_eq!((haptics.click, haptics.resonance.as_deref()), (None, Some("/sys/class/qcom-haptics/lra_frequency_hz")), "its click is made");
        assert_eq!(haptics.strength, Strength { lightest: 0.25, strongest: 0.75 });
        assert_eq!(cfg.backup.efs, ["modemst1", "modemst2", "fsg", "fsc", "persist"]);
        assert!(cfg.backup.data_skipped.contains(&"vendor/secure_element".to_string()));
        assert!(cfg.wipe.logs.contains(&"/data/vendor/bsplog".to_string()));
    }

    #[test]
    fn parses_piano() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/piano.toml")).unwrap();
        assert_eq!(cfg.device.codename, "piano");
        assert_eq!(cfg.device.brand, "Xiaomi");
        assert_eq!(cfg.device.model, "Xiaomi Pad 8 Pro");
        assert_eq!(cfg.device.os_version_prop, "ro.mi.os.version.incremental");
        let soc = cfg.device.soc.expect("piano names its SoC");
        assert_eq!(soc.en, "Snapdragon® 8 Elite Mobile Platform");
        assert_eq!((cfg.display.width, cfg.display.height, cfg.display.density), (2136, 3200, 440));
        assert_eq!(cfg.display.diagonal, Some(11.2));
        assert_eq!(cfg.display.corner_radius, 54);
        assert!(cfg.modules.sources.iter().any(|s| s == "system_dlkm"), "rfkill is one of the kernel's own modules");
        assert_eq!(cfg.modules.load, ["nt36532_touch", "mitee_dlkm"], "the weaver's trusted environment is reached through the second");
        assert_eq!(
            cfg.touch.writes,
            [Write { path: "/sys/class/touch/touch_dev/enable_touch_raw".into(), value: "1".into() }],
            "its driver hands the frames it reads to the service that reads them"
        );
        assert_eq!(cfg.touch.reversed, ["y"], "its panel counts the screen's vertical axis from the bottom");
        assert_eq!(cfg.firmware, Some(FirmwarePartition { partition: "modem".into() }));
        assert_eq!(cfg.partitions.mounted, ["persist"], "the secure world keeps its own files there");
        assert!(cfg.logd.is_none(), "its stock recovery carries the libraries its logd loads");
        assert!(cfg.haptics.is_none(), "a tablet has no motor to click");
        let paths: Vec<&str> = cfg.services.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/vendor/bin/sh",
                "/odm/bin/hw/vendor.xiaomi.hw.touchfeature-service",
                "/vendor/bin/qseecomd",
                "/vendor/bin/hw/vendor.qti.hardware.qseecom@1.0-service",
                "/vendor/bin/qwesd",
                "/vendor/bin/ssgtzd",
                "/vendor/bin/hw/android.hardware.security.keymint-service-qti",
                "/vendor/bin/hw/android.hardware.secure_element-service.qti",
                "/vendor/bin/hw/android.hardware.boot-service.qti",
                "/odm/bin/hw/android.hardware.weaver",
            ]
        );
        let [post_boot, touch, _, opens_apps, qwesd, ssgtzd, keymint, element, boot, weaver] = &cfg.services[..] else {
            panic!("ten programs")
        };
        assert!(post_boot.oneshot, "the tablet's own settings for after start are made once");
        assert_eq!(post_boot.args, ["/vendor/bin/init.kernel.post_boot.sh"]);
        assert_eq!(
            touch.ready_service.as_deref(),
            Some("vendor.xiaomi.hw.touchfeature.ITouchFeature/default"),
            "its driver hands the frames it reads to this, and no touch reaches an input device without it"
        );
        assert_eq!(
            opens_apps.ready_service.as_deref(),
            Some("vendor.qti.hardware.qseecom.IQSEECom/default"),
            "the programs that name an app of the secure world's reach it through this"
        );
        assert_eq!(
            (qwesd.user.as_deref(), &qwesd.groups[..]),
            (Some("system"), &["system".to_string(), "inet".to_string()][..]),
            "its own init script starts it at boot, before the key service"
        );
        assert_eq!(
            (ssgtzd.user.as_deref(), &ssgtzd.groups[..]),
            (Some("system"), &["system".to_string(), "vendor_ssgtzd".to_string(), "wakelock".to_string()][..]),
            "the number its init script calls oem_2912 is named vendor_ssgtzd here"
        );
        assert_eq!(keymint.ready_service.as_deref(), Some("android.hardware.security.keymint.IKeyMintDevice/default"));
        assert!(keymint.registers.contains(&"android.hardware.security.sharedsecret.ISharedSecret/default".to_string()));
        assert!(
            keymint.libraries.iter().any(|l| l == "/mnt/system/system/lib64"),
            "its libhardware asks for the installed system's libvndksupport"
        );
        assert_eq!(element.ready_service, None, "a weaver that finds it late costs the recovery nothing it had");
        assert_eq!(boot.ready_service.as_deref(), Some("android.hardware.boot.IBootControl/default"));
        assert_eq!(weaver.ready_service.as_deref(), Some("android.hardware.weaver.IWeaver/default"), "the weaver skyward's crypto asks for");
        assert_eq!(
            (weaver.user.as_deref(), &weaver.groups[..]),
            (Some("system"), &["system".to_string(), "drmrpc".to_string()][..]),
            "the tablet keeps its weaver beside the services of its own credentials"
        );
        let wlan = cfg.wlan.expect("the tablet's WLAN is described");
        assert_eq!(wlan.modules, ["rfkill", "qca_cld3_peach_v2"]);
        assert_eq!(wlan.firmware.len(), 2, "both the settings and the address are given to the loader");
        assert_eq!(
            wlan.firmware[1].path, "/mnt/vendor/persist/wlan/wlan_mac.bin",
            "the address is read where persist is mounted already, rather than mounted again"
        );
        assert_eq!(wlan.libraries, ["/system/lib64", "/vendor/lib64"], "the recovery's own libraries come first");
        assert_eq!(wlan.service, "android.hardware.wifi.supplicant.ISupplicant/default");
        assert_eq!(cfg.update.expect("its update server is known").server, "https://update.miui.com/updates/miotaV3.php");
        assert_eq!(cfg.backup.efs, ["modemst1", "modemst2", "fsg", "fsc", "persist"]);
        assert!(cfg.backup.data_skipped.contains(&"vendor/secure_element".to_string()));
        assert!(cfg.wipe.logs.contains(&"/data/miuilog".to_string()));
    }

    #[test]
    fn what_a_backup_takes_in_is_named_plainly() {
        let base = "[device]\ncodename = \"x\"\n";
        assert!(DeviceConfig::parse(&format!("{base}[backup]\nefs = [\"by-name/fsg\"]\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}[backup]\ndata_skipped = [\"/data/vendor\"]\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}[backup]\ndata_skipped = [\"vendor/../system\"]\n")).is_err());
        assert!(DeviceConfig::parse(&format!("{base}[backup]\ndata_skipped = [\"vendor_de/*/fpdata\"]\n")).is_ok());
    }

    #[test]
    fn the_phones_own_logs_are_folders_inside_the_data_partition() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        assert!(cfg.wipe.logs.contains(&"/data/vendor/diag".to_string()));
        assert!(DeviceConfig::parse("[device]\ncodename = \"x\"\n").unwrap().wipe.logs.is_empty(), "a phone may have none");
        for bad in ["/data", "/data/", "/metadata/logs", "data/log", "/data/../system"] {
            let text = format!("[device]\ncodename = \"x\"\n[wipe]\nlogs = [\"{bad}\"]\n");
            assert!(DeviceConfig::parse(&text).is_err(), "{bad} is refused");
        }
    }

    #[test]
    fn ishtar_brings_its_wlan_up_from_its_own_partitions() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        let wlan = cfg.wlan.expect("the 13 Ultra's WLAN is described");
        assert_eq!(wlan.modules, ["qca_cld3_kiwi_v2"]);
        assert_eq!(wlan.writes, [Write { path: "/sys/kernel/cnss/fs_ready".into(), value: "1".into() }]);
        assert_eq!(wlan.firmware[0].partition, None);
        assert_eq!(wlan.firmware[1].partition.as_deref(), Some("persist"));
        assert_eq!(wlan.firmware[1].path, "kiwi_v2/wlan_mac.bin");
        assert_eq!(wlan.interface, "wlan0");
        assert_eq!(wlan.service, "android.hardware.wifi.supplicant.ISupplicant/default");
        assert_eq!(wlan.probe, "http://connect.rom.miui.com/generate_204");
        assert_eq!(cfg.update.expect("its update server is known").server, "https://update.miui.com/updates/miotaV3.php");
    }

    #[test]
    fn nezha_brings_its_wlan_up_from_its_own_partitions() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/nezha.toml")).unwrap();
        assert!(cfg.modules.sources.iter().any(|s| s == "system_dlkm"), "the kernel's own modules are a source");
        let wlan = cfg.wlan.expect("the 17 Ultra's WLAN is described");
        assert_eq!(wlan.modules, ["rfkill", "qca_cld3_peach_v2"]);
        assert_eq!(wlan.writes, [Write { path: "/sys/kernel/cnss/fs_ready".into(), value: "1".into() }]);
        assert!(wlan.firmware.is_empty(), "the vendor's own links reach its settings and persist's MAC address");
        assert_eq!(wlan.libraries, ["/system/lib64", "/vendor/lib64"], "the recovery's own libraries come first");
        assert_eq!(wlan.probe, "http://connect.rom.miui.com/generate_204", "as the stock system asks it");
        assert_eq!(cfg.update.expect("its update server is known").server, "https://update.miui.com/updates/miotaV3.php");
    }

    #[test]
    fn a_wlan_is_probed_over_plain_http() {
        let base = "[device]\ncodename = \"x\"\n[wlan]\nmodules = []\ninterface = \"wlan0\"\nsupplicant = \"/s\"\nservice = \"a/b\"\n";
        assert!(DeviceConfig::parse(&format!("{base}probe = \"http://connect.rom.miui.com/generate_204\"\n")).is_ok());
        assert!(DeviceConfig::parse(&format!("{base}probe = \"https://connect.rom.miui.com/generate_204\"\n")).is_err(), "a login cannot answer in its stead");
        assert!(DeviceConfig::parse(base).is_err(), "a WLAN says how it is probed");
    }

    #[test]
    fn firmware_is_named_from_the_loaders_folders_and_found_plainly() {
        let base = "[device]\ncodename = \"x\"\n[wlan]\nmodules = []\ninterface = \"wlan0\"\nsupplicant = \"/s\"\nservice = \"a/b\"\nprobe = \"http://p/\"\n";
        let with = |file: &str| DeviceConfig::parse(&format!("{base}firmware = [{file}]\n"));
        assert!(with("{ name = \"wlan/a.ini\", path = \"/vendor/etc/a.ini\" }").is_ok());
        assert!(with("{ name = \"wlan/mac.bin\", partition = \"persist\", path = \"kiwi/mac.bin\" }").is_ok());
        assert!(with("{ name = \"/etc/firmware/a.ini\", path = \"/vendor/etc/a.ini\" }").is_err());
        assert!(with("{ name = \"../a.ini\", path = \"/vendor/etc/a.ini\" }").is_err());
        assert!(with("{ name = \"a.ini\", path = \"vendor/etc/a.ini\" }").is_err());
        assert!(with("{ name = \"mac.bin\", partition = \"persist\", path = \"/kiwi/mac.bin\" }").is_err());
        assert!(with("{ name = \"mac.bin\", partition = \"by-name/persist\", path = \"mac.bin\" }").is_err());
    }

    #[test]
    fn services_use_the_partitions_they_run_from() {
        let cfg = DeviceConfig::parse(include_str!("../../../devices/ishtar.toml")).unwrap();
        let [report, sensor, listeners, ..] = &cfg.services[..] else { panic!("ishtar keeps several programs running") };
        assert!(report.uses(Path::new("/odm")));
        assert!(sensor.uses(Path::new("/odm")));
        assert!(!report.uses(Path::new("/odm_dlkm")), "a longer name is another partition");
        assert!(!sensor.uses(Path::new("/mnt/system")), "its /system/lib64 is the recovery's own");
        assert!(!report.uses(Path::new("/vendor")));
        assert!(listeners.uses(Path::new("/vendor")));
    }
}
