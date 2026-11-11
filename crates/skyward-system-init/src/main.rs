//! The first program of a system Skyward installed beside the installed
//! one, as the init_boot image Skyward made for it starts it. Before the
//! system's own first program, it loads the drivers the installed system's
//! first stage loads, as that stage loads them, finds the system's root
//! where Skyward put it in the data partition, lays it out as a device,
//! mounts it, and hands over to the system's own first program there.
//!
//! Whatever stops it on the way brings up the recovery, with a request in
//! misc that says why: an error at once, a start that stops moving after a
//! while, and a phone restarted before the hand-over, through the request
//! it keeps in misc until then. The recovery partition of the slot holds
//! Skyward, and its ramdisk is unpacked over the one the kernel unpacked,
//! as the kernel unpacks it when the bootloader starts the recovery: its
//! own first program then takes over in this one's place, and the recovery
//! comes up whatever the bootloader makes of a restart. A start that stops
//! moving, and a hand-over that fails once that ramdisk is out of reach,
//! restart into the recovery instead; a bootloader that starts the slot
//! again leaves the request in misc to this program at the next start,
//! which brings the recovery up before anything else, as it does for a
//! request a system left there. Where the recovery comes up neither way,
//! the phone stays where it stopped, rather than start the same again and
//! again.
//!
//! Started as `map`, it only lays the root out and says where, for a
//! system that brings a first ramdisk of its own.

// What reads the request and the kernel's files serves the phone and the
// host tests.
#![cfg_attr(any(not(target_os = "android"), test), allow(dead_code))]
// On the phone the C library calls `main` itself: see it below.
#![cfg_attr(all(target_os = "android", not(test)), no_main)]

use anyhow::{Context, Result};

/// The longest reason a request in misc carries, which shares the
/// arguments' 768 bytes with the rest of the request.
const REASON_MAX: usize = 600;

/// `why` as a request's reason: on one line, and short enough to fit.
fn reason(why: &str) -> String {
    let line: String = why.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    let mut end = line.len().min(REASON_MAX);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

/// The modules /proc/modules lists as still loading: those whose own start
/// has not returned.
fn loading(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let fields: Vec<&str> = l.split_whitespace().collect();
            (fields.get(4) == Some(&"Loading")).then(|| fields[0].to_string())
        })
        .collect()
}

/// Why a start that stopped moving `during` a step was given up after
/// `secs` seconds, with the modules still `loading` then.
fn stalled(during: &str, secs: u64, loading: &[String]) -> String {
    match loading {
        [] => format!("the start stopped while {during}, and was given up after {secs} s"),
        names => format!("the start stopped while {during}, with {} still loading, and was given up after {secs} s", names.join(", ")),
    }
}

/// The device a uevent file describes: its name, the partition name it
/// carries, and its (major, minor).
fn uevent(text: &str) -> Result<(String, Option<String>, (u32, u32))> {
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(str::to_string);
    let number = |key: &str| -> Result<u32> { field(key).with_context(|| format!("no {key}"))?.parse().with_context(|| format!("{key} is no number")) };
    Ok((field("DEVNAME=").context("no DEVNAME=")?, field("PARTNAME="), (number("MAJOR=")?, number("MINOR=")?)))
}

/// The program as the C library starts it, with none of Rust's own start
/// before it: that asks the C library where the main thread's stack is,
/// which bionic reads from /proc, and nothing has mounted /proc yet when
/// the kernel starts the first program.
#[cfg(all(target_os = "android", not(test)))]
#[no_mangle]
unsafe extern "C" fn main(argc: libc::c_int, argv: *const *const libc::c_char) -> libc::c_int {
    use std::os::unix::ffi::OsStrExt;
    let started_as = match argc {
        0 => &[][..],
        // The C library hands over the arguments it was started with.
        _ => unsafe { std::ffi::CStr::from_ptr(*argv) }.to_bytes(),
    };
    match std::path::Path::new(std::ffi::OsStr::from_bytes(started_as)).file_name().and_then(|n| n.to_str()) {
        Some("map") => sys::map(),
        _ => sys::init(),
    }
}

#[cfg(not(target_os = "android"))]
fn main() {
    eprintln!("system-init runs only on the phone, as a system's first program");
    std::process::exit(1);
}

#[cfg(target_os = "android")]
mod sys {
    use super::{loading, reason, stalled, uevent};
    use anyhow::{bail, ensure, Context, Result};
    use skyward_block::area::HEADER;
    use skyward_block::{dm, BY_NAME};
    use skyward_bootimg::{BootImage, Ramdisk};
    use skyward_sys::bcb::{self, Bcb};
    use skyward_systems::boot::{Start, INIT, START};
    use std::convert::Infallible;
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// How long the storage gets to show its partitions once its drivers
    /// are in.
    const PARTITION_TIMEOUT: Duration = Duration::from_secs(15);
    /// How long the start gets to reach the system's own first program
    /// before it is given up; then how long misc gets to show, with the
    /// drivers the start had yet to load, and the restart to finish before
    /// the phone is restarted at once.
    const GIVE_UP: Duration = Duration::from_secs(60);
    const MISC_WAIT: Duration = Duration::from_secs(30);
    const RESTART_WAIT: Duration = Duration::from_secs(10);
    /// The root, laid out as a device, and where its node goes.
    const ROOT_DEVICE: &str = "skyward-root";
    const ROOT_NODE: &str = "/dev/skyward-root";
    const NEW_ROOT: &str = "/root";
    const MISC_NODE: &str = "/dev/block/misc";
    /// The device-mapper's control node, where the device-mapper code
    /// looks for it.
    const DEVICE_MAPPER: &str = "/dev/device-mapper";

    /// What the start is doing, for the watch to say where it stopped.
    static STEP: Mutex<String> = Mutex::new(String::new());
    /// Whether the request for the recovery, with its reason, is in misc.
    static ASKED: AtomicBool = AtomicBool::new(false);
    /// Whether the start was given up, after which it hands nothing over.
    static GIVEN_UP: AtomicBool = AtomicBool::new(false);
    /// The folders the start made for the kernel's filesystems and the
    /// root, which the ramdisks the kernel unpacked do not have.
    static MADE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    /// The start's /dev, held from when it is mounted, for [`log`] to find
    /// the kernel's log in once /dev is moved into the root or taken down.
    static DEV: OnceLock<File> = OnceLock::new();

    fn step(now: &str) {
        log(&format!("now {now}"));
        *STEP.lock().expect("nothing panics while it holds the step") = now.to_string();
    }

    fn c(path: &str) -> CString {
        CString::new(path).expect("no NUL in a path written here")
    }

    fn check(rc: libc::c_int, what: impl FnOnce() -> String) -> Result<()> {
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).with_context(what);
        }
        Ok(())
    }

    fn mount(source: &str, target: &str, kind: Option<&str>, flags: libc::c_ulong, data: Option<&str>) -> Result<()> {
        let kind = kind.map(c);
        let data = data.map(c);
        let rc = unsafe {
            libc::mount(
                c(source).as_ptr(),
                c(target).as_ptr(),
                kind.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
                flags,
                data.as_ref().map_or(std::ptr::null(), |d| d.as_ptr() as *const libc::c_void),
            )
        };
        check(rc, || format!("mount {source} on {target}"))
    }

    /// Unmount what is mounted on `target`, if anything is.
    fn unmount(target: &str, flags: libc::c_int) -> Result<()> {
        if unsafe { libc::umount2(c(target).as_ptr(), flags) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINVAL) {
                return Err(e).with_context(|| format!("unmount {target}"));
            }
        }
        Ok(())
    }

    fn node(path: &str, kind: libc::mode_t, mode: libc::mode_t, dev: (u32, u32)) -> Result<()> {
        if Path::new(path).exists() {
            return Ok(());
        }
        if unsafe { libc::mknod(c(path).as_ptr(), kind | mode, libc::makedev(dev.0, dev.1)) } != 0 {
            let e = std::io::Error::last_os_error();
            // The watch may have made it in the meantime.
            if e.raw_os_error() == Some(libc::EEXIST) {
                return Ok(());
            }
            return Err(e).with_context(|| format!("make {path}"));
        }
        Ok(())
    }

    /// A line in the kernel's log, from when the start has its /dev. Each
    /// line opens the log afresh: the kernel lets one opening write only so
    /// many lines in a while, and drops the rest.
    fn log(line: &str) {
        let Some(dev) = DEV.get() else {
            return;
        };
        let fd = unsafe { libc::openat(dev.as_raw_fd(), c"kmsg".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return;
        }
        let kmsg = unsafe { File::from_raw_fd(fd) };
        let _ = (&kmsg).write_all(format!("<3>skyward system-init: {line}\n").as_bytes());
    }

    /// The kernel's own filesystems, and the nodes the rest needs, in a
    /// /dev of its own: the kernel these systems start with has no
    /// devtmpfs to make them.
    fn basics() -> Result<()> {
        for dir in ["/proc", "/sys", "/dev", NEW_ROOT] {
            if !Path::new(dir).exists() {
                fs::create_dir(dir).with_context(|| format!("make {dir}"))?;
                MADE.lock().expect("nothing panics while it holds the folders made").push(dir);
            }
        }
        mount("proc", "/proc", Some("proc"), 0, None)?;
        mount("sysfs", "/sys", Some("sysfs"), 0, None)?;
        mount("tmpfs", "/dev", Some("tmpfs"), libc::MS_NOSUID, Some("mode=0755"))?;
        fs::create_dir_all(BY_NAME).with_context(|| format!("make {BY_NAME}"))?;
        node("/dev/console", libc::S_IFCHR, 0o600, (5, 1))?;
        node("/dev/null", libc::S_IFCHR, 0o666, (1, 3))?;
        node("/dev/kmsg", libc::S_IFCHR, 0o644, (1, 11))?;
        // After the streams, so that it takes none of their places.
        streams()?;
        DEV.set(File::open("/dev").context("open /dev")?).expect("the start mounts /dev once");
        Ok(())
    }

    /// The standard streams, which the kernel gives the first program only
    /// where the ramdisk has a console: nothing to read, and what is
    /// written goes to the kernel's log. Otherwise the files opened next
    /// would take their places, and a message for stderr would go into
    /// them.
    fn streams() -> Result<()> {
        use std::os::unix::io::IntoRawFd;
        let null = OpenOptions::new().read(true).write(true).open("/dev/null").context("open /dev/null")?.into_raw_fd();
        let kmsg = OpenOptions::new().write(true).open("/dev/kmsg").context("open /dev/kmsg")?.into_raw_fd();
        for (fd, to) in [(null, 0), (kmsg, 1), (kmsg, 2)] {
            if fd != to && unsafe { libc::dup2(fd, to) } == -1 {
                return Err(std::io::Error::last_os_error()).with_context(|| format!("make fd {to} a standard stream"));
            }
        }
        for fd in [null, kmsg] {
            if fd > 2 {
                unsafe { libc::close(fd) };
            }
        }
        Ok(())
    }

    /// Load the vendor ramdisk's modules as the installed system's first
    /// stage loads them, with the settings the kernel's command line gives
    /// them, leaving out those `skip` names. Where one does not load, the
    /// rest still do, and the error names those not loaded.
    fn modules(skip: &[String]) -> Result<()> {
        let cmdline = fs::read_to_string("/proc/cmdline").context("read /proc/cmdline")?;
        let bootconfig = match fs::read_to_string("/proc/bootconfig") {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).context("read /proc/bootconfig"),
        };
        let loaded = skyward_modules::modprobe::first_stage(&cmdline, &bootconfig, skip).context("load the drivers")?;
        log(&format!("{} modules loaded from {}", loaded.count, loaded.dir.display()));
        let Some((_, first)) = loaded.failed.first() else {
            return Ok(());
        };
        for (_, e) in &loaded.failed {
            log(&format!("{e:#}"));
        }
        let names: Vec<&str> = loaded.failed.iter().map(|(n, _)| n.as_str()).collect();
        bail!("{} drivers were not loaded ({}): {first:#}", names.len(), names.join(", "))
    }

    /// The device-mapper's control node.
    fn device_mapper() -> Result<()> {
        let text = fs::read_to_string("/sys/class/misc/device-mapper/dev").context("read the device-mapper's numbers")?;
        let (major, minor) = text.trim().split_once(':').context("the device-mapper's numbers")?;
        node(DEVICE_MAPPER, libc::S_IFCHR, 0o600, (major.parse()?, minor.parse()?))
    }

    /// Wait up to `within` for the partition named `name` to show, and make
    /// its node at `at`; returns its (major, minor).
    fn partition(name: &str, at: &str, within: Duration) -> Result<(u32, u32)> {
        let start = Instant::now();
        loop {
            for entry in fs::read_dir("/sys/class/block").context("read /sys/class/block")? {
                let path = entry?.path().join("uevent");
                let (_, partname, dev) = uevent(&fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?)?;
                if partname.as_deref() == Some(name) {
                    node(at, libc::S_IFBLK, 0o600, dev)?;
                    return Ok(dev);
                }
            }
            if start.elapsed() > within {
                bail!("no partition {name} showed within {within:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The node of the partition `name` among the links [`BY_NAME`] holds,
    /// where the code shared with the recovery looks for it; returns its
    /// (major, minor).
    fn by_name(name: &str) -> Result<(u32, u32)> {
        partition(name, &format!("{BY_NAME}/{name}"), PARTITION_TIMEOUT)
    }

    /// Lay the root `start` names out as a device, once its area is found
    /// still to start with its header; returns the device's node.
    fn root(start: &Start) -> Result<&'static str> {
        let at = format!("/dev/block/{}", start.root.partition);
        let device = partition(&start.root.partition, &at, PARTITION_TIMEOUT)?;
        device_mapper()?;
        let f = File::open(&at).with_context(|| format!("open {at}"))?;
        let mut header = vec![0u8; HEADER as usize];
        f.read_exact_at(&mut header, start.root.extents[0].0).with_context(|| format!("read {at}"))?;
        start.root.check(&header)?;
        let targets = start.root.targets(HEADER, start.root.size - HEADER, device)?;
        let dev = dm::activate(ROOT_DEVICE, &targets, false)?;
        node(ROOT_NODE, libc::S_IFBLK, 0o600, dev)?;
        Ok(ROOT_NODE)
    }

    fn read_start() -> Result<Start> {
        let path = Path::new("/").join(START);
        Start::parse(&fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?)
    }

    /// Whether `program` is there to start once the root mounted at
    /// [`NEW_ROOT`] is the root: a file someone may run, looked up from that
    /// root, where the links on the way point there too, by a child that
    /// goes into it.
    fn startable(program: &str) -> Result<bool> {
        let (root, path) = (c(NEW_ROOT), c(program));
        match unsafe { libc::fork() } {
            -1 => Err(std::io::Error::last_os_error()).context("start a child to look into the root"),
            0 => unsafe {
                let mut found: libc::stat = std::mem::zeroed();
                let runs = libc::chroot(root.as_ptr()) == 0 && libc::stat(path.as_ptr(), &mut found) == 0 && found.st_mode & libc::S_IFMT == libc::S_IFREG && found.st_mode & 0o111 != 0;
                libc::_exit(if runs { 0 } else { 1 })
            },
            child => {
                let mut status = 0;
                if unsafe { libc::waitpid(child, &mut status, 0) } != child {
                    return Err(std::io::Error::last_os_error()).context("wait for the child that looks into the root");
                }
                Ok(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0)
            }
        }
    }

    /// Become `program`, with no arguments past its name, as the kernel
    /// starts the first program.
    fn exec(program: &str) -> Result<Infallible> {
        let path = c(program);
        let argv = [path.as_ptr(), std::ptr::null()];
        unsafe { libc::execv(path.as_ptr(), argv.as_ptr()) };
        Err(std::io::Error::last_os_error()).with_context(|| format!("start {program}"))
    }

    /// Go over to the root mounted at [`NEW_ROOT`], the kernel's
    /// filesystems taken along, and become the system's own first program.
    fn hand_over(init: &str) -> Result<Infallible> {
        for dir in ["/dev", "/proc", "/sys"] {
            let to = format!("{NEW_ROOT}{dir}");
            if Path::new(&to).is_dir() {
                mount(dir, &to, None, libc::MS_MOVE, None)?;
            } else {
                check(unsafe { libc::umount2(c(dir).as_ptr(), libc::MNT_DETACH) }, || format!("unmount {dir}"))?;
            }
        }
        check(unsafe { libc::chdir(c(NEW_ROOT).as_ptr()) }, || format!("go into {NEW_ROOT}"))?;
        mount(".", "/", None, libc::MS_MOVE, None)?;
        check(unsafe { libc::chroot(c(".").as_ptr()) }, || "make the new root the root".into())?;
        check(unsafe { libc::chdir(c("/").as_ptr()) }, || "go to the new root".into())?;
        exec(init)
    }

    /// The request for the recovery, saying `why` the start stopped.
    fn request(why: &str) -> Bcb {
        Bcb { command: "boot-recovery".into(), args: vec!["--system_stopped".into(), format!("--reason={}", reason(why))] }
    }

    /// Leave the request for the recovery in misc, saying why the start
    /// stopped: once, by whichever of the start and the watch gets there
    /// first.
    fn report(why: &str) {
        log(why);
        if ASKED.swap(true, Ordering::SeqCst) {
            return;
        }
        if !Path::new(MISC_NODE).exists() {
            log("no misc partition showed, so the recovery is not told why the start stopped");
            return;
        }
        if let Err(e) = bcb::write(Path::new(MISC_NODE), &request(why)) {
            log(&format!("why the start stopped was not written to misc: {e:#}"));
        }
    }

    /// Whether misc asks the bootloader for the recovery.
    fn misc_asks() -> bool {
        Path::new(MISC_NODE).exists() && bcb::read(Path::new(MISC_NODE)).is_ok_and(|held| held.asks_for_recovery())
    }

    /// Take down what the start set up, for the recovery's own first
    /// program to set it up as it does when the bootloader starts it: the
    /// root unmounted and its device removed, the kernel's filesystems
    /// unmounted, the folders made for them taken away, and what the start
    /// read taken out of the ramdisk.
    fn take_down() -> Result<()> {
        unmount(NEW_ROOT, 0)?;
        if Path::new(DEVICE_MAPPER).exists() && dm::status(ROOT_DEVICE)?.is_some() {
            dm::remove(ROOT_DEVICE)?;
        }
        for dir in ["/dev", "/sys", "/proc"] {
            unmount(dir, libc::MNT_DETACH)?;
        }
        for dir in MADE.lock().expect("nothing panics while it holds the folders made").drain(..) {
            fs::remove_dir(dir).with_context(|| format!("take away {dir}"))?;
        }
        let start = Path::new("/").join(START);
        fs::remove_file(&start).with_context(|| format!("take away {}", start.display()))
    }

    /// Bring the recovery up in this program's place: Skyward's ramdisk,
    /// from the recovery partition of the slot the phone started from,
    /// unpacked over the one the kernel unpacked, as the kernel unpacks it
    /// when the bootloader starts the recovery, and its own first program
    /// started, once what the start set up is taken down.
    fn recovery() -> Result<Infallible> {
        let name = format!("recovery{}", skyward_block::slot_suffix()?.unwrap_or_default());
        by_name(&name)?;
        let at = format!("{BY_NAME}/{name}");
        let image = BootImage::parse(&fs::read(&at).with_context(|| format!("read {at}"))?).with_context(|| format!("read the image in {at}"))?;
        let ramdisk = Ramdisk::decode(&image.ramdisk).with_context(|| format!("read the ramdisk in {at}"))?;
        take_down()?;
        ramdisk.unpack(Path::new("/")).with_context(|| format!("unpack the ramdisk in {at}"))?;
        log(&format!("the ramdisk in {at} is unpacked, and its first program takes over"));
        exec(&format!("/{INIT}"))
    }

    /// What the phone does where the recovery does not come up in this
    /// program's place.
    enum Otherwise {
        /// Restart into it, where the request for it is in misc.
        Restart,
        /// Stay here: misc asked for the recovery at this start already,
        /// and the bootloader started the slot over such a restart.
        Stay,
    }

    /// Bring the recovery up in this program's place, once, by whichever of
    /// the start and the watch gets there first; where it does not come
    /// up, do `otherwise`.
    fn to_recovery(otherwise: Otherwise) -> ! {
        if !GIVEN_UP.swap(true, Ordering::SeqCst) {
            step("bringing the recovery up");
            // Misc is read before the start's nodes are taken down.
            let asked = misc_asks();
            let Err(e) = recovery();
            log(&format!("the recovery did not come up here: {e:#}"));
            match otherwise {
                Otherwise::Restart => restart_or_stay(asked),
                Otherwise::Stay => log("a restart comes back here, so the phone stays"),
            }
        }
        stay()
    }

    /// Restart into the recovery where misc asks for it, `asked`, so that
    /// a bootloader that starts the slot again brings this program to it
    /// at the next start; without the request, the phone stays here.
    fn restart_or_stay(asked: bool) {
        match asked {
            true => restart(),
            false => log("misc holds no request for the recovery, so the phone stays here"),
        }
    }

    /// The first program must not end; the kernel would stop.
    fn stay() -> ! {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    /// Restart into the recovery: at once, the drivers left as they are,
    /// where their own stopping holds the restart up.
    fn restart() {
        step("restarting into the recovery");
        unsafe { libc::sync() };
        std::thread::spawn(|| {
            let recovery = c("recovery");
            unsafe {
                libc::syscall(libc::SYS_reboot, libc::LINUX_REBOOT_MAGIC1, libc::LINUX_REBOOT_MAGIC2, libc::LINUX_REBOOT_CMD_RESTART2, recovery.as_ptr());
            }
        });
        std::thread::sleep(RESTART_WAIT);
        log("the restart did not finish, so the phone restarts at once");
        if let Err(e) = fs::write("/proc/sysrq-trigger", "b") {
            log(&format!("restart at once: {e}"));
        }
    }

    /// Give the start up where it has not reached the system's own first
    /// program in time: leave the request for the recovery in misc, saying
    /// where it stopped and which modules were still loading, and restart
    /// into the recovery, as the start that stopped may hold this program
    /// in the kernel. Where misc has not shown, the drivers the start had
    /// yet to load are loaded first, leaving out those still loading, so
    /// that the storage's come up all the same.
    fn watch() {
        std::thread::sleep(GIVE_UP);
        let during = STEP.lock().expect("nothing panics while it holds the step").clone();
        let still = match fs::read_to_string("/proc/modules") {
            Ok(text) => loading(&text),
            Err(e) => {
                log(&format!("read /proc/modules: {e}"));
                Vec::new()
            }
        };
        if !Path::new(MISC_NODE).exists() {
            let skip = still.clone();
            std::thread::spawn(move || {
                if let Err(e) = modules(&skip) {
                    log(&format!("{e:#}"));
                }
            });
            if let Err(e) = partition("misc", MISC_NODE, MISC_WAIT) {
                log(&format!("{e:#}"));
            }
        }
        report(&stalled(&during, GIVE_UP.as_secs(), &still));
        if !GIVEN_UP.swap(true, Ordering::SeqCst) {
            restart_or_stay(misc_asks());
        }
    }

    /// The start, from the kernel's filesystems to the hand-over. The
    /// drivers come first and misc is looked for after them whatever they
    /// came to, so that whatever stops the start finds the recovery. A
    /// request in misc for the recovery, which a system or a start before
    /// this one left and the bootloader started the slot over, brings it up
    /// before anything is started.
    fn start() -> Result<Infallible> {
        step("mounting the kernel's filesystems");
        basics()?;
        step("loading the drivers");
        let drivers = modules(&[]);
        step("waiting for the misc partition");
        let found = partition("misc", MISC_NODE, PARTITION_TIMEOUT);
        drivers?;
        found?;
        step("reading misc");
        let held = bcb::read(Path::new(MISC_NODE))?;
        if held.asks_for_recovery() {
            log("misc asks for the recovery, and the bootloader started the slot");
            ASKED.store(true, Ordering::SeqCst);
            to_recovery(Otherwise::Stay);
        }
        step("reading what to start");
        let start = read_start()?;
        log(&format!("starting {}", start.id));
        keep_way_back(&start)?;
        boot(&start, &held)
    }

    /// Keep the request for the recovery in misc while the start goes on,
    /// so that a phone restarted on the way brings the recovery up at its
    /// next start; what misc held before is put back at the hand-over.
    fn keep_way_back(start: &Start) -> Result<()> {
        bcb::write(Path::new(MISC_NODE), &request(&format!("the phone restarted while {} was starting", start.id)))
    }

    /// Lay the system's root out, mount it, find the system's own first
    /// program in it, put back what misc held before the start, and hand
    /// over to that program. Once the hand-over is under way, the ramdisk
    /// the recovery comes up from in place is out of reach, and a hand-over
    /// that fails restarts into the recovery.
    fn boot(start: &Start, before: &Bcb) -> Result<Infallible> {
        step("laying the root out");
        let node = root(start)?;
        step("mounting the root");
        mount(node, NEW_ROOT, Some(&start.filesystem), 0, None)?;
        step(&format!("finding {} in the root", start.init));
        ensure!(startable(&start.init)?, "the root holds no program {} to start", start.init);
        step("putting back what misc held");
        bcb::write(Path::new(MISC_NODE), before)?;
        if GIVEN_UP.load(Ordering::SeqCst) {
            bail!("the start was given up on the way");
        }
        step(&format!("handing over to {}", start.init));
        let Err(e) = hand_over(&start.init);
        report(&format!("{e:#}"));
        if !GIVEN_UP.swap(true, Ordering::SeqCst) {
            restart_or_stay(misc_asks());
        }
        stay()
    }

    /// Start the system, or bring the recovery up to say why it stopped.
    pub fn init() -> ! {
        std::thread::Builder::new().name("watch".into()).spawn(watch).expect("a thread to watch the start");
        let Err(e) = start();
        step("telling the recovery why the start stopped");
        if !Path::new(MISC_NODE).exists() {
            if let Err(e) = partition("misc", MISC_NODE, PARTITION_TIMEOUT) {
                log(&format!("{e:#}"));
            }
        }
        report(&format!("{e:#}"));
        to_recovery(Otherwise::Restart)
    }

    /// Lay the root out for a system's own first ramdisk and print its
    /// node.
    pub fn map() -> ! {
        let done = (|| -> Result<&'static str> {
            let start = read_start()?;
            fs::create_dir_all(BY_NAME).with_context(|| format!("make {BY_NAME}"))?;
            root(&start)
        })();
        match done {
            Ok(node) => {
                let _ = std::io::stdout().write_all(Path::new(node).as_os_str().as_bytes());
                let _ = std::io::stdout().write_all(b"\n");
                std::process::exit(0)
            }
            Err(e) => {
                eprintln!("map: {e:#}");
                std::process::exit(1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_goes_into_misc_on_one_short_line() {
        assert_eq!(reason("could not\nmount"), "could not mount");
        let long = "长".repeat(400);
        let r = reason(&long);
        assert!(r.len() <= REASON_MAX && long.starts_with(&r), "cut at a character's edge");
    }

    #[test]
    fn the_modules_still_loading_are_those_whose_start_has_not_returned() {
        let text = "hwid 16384 0 - Loading 0x0000000000000000 (O)\nqcom_wdt_core 32768 1 gh_virt_wdt, Live 0x0000000000000000 (O)\nsmem 20480 2 - Live 0x0000000000000000\n";
        assert_eq!(loading(text), ["hwid"]);
        assert!(loading("").is_empty());
    }

    #[test]
    fn a_start_given_up_says_where_it_stopped() {
        assert_eq!(stalled("loading the drivers", 60, &["hwid".into()]), "the start stopped while loading the drivers, with hwid still loading, and was given up after 60 s");
        assert_eq!(stalled("mounting the root", 60, &[]), "the start stopped while mounting the root, and was given up after 60 s");
    }

    #[test]
    fn a_partitions_uevent_names_it_and_its_numbers() {
        let text = "MAJOR=259\nMINOR=14\nDEVNAME=sda30\nDEVTYPE=partition\nDISKSEQ=33\nPARTN=30\nPARTNAME=userdata\n";
        assert_eq!(uevent(text).unwrap(), ("sda30".into(), Some("userdata".into()), (259, 14)));
        assert_eq!(uevent("MAJOR=8\nMINOR=0\nDEVNAME=sda\nDEVTYPE=disk\n").unwrap().1, None, "a whole disk has no partition name");
        assert!(uevent("DEVNAME=sda\n").is_err());
    }
}
