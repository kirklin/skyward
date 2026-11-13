use anyhow::{bail, ensure, Context, Result};
use procfs::process::{FDTarget, MMapPath, Process};
use skyward_config::Service;
use skyward_programs::script::{Socket, SocketKind};
use skyward_programs::secure::Started;
use skyward_sys::ueventd;
use skyward_vab::Mounted;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
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

/// Where init makes the sockets it hands its programs, and what it names
/// the variable that tells a program which of its open files one of them
/// is: this and then the socket's name.
const SOCKETS: &str = "/dev/socket";
const SOCKET_VARIABLE: &str = "ANDROID_SOCKET_";

/// How many callers init has a socket it listens on hold while they wait:
/// as many as the kernel lets it.
const WAITING_CALLERS: libc::c_int = 1 << 30;

/// The variable a program's loader reads the folders of its libraries
/// from.
const LIBRARY_PATH: &str = "LD_LIBRARY_PATH";

/// Keep each program running, one thread per program: start it, and start
/// it again whenever it exits. They are started in the order they are
/// given, and one that says what it will be ready with holds up those
/// after it until it is. A program still running from an earlier instance
/// of the recovery service is waited on instead of started twice, unless
/// it is one to start anew, which is ended first. A program that runs once
/// is run to its end where it stands in that order. Each runs as the user,
/// in the groups and with the capabilities the phone's own init gives it,
/// with the sockets and the environment init hands it.
///
/// `libraries` is a directory of shared libraries every one of them may
/// need, searched after whatever it names itself.
pub fn start(programs: &[Started], libraries: Option<&Path>) -> Result<()> {
    let known = accounts();
    for program in programs {
        let (s, who, given) = readied(program, libraries, &known)?;
        if s.oneshot {
            let status = to_its_end(&s, &who, &given)?;
            ensure!(status.success(), "{} {} ended: {status}", s.path, s.args.join(" "));
            skyward_sys::kmsg(&format!("ran {} {}", s.path, s.args.join(" ")))?;
            continue;
        }
        if program.anew {
            end_left(&s)?;
        }
        // A property the program an earlier run of the recovery service
        // started has set says nothing of the one started now, which the
        // programs after it wait for.
        if let Some(property) = s.ready_property.as_ref().filter(|_| running(&s.path).is_none())
            && skyward_sys::prop(property).is_some_and(|value| !value.is_empty())
        {
            skyward_sys::set_prop(property, "")?;
        }
        let waiting = s.clone();
        let name = Path::new(&s.path).file_name().map_or_else(|| s.path.clone(), |n| n.to_string_lossy().into_owned());
        thread::Builder::new().name(name).spawn(move || supervise(&s, &who, &given)).context("start a supervising thread")?;
        ready(&waiting)?;
    }
    Ok(())
}

/// Run `program` once to its end and give how it ended, as init runs a
/// program one of its scripts has it run before it goes on: however the
/// program ends, init goes on, and so may whoever asks here. It runs as
/// [`start`] runs one, with `libraries` as there.
pub fn once(program: &Started, libraries: Option<&Path>) -> Result<ExitStatus> {
    let (s, who, given) = readied(program, libraries, &accounts())?;
    to_its_end(&s, &who, &given)
}

/// The accounts the phone knows, each by its name.
fn accounts() -> HashMap<String, u32> {
    ueventd::accounts(&ueventd::PASSWD_FILES.iter().chain(&ueventd::GROUP_FILES).copied().collect::<Vec<_>>())
}

/// What `program` is started with, worked out before it is: the program
/// itself, which has to be there, with `libraries` after its own, who it
/// runs as and what init hands it.
fn readied(program: &Started, libraries: Option<&Path>, known: &HashMap<String, u32>) -> Result<(Service, Credentials, Given)> {
    let mut s = program.service.clone();
    if !Path::new(&s.path).is_file() {
        bail!("{} is missing; is its partition mounted?", s.path);
    }
    if let Some(dir) = libraries {
        s.libraries.push(dir.to_string_lossy().into_owned());
    }
    let who = Credentials::of(&s, known)?;
    let given = Given::of(program, known)?;
    Ok((s, who, given))
}

/// Start the program, wait until it ends and give how it ended.
fn to_its_end(s: &Service, who: &Credentials, given: &Given) -> Result<ExitStatus> {
    let handed = given.make()?;
    command(s, who, given, &handed).status().with_context(|| format!("run {}", s.path))
}

/// What init hands a program as it starts it besides what its line gives
/// it, worked out before the program is started: the sockets it makes for
/// it, each with the user and the group it is given to, and what it sets
/// in its environment.
#[derive(Debug, Clone)]
struct Given {
    sockets: Vec<(Socket, libc::uid_t, libc::gid_t)>,
    environment: Vec<(String, String)>,
}

impl Given {
    /// What `program` is handed, by the names the phone gives its
    /// accounts: a socket that names no user or no group is root's.
    fn of(program: &Started, known: &HashMap<String, u32>) -> Result<Self> {
        let path = &program.service.path;
        let id = |name: &Option<String>| match name {
            Some(name) => ueventd::account(known, name).with_context(|| format!("{path}: the phone has no account named {name}")),
            None => Ok(0),
        };
        let sockets = program.sockets.iter().map(|s| Ok((s.clone(), id(&s.user)?, id(&s.group)?))).collect::<Result<Vec<_>>>()?;
        Ok(Self { sockets, environment: program.environment.clone() })
    }

    /// Make each socket, as init makes them afresh every time it starts
    /// the program, and give each with the variable that names it to the
    /// program: the socket's name, with whatever in it is neither a letter
    /// nor a digit turned into an underscore.
    fn make(&self) -> Result<Vec<(String, OwnedFd)>> {
        let mut handed = Vec::new();
        for (socket, uid, gid) in &self.sockets {
            let name: String = socket.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            handed.push((format!("{SOCKET_VARIABLE}{name}"), make_socket(socket, *uid, *gid)?));
        }
        Ok(handed)
    }
}

/// Make the socket under [`SOCKETS`] as init makes it: in the place of one
/// of its name left there, bound to its name, given to its user and its
/// group with its mode, and listening only where its line says so, a
/// program otherwise listening on it itself.
fn make_socket(socket: &Socket, uid: libc::uid_t, gid: libc::gid_t) -> Result<OwnedFd> {
    let path = Path::new(SOCKETS).join(&socket.name);
    let kind = match socket.kind {
        SocketKind::Stream => libc::SOCK_STREAM,
        SocketKind::Dgram => libc::SOCK_DGRAM,
        SocketKind::Seqpacket => libc::SOCK_SEQPACKET,
    };
    let done = |r: libc::c_int, what: &str| match r {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()).with_context(|| format!("{what} the socket {}", path.display())),
    };
    let fd = unsafe { libc::socket(libc::AF_UNIX, kind | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error()).with_context(|| format!("make the socket {}", path.display()));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("take away the {} left there", path.display())),
    }
    if socket.passcred {
        let on: libc::c_int = 1;
        let size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        done(unsafe { libc::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PASSCRED, (&on as *const libc::c_int).cast(), size) }, "have each peer's credentials come over")?;
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let name = path.as_os_str().as_bytes();
    // The name ends in a zero byte, which the address is already full of.
    ensure!(name.len() < address.sun_path.len(), "{} is too long a name for a socket", path.display());
    for (to, from) in address.sun_path.iter_mut().zip(name) {
        *to = *from as libc::c_char;
    }
    let size = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    done(unsafe { libc::bind(fd.as_raw_fd(), (&address as *const libc::sockaddr_un).cast(), size) }, "bind")?;
    std::os::unix::fs::lchown(&path, Some(uid), Some(gid)).with_context(|| format!("give the socket {} to {uid}:{gid}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(socket.mode)).with_context(|| format!("give the socket {} the mode {:o}", path.display(), socket.mode))?;
    if socket.listen {
        done(unsafe { libc::listen(fd.as_raw_fd(), WAITING_CALLERS) }, "listen on")?;
    }
    Ok(fd)
}

/// End the program an earlier run of the recovery service left running,
/// and wait until the service it was ready with is gone from the service
/// directory, so that waiting for the one started next waits for that one.
fn end_left(s: &Service) -> Result<()> {
    let Some(pid) = running(&s.path) else { return Ok(()) };
    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } != 0 {
        let e = io::Error::last_os_error();
        // It ended between being found and being told to.
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(e).with_context(|| format!("end {}, pid {pid}, left by an earlier recovery service", s.path));
        }
    }
    wait_for(pid)?;
    let until = Instant::now() + READY_TIMEOUT;
    while s.ready_service.as_deref().is_some_and(registered) {
        ensure!(Instant::now() < until, "{}, pid {pid}, is ended and still registered {} seconds on", s.path, READY_TIMEOUT.as_secs());
        thread::sleep(READY_POLL);
    }
    skyward_sys::kmsg(&format!("{}: pid {pid}, left by an earlier recovery service, ended to be started anew", s.path))
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
        if let Some(name) = &s.ready_property
            && skyward_sys::prop(name).is_some_and(|v| !v.is_empty())
        {
            return Ok(());
        }
        // Either directory may itself be starting again, and until it
        // answers nothing can be looked up in it.
        if let Some(name) = &s.ready_service
            && registered(name)
        {
            return Ok(());
        }
        if let Some(name) = &s.ready_hardware_service
            && registered_hardware(name)
        {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("{} {waited_on} in {} seconds", s.path, READY_TIMEOUT.as_secs());
        }
        thread::sleep(READY_POLL);
    }
}

fn supervise(s: &Service, who: &Credentials, given: &Given) -> ! {
    loop {
        let started = Instant::now();
        let ended = match running(&s.path) {
            Some(pid) => wait_for(pid).map(|()| format!("pid {pid}, left by an earlier recovery service, exited")),
            None => run(s, who, given),
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
fn run(s: &Service, who: &Credentials, given: &Given) -> Result<String> {
    let handed = given.make()?;
    let mut child = command(s, who, given, &handed).spawn().with_context(|| format!("start {}", s.path))?;
    // The program has its sockets now, and as init does, the recovery
    // keeps none of them open itself.
    drop(handed);
    let status = child.wait().context("wait for it")?;
    Ok(format!("pid {} ended: {status}", child.id()))
}

/// The program as it is started: with what it is given, from the top of the
/// tree, reading and writing nothing of the recovery's, with the
/// environment init sets it and each of the sockets in `handed` under the
/// variable that names it, finding its libraries where it names them, and
/// as `who`. Where init sets it folders of libraries of its own, they are
/// searched after those it is given here.
fn command(s: &Service, who: &Credentials, given: &Given, handed: &[(String, OwnedFd)]) -> Command {
    let mut cmd = Command::new(&s.path);
    cmd.args(&s.args).current_dir("/").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut libraries = s.libraries.clone();
    for (name, value) in &given.environment {
        match name == LIBRARY_PATH {
            true => libraries.push(value.clone()),
            false => {
                cmd.env(name, value);
            }
        }
    }
    if !libraries.is_empty() {
        cmd.env(LIBRARY_PATH, libraries.join(":"));
    }
    for (variable, socket) in handed {
        cmd.env(variable, socket.as_raw_fd().to_string());
    }
    let who = who.clone();
    let sockets: Vec<RawFd> = handed.iter().map(|(_, socket)| socket.as_raw_fd()).collect();
    // Only system calls run between the fork and the exec, on what was
    // worked out before it. The sockets are open in the recovery so that
    // no other program it starts gets them, and in the program about to
    // run they have to stay open through the exec.
    unsafe {
        cmd.pre_exec(move || {
            for socket in &sockets {
                if libc::fcntl(*socket, libc::F_SETFD, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            who.take_on()
        })
    };
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
