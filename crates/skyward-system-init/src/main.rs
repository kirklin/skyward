//! The first program of a system Skyward installed beside the installed
//! one, as the init_boot image Skyward made for it starts it. Before the
//! system's own first program, it loads the drivers the installed system's
//! first stage loads, as that stage loads them, finds the system's root
//! where Skyward put it in the data partition, lays it out as a device,
//! mounts it, and hands over to the system's own first program there.
//! Whatever stops it on the way says why in the kernel's log, and the phone
//! stays where it stopped.
//!
//! Started as `map`, it only lays the root out and says where, for a
//! system that brings a first ramdisk of its own.

// What reads the kernel's files serves the phone and the host tests.
#![cfg_attr(any(not(target_os = "android"), test), allow(dead_code))]
// On the phone the C library calls `main` itself: see it below.
#![cfg_attr(all(target_os = "android", not(test)), no_main)]

use anyhow::{Context, Result};

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
    use super::uevent;
    use anyhow::{bail, ensure, Context, Result};
    use skyward_block::area::HEADER;
    use skyward_block::{dm, BY_NAME};
    use skyward_systems::boot::{Start, START};
    use std::convert::Infallible;
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::path::Path;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    /// How long the storage gets to show its partitions once its drivers
    /// are in.
    const PARTITION_TIMEOUT: Duration = Duration::from_secs(15);
    /// The root, laid out as a device, and where its node goes.
    const ROOT_DEVICE: &str = "skyward-root";
    const ROOT_NODE: &str = "/dev/skyward-root";
    const NEW_ROOT: &str = "/root";
    /// The device-mapper's control node, where the device-mapper code
    /// looks for it.
    const DEVICE_MAPPER: &str = "/dev/device-mapper";

    /// The start's /dev, held from when it is mounted, for [`log`] to find
    /// the kernel's log in once /dev is moved into the root.
    static DEV: OnceLock<File> = OnceLock::new();

    fn step(now: &str) {
        log(&format!("now {now}"));
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

    fn node(path: &str, kind: libc::mode_t, mode: libc::mode_t, dev: (u32, u32)) -> Result<()> {
        if Path::new(path).exists() {
            return Ok(());
        }
        check(unsafe { libc::mknod(c(path).as_ptr(), kind | mode, libc::makedev(dev.0, dev.1)) }, || format!("make {path}"))
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
            fs::create_dir_all(dir).with_context(|| format!("make {dir}"))?;
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
    /// them. Where one does not load, the rest still do, and the error
    /// names those not loaded.
    fn modules() -> Result<()> {
        let cmdline = fs::read_to_string("/proc/cmdline").context("read /proc/cmdline")?;
        let bootconfig = match fs::read_to_string("/proc/bootconfig") {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).context("read /proc/bootconfig"),
        };
        let loaded = skyward_modules::modprobe::first_stage(&cmdline, &bootconfig, &[]).context("load the drivers")?;
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

    /// The first program must not end; the kernel would stop.
    fn stay() -> ! {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    /// The start, from the kernel's filesystems to the hand-over.
    fn start() -> Result<Infallible> {
        step("mounting the kernel's filesystems");
        basics()?;
        step("loading the drivers");
        modules()?;
        step("reading what to start");
        let start = read_start()?;
        log(&format!("starting {}", start.id));
        boot(&start)
    }

    /// Lay the system's root out, mount it, find the system's own first
    /// program in it, and hand over to that program.
    fn boot(start: &Start) -> Result<Infallible> {
        step("laying the root out");
        let node = root(start)?;
        step("mounting the root");
        mount(node, NEW_ROOT, Some(&start.filesystem), 0, None)?;
        step(&format!("finding {} in the root", start.init));
        ensure!(startable(&start.init)?, "the root holds no program {} to start", start.init);
        step(&format!("handing over to {}", start.init));
        hand_over(&start.init)
    }

    /// Start the system; where the start stops, say why in the kernel's
    /// log and stay.
    pub fn init() -> ! {
        let Err(e) = start();
        log(&format!("the start stopped: {e:#}"));
        stay()
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
    fn a_partitions_uevent_names_it_and_its_numbers() {
        let text = "MAJOR=259\nMINOR=14\nDEVNAME=sda30\nDEVTYPE=partition\nDISKSEQ=33\nPARTN=30\nPARTNAME=userdata\n";
        assert_eq!(uevent(text).unwrap(), ("sda30".into(), Some("userdata".into()), (259, 14)));
        assert_eq!(uevent("MAJOR=8\nMINOR=0\nDEVNAME=sda\nDEVTYPE=disk\n").unwrap().1, None, "a whole disk has no partition name");
        assert!(uevent("DEVNAME=sda\n").is_err());
    }
}
