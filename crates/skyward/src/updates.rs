//! Asking the phone maker's update server for the newest system and
//! downloading its full package, on a thread of their own that tells the
//! screen how far they have come.

use anyhow::{Context, Result};
use skyward_net::download::{self, Damaged, Failure, NoRoom, Progress, Refused, Sum};
use skyward_net::http::{self, Servers};
use skyward_net::reach::{self, Reach};
use skyward_net::release::{self, Installed, Release, Unanswered};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The most often how far a download has come is told.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// How long the speed shown is taken over.
const RATE_OVER: Duration = Duration::from_secs(3);

/// How many tries in a row may fail with nothing more downloaded before
/// the download gives up, and how long it waits between them.
const TRIES: u32 = 5;
const BETWEEN: Duration = Duration::from_secs(3);

pub enum Task {
    /// Ask `server` for the newest system for `installed`. Where that
    /// fails, `probe`, the page that tells whether the network joined
    /// reaches the internet, says whether the network is why.
    Check { server: String, probe: String, installed: Installed },
    /// Download `release`'s full package to `path`; `server` is asked again
    /// for where it is when the addresses it gave have run out.
    Download { server: String, installed: Installed, release: Release, path: PathBuf },
}

pub enum Event {
    Checked(Result<Option<Release>, Unanswered>),
    /// How far the download has come, and how fast it goes, in bytes a
    /// second.
    Progress(Progress, u64),
    Ended(End),
}

pub enum End {
    Downloaded,
    Stopped,
    Failed(Failure),
}

pub struct Worker {
    events: Receiver<Event>,
    /// Readable while events wait.
    wake: OwnedFd,
    stop: Arc<AtomicBool>,
    /// The connection in use, which stopping cuts at once.
    agent: Arc<Mutex<Option<ureq::Agent>>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(task: Task, servers: Servers) -> Result<Worker> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error()).context("make the update worker's wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let agent: Arc<Mutex<Option<ureq::Agent>>> = Arc::new(Mutex::new(None));
        let (stopping, using) = (stop.clone(), agent.clone());
        let thread = thread::Builder::new()
            .name("updates".into())
            .spawn(move || {
                let tell = |event: Event| {
                    // The screen only stops listening once it has let the
                    // worker go.
                    let _ = tx.send(event);
                    unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
                };
                match task {
                    // The package's exact length is asked of where it is
                    // downloaded from: the server gives it rounded, in units
                    // of 1024. Without it the rounded one shows.
                    Task::Check { server, probe, installed } => {
                        let a = http::agent(servers.clone());
                        *using.lock().expect("the agent is not held by a thread that panicked") = Some(a.clone());
                        let mut answer = release::ask(&a, &server, &installed).map_err(|e| unanswered(&e, &server, &probe, &servers));
                        if let Ok(Some(r)) = &mut answer {
                            match release::length(&a, &r.urls[0]) {
                                Ok(length) => r.length = Some(length),
                                Err(e) => {
                                    let _ = skyward_sys::kmsg(&format!("how long {} is: {e:#}", r.name));
                                }
                            }
                        }
                        tell(Event::Checked(answer));
                    }
                    Task::Download { server, installed, release, path } => {
                        let end = fetch(&servers, &server, &installed, release, &path, &stopping, &using, &tell);
                        tell(Event::Ended(end));
                    }
                }
            })
            .context("start the update worker")?;
        Ok(Worker { events, wake, stop, agent, thread: Some(thread) })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.events.try_iter().collect()
    }

    /// Stop at once, cutting the connection in use.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(a) = self.agent.lock().expect("the agent is not held by a thread that panicked").take() {
            a.shutdown();
        }
    }
}

impl Drop for Worker {
    /// The screen lets the worker go once its work has ended, or once it
    /// has been told to stop.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the update worker does not panic");
        }
    }
}

/// Why `server` gave no answer: that the network joined wants a login
/// first, or reaches nothing, where `probe` shows it, rather than how
/// asking failed through such a network.
fn unanswered(e: &anyhow::Error, server: &str, probe: &str, servers: &Servers) -> Unanswered {
    let dns = servers.lock().expect("the DNS servers are not held by a thread that panicked").clone();
    let reach = reach::probe(probe, &dns);
    let _ = skyward_sys::kmsg(&format!("asking {server}: {e:#}; the network joined {reach}"));
    match reach {
        Reach::Online => Unanswered::Failed(format!("{e:#}")),
        Reach::Login(_) => Unanswered::Login,
        Reach::Offline(why) => Unanswered::Offline(why),
    }
}

/// Download the package, trying again while the connection fails and
/// something more comes each time, and asking the server again for where it
/// is once the addresses it gave run out.
#[allow(clippy::too_many_arguments)]
fn fetch(
    servers: &Servers,
    server: &str,
    installed: &Installed,
    mut release: Release,
    path: &std::path::Path,
    stop: &AtomicBool,
    using: &Mutex<Option<ureq::Agent>>,
    tell: &dyn Fn(Event),
) -> End {
    let mut url = 0;
    let mut failed = 0;
    let mut asked_again = false;
    let mut samples: Vec<(Instant, u64)> = Vec::new();
    let mut told = Instant::now() - PROGRESS_EVERY;
    loop {
        if stop.load(Ordering::Relaxed) {
            return End::Stopped;
        }
        let a = http::agent(servers.clone());
        *using.lock().expect("the agent is not held by a thread that panicked") = Some(a.clone());
        let mut came = None::<u64>;
        let mut gained = false;
        let mut last = None;
        let mut progress = |p: Progress| {
            if let Progress::Downloading { done, .. } = p {
                let now = Instant::now();
                if came.is_some_and(|before| done > before) {
                    gained = true;
                }
                came = Some(done);
                samples.push((now, done));
                samples.retain(|(at, _)| now.duration_since(*at) <= RATE_OVER);
            }
            let rate = match (samples.first(), samples.last()) {
                (Some((t0, d0)), Some((t1, d1))) if t1 > t0 => ((d1 - d0) as f64 / t1.duration_since(*t0).as_secs_f64()) as u64,
                _ => 0,
            };
            if told.elapsed() >= PROGRESS_EVERY {
                told = Instant::now();
                tell(Event::Progress(p, rate));
            }
            last = Some((p, rate));
        };
        let result = download::fetch(&a, &release.urls[url], path, &Sum::Md5(release.md5.clone()), stop, &mut progress);
        // How far it came shows in the end, whatever stopped it.
        if let Some((p, rate)) = last {
            tell(Event::Progress(p, rate));
        }
        let e = match result {
            Ok(()) => return End::Downloaded,
            Err(e) => e,
        };
        if e.is::<skyward_net::Stopped>() || stop.load(Ordering::Relaxed) {
            return End::Stopped;
        }
        if let Some(room) = e.downcast_ref::<NoRoom>() {
            return End::Failed(Failure::NoRoom { needed: room.needed, free: room.free });
        }
        if e.is::<Damaged>() {
            return End::Failed(Failure::Damaged);
        }
        let _ = skyward_sys::kmsg(&format!("downloading {} from {}: {e:#}", release.name, host(&release.urls[url])));
        if e.is::<Refused>() && !asked_again {
            // A signed address runs out a day after the server gave it.
            asked_again = true;
            match release::ask(&a, server, installed) {
                Ok(Some(fresh)) if fresh.md5 == release.md5 => {
                    release = fresh;
                    url = 0;
                    continue;
                }
                Ok(_) => return End::Failed(Failure::Changed),
                Err(again) => {
                    let _ = skyward_sys::kmsg(&format!("asking where {} is: {again:#}", release.name));
                }
            }
        }
        failed = if gained { 1 } else { failed + 1 };
        if failed >= TRIES {
            return End::Failed(Failure::Other(format!("{e:#}")));
        }
        url = (url + 1) % release.urls.len();
        let until = Instant::now() + BETWEEN;
        while Instant::now() < until {
            if stop.load(Ordering::Relaxed) {
                return End::Stopped;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The host of a download's address, for the log; the signature after it
/// stays out.
fn host(url: &str) -> &str {
    url.split('/').nth(2).unwrap_or(url)
}
