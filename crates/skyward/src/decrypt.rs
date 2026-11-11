//! Opening the data partition and its users' own files on a thread of its
//! own, so the screen keeps up: the keys come from the secure world, and
//! what a user enters at their screen lock is checked slowly on purpose.

use anyhow::{Context, Result};
use skyward_binder::driver::Binder;
use skyward_crypto::data::{self, Closed, Opening};
use skyward_crypto::keymint::KeyMint;
use skyward_config::Decrypt;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

pub enum Job {
    /// Open the partition as far as it opens with nothing a user knows.
    Open,
    /// Open the own files of `user` with what they entered at their screen
    /// lock; `closed` are the users whose own files are still closed.
    Unlock { closed: Vec<Closed>, user: u32, credential: Vec<u8> },
}

/// How a job ended: what it opened, or why it did not.
pub enum Ended {
    Opened(Result<Opening, String>),
    Unlocked(Result<Vec<Closed>, String>),
}

pub struct Decryptor {
    ended: Receiver<Ended>,
    /// Readable once the job has ended.
    wake: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl Decryptor {
    pub fn start(job: Job, policy: Decrypt) -> Result<Decryptor> {
        skyward_sys::kmsg(&format!("decryption policy: read_only={}, auto_fsck={}, key_upgrades={}",
            policy.read_only, policy.checks_filesystem(), policy.upgrades_keys()))?;
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error()).context("make the decrypting thread's wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (tx, ended) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("decrypt".into())
            .spawn(move || {
                let ended_as = match job {
                    Job::Open => Ended::Opened(open(policy)),
                    Job::Unlock { closed, user, credential } => {
                        Ended::Unlocked(with_key_service(policy, |service| data::unlock(service, &closed, user, &credential)))
                    }
                };
                // The screen only stops listening once it has let the
                // thread go.
                let _ = tx.send(ended_as);
                unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
            })
            .context("start the decrypting thread")?;
        Ok(Decryptor { ended, wake, thread: Some(thread) })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// How the job ended, once it has.
    pub fn ended(&self) -> Option<Ended> {
        let mut drain = [0u8; 8];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.ended.try_recv().ok()
    }
}

impl Drop for Decryptor {
    /// The screen lets the thread go once its job has ended.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the decrypting thread does not panic");
        }
    }
}

/// Do `work` with the phone's key service, reached afresh for it, and say
/// why it failed in words for the page.
pub fn options(policy: Decrypt) -> data::Options {
    data::Options { read_only: policy.read_only, check_filesystem: policy.checks_filesystem() }
}

pub fn open(policy: Decrypt) -> Result<Opening, String> {
    with_key_service(policy, |service| data::open_with_options(service, options(policy)))
}

pub fn with_key_service<T>(policy: Decrypt, work: impl FnOnce(&KeyMint) -> Result<T>) -> Result<T, String> {
    let done = (|| {
        let binder = Binder::open()?;
        let service = KeyMint::open(&binder)?;
        let service = if policy.upgrades_keys() { service } else { service.without_upgrades() };
        work(&service)
    })();
    done.map_err(|e| format!("{e:#}"))
}
