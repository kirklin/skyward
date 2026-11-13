use anyhow::{bail, ensure, Context, Result};
use procfs::process::{FDTarget, MMapPath, MountInfos, Process};
use skyward_config::Service;
use skyward_sys::{library_mounts::{self, LibraryMounts}, ueventd};
use skyward_vab::Mounted;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
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
        crate::provide_recovery_libraries(&s.recovery_libraries)?;
        let mut s = s.clone();
        if let Some(dir) = libraries {
            s.libraries.push(dir.to_string_lossy().into_owned());
        }
        let who = Credentials::of(&s, &known)?;
        let mounts = Arc::new(LibraryMounts::prepare(s.library_overrides.iter().map(|m| (&m.source, &m.target)))
            .with_context(|| format!("prepare library overrides for {}", s.path))?);
        if s.oneshot {
            let status = command(&s, &who, &mounts).status().with_context(|| format!("run {}", s.path))?;
            ensure!(status.success(), "{} {} ended: {status}", s.path, s.args.join(" "));
            skyward_sys::kmsg(&format!("ran {} {}", s.path, s.args.join(" ")))?;
            continue;
        }
        // An old instance must have the same library view before it can
        // satisfy readiness or be adopted by the supervising thread.
        running_instance(&s, &mounts)?;
        let waiting = s.clone();
        let waiting_mounts = Arc::clone(&mounts);
        let name = Path::new(&s.path).file_name().map_or_else(|| s.path.clone(), |n| n.to_string_lossy().into_owned());
        thread::Builder::new().name(name).spawn(move || supervise(&s, &who, &mounts)).context("start a supervising thread")?;
        ready(&waiting, &waiting_mounts)?;
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
fn ready(s: &Service, mounts: &LibraryMounts) -> Result<()> {
    let waited_on = match (&s.ready_property, &s.ready_service, &s.ready_hardware_service) {
        (Some(p), _, _) => format!("set no {p}"),
        (_, Some(n), _) => format!("registered no {n}"),
        (_, _, Some(n)) => format!("registered no {n}"),
        _ => return Ok(()),
    };
    let until = Instant::now() + READY_TIMEOUT;
    loop {
        // A registered name alone may belong to an older process. Verify
        // the executable and its per-process library view as well.
        let running = mounts.is_empty() || running_instance(s, mounts)?.is_some();
        if let Some(name) = &s.ready_property {
            if running && skyward_sys::prop(name).is_some_and(|v| !v.is_empty()) {
                return Ok(());
            }
        }
        // Either directory may itself be starting again, and until it
        // answers nothing can be looked up in it.
        if let Some(name) = &s.ready_service {
            if running && registered(name) {
                return Ok(());
            }
        }
        if let Some(name) = &s.ready_hardware_service {
            if running && registered_hardware(name) {
                return Ok(());
            }
        }
        if Instant::now() >= until {
            bail!("{} {waited_on} in {} seconds", s.path, READY_TIMEOUT.as_secs());
        }
        thread::sleep(READY_POLL);
    }
}

fn supervise(s: &Service, who: &Credentials, mounts: &Arc<LibraryMounts>) -> ! {
    loop {
        let started = Instant::now();
        let ended = match running_instance(s, mounts) {
            Ok(Some(pid)) => wait_for(pid).map(|()| format!("pid {pid}, left by an earlier recovery service, exited")),
            Ok(None) => run(s, who, mounts),
            Err(e) => Err(e),
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
fn run(s: &Service, who: &Credentials, mounts: &Arc<LibraryMounts>) -> Result<String> {
    let mut child = command(s, who, mounts).spawn().with_context(|| format!("start {}", s.path))?;
    let status = child.wait().context("wait for it")?;
    Ok(format!("pid {} ended: {status}", child.id()))
}

/// The program as it is started: with what it is given, from the top of the
/// tree, reading and writing nothing of the recovery's, finding its
/// libraries where it names them, and as `who`.
fn command(s: &Service, who: &Credentials, mounts: &Arc<LibraryMounts>) -> Command {
    let mut cmd = Command::new(&s.path);
    cmd.args(&s.args).current_dir("/").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if !s.libraries.is_empty() {
        cmd.env("LD_LIBRARY_PATH", s.libraries.join(":"));
    }
    let who = who.clone();
    let memory_data = s.memory_data;
    let mounts = Arc::clone(mounts);
    // Only system calls run between the fork and the exec, on what was
    // worked out before it.
    unsafe {
        cmd.pre_exec(move || {
            if memory_data || !mounts.is_empty() {
                library_mounts::private_namespace()?;
            }
            if memory_data {
                private_memory_data(who.uid, who.gid)?;
            }
            mounts.apply()?;
            who.take_on()
        })
    };
    cmd
}

/// A calibration daemon uses fixed paths below /data while that partition
/// is still closed. Give only the child a bounded RAM data tree. Private
/// mount propagation keeps a later data mount in the recovery from
/// covering this tree. Only system calls and fixed strings run between
/// fork and exec, after the shared private namespace has been established
/// and before the child takes on its configured account.
fn private_memory_data(uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    let done = |result| if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) };
    let c = |bytes: &'static [u8]| bytes.as_ptr().cast::<libc::c_char>();
    done(unsafe {
        libc::mount(
            c(b"skyward-service-data\0"),
            c(b"/data\0"),
            c(b"tmpfs\0"),
            libc::MS_NODEV | libc::MS_NOSUID | libc::MS_NOEXEC,
            c(b"size=16m,nr_inodes=512,mode=0755\0").cast(),
        )
    })?;
    done(unsafe { libc::mkdir(c(b"/data/vendor\0"), 0o755) })?;
    // Recovery runs with umask 0077. The daemon must still be able to
    // traverse the root-owned parent after it gives up root.
    done(unsafe { libc::chmod(c(b"/data/vendor\0"), 0o755) })?;
    for path in [b"/data/vendor/wifi\0".as_slice(), b"/data/vendor/wifi/sockets\0", b"/data/vendor/wlan_logs\0"] {
        done(unsafe { libc::mkdir(c(path), 0o770) })?;
        done(unsafe { libc::chown(c(path), uid, gid) })?;
        done(unsafe { libc::chmod(c(path), 0o770) })?;
    }
    Ok(())
}

/// Lock in memory every file the running programs have mapped or open on
/// the `mounted` partitions, so they keep working once those partitions
/// are written over. Return mount points whose devices must stay mapped,
/// including mounts retained in a service's private namespace even when
/// no file on them is open. Locked mappings stay for the recovery's life.
pub fn pin(services: &[Service], mounted: &[Mounted]) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    let mut devices = BTreeSet::new();
    let ours = fs::read_link("/proc/self/ns/mnt").context("read the recovery's mount namespace")?;
    // A program run once has ended, and one between restarts has nothing in
    // memory to keep either.
    for s in services.iter().filter(|s| !s.oneshot) {
        let mounts = LibraryMounts::prepare(s.library_overrides.iter().map(|m| (&m.source, &m.target)))
            .with_context(|| format!("prepare library overrides for {}", s.path))?;
        let Some(pid) = running_instance(s, &mounts)? else { continue };
        let process = Process::new(pid as i32).with_context(|| format!("open process {pid}"))?;
        for map in process.maps().with_context(|| format!("read the mappings of {}", s.path))? {
            if let MMapPath::Path(p) = map.pathname {
                let identity = (libc::makedev(map.dev.0 as _, map.dev.1 as _), map.inode);
                files.insert((pid, p, Some(identity)));
            }
        }
        for fd in process.fd().with_context(|| format!("read the files {} has open", s.path))? {
            if let FDTarget::Path(p) = fd.with_context(|| format!("read a file {} has open", s.path))?.target {
                files.insert((pid, p, None));
            }
        }
        devices.extend(kept_mounted(
            fs::read_link(format!("/proc/{pid}/ns/mnt")),
            &ours,
            || process.mountinfo().with_context(|| format!("read the mount table of {} (pid {pid})", s.path)),
        ).with_context(|| format!("inspect mounts retained by {} (pid {pid})", s.path))?);
    }
    let mut busy: Vec<PathBuf> = Vec::new();
    let mut locked = BTreeSet::new();
    for (pid, logical, mapped_identity) in files {
        let Some(m) = mounted.iter().find(|m| logical.starts_with(&m.mount_point)) else { continue };
        let path = library_mounts::process_path(pid, &logical)?;
        let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path).with_context(|| format!("open {} in pid {pid}'s mount view", logical.display()))?;
        let meta = file.metadata().with_context(|| format!("stat {} in pid {pid}'s mount view", logical.display()))?;
        if !meta.is_file() {
            continue;
        }
        let identity = (meta.dev(), meta.ino());
        ensure!(mapped_identity.is_none_or(|expected| expected == identity), "{} in pid {pid}'s mount view no longer names its mapped inode", logical.display());
        if locked.insert(identity) {
            lock(&file, &logical)?;
            let _ = skyward_sys::kmsg(&format!("kept {} from pid {pid}'s mount view in memory", logical.display()));
        }
        if !busy.contains(&m.mount_point) {
            busy.push(m.mount_point.clone());
        }
    }
    let partitions: Vec<(u64, PathBuf)> = mounted
        .iter()
        .map(|m| {
            let device = fs::metadata(&m.device).with_context(|| format!("stat {}", m.device.display()))?;
            Ok((device.rdev(), m.mount_point.clone()))
        })
        .collect::<Result<_>>()?;
    for kept in retained_mounts(&devices, &partitions) {
        if !busy.contains(&kept) {
            let _ = skyward_sys::kmsg(&format!("retaining {} mapping for a private service namespace", kept.display()));
            busy.push(kept);
        }
    }
    Ok(busy)
}

/// The devices a running service keeps mounted in a mount namespace of
/// its own, from its mount table: none while it shares the recovery's
/// namespace, whose mounts the recovery takes away with its own. A
/// namespace or table that cannot be read, or a line of it that cannot
/// be, is an error, not a guess.
fn kept_mounted(namespace: io::Result<PathBuf>, ours: &Path, table: impl FnOnce() -> Result<MountInfos>) -> Result<BTreeSet<u64>> {
    let theirs = namespace.context("read the service's mount namespace")?;
    if theirs.as_path() == ours {
        return Ok(BTreeSet::new());
    }
    let mut devices = BTreeSet::new();
    for mount in table().context("read the service's mount table")? {
        let (major, minor) = mount.majmin.split_once(':').context("invalid service mount device number")?;
        let major = major.parse::<u32>().context("invalid service mount major number")?;
        let minor = minor.parse::<u32>().context("invalid service mount minor number")?;
        devices.insert(libc::makedev(major, minor));
    }
    Ok(devices)
}

/// Which of the mounted partitions, by device, still have a mount
/// somewhere: their mount points, each once, in the order mounted.
fn retained_mounts(devices: &BTreeSet<u64>, mounted: &[(u64, PathBuf)]) -> Vec<PathBuf> {
    let mut busy = Vec::new();
    for (device, point) in mounted {
        if devices.contains(device) && !busy.contains(point) {
            busy.push(point.clone());
        }
    }
    busy
}

/// Map `path` whole and lock the mapping: its pages then stay in memory
/// for every process that maps the file.
fn lock(file: &fs::File, path: &Path) -> Result<()> {
    let len = file.metadata().with_context(|| format!("stat {}", path.display()))?.len() as usize;
    if len == 0 {
        return Ok(());
    }
    let at = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    if at == libc::MAP_FAILED {
        return Err(io::Error::last_os_error()).with_context(|| format!("map {}", path.display()));
    }
    if unsafe { libc::mlock(at, len) } != 0 {
        let error = io::Error::last_os_error();
        unsafe { libc::munmap(at, len) };
        return Err(error).with_context(|| format!("lock {} in memory", path.display()));
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
    running_pids(path).into_iter().next()
}

fn running_pids(path: &str) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else { return Vec::new() };
    entries.flatten().filter_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        (fs::read_link(e.path().join("exe")).ok()? == Path::new(path)).then_some(pid)
    }).collect()
}

fn running_instance(s: &Service, mounts: &LibraryMounts) -> Result<Option<u32>> {
    if mounts.is_empty() {
        return Ok(running(&s.path));
    }
    let mut found = None;
    for pid in running_pids(&s.path) {
        mounts.verify_process(pid).with_context(|| format!("{} already runs as pid {pid} with an incompatible library view", s.path))?;
        ensure!(found.is_none(), "{} has more than one running instance; refusing to adopt or start another", s.path);
        found = Some(pid);
    }
    Ok(found)
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
    use super::*;
    use procfs::FromBufRead;

    /// mi_product as mounted: its device and where the recovery has it.
    fn mi_product() -> (u64, PathBuf) {
        (libc::makedev(259, 5), PathBuf::from("/mi_product"))
    }

    /// One line of a service's mount table, mi_product's device mounted
    /// at `point`.
    fn mountinfo(point: &str) -> MountInfos {
        MountInfos::from_buf_read(io::Cursor::new(format!("963 40 259:5 / {point} ro,relatime - erofs /dev/block/dm-9 ro\n"))).unwrap()
    }

    #[test]
    fn a_mount_kept_in_a_service_s_own_namespace_retains_its_partition() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026532622]")), Path::new("mnt:[4026531840]"), || Ok(mountinfo("/mi_product"))).unwrap();
        assert_eq!(retained_mounts(&devices, &[mi_product()]), [PathBuf::from("/mi_product")], "no file of it is mapped or open anywhere");
    }

    #[test]
    fn a_service_sharing_the_recovery_s_namespace_keeps_no_mounts() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026531840]")), Path::new("mnt:[4026531840]"), || bail!("a shared namespace must not read a private mount table")).unwrap();
        assert!(devices.is_empty(), "what it has mounted the recovery takes away with its own");
    }

    #[test]
    fn a_device_no_partition_has_keeps_none_of_them() {
        let devices = BTreeSet::from([libc::makedev(259, 6)]);
        assert!(retained_mounts(&devices, &[mi_product()]).is_empty(), "matched by device, not by path");
    }

    #[test]
    fn a_bind_at_another_path_keeps_the_device() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026532622]")), Path::new("mnt:[4026531840]"), || Ok(mountinfo("/vendor/firmware_mnt/bind"))).unwrap();
        assert_eq!(retained_mounts(&devices, &[mi_product()]), [PathBuf::from("/mi_product")], "the bind shows nowhere the recovery looks");
    }

    #[test]
    fn a_partition_kept_twice_is_told_once() {
        let mut table = mountinfo("/mi_product");
        table.0.extend(mountinfo("/private/bind"));
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Ok(table)).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(retained_mounts(&devices, &[mi_product(), mi_product()]), [PathBuf::from("/mi_product")]);
    }

    #[test]
    fn a_namespace_or_table_that_cannot_be_read_is_an_error() {
        assert!(kept_mounted(Err(io::Error::from(io::ErrorKind::PermissionDenied)), Path::new("mnt:[1]"), || Ok(mountinfo("/mi_product"))).is_err());
        assert!(kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Err(io::Error::from(io::ErrorKind::PermissionDenied).into())).is_err());
        assert!(MountInfos::from_buf_read(io::Cursor::new("nonsense\n")).is_err());
    }

    #[test]
    fn invalid_service_mount_device_number_is_refused() {
        let mut table = mountinfo("/mi_product");
        table.0[0].majmin = "invalid".into();
        assert!(kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Ok(table)).is_err());
    }
}
