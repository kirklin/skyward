use anyhow::{anyhow, bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub mod bcb;
pub mod clock;
pub mod control;
pub mod cpu;
#[cfg(target_os = "android")]
pub mod sleep;
#[cfg(target_os = "android")]
pub mod uevent;
pub mod ueventd;
pub mod usb;

/// A system property, or `None` when it is unset.
#[cfg(target_os = "android")]
pub fn prop(name: &str) -> Option<String> {
    use std::ffi::{CStr, CString};
    let key = CString::new(name).expect("property names contain no NUL");
    let mut buf = [0 as libc::c_char; libc::PROP_VALUE_MAX as usize];
    let n = unsafe { libc::__system_property_get(key.as_ptr(), buf.as_mut_ptr()) };
    if n <= 0 {
        return None;
    }
    Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

/// Hosts have no property area.
#[cfg(not(target_os = "android"))]
pub fn prop(_name: &str) -> Option<String> {
    None
}

/// A system property read as a yes or a no, the way Android reads one;
/// `default` where it is unset or says neither.
pub fn bool_prop(name: &str, default: bool) -> bool {
    yes_or_no(prop(name).as_deref()).unwrap_or(default)
}

/// What a property's value says, as Android reads a yes or a no.
fn yes_or_no(value: Option<&str>) -> Option<bool> {
    match value? {
        "1" | "y" | "yes" | "on" | "true" => Some(true),
        "0" | "n" | "no" | "off" | "false" => Some(false),
        _ => None,
    }
}

/// Every system property whose name starts with `prefix`, with its value.
#[cfg(target_os = "android")]
pub fn props(prefix: &str) -> Vec<(String, String)> {
    use std::ffi::{c_char, c_void, CStr};
    extern "C" {
        fn __system_property_read_callback(
            pi: *const libc::prop_info,
            callback: unsafe extern "C" fn(cookie: *mut c_void, name: *const c_char, value: *const c_char, serial: u32),
            cookie: *mut c_void,
        );
    }
    unsafe extern "C" fn take(cookie: *mut c_void, name: *const c_char, value: *const c_char, _serial: u32) {
        let all = unsafe { &mut *cookie.cast::<Vec<(String, String)>>() };
        let (name, value) = unsafe { (CStr::from_ptr(name), CStr::from_ptr(value)) };
        all.push((name.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()));
    }
    unsafe extern "C" fn each(pi: *const libc::prop_info, cookie: *mut c_void) {
        unsafe { __system_property_read_callback(pi, take, cookie) };
    }
    let mut all: Vec<(String, String)> = Vec::new();
    unsafe { libc::__system_property_foreach(each, (&mut all as *mut Vec<(String, String)>).cast()) };
    all.retain(|(name, _)| name.starts_with(prefix));
    all
}

/// Hosts have no property area.
#[cfg(not(target_os = "android"))]
pub fn props(_prefix: &str) -> Vec<(String, String)> {
    Vec::new()
}

/// Ask init to set a property; init refuses names the caller's SELinux
/// domain may not set.
#[cfg(target_os = "android")]
pub fn set_prop(name: &str, value: &str) -> Result<()> {
    use std::ffi::CString;
    let (k, v) = (CString::new(name)?, CString::new(value)?);
    if unsafe { libc::__system_property_set(k.as_ptr(), v.as_ptr()) } != 0 {
        bail!("init refused {name}={value}");
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn set_prop(name: &str, _value: &str) -> Result<()> {
    bail!("this host has no property service to set {name}")
}

/// Wait until property `name` reads `value`, for at most `timeout`: init
/// sets some properties only once it has done what another one asked of
/// it.
pub fn wait_prop(name: &str, value: &str, timeout: Duration) -> Result<()> {
    let until = Instant::now() + timeout;
    loop {
        let now = prop(name);
        if now.as_deref() == Some(value) {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("{name} is {now:?} after {} ms, not {value:?}", timeout.as_millis());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Where a power request sends the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    System,
    Recovery,
    /// The recovery's own userspace fastboot. On a phone with dynamic
    /// partitions init leaves `--fastboot` for the recovery in misc and
    /// boots it.
    Fastboot,
    Bootloader,
    /// Qualcomm emergency download mode.
    Edl,
    Off,
}

impl Power {
    pub fn parse(name: &str) -> Option<Power> {
        Some(match name {
            "system" => Power::System,
            "recovery" => Power::Recovery,
            "fastboot" => Power::Fastboot,
            "bootloader" => Power::Bootloader,
            "edl" => Power::Edl,
            "off" => Power::Off,
            _ => return None,
        })
    }

    /// The `sys.powerctl` value; init syncs and unmounts before it acts.
    pub fn command(self) -> &'static str {
        match self {
            Power::System => "reboot,",
            Power::Recovery => "reboot,recovery",
            Power::Fastboot => "reboot,fastboot",
            Power::Bootloader => "reboot,bootloader",
            Power::Edl => "reboot,edl",
            Power::Off => "shutdown,",
        }
    }
}

pub fn power(target: Power) -> Result<()> {
    set_prop("sys.powerctl", target.command())
}

/// The folder the recovery service keeps what it makes for itself in, for
/// as long as the phone stays in recovery: its logs, the screenshots that
/// wait for internal storage, the supplicant's settings. Install scripts
/// written for other recoveries often empty /tmp before they start; this
/// folder is out of their way.
pub const RUN: &str = "/skyward/run";

/// Where the recovery service keeps the lines it writes to the kernel log
/// for as long as the phone stays in recovery.
pub const LOG: &str = "/skyward/run/recovery.log";

/// The file [`kmsg`] writes its lines to as well, once [`keep_log`] opened
/// it.
static KEPT: OnceLock<Mutex<File>> = OnceLock::new();

/// Keep every line [`kmsg`] writes from now on in the file at `path` as
/// well, after what an earlier run left there.
pub fn keep_log(path: &Path) -> Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path).with_context(|| format!("open {}", path.display()))?;
    KEPT.set(Mutex::new(file)).map_err(|_| anyhow!("the log is kept in a file already"))
}

/// Append a line to the kernel log, where it survives until the next boot
/// and shows in `dmesg`, and to the file [`keep_log`] opened, stamped with
/// the seconds since the phone started as the kernel log stamps it.
pub fn kmsg(line: &str) -> Result<()> {
    let mut f = OpenOptions::new().write(true).open("/dev/kmsg")?;
    f.write_all(format!("<6>skyward: {line}\n").as_bytes())?;
    if let Some(kept) = KEPT.get() {
        let mut kept = kept.lock().expect("nothing panics while writing the log");
        writeln!(kept, "[{:>12.6}] {line}", since_boot()?.as_secs_f64()).with_context(|| format!("write {LOG}"))?;
    }
    Ok(())
}

/// How long the phone has been running, on the clock the kernel stamps its
/// log with.
fn since_boot() -> Result<Duration> {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) } != 0 {
        return Err(std::io::Error::last_os_error()).context("read the time since the phone started");
    }
    Ok(Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
}

/// The kernel log as `dmesg` prints it: as much of it as the kernel still
/// holds, each line without the level it was written at.
#[cfg(target_os = "android")]
pub fn kernel_log() -> Result<String> {
    const READ_ALL: libc::c_int = 3;
    const SIZE_BUFFER: libc::c_int = 10;
    let size = unsafe { libc::klogctl(SIZE_BUFFER, std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(std::io::Error::last_os_error()).context("ask the kernel how much log it holds");
    }
    let mut held = vec![0u8; size as usize];
    let n = unsafe { libc::klogctl(READ_ALL, held.as_mut_ptr().cast(), size) };
    if n < 0 {
        return Err(std::io::Error::last_os_error()).context("read the kernel log");
    }
    held.truncate(n as usize);
    Ok(without_levels(&String::from_utf8_lossy(&held)))
}

/// Kernel log lines without the `<6>` and the like that say the level each
/// was written at.
#[cfg(any(target_os = "android", test))]
fn without_levels(log: &str) -> String {
    log.lines().map(|line| line.strip_prefix('<').and_then(|l| l.split_once('>')).map_or(line, |(_, rest)| rest)).fold(
        String::with_capacity(log.len()),
        |mut out, line| {
            out.push_str(line);
            out.push('\n');
            out
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_log_lines_lose_their_levels() {
        let held = "<6>[    5.499346] skyward: ro.build.version.release is 16\n<3>[    6.1] dwc3 a600000.dwc3: a > b\nno level\n";
        assert_eq!(without_levels(held), "[    5.499346] skyward: ro.build.version.release is 16\n[    6.1] dwc3 a600000.dwc3: a > b\nno level\n");
    }

    #[test]
    fn power_names_round_trip() {
        for (name, cmd) in [
            ("system", "reboot,"),
            ("recovery", "reboot,recovery"),
            ("fastboot", "reboot,fastboot"),
            ("bootloader", "reboot,bootloader"),
            ("edl", "reboot,edl"),
            ("off", "shutdown,"),
        ] {
            assert_eq!(Power::parse(name).unwrap().command(), cmd);
        }
        assert_eq!(Power::parse("download"), None);
    }

    #[test]
    fn a_property_says_yes_or_no_in_the_words_android_takes() {
        for yes in ["1", "y", "yes", "on", "true"] {
            assert_eq!(yes_or_no(Some(yes)), Some(true), "{yes}");
        }
        for no in ["0", "n", "no", "off", "false"] {
            assert_eq!(yes_or_no(Some(no)), Some(false), "{no}");
        }
        assert_eq!(yes_or_no(Some("True")), None, "the words are read as written");
        assert_eq!(yes_or_no(Some("")), None);
        assert_eq!(yes_or_no(None), None);
    }
}
