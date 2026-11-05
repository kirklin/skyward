use anyhow::{bail, ensure, Context, Result};
use procfs::process::{FDTarget, MMapPath, Process};
use skyward_config::Service;
use skyward_sys::ueventd;
use skyward_vab::Mounted;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// A program that ends sooner than this after starting waits this long
/// before it is started again.
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// How long a program gets to become what it says it will be.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(100);

/// Where the kernel says the highest capability it knows.
const CAP_LAST_CAP: &str = "/proc/sys/kernel/cap_last_cap";

/// The version of the kernel's capability sets that holds 64 of them.
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// Keep each program running, one thread per program: start it, and start
/// it again whenever it exits. They are started in the order they are
/// written, and one that says what it will be ready with holds up those
/// after it until it is. A program still running from an earlier instance
/// of the recovery service is waited on instead of started twice. A
/// program that runs once is run to its end where it stands in that order.
/// Each runs as the user, in the groups and with the capabilities the
/// phone's own init gives it.
///
/// `libraries` is a directory of shared libraries every one of them may
/// need, searched after whatever it names itself.
pub fn start(services: &[Service], libraries: Option<&Path>) -> Result<()> {
    let known = ueventd::accounts(&ueventd::PASSWD_FILES.iter().chain(&ueventd::GROUP_FILES).copied().collect::<Vec<_>>());
    for s in services {
        if !Path::new(&s.path).is_file() {
            bail!("{} is missing; is its partition mounted?", s.path);
        }
        let mut s = s.clone();
        if let Some(dir) = libraries {
            s.libraries.push(dir.to_string_lossy().into_owned());
        }
        let who = Credentials::of(&s, &known)?;
        if s.oneshot {
            let status = command(&s, &who).status().with_context(|| format!("run {}", s.path))?;
            ensure!(status.success(), "{} {} ended: {status}", s.path, s.args.join(" "));
            skyward_sys::kmsg(&format!("ran {} {}", s.path, s.args.join(" ")))?;
            continue;
        }
        let waiting = s.clone();
        let name = Path::new(&s.path).file_name().map_or_else(|| s.path.clone(), |n| n.to_string_lossy().into_owned());
        thread::Builder::new().name(name).spawn(move || supervise(&s, &who)).context("start a supervising thread")?;
        ready(&waiting)?;
    }
    Ok(())
}

/// Who a program runs as and what of root's it keeps, as the phone's own
/// init has it run, worked out before it is started: between the fork and
/// the exec nothing may be looked up.
#[derive(Debug, Clone)]
struct Credentials {
    uid: libc::uid_t,
    gid: libc::gid_t,
    /// The other groups it is in.
    groups: Vec<libc::gid_t>,
    kept: Option<Kept>,
}

/// The capabilities a program keeps, as masks.
#[derive(Debug, Clone, Copy)]
struct Kept {
    mask: u64,
    /// Those and the right to drop the others for good, which goes once
    /// they are dropped.
    dropping: u64,
    /// The highest capability the kernel knows.
    last: u32,
}

impl Credentials {
    /// Who `s` runs as, by the names the phone gives its accounts: root,
    /// in no other group, keeping all of root's, where it names none.
    fn of(s: &Service, known: &HashMap<String, u32>) -> Result<Self> {
        let id = |name: &String| ueventd::account(known, name).with_context(|| format!("{}: the phone has no account named {name}", s.path));
        let uid = s.user.as_ref().map(id).transpose()?.unwrap_or(0);
        let mut groups = s.groups.iter().map(id).collect::<Result<Vec<_>>>()?;
        let gid = if groups.is_empty() { 0 } else { groups.remove(0) };
        let kept = if s.capabilities.is_empty() {
            None
        } else {
            let bit = |name: &str| skyward_config::capability(name).map(|n| 1u64 << n).with_context(|| format!("{}: {name} is not a capability", s.path));
            let mask = s.capabilities.iter().try_fold(0, |mask, c| bit(c).map(|b| mask | b))?;
            let last = fs::read_to_string(CAP_LAST_CAP).with_context(|| format!("read {CAP_LAST_CAP}"))?;
            let last = last.trim().parse().with_context(|| format!("read the number in {CAP_LAST_CAP}"))?;
            Some(Kept { mask, dropping: mask | bit("SETPCAP")?, last })
        };
        Ok(Self { uid, gid, groups, kept })
    }

    /// Become who the program runs as, in the child about to run it, as
    /// init does: take its groups and its user, keeping the capabilities
    /// through the change where it keeps some, then drop every other one
    /// for good and hand those it keeps on through the exec.
    fn take_on(&self) -> io::Result<()> {
        let done = |r: libc::c_int| if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) };
        let prctl = |option: libc::c_int, a: libc::c_ulong, b: libc::c_ulong| done(unsafe { libc::prctl(option, a, b, 0 as libc::c_ulong, 0 as libc::c_ulong) });
        if self.kept.is_some() {
            prctl(libc::PR_SET_KEEPCAPS, 1, 0)?;
        }
        done(unsafe { libc::setgroups(self.groups.len(), self.groups.as_ptr()) })?;
        done(unsafe { libc::setresgid(self.gid, self.gid, self.gid) })?;
        done(unsafe { libc::setresuid(self.uid, self.uid, self.uid) })?;
        let Some(kept) = self.kept else { return Ok(()) };
        set_capabilities(kept.dropping)?;
        for cap in (0..=kept.last).filter(|c| kept.mask & 1 << c == 0) {
            prctl(libc::PR_CAPBSET_DROP, cap.into(), 0)?;
        }
        set_capabilities(kept.mask)?;
        for cap in (0..=kept.last).filter(|c| kept.mask & 1 << c != 0) {
            prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_RAISE as libc::c_ulong, cap.into())?;
        }
        Ok(())
    }
}

/// Make `mask` the capabilities the process has, may take up and hands on.
fn set_capabilities(mask: u64) -> io::Result<()> {
    #[repr(C)]
    struct Header {
        version: u32,
        pid: libc::c_int,
    }
    #[repr(C)]
    struct Sets {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header { version: CAPABILITY_VERSION_3, pid: 0 };
    let half = |bits: u32| Sets { effective: bits, permitted: bits, inheritable: bits };
    let sets = [half(mask as u32), half((mask >> 32) as u32)];
    if unsafe { libc::syscall(libc::SYS_capset, &header, sets.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Wait until the program has what it says it will be ready with.
fn ready(s: &Service) -> Result<()> {
    let waited_on = match (&s.ready_property, &s.ready_service, &s.ready_hardware_service) {
        (Some(p), _, _) => format!("set no {p}"),
        (_, Some(n), _) => format!("registered no {n}"),
        (_, _, Some(n)) => format!("registered no {n}"),
        _ => return Ok(()),
    };
    let until = Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(name) = &s.ready_property {
            if skyward_sys::prop(name).is_some_and(|v| !v.is_empty()) {
                return Ok(());
            }
        }
        // Either directory may itself be starting again, and until it
        // answers nothing can be looked up in it.
        if let Some(name) = &s.ready_service {
            if registered(name) {
                return Ok(());
            }
        }
        if let Some(name) = &s.ready_hardware_service {
            if registered_hardware(name) {
                return Ok(());
            }
        }
        if Instant::now() >= until {
            bail!("{} {waited_on} in {} seconds", s.path, READY_TIMEOUT.as_secs());
        }
        thread::sleep(READY_POLL);
    }
}

fn supervise(s: &Service, who: &Credentials) -> ! {
    loop {
        let started = Instant::now();
        let ended = match running(&s.path) {
            Some(pid) => wait_for(pid).map(|()| format!("pid {pid}, left by an earlier recovery service, exited")),
            None => run(s, who),
        };
        let line = match ended {
            Ok(how) => format!("{}: {how}", s.path),
            Err(e) => format!("{}: {e:#}", s.path),
        };
        let _ = skyward_sys::kmsg(&line);
        if started.elapsed() < RESTART_DELAY {
            thread::sleep(RESTART_DELAY);
        }
    }
}

/// Start the program and wait for it to end.
fn run(s: &Service, who: &Credentials) -> Result<String> {
    let mut child = command(s, who).spawn().with_context(|| format!("start {}", s.path))?;
    let status = child.wait().context("wait for it")?;
    Ok(format!("pid {} ended: {status}", child.id()))
}

/// The program as it is started: with what it is given, from the top of the
/// tree, reading and writing nothing of the recovery's, finding its
/// libraries where it names them, and as `who`.
fn command(s: &Service, who: &Credentials) -> Command {
    let mut cmd = Command::new(&s.path);
    cmd.args(&s.args).current_dir("/").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if !s.libraries.is_empty() {
        cmd.env("LD_LIBRARY_PATH", s.libraries.join(":"));
    }
    let who = who.clone();
    // Only system calls run between the fork and the exec, on what was
    // worked out before it.
    unsafe { cmd.pre_exec(move || who.take_on()) };
    cmd
}

/// Lock in memory every file the running programs have mapped or open on
/// the `mounted` partitions, so they keep working once those partitions
/// are written over, and give the mount points the files are on. The
/// locked mappings stay for the rest of the recovery's life.
pub fn pin(services: &[Service], mounted: &[Mounted]) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    // A program run once has ended, and one between restarts has nothing in
    // memory to keep either.
    for s in services.iter().filter(|s| !s.oneshot) {
        let Some(pid) = running(&s.path) else { continue };
        let process = Process::new(pid as i32).with_context(|| format!("open process {pid}"))?;
        for map in process.maps().with_context(|| format!("read the mappings of {}", s.path))? {
            if let MMapPath::Path(p) = map.pathname {
                files.insert(p);
            }
        }
        for fd in process.fd().with_context(|| format!("read the files {} has open", s.path))? {
            if let FDTarget::Path(p) = fd.with_context(|| format!("read a file {} has open", s.path))?.target {
                files.insert(p);
            }
        }
    }
    let mut busy: Vec<PathBuf> = Vec::new();
    for file in files {
        let Some(m) = mounted.iter().find(|m| file.starts_with(&m.mount_point)) else { continue };
        if !fs::metadata(&file).with_context(|| format!("stat {}", file.display()))?.is_file() {
            continue;
        }
        lock(&file)?;
        let _ = skyward_sys::kmsg(&format!("kept {} in memory", file.display()));
        if !busy.contains(&m.mount_point) {
            busy.push(m.mount_point.clone());
        }
    }
    Ok(busy)
}

/// Map `path` whole and lock the mapping: its pages then stay in memory
/// for every process that maps the file.
fn lock(path: &Path) -> Result<()> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file.metadata().with_context(|| format!("stat {}", path.display()))?.len() as usize;
    if len == 0 {
        return Ok(());
    }
    let at = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    if at == libc::MAP_FAILED {
        return Err(io::Error::last_os_error()).with_context(|| format!("map {}", path.display()));
    }
    if unsafe { libc::mlock(at, len) } != 0 {
        return Err(io::Error::last_os_error()).with_context(|| format!("lock {} in memory", path.display()));
    }
    Ok(())
}

/// Whether a service has registered. The service directory may itself be
/// starting again, and until it answers nothing is registered as far as
/// anyone can tell.
pub fn registered(name: &str) -> bool {
    match skyward_binder::driver::Binder::open() {
        Ok(binder) => matches!(skyward_binder::service::find(&binder, name), Ok(Some(_))),
        Err(_) => false,
    }
}

/// Whether a hardware service of the older kind has registered with the
/// directory its kind uses. It is named as the interface and then which of
/// them it is.
pub fn registered_hardware(name: &str) -> bool {
    let Some((interface, instance)) = name.rsplit_once('/') else { return false };
    match skyward_binder::driver::Binder::hardware() {
        Ok(binder) => matches!(skyward_binder::hidl::find(&binder, interface, instance), Ok(Some(_))),
        Err(_) => false,
    }
}

/// Whether the service directory takes calls. What it is asked about does
/// not matter; that it answers does.
pub fn reachable() -> bool {
    match skyward_binder::driver::Binder::open() {
        Ok(binder) => skyward_binder::service::find(&binder, "android.os.IServiceManager").is_ok(),
        Err(_) => false,
    }
}

/// A process already running `path`.
pub fn running(path: &str) -> Option<u32> {
    fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        (fs::read_link(e.path().join("exe")).ok()? == Path::new(path)).then_some(pid)
    })
}

/// Wait for a process that is not our child to exit.
fn wait_for(pid: u32) -> Result<()> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if fd < 0 {
        let e = io::Error::last_os_error();
        // It exited between being found and being opened.
        if e.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(e).context("open a pidfd");
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let mut pfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    loop {
        if unsafe { libc::poll(&mut pfd, 1, -1) } >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e).context("wait on the pidfd");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_program, PHONE_LIBRARIES};

    fn write(name: &str, head: &[u8]) -> std::path::PathBuf {
        let at = std::env::temp_dir().join(name);
        std::fs::write(&at, head).expect("write the file");
        at
    }

    #[test]
    fn a_script_is_not_a_program_the_linker_can_load() {
        let at = write("skyward-test-script", b"#!/system/bin/sh\necho hi\n");
        assert!(!is_program(at.to_str().expect("a path")));
        let _ = std::fs::remove_file(at);
    }

    #[test]
    fn a_program_is_one_the_linker_can_load() {
        let at = write("skyward-test-program", b"\x7fELF\x02\x01\x01\x00");
        assert!(is_program(at.to_str().expect("a path")));
        let _ = std::fs::remove_file(at);
    }

    #[test]
    fn a_file_that_is_not_there_is_not_a_program() {
        assert!(!is_program("/nowhere/at/all/skyward"));
    }

    #[test]
    fn the_phone_s_libraries_include_the_bootstrap_folder() {
        // The library of the loader lives in a folder of its own beside the
        // others, and a linker that is not given it says so.
        assert!(PHONE_LIBRARIES.contains("/bootstrap:"), "{PHONE_LIBRARIES}");
    }
}
