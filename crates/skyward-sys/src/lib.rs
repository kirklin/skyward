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
pub mod worker;

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
    unsafe extern "C" {
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

/// Whether the file [`keep_log`] opened was there already, an earlier run
/// of the recovery service having kept its log in it.
static RUNS_AGAIN: OnceLock<bool> = OnceLock::new();

/// Keep every line [`kmsg`] writes from now on in the file at `path` as
/// well, after what an earlier run left there.
pub fn keep_log(path: &Path) -> Result<()> {
    let again = path.exists();
    let file = OpenOptions::new().create(true).append(true).open(path).with_context(|| format!("open {}", path.display()))?;
    KEPT.set(Mutex::new(file)).map_err(|_| anyhow!("the log is kept in a file already"))?;
    RUNS_AGAIN.set(again).map_err(|_| anyhow!("the log is kept in a file already"))
}

/// Whether the recovery service runs again since the phone started, as the
/// log an earlier run of it kept tells. What that run set up in the phone's
/// drivers is as it left it, a touch controller at rest among it.
pub fn runs_again() -> bool {
    RUNS_AGAIN.get().copied().unwrap_or(false)
}

/// What every record written to the kernel log opens with: the level it
/// is written at, and whose it is.
const RECORD_OPENS: &str = "<6>skyward: ";

/// The most bytes the kernel takes in one write to its log, with what the
/// record opens with and the end of its line counted in, on the kernels
/// that take the fewest: 1024 less the 48 a kernel that names the caller
/// of each record keeps for what it puts before one. A longer write is
/// turned away whole.
const RECORD_MOST: usize = 1024 - 48;

/// Append `text` to the kernel log, where it survives until the next boot
/// and shows in `dmesg`, and to the file [`keep_log`] opened, stamped with
/// the seconds since the phone started as the kernel log stamps it. The
/// file takes the text first and whole; the kernel takes each of its lines
/// as a record of its own, and a line longer than a record as several.
pub fn kmsg(text: &str) -> Result<()> {
    let kept = match KEPT.get() {
        Some(kept) => {
            let mut kept = kept.lock().expect("nothing panics while writing the log");
            since_boot().and_then(|at| writeln!(kept, "[{:>12.6}] {text}", at.as_secs_f64()).with_context(|| format!("write {LOG}")))
        }
        None => Ok(()),
    };
    let mut f = OpenOptions::new().write(true).open("/dev/kmsg")?;
    for record in written(text) {
        f.write_all(record.as_bytes())?;
    }
    kept
}

/// Why something done over and over last failed, as the log was last told:
/// a reading taken each second that keeps failing for one reason is told
/// of once, and again when the reason changes or after it went well.
#[derive(Debug, Default)]
pub struct ToldOnce(Option<String>);

impl ToldOnce {
    /// Nothing told yet.
    pub const fn new() -> ToldOnce {
        ToldOnce(None)
    }

    /// Take in how it went this time, `why` it failed or none where it
    /// went well: whether that is a failure the log has yet to be told of.
    pub fn untold(&mut self, why: Option<&str>) -> bool {
        let untold = why.is_some() && self.0.as_deref() != why;
        self.0 = why.map(str::to_string);
        untold
    }
}

/// What is written to the kernel log for `text`, a write for each record.
fn written(text: &str) -> Vec<String> {
    records(text, RECORD_MOST - RECORD_OPENS.len() - 1).into_iter().map(|record| format!("{RECORD_OPENS}{record}\n")).collect()
}

/// `text` as the records the kernel log takes it in: each of its lines,
/// and a line of more than `most` bytes as several, cut where a character
/// ends. Text without a line in it is one empty record.
fn records(text: &str, most: usize) -> Vec<&str> {
    let mut records = Vec::new();
    for mut line in text.lines() {
        while line.len() > most {
            let mut end = most;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            let (record, rest) = line.split_at(end);
            records.push(record);
            line = rest;
        }
        records.push(line);
    }
    if records.is_empty() {
        records.push("");
    }
    records
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
    fn text_goes_to_the_kernel_log_a_record_for_each_line() {
        assert_eq!(records("erasing ended: done", 963), ["erasing ended: done"]);
        assert_eq!(records("mkfs ended with 1:\nInfo: sector size = 4096\n\nError: device is busy\n", 963), ["mkfs ended with 1:", "Info: sector size = 4096", "", "Error: device is busy"]);
        assert_eq!(records("", 963), [""], "a record still says that something was logged");
        assert_eq!(written("the phone slept and woke"), ["<6>skyward: the phone slept and woke\n"]);
    }

    #[test]
    fn a_line_longer_than_a_record_is_several_cut_where_a_character_ends() {
        let line = "x".repeat(2000);
        let cut = records(&line, 963);
        assert_eq!(cut.iter().map(|record| record.len()).collect::<Vec<_>>(), [963, 963, 74]);
        assert_eq!(cut.concat(), line);
        let exact = "y".repeat(963);
        assert_eq!(records(&exact, 963), [exact.as_str()], "a line a record just holds is one");
        // Each of these characters takes three bytes.
        let wide = "擦除结束".repeat(3);
        let cut = records(&wide, 10);
        assert_eq!(cut, ["擦除结", "束擦除", "结束擦", "除结束"]);
        assert_eq!(cut.concat(), wide);
    }

    /// The output of a tool that makes a filesystem, as the erase page
    /// logs it when the tool fails, is longer than the kernel takes in one
    /// write, and goes in whole in writes the kernel takes.
    #[test]
    fn no_write_to_the_kernel_log_is_longer_than_the_kernel_takes() {
        let said = format!("erasing ended: Format failed: /system/bin/make_f2fs ended with 1: {}\n{}", "Info: ".repeat(400), "错误：设备正忙。".repeat(200));
        let writes = written(&said);
        assert!(writes.len() > 2);
        for write in &writes {
            assert!(write.len() <= RECORD_MOST, "{} bytes", write.len());
            assert!(write.starts_with(RECORD_OPENS) && write.ends_with('\n'));
        }
        let logged: String = writes.iter().map(|write| &write[RECORD_OPENS.len()..write.len() - 1]).collect();
        assert_eq!(logged, said.replace('\n', ""));
    }

    #[test]
    fn a_failure_that_keeps_coming_is_told_of_once() {
        let mut told = ToldOnce::new();
        assert!(!told.untold(None), "nothing failed");
        assert!(told.untold(Some("read status: I/O error")));
        assert!(!told.untold(Some("read status: I/O error")), "the same a second later");
        assert!(told.untold(Some("read capacity: no data")), "another reason");
        assert!(!told.untold(None), "it went well");
        assert!(told.untold(Some("read capacity: no data")), "the same reason, after it went well in between");
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
