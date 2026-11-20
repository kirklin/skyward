//! Letting the phone sleep while nothing needs it awake, as its own system
//! does once its screen is off: the kernel suspends it until a key, the
//! charger, an alarm or the network wakes it.

use crate::worker::{self, Told};
use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The kernel's count of wakeup events: read once none is in progress, as
/// the read waits while one is, then written back, which fails if another
/// came in between, so a suspend never loses an event that came as it
/// began.
const WAKEUP_COUNT: &str = "/sys/power/wakeup_count";

/// Where writing `mem` suspends the phone, and returns once it wakes.
const STATE: &str = "/sys/power/state";

/// How `mem` suspends the phone. Set to `s2idle`, as the phone's own system
/// sets it, every CPU stays online and idles in its deepest state. Suspended
/// `deep`, the kernel takes the CPUs offline one by one, and as the last of
/// a cluster goes it removes the cluster's cooling device; the uevent that
/// sends counts as a wakeup event, and the suspend stops there.
const MEM_SLEEP: &str = "/sys/power/mem_sleep";

/// How long to wait after the kernel would not suspend before trying again:
/// at first, and at most, doubling between.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MOST: Duration = Duration::from_secs(60);

/// How a sleep went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slept {
    /// The phone slept, and something woke it.
    Woke,
    /// The phone did not sleep: why.
    Refused(String),
}

/// Whether the screen's loop lets the phone sleep, and when the thread may
/// next try after a sleep that was refused.
struct Leave {
    allowed: bool,
    not_before: Instant,
}

/// A thread that suspends the phone while the screen's loop lets it. After
/// each sleep, done or refused, it waits to be let again, so the loop sees
/// what woke the phone, such as the power key, before it sleeps again.
pub struct Sleeper {
    leave: Arc<(Mutex<Leave>, Condvar)>,
    told: Told<Slept>,
}

impl Sleeper {
    pub fn start() -> Result<Sleeper> {
        fs::write(MEM_SLEEP, "s2idle").with_context(|| format!("write s2idle to {MEM_SLEEP}"))?;
        let (tell, told) = worker::telling().context("make the sleeper's wake pipe")?;
        let leave = Arc::new((Mutex::new(Leave { allowed: false, not_before: Instant::now() }), Condvar::new()));
        let shared = leave.clone();
        thread::Builder::new()
            .name("sleeper".into())
            .spawn(move || {
                let mut retry = RETRY_FIRST;
                loop {
                    let (lock, cvar) = &*shared;
                    {
                        let mut l = lock.lock().expect("the sleeper's leave is not held by a thread that panicked");
                        while !l.allowed || Instant::now() < l.not_before {
                            l = if l.allowed {
                                let left = l.not_before.saturating_duration_since(Instant::now());
                                cvar.wait_timeout(l, left).expect("the sleeper's leave is not held by a thread that panicked").0
                            } else {
                                cvar.wait(l).expect("the sleeper's leave is not held by a thread that panicked")
                            };
                        }
                    }
                    let slept = sleep(|| lock.lock().expect("the sleeper's leave is not held by a thread that panicked").allowed);
                    let Some(slept) = slept else { continue };
                    {
                        let mut l = lock.lock().expect("the sleeper's leave is not held by a thread that panicked");
                        l.allowed = false;
                        match &slept {
                            Slept::Woke => retry = RETRY_FIRST,
                            Slept::Refused(_) => {
                                l.not_before = Instant::now() + retry;
                                retry = (retry * 2).min(RETRY_MOST);
                            }
                        }
                    }
                    // The screen's loop only stops listening once the
                    // recovery ends.
                    tell.tell(slept);
                }
            })
            .context("start the sleeper")?;
        Ok(Sleeper { leave, told })
    }

    /// Readable while sleeps wait to be told.
    pub fn fd(&self) -> RawFd {
        self.told.fd()
    }

    /// Let the phone sleep, or keep it awake. Let, the thread suspends it as
    /// soon as the kernel will, and after the sleep waits to be let again.
    pub fn allow(&self, allowed: bool) {
        let (lock, cvar) = &*self.leave;
        let mut l = lock.lock().expect("the sleeper's leave is not held by a thread that panicked");
        if l.allowed != allowed {
            l.allowed = allowed;
            cvar.notify_one();
        }
    }

    /// The sleeps since the last call, and how each went.
    pub fn slept(&self) -> Vec<Slept> {
        self.told.take()
    }
}

/// Files whose news keeps the phone awake until the screen's loop has read
/// it, gathered in an epoll set the loop waits on: from when one becomes
/// readable until the loop, having read it all, waits on the set again, the
/// kernel counts a wakeup event in progress. So the phone never sleeps with
/// a key press, or a cable going in or out, that the loop has yet to see.
pub struct Wakeful {
    epoll: OwnedFd,
}

impl Wakeful {
    pub fn new() -> Result<Wakeful> {
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("make an epoll set");
        }
        Ok(Wakeful { epoll: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    /// Follow `fd` until it is closed.
    pub fn add(&self, fd: RawFd) -> Result<()> {
        let mut event = libc::epoll_event { events: (libc::EPOLLIN | libc::EPOLLWAKEUP) as u32, u64: fd as u64 };
        if unsafe { libc::epoll_ctl(self.epoll.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut event) } != 0 {
            return Err(io::Error::last_os_error()).context("follow a file in the epoll set");
        }
        Ok(())
    }

    /// Readable while a file followed is. Each poll of it lets the files
    /// read empty since stop keeping the phone awake.
    pub fn fd(&self) -> RawFd {
        self.epoll.as_raw_fd()
    }
}

/// Suspend the phone, unless `allowed` says the screen's loop took its
/// leave back while the thread waited for the wakeup events in progress to
/// end: none then.
fn sleep(allowed: impl Fn() -> bool) -> Option<Slept> {
    let count = match fs::read_to_string(WAKEUP_COUNT) {
        Ok(count) => count,
        Err(e) => return Some(Slept::Refused(format!("read {WAKEUP_COUNT}: {e}"))),
    };
    if !allowed() {
        return None;
    }
    if let Err(e) = fs::write(WAKEUP_COUNT, count.trim()) {
        return Some(Slept::Refused(format!("a wakeup event came as the phone was to sleep: {e}")));
    }
    Some(match fs::write(STATE, "mem") {
        Ok(()) => Slept::Woke,
        Err(e) => Slept::Refused(format!("the kernel would not suspend: {e}")),
    })
}
