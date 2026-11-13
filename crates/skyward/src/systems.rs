//! The second system's work for the pages, on a thread of its own so the
//! screen goes on while the boot partitions are written: the systems read,
//! a switch between them, and the second system removed, one task after
//! another, each ending with the systems read again.

use anyhow::{Context, Result};
use skyward_systems::{install, Overview, State, Who};
use skyward_vab::Slot;
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Sender};

pub enum Task {
    /// Read the systems.
    Read,
    /// Finish a switch cut off before, where there is one.
    Resume,
    /// Have this system start next.
    Switch(Who),
    /// Have the system a request in misc names start next: the installed
    /// one by a word, the second by its id.
    SwitchNamed(String),
    /// Remove the second system.
    Remove,
}

pub enum Event {
    /// A switch cut off before was finished: this system starts next.
    Resumed(Who),
    /// A switch cut off before could not be finished, and why.
    NotResumed(String),
    /// A switch went through: this system starts next.
    Switched(Who),
    /// The task failed, and why.
    Failed(String),
    /// The systems as read once the task ended, or why they could not be;
    /// every task ends with them.
    Read(Result<Overview, String>),
}

/// The slot the bootloader starts next, as the phone's boot control holds
/// it.
pub fn next_slot() -> Result<Slot> {
    let (binder, hardware) = (skyward_binder::driver::Binder::open()?, skyward_binder::driver::Binder::hardware()?);
    let control = skyward_vab::boot::BootControl::open(&binder, &hardware)?;
    Ok(control.slots()?.active)
}

/// The systems as they are now: the installed one named by its build's
/// properties, its version by `version_prop`, whether its vendor_boot or
/// dtbo in the slot that starts next changed since the second system was
/// installed, and what the boot partitions hold, which are those of the
/// slot the phone started from where nothing is kept of them.
fn read(version_prop: &str) -> Result<Overview> {
    let current = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let props = skyward_info::installed_props(current)?;
    let state = install::state()?;
    let changed = match state.system {
        Some(_) => install::changed(&state, next_slot()?)?,
        None => false,
    };
    let held = install::held(&state, current)?;
    Ok(install::overview(&state, skyward_systems::installed_name(&props, version_prop), changed, held))
}

/// The system whose boot set is in the boot partitions.
fn starting(state: &State) -> Who {
    match state.occupied {
        Some(_) => Who::Other,
        None => Who::Installed,
    }
}

/// Finish a switch cut off before, where there was one, and say which
/// system starts next then.
fn resume() -> Result<Option<Who>> {
    Ok(match install::resume()? {
        true => Some(starting(&install::state()?)),
        false => None,
    })
}

/// Have `to` start next, once a switch cut off before is finished; a
/// system that starts already is left as it is.
fn switch(to: Who) -> Result<()> {
    install::resume()?;
    if starting(&install::state()?) == to {
        return Ok(());
    }
    install::switch_to(next_slot()?, to)
}

/// What a switch tells: the system it went to, or why it did not go.
fn switched(done: Result<Who>) -> Event {
    match done {
        Ok(to) => Event::Switched(to),
        Err(e) => Event::Failed(format!("{e:#}")),
    }
}

/// The system a request in misc names.
fn named(name: &str) -> Result<Who> {
    Who::named(name, &install::state()?).with_context(|| format!("no system named {name:?} is installed"))
}

/// The work on the systems, from a thread of its own.
pub struct Worker {
    /// The thread ends once it has no more tasks to take: this end goes
    /// before the thread is waited for, being the field before it.
    tasks: Sender<Task>,
    /// The tasks sent and not yet ended.
    pending: usize,
    worker: skyward_sys::worker::Worker<Event>,
}

impl Worker {
    /// Start the thread; `version_prop` is the build property naming the
    /// installed system's version, as the phone's configuration says.
    pub fn start(version_prop: String) -> Result<Worker> {
        let (tasks, todo) = mpsc::channel::<Task>();
        let worker = skyward_sys::worker::Worker::start("systems", move |tell| {
            for task in todo {
                let told = match task {
                    Task::Read => None,
                    Task::Resume => match resume() {
                        Ok(resumed) => resumed.map(Event::Resumed),
                        Err(e) => Some(Event::NotResumed(format!("{e:#}"))),
                    },
                    Task::Switch(to) => Some(switched(switch(to).map(|()| to))),
                    Task::SwitchNamed(name) => Some(switched(named(&name).and_then(|to| switch(to).map(|()| to)))),
                    Task::Remove => install::remove().err().map(|e| Event::Failed(format!("{e:#}"))),
                };
                if let Some(event) = told {
                    tell.tell(event);
                }
                tell.tell(Event::Read(read(&version_prop).map_err(|e| format!("{e:#}"))));
            }
        })?;
        Ok(Worker { tasks, pending: 0, worker })
    }

    pub fn send(&mut self, task: Task) {
        self.tasks.send(task).expect("the systems' worker takes tasks while the screen holds it");
        self.pending += 1;
    }

    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.worker.fd()
    }

    /// Whether a task is under way or waiting.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// The events since the last call.
    pub fn events(&mut self) -> Vec<Event> {
        let events = self.worker.events();
        self.pending -= events.iter().filter(|e| matches!(e, Event::Read(_))).count();
        events
    }
}
