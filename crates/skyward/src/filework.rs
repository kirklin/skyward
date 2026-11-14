//! Copying, moving, deleting and relabeling on a thread of its own, telling
//! the screen how far it has come.

use crate::relabel;
use anyhow::Result;
use skyward_files::{Cancelled, Clash, Ownership, Progress, Relabeling};
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most often how far the work has come is told.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

pub enum Task {
    Copy { from: PathBuf, into: PathBuf, clash: Clash, ownership: Ownership },
    Move { from: PathBuf, into: PathBuf, clash: Clash, ownership: Ownership },
    Delete { path: PathBuf },
    /// Give the data partition's files the labels the rules in `rules`
    /// give them.
    Relabel { rules: Vec<PathBuf> },
}

#[derive(Clone)]
pub enum End {
    Done,
    /// Stopped as asked.
    Stopped,
    Failed(String),
}

#[derive(Clone)]
pub enum Event {
    Progress(Progress),
    Relabeling(Relabeling),
    Ended(End),
}

pub struct Worker {
    cancel: Arc<AtomicBool>,
    /// The screen lets it go once the work has ended.
    worker: skyward_sys::worker::Worker<Event>,
}

impl Worker {
    pub fn start(task: Task) -> Result<Worker> {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = skyward_sys::worker::Worker::start("files", move |tell| {
            let mut told = Instant::now() - PROGRESS_EVERY;
            let mut last = None;
            let mut report = |event: Event| {
                if told.elapsed() >= PROGRESS_EVERY {
                    told = Instant::now();
                    tell.tell(event.clone());
                }
                last = Some(event);
            };
            let mut progress = |p: Progress| report(Event::Progress(p));
            let ended = match task {
                Task::Copy { from, into, clash, ownership } => skyward_files::copy(&[from], &into, clash, ownership, &stop, &mut progress),
                Task::Move { from, into, clash, ownership } => {
                    skyward_files::move_into(&[from], &into, clash, ownership, &stop, &mut progress)
                }
                Task::Delete { path } => skyward_files::delete(&[path], &stop, &mut progress),
                Task::Relabel { rules } => relabel::run(&rules, &stop, &mut |r| report(Event::Relabeling(r))),
            };
            // Work that stops or fails shows how far it came in the end.
            if let Some(event) = last {
                tell.tell(event);
            }
            tell.tell(Event::Ended(match ended {
                Ok(()) => End::Done,
                Err(e) if e.is::<Cancelled>() => End::Stopped,
                Err(e) => End::Failed(format!("{e:#}")),
            }));
        })?;
        Ok(Worker { cancel, worker })
    }

    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.worker.fd()
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        self.worker.events()
    }

    /// Ask the work to stop; it does between two chunks of a file.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
