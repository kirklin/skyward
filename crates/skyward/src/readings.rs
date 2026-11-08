//! The phone's readings taken on a thread of their own, so the screen keeps
//! up: the battery's driver takes milliseconds to answer each read.

use anyhow::{Context, Result};
use skyward_config::DeviceConfig;
use skyward_info::{Battery, DeviceInfo};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// The battery is read this often, and at once when asked; all the
/// readings every so many times instead.
const BATTERY_EVERY: Duration = Duration::from_secs(1);
const ALL_EVERY: u32 = 30;

pub enum Reading {
    /// The battery, and when reading it started.
    Battery(Result<Option<Battery>>, Instant),
    /// All the readings, and when taking them started.
    All(Box<Result<DeviceInfo>>, Instant),
}

pub struct Readings {
    taken: Receiver<Reading>,
    asks: Sender<()>,
    /// Readable while readings wait.
    wake: OwnedFd,
}

impl Readings {
    /// Start taking the readings of the phone `cfg` describes, the first a
    /// second from now. The thread ends once the screen lets them go.
    pub fn start(cfg: &DeviceConfig) -> Result<Readings> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error()).context("make the readings' wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (tx, taken) = mpsc::channel();
        let (asks, asked) = mpsc::channel::<()>();
        let cfg = cfg.clone();
        thread::Builder::new()
            .name("readings".into())
            .spawn(move || {
                let mut ticks = 0u32;
                loop {
                    let at_once = match asked.recv_timeout(BATTERY_EVERY) {
                        Ok(()) => {
                            while asked.try_recv().is_ok() {}
                            true
                        }
                        Err(RecvTimeoutError::Timeout) => false,
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    if !at_once {
                        ticks += 1;
                    }
                    let at = Instant::now();
                    let reading = if !at_once && ticks.is_multiple_of(ALL_EVERY) {
                        Reading::All(Box::new(skyward_info::collect(&cfg)), at)
                    } else {
                        Reading::Battery(skyward_info::battery_now(), at)
                    };
                    if tx.send(reading).is_err() {
                        return;
                    }
                    unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
                }
            })
            .context("start the readings thread")?;
        Ok(Readings { taken, asks, wake })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Read the battery at once.
    pub fn ask(&self) {
        self.asks.send(()).expect("the readings thread runs while its readings are taken");
    }

    /// The readings taken since the last call.
    pub fn taken(&self) -> Vec<Reading> {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.taken.try_iter().collect()
    }
}
