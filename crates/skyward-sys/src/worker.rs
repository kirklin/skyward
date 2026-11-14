//! Work done on a thread of its own, away from a loop that waits on file
//! descriptors, and what the thread tells the loop: events over a channel,
//! and a pipe written as each is sent, so the loop waits on the pipe's
//! read end among its other descriptors and takes the events once it is
//! readable.

use anyhow::{Context, Result};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// The end events are told into.
pub struct Teller<E> {
    events: Sender<E>,
    /// The write end of the pipe.
    signal: Arc<OwnedFd>,
}

impl<E> Clone for Teller<E> {
    fn clone(&self) -> Teller<E> {
        Teller { events: self.events.clone(), signal: self.signal.clone() }
    }
}

impl<E> Teller<E> {
    /// Tell `event` and wake whoever listens. Gives whether anyone still
    /// does: the other end may have gone.
    pub fn tell(&self, event: E) -> bool {
        let heard = self.events.send(event).is_ok();
        if heard {
            // A full pipe already wakes the listener.
            unsafe { libc::write(self.signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
        }
        heard
    }
}

/// The end events are heard at.
pub struct Told<E> {
    events: Receiver<E>,
    /// The read end of the pipe.
    wake: OwnedFd,
}

impl<E> Told<E> {
    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// The events since the last call.
    pub fn take(&self) -> Vec<E> {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.events.try_iter().collect()
    }
}

/// A channel for events with a pipe beside it: the end they are told into,
/// and the end they are heard at.
pub fn telling<E>() -> io::Result<(Teller<E>, Told<E>)> {
    let (wake, signal) = pipe()?;
    let (tx, events) = mpsc::channel();
    Ok((Teller { events: tx, signal: Arc::new(signal) }, Told { events, wake }))
}

/// A pipe neither end of which blocks or is kept by a program started
/// from here: its read end, then its write end.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// The same where the kernel makes no pipe with both set at once.
#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let ends = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for end in [&ends.0, &ends.1] {
        if unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0
            || unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(ends)
}

/// Work on a thread of its own, whose events the loop that started it
/// takes. Letting the worker go waits for the work to end.
pub struct Worker<E> {
    name: &'static str,
    told: Told<E>,
    thread: Option<JoinHandle<()>>,
}

impl<E: Send + 'static> Worker<E> {
    /// Run `work` on a thread named `name`; what it tells comes back as
    /// events.
    pub fn start(name: &'static str, work: impl FnOnce(Teller<E>) + Send + 'static) -> Result<Worker<E>> {
        let (teller, told) = telling().with_context(|| format!("make the {name} thread's wake pipe"))?;
        let thread = thread::Builder::new().name(name.into()).spawn(move || work(teller)).with_context(|| format!("start the {name} thread"))?;
        Ok(Worker { name, told, thread: Some(thread) })
    }
}

impl<E> Worker<E> {
    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.told.fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<E> {
        self.told.take()
    }

    /// Whether the work has ended.
    pub fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl<E> Drop for Worker<E> {
    /// Whoever holds the worker lets it go once its work has ended, or
    /// has been told to.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            panic!("the {} thread panicked", self.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Whether `fd` is readable within `within`.
    fn readable(fd: RawFd, within: Duration) -> bool {
        let mut waited = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut waited, 1, within.as_millis() as libc::c_int) };
        assert!(ready >= 0, "{}", io::Error::last_os_error());
        ready == 1 && waited.revents & libc::POLLIN != 0
    }

    #[test]
    fn what_the_work_tells_wakes_the_loop_and_is_taken_in_order() {
        let (go, gone) = mpsc::channel::<()>();
        let (all, told) = mpsc::channel::<()>();
        let worker = Worker::start("counting", move |tell| {
            gone.recv().unwrap();
            for n in 0..3u32 {
                assert!(tell.tell(n));
            }
            all.send(()).unwrap();
            // The work holds its end until it is let go.
            gone.recv().unwrap();
        })
        .unwrap();
        assert!(!readable(worker.fd(), Duration::ZERO), "nothing waits before anything is told");
        assert!(worker.events().is_empty());
        assert!(!worker.ended());
        go.send(()).unwrap();
        assert!(readable(worker.fd(), Duration::from_secs(10)), "the pipe wakes the loop");
        told.recv().unwrap();
        assert_eq!(worker.events(), [0, 1, 2]);
        assert!(!readable(worker.fd(), Duration::ZERO), "taking the events empties the pipe");
        go.send(()).unwrap();
    }

    #[test]
    fn letting_a_worker_go_waits_for_its_work_to_end() {
        let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ends = ended.clone();
        let worker = Worker::<()>::start("slow", move |_| {
            thread::sleep(Duration::from_millis(50));
            ends.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
        drop(worker);
        assert!(ended.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_worker_whose_work_returned_has_ended_with_its_events_still_to_take() {
        let worker = Worker::start("brief", |tell| {
            tell.tell("done");
        })
        .unwrap();
        assert!(readable(worker.fd(), Duration::from_secs(10)));
        while !worker.ended() {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(worker.events(), ["done"]);
    }

    #[test]
    fn telling_says_when_nobody_listens_any_more() {
        let (teller, told) = telling::<u8>().unwrap();
        let other = teller.clone();
        assert!(teller.tell(1) && other.tell(2));
        assert_eq!(told.take(), [1, 2]);
        drop(told);
        assert!(!teller.tell(3));
    }

    #[test]
    fn more_events_than_the_pipe_holds_are_all_taken() {
        let (teller, told) = telling::<u32>().unwrap();
        // A pipe holds 64 KiB at least; the events told past that still
        // go over the channel.
        let many = 200_000u32;
        for n in 0..many {
            assert!(teller.tell(n));
        }
        assert!(readable(told.fd(), Duration::ZERO));
        let heard = told.take();
        assert_eq!(heard.len(), many as usize);
        assert!(heard.iter().copied().eq(0..many));
        assert!(!readable(told.fd(), Duration::ZERO));
    }
}
