//! Serving MTP on a thread of its own: the function's endpoints, the
//! responder answering the computer over the storage offered, and what the
//! phone asks of the thread meanwhile.

use crate::codes::{op, response};
use crate::dataset::Command;
use crate::responder::{Device, Incoming, Outgoing, Received, Reply, Responder, Storage, DATASET_MAX};
use crate::usb::{Halt, Next, Port};
use anyhow::{anyhow, Context, Result};
use jiff::tz::TimeZone;
use skyward_sys::kmsg;
use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
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
    rx: Receiver<Message>,
    /// Readable while messages wait.
    wake: OwnedFd,
    pending: VecDeque<Message>,
    /// Set while the storage is wanted back, which ends long work on files.
    stop: Arc<AtomicBool>,
}

impl Mailbox {
    /// Take in what came.
    fn collect(&mut self) {
        let mut drain = [0u8; 64];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.pending.extend(self.rx.try_iter());
    }

    /// Whether what came ends a transfer under way: the storage changes, or
    /// the service stops.
    fn urgent(&self) -> bool {
        self.pending.iter().any(|m| matches!(m, Message::Offer(..) | Message::Stop))
    }
}

/// MTP served on a thread of its own.
pub struct Service {
    messages: Sender<Message>,
    /// The write end of the thread's wake pipe.
    wake: OwnedFd,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl Service {
    /// Set the MTP function up and serve it: init can bind it from now on,
    /// and the computer finds no storage until one is offered.
    pub fn start(device: Device, zone: TimeZone) -> Result<Service> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(io::Error::last_os_error()).context("make the MTP thread's wake pipe");
        }
        let (wake_read, wake) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let port = Port::open(wake_read.as_raw_fd())?;
        let stop = Arc::new(AtomicBool::new(false));
        let responder = Responder::new(device, zone, stop.clone());
        let (messages, rx) = mpsc::channel();
        let mail = Mailbox { rx, wake: wake_read, pending: VecDeque::new(), stop: stop.clone() };
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
                    if skyward_sys::prop("sys.usb.state").as_deref() == Some("mtp,adb") {
                        if let Err(e) = skyward_sys::usb::switch("adb", PORT_SWITCH) {
                            let _ = kmsg(&format!("mtp: the port kept file transfer: {e:#}"));
                        }
                    }
                }
                drop(port);
                served
            })
            .context("start the MTP thread")?;
        Ok(Service { messages, wake, stop, thread: Some(thread) })
    }

    /// Whether the thread still serves; one that failed says why once
    /// stopped.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn send(&self, message: Message) -> Result<()> {
        self.messages.send(message).map_err(|_| anyhow!("the MTP thread has ended"))?;
        // A full pipe wakes the thread already.
        unsafe { libc::write(self.wake.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
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
    let mut trace_followups = 0u8;
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
        let trace_delete = cmd.code == op::DELETE_OBJECT && skyward_sys::prop("debug.skyward.mtp_delete_trace").as_deref() == Some("1");
        let trace = trace_delete || (trace_followups > 0 && matches!(cmd.code,
            op::GET_NUM_OBJECTS | op::GET_OBJECT_HANDLES | op::GET_OBJECT_INFO | op::GET_OBJECT_PROP_VALUE | op::GET_OBJECT_PROP_LIST));
        if trace_delete {
            trace_followups = 16;
            port.trace_next_commands(16);
        } else {
            trace_followups = trace_followups.saturating_sub(1);
        }
        match answer(port, r, mail, &cmd, trace) {
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
fn answer(port: &mut Port, r: &mut Responder, mail: &mut Mailbox, cmd: &Command, traced: bool) -> Result<(), Halt> {
    // Only a bounded window after a diagnostic DeleteObject is traced.
    // Never log object handles, paths, datasets or error text here.
    let trace = |stage: &str, code: Option<u16>| {
        if traced {
            let code = code.map(|c| format!(" response={c:#06x}")).unwrap_or_default();
            let _ = kmsg(&format!("mtp: delete op={:#06x} tid={} stage={stage}{code}", cmd.code, cmd.transaction));
        }
    };
    trace("command-received", None);
    if traced {
        match cmd.code {
            op::GET_OBJECT_HANDLES => {
                if let Some([_, format, parent]) = cmd.take::<3>() {
                    let _ = kmsg(&format!("mtp: delete op={:#06x} tid={} stage=enumeration-request parent={} format={format:#010x}", cmd.code, cmd.transaction, r.handle_scope(parent)));
                }
            }
            op::GET_OBJECT_PROP_LIST => {
                if let Some([handle, format, property, group, depth]) = cmd.take::<5>() {
                    let _ = kmsg(&format!("mtp: delete op={:#06x} tid={} stage=enumeration-request scope={} depth={depth:#010x} format={format:#010x} property={property:#010x} group={group:#010x}", cmd.code, cmd.transaction, r.handle_scope(handle)));
                }
            }
            _ => {}
        }
    }
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
    trace("reply-ready", Some(reply.code));
    let count = match &reply.data {
        Some(Outgoing::Dataset(bytes)) if traced && matches!(cmd.code, op::GET_OBJECT_HANDLES | op::GET_OBJECT_PROP_LIST) => {
            bytes.get(..4).and_then(|head| head.try_into().ok()).map(u32::from_le_bytes)
        }
        _ => None,
    };
    if let Some(count) = count {
        let label = if cmd.code == op::GET_OBJECT_HANDLES { "handleCount" } else { "entryCount" };
        let _ = kmsg(&format!("mtp: delete op={:#06x} tid={} stage=enumeration-result {label}={count}", cmd.code, cmd.transaction));
    }
    if let Some(why) = &reply.why {
        kmsg(&format!("mtp: {:#06x} answered {:#06x}: {why}", cmd.code, reply.code)).map_err(|e| Halt::Failed(io::Error::other(e)))?;
    }
    if cmd.code == op::OPEN_SESSION && reply.code == response::OK {
        kmsg("mtp: the computer opened a session").map_err(|e| Halt::Failed(io::Error::other(e)))?;
    }
    match reply.data {
        Some(Outgoing::Dataset(bytes)) => {
            trace("dataset-start", Some(reply.code));
            let sent = port.send_data(cmd.code, cmd.transaction, &bytes, &mut stop);
            trace(if sent.is_ok() { "dataset-complete" } else { "dataset-failed" }, Some(reply.code));
            sent?;
        }
        Some(Outgoing::File { file, offset, length }) => port.send_file(cmd.code, cmd.transaction, &file, offset, length, &mut stop)?,
        None => {}
    }
    trace("response-start", Some(reply.code));
    let sent = port.send_response(reply.code, cmd.transaction, &reply.params, &mut stop);
    let stage = match &sent {
        Ok(()) => "response-complete",
        Err(Halt::Cancelled) => "response-cancelled",
        Err(Halt::Stopped) => "response-stopped",
        Err(Halt::Aborted(_)) => "response-aborted",
        Err(Halt::Disconnected) => "response-disconnected",
        Err(Halt::Failed(_)) => "response-failed",
    };
    trace(stage, Some(reply.code));
    sent
}
