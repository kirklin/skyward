//! Serving MTP on a thread of its own: the function's endpoints, the
//! responder answering the computer over the storage offered, and what the
//! phone asks of the thread meanwhile.

use crate::codes::{op, response};
use crate::dataset::Command;
use crate::responder::{Device, Incoming, Outgoing, Received, Reply, Responder, Storage, DATASET_MAX};
use crate::usb::{Halt, Next, Port};
use anyhow::{anyhow, ensure, Context, Result};
use jiff::tz::TimeZone;
use skyward_sys::kmsg;
use skyward_sys::worker::{self, Teller, Told};
use std::collections::VecDeque;
use std::io;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What init waits for, beside adbd's own, before it binds the function:
/// its descriptors are written.
const READY: &str = "sys.usb.ffs.mtp.ready";

/// How long the phone waits for the thread to give a storage back.
const GIVE_BACK: Duration = Duration::from_secs(10);

/// How long init may take to hand the port to adb alone.
const PORT_SWITCH: Duration = Duration::from_secs(5);

enum Message {
    /// Offer this storage, or none; the sender hears once nothing is under
    /// way in the storage offered before.
    Offer(Option<Storage>, Sender<()>),
    Zone(TimeZone),
    Stop,
}

/// What the phone has asked of the thread, kept until it can be done.
struct Mailbox {
    /// Readable while messages wait.
    asked: Told<Message>,
    pending: VecDeque<Message>,
    /// Set while the storage is wanted back, which ends long work on files.
    stop: Arc<AtomicBool>,
}

impl Mailbox {
    /// Take in what came.
    fn collect(&mut self) {
        self.pending.extend(self.asked.take());
    }

    /// Whether what came ends a transfer under way: the storage changes, or
    /// the service stops.
    fn urgent(&self) -> bool {
        self.pending.iter().any(|m| matches!(m, Message::Offer(..) | Message::Stop))
    }
}

/// MTP served on a thread of its own.
pub struct Service {
    /// What the phone asks of the thread, each of which wakes it.
    messages: Teller<Message>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl Service {
    /// Set the MTP function up and serve it: init can bind it from now on,
    /// and the computer finds no storage until one is offered.
    pub fn start(device: Device, zone: TimeZone) -> Result<Service> {
        let (messages, asked) = worker::telling().context("make the MTP thread's wake pipe")?;
        let port = Port::open(asked.fd())?;
        let stop = Arc::new(AtomicBool::new(false));
        let responder = Responder::new(device, zone, stop.clone());
        let mail = Mailbox { asked, pending: VecDeque::new(), stop: stop.clone() };
        skyward_sys::set_prop(READY, "1")?;
        let thread = thread::Builder::new()
            .name("mtp".into())
            .spawn(move || {
                let mut port = port;
                let served = serve(&mut port, responder, mail);
                if let Err(e) = &served {
                    let _ = kmsg(&format!("mtp: file transfer stops: {e:#}"));
                    // The port loses the function with its descriptors, and
                    // with it adb, unless it offers adb alone first.
                    skyward_sys::usb::set_mtp(false);
                    if skyward_sys::prop("sys.usb.state").as_deref() == Some("mtp,adb")
                        && let Err(e) = skyward_sys::usb::switch("adb", PORT_SWITCH)
                    {
                        let _ = kmsg(&format!("mtp: the port kept file transfer: {e:#}"));
                    }
                }
                drop(port);
                served
            })
            .context("start the MTP thread")?;
        Ok(Service { messages, stop, thread: Some(thread) })
    }

    /// Whether the thread still serves; one that failed says why once
    /// stopped.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn send(&self, message: Message) -> Result<()> {
        ensure!(self.messages.tell(message), "the MTP thread has ended");
        Ok(())
    }

    /// Offer `storage`, or none. Once this returns, nothing the computer
    /// had under way in the storage offered before goes on: a transfer is
    /// broken off and long work on files stops.
    pub fn offer(&self, storage: Option<Storage>) -> Result<()> {
        let (done, given) = mpsc::channel();
        self.stop.store(true, Ordering::Relaxed);
        self.send(Message::Offer(storage, done))?;
        given.recv_timeout(GIVE_BACK).map_err(|_| anyhow!("the MTP thread has not let go of the storage"))
    }

    /// The zone the times of files are told in.
    pub fn set_zone(&self, zone: TimeZone) -> Result<()> {
        self.send(Message::Zone(zone))
    }

    /// Stop serving. The function's descriptors go with the thread, so the
    /// port must offer the function no longer before.
    pub fn stop(mut self) -> Result<()> {
        self.end()
    }

    fn end(&mut self) -> Result<()> {
        let Some(thread) = self.thread.take() else { return Ok(()) };
        self.stop.store(true, Ordering::Relaxed);
        // A thread that has ended already says why when joined.
        let _ = self.send(Message::Stop);
        let served = thread.join().expect("the MTP thread does not panic");
        skyward_sys::set_prop(READY, "0")?;
        served
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Err(e) = self.end() {
            let _ = kmsg(&format!("mtp: {e:#}"));
        }
    }
}

/// How a session with the computer ended.
enum Ended {
    Disconnected,
    /// The phone asked the thread to stop.
    Stop,
}

/// The thread: wait for the computer, answer it while it stays, and do what
/// the phone asks in between, until it asks the thread to stop.
fn serve(port: &mut Port, mut r: Responder, mut mail: Mailbox) -> Result<()> {
    loop {
        if !port.enabled() && !port.wait_enabled()? {
            mail.collect();
            if attend(port, &mut r, &mut mail, 0)? {
                return Ok(());
            }
            continue;
        }
        kmsg("mtp: the computer configured the function")?;
        let ended = session(port, &mut r, &mut mail);
        r.disconnected();
        port.disconnected().context("give up what was under way")?;
        match ended? {
            Ended::Stop => return Ok(()),
            Ended::Disconnected => kmsg("mtp: the computer let the function go")?,
        }
    }
}

/// Do what the phone asked; whether it asked the thread to stop.
fn attend(port: &mut Port, r: &mut Responder, mail: &mut Mailbox, transaction: u32) -> Result<bool> {
    while let Some(message) = mail.pending.pop_front() {
        match message {
            Message::Offer(storage, done) => {
                let told = r.offer(storage).context("offer the storage")?;
                mail.stop.store(false, Ordering::Relaxed);
                for e in told {
                    port.event(e.code, transaction, e.param)?;
                }
                // The phone may have given up waiting.
                let _ = done.send(());
            }
            Message::Zone(zone) => r.set_zone(zone),
            Message::Stop => return Ok(true),
        }
    }
    Ok(false)
}

/// Answer the computer's commands until it lets the function go or the
/// phone stops the thread. Changes other programs make to the folders the
/// computer has read are told as they come.
fn session(port: &mut Port, r: &mut Responder, mail: &mut Mailbox) -> Result<Ended> {
    let mut transaction = 0;
    loop {
        let bytes = match port.command(r.fd()) {
            Ok(Next::Command(bytes)) => bytes,
            Ok(Next::Changed) => {
                for e in r.changes() {
                    port.event(e.code, transaction, e.param)?;
                }
                continue;
            }
            Ok(Next::Woken) => {
                mail.collect();
                if attend(port, r, mail, transaction)? {
                    return Ok(Ended::Stop);
                }
                continue;
            }
            Err(Halt::Disconnected) => return Ok(Ended::Disconnected),
            // A read the bus broke off is started again.
            Err(Halt::Aborted(e)) => {
                kmsg(&format!("mtp: reading a command broke off: {e}"))?;
                continue;
            }
            Err(Halt::Failed(e)) => return Err(e).context("read a command"),
            Err(halt @ (Halt::Cancelled | Halt::Stopped)) => unreachable!("waiting for a command is no transaction to end: {halt:?}"),
        };
        let Some(cmd) = Command::read(&bytes) else {
            kmsg(&format!("mtp: {} bytes came that are no command", bytes.len()))?;
            continue;
        };
        transaction = cmd.transaction;
        match answer(port, r, mail, &cmd) {
            Ok(()) => {}
            Err(Halt::Cancelled) => kmsg(&format!("mtp: the computer cancelled {:#06x}", cmd.code))?,
            Err(Halt::Stopped) => kmsg(&format!("mtp: {:#06x} stopped for the storage", cmd.code))?,
            Err(Halt::Aborted(e)) => kmsg(&format!("mtp: {:#06x} given up: {e}", cmd.code))?,
            Err(Halt::Disconnected) => return Ok(Ended::Disconnected),
            Err(Halt::Failed(e)) => return Err(e).with_context(|| format!("answer {:#06x}", cmd.code)),
        }
        mail.collect();
        if attend(port, r, mail, transaction)? {
            return Ok(Ended::Stop);
        }
    }
}

/// One transaction: the data that comes with the command, the answer and
/// its data, and the response.
fn answer(port: &mut Port, r: &mut Responder, mail: &mut Mailbox, cmd: &Command) -> Result<(), Halt> {
    let mut stop = || {
        mail.collect();
        mail.urgent()
    };
    let reply = match r.incoming(cmd) {
        Incoming::None => r.answer(cmd, None),
        Incoming::Dataset => {
            let mut data = Vec::new();
            let (_, _, failed) = port.receive(&mut stop, false, &mut |piece| {
                if data.len() + piece.len() > DATASET_MAX {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "the dataset is longer than any the phone takes"));
                }
                data.extend_from_slice(piece);
                Ok(())
            })?;
            match failed {
                Some(e) => Reply::failed(response::INVALID_DATASET, e),
                None => r.answer(cmd, Some(&data)),
            }
        }
        Incoming::File { file, offset, unbounded } => {
            let mut at = offset;
            let came = port.receive(&mut stop, unbounded, &mut |piece| {
                file.write_all_at(piece, at)?;
                at += piece.len() as u64;
                Ok(())
            });
            drop(file);
            let outcome = match came {
                Ok((_, n, None)) => Received::Done(n),
                Ok((_, _, Some(e))) => Received::Failed(e),
                Err(halt) => {
                    r.received(cmd, Received::Stopped);
                    return Err(halt);
                }
            };
            match r.received(cmd, outcome) {
                Some(reply) => reply,
                None => return Ok(()),
            }
        }
        Incoming::Refused(reply) => {
            port.receive(&mut stop, false, &mut |_| Ok(()))?;
            reply
        }
    };
    if let Some(why) = &reply.why {
        kmsg(&format!("mtp: {:#06x} answered {:#06x}: {why}", cmd.code, reply.code)).map_err(|e| Halt::Failed(io::Error::other(e)))?;
    }
    if cmd.code == op::OPEN_SESSION && reply.code == response::OK {
        kmsg("mtp: the computer opened a session").map_err(|e| Halt::Failed(io::Error::other(e)))?;
    }
    match reply.data {
        Some(Outgoing::Dataset(bytes)) => port.send_data(cmd.code, cmd.transaction, &bytes, &mut stop)?,
        Some(Outgoing::File { file, offset, length }) => port.send_file(cmd.code, cmd.transaction, &file, offset, length, &mut stop)?,
        None => {}
    }
    port.send_response(reply.code, cmd.transaction, &reply.params, &mut stop)
}
