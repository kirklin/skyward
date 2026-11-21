//! The MTP function over USB through FunctionFS, laid out as the stock
//! system lays it out: its descriptors, the class requests the computer
//! sends it on the control endpoint, and transfers on its endpoints through
//! the kernel's asynchronous I/O, so a control request, or the phone's own
//! need to stop, is answered while a transfer waits on the computer.

use crate::codes::{container, request, response};
use crate::dataset::{self, Header, HEADER};
use anyhow::{Context, Result};
use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use zerocopy::little_endian::{U16, U32};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

const EP0: &str = "/dev/usb-ffs/mtp/ep0";
/// Bulk in, phone to computer.
const EP_IN: &str = "/dev/usb-ffs/mtp/ep1";
/// Bulk out, computer to phone.
const EP_OUT: &str = "/dev/usb-ffs/mtp/ep2";
/// Interrupt in, the events.
const EP_EVENT: &str = "/dev/usb-ffs/mtp/ep3";

/// How much one transfer of a file moves: two of these take turns, one on
/// the bus while the other is read from the file or written to it.
const CHUNK: usize = 1 << 20;

/// How much of a file one transfer to the computer moves. WPD reads a file
/// in its own requests and gives up on a transfer that outlasts them, so
/// this stays at the size the phone's own MTP stack uses.
const FILE_CHUNK: usize = 1 << 20;

/// What a command is read into; one is a single short packet.
const COMMAND_MAX: usize = 512;

// From linux/usb/functionfs.h and linux/usb/ch9.h.
const DESCRIPTORS_MAGIC_V2: u32 = 3;
const STRINGS_MAGIC: u32 = 2;
const HAS_FS_DESC: u32 = 1;
const HAS_HS_DESC: u32 = 2;
const HAS_SS_DESC: u32 = 4;
const HAS_MS_OS_DESC: u32 = 8;
const DT_INTERFACE: u8 = 4;
const DT_ENDPOINT: u8 = 5;
const DT_SS_ENDPOINT_COMP: u8 = 0x30;
const DIR_IN: u8 = 0x80;
const XFER_BULK: u8 = 2;
const XFER_INT: u8 = 3;
const TYPE_MASK: u8 = 0x60;
const TYPE_CLASS: u8 = 0x20;

/// A still image device that speaks PTP, which MTP extends.
const STILL_IMAGE_CLASS: u8 = 6;
const STILL_IMAGE_SUBCLASS: u8 = 1;
const PTP_PROTOCOL: u8 = 1;

/// The event endpoint's packets, and how often the computer polls it.
const EVENT_PACKET: u16 = 28;
const EVENT_INTERVAL: u8 = 6;

const BIND: u8 = 0;
const UNBIND: u8 = 1;
const ENABLE: u8 = 2;
const DISABLE: u8 = 3;
const SETUP: u8 = 4;
const SUSPEND: u8 = 5;
const RESUME: u8 = 6;

/// FUNCTIONFS_ENDPOINT_REVMAP: an endpoint file's address on the bus.
const ENDPOINT_REVMAP: libc::c_ulong = 0x6781;
/// FUNCTIONFS_ENDPOINT_DESC: the descriptor an endpoint runs with now, at
/// the speed the computer connected at; nine bytes with the audio fields.
const ENDPOINT_DESC: libc::c_ulong = 0x8009_6782;

// From linux/aio_abi.h.
const IOCB_CMD_PREAD: u16 = 0;
const IOCB_CMD_PWRITE: u16 = 1;
const IOCB_FLAG_RESFD: u32 = 1;

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct InterfaceDesc {
    length: u8,
    descriptor_type: u8,
    interface_number: u8,
    alternate_setting: u8,
    num_endpoints: u8,
    interface_class: u8,
    interface_subclass: u8,
    interface_protocol: u8,
    interface: u8,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct EndpointDesc {
    length: u8,
    descriptor_type: u8,
    endpoint_address: u8,
    attributes: u8,
    max_packet_size: U16,
    interval: u8,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct SsCompanionDesc {
    length: u8,
    descriptor_type: u8,
    max_burst: u8,
    attributes: u8,
    bytes_per_interval: U16,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct Function {
    interface: InterfaceDesc,
    sink: EndpointDesc,
    source: EndpointDesc,
    event: EndpointDesc,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct SsFunction {
    interface: InterfaceDesc,
    sink: EndpointDesc,
    sink_comp: SsCompanionDesc,
    source: EndpointDesc,
    source_comp: SsCompanionDesc,
    event: EndpointDesc,
    event_comp: SsCompanionDesc,
}

/// usb_os_desc_header; `count` is bCount and its reserved byte.
#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct OsDescHeader {
    interface: u8,
    length: U32,
    bcd_version: U16,
    index: U16,
    count: U16,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct ExtCompatDesc {
    first_interface_number: u8,
    reserved1: u8,
    compatible_id: [u8; 8],
    sub_compatible_id: [u8; 8],
    reserved2: [u8; 6],
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct Descriptors {
    magic: U32,
    length: U32,
    flags: U32,
    fs_count: U32,
    hs_count: U32,
    ss_count: U32,
    os_count: U32,
    fs: Function,
    hs: Function,
    ss: SsFunction,
    compat_header: OsDescHeader,
    compat: ExtCompatDesc,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct Strings {
    magic: U32,
    length: U32,
    str_count: U32,
    lang_count: U32,
    lang: U16,
    interface: [u8; 4],
}

/// usb_functionfs_event: the setup packet, then the event's type.
#[derive(FromBytes, KnownLayout, Immutable)]
#[repr(C)]
struct Event {
    request_type: u8,
    request: u8,
    value: U16,
    index: U16,
    length: U16,
    kind: u8,
    pad: [u8; 3],
}

// The sizes the kernel's own definitions come to.
const _: () = assert!(size_of::<Descriptors>() == 171);
const _: () = assert!(size_of::<Strings>() == 22);
const _: () = assert!(size_of::<Event>() == 12);

fn endpoint(address: u8, attributes: u8, max_packet_size: u16, interval: u8) -> EndpointDesc {
    EndpointDesc { length: 7, descriptor_type: DT_ENDPOINT, endpoint_address: address, attributes, max_packet_size: U16::new(max_packet_size), interval }
}

fn interface() -> InterfaceDesc {
    InterfaceDesc {
        length: 9,
        descriptor_type: DT_INTERFACE,
        interface_number: 0,
        alternate_setting: 0,
        num_endpoints: 3,
        interface_class: STILL_IMAGE_CLASS,
        interface_subclass: STILL_IMAGE_SUBCLASS,
        interface_protocol: PTP_PROTOCOL,
        interface: 1,
    }
}

fn function(max_packet_size: u16) -> Function {
    Function {
        interface: interface(),
        sink: endpoint(1 | DIR_IN, XFER_BULK, max_packet_size, 0),
        source: endpoint(2, XFER_BULK, max_packet_size, 0),
        event: endpoint(3 | DIR_IN, XFER_INT, EVENT_PACKET, EVENT_INTERVAL),
    }
}

fn companion(max_burst: u8, bytes_per_interval: u16) -> SsCompanionDesc {
    SsCompanionDesc { length: 6, descriptor_type: DT_SS_ENDPOINT_COMP, max_burst, attributes: 0, bytes_per_interval: U16::new(bytes_per_interval) }
}

fn descriptors() -> Descriptors {
    Descriptors {
        magic: U32::new(DESCRIPTORS_MAGIC_V2),
        length: U32::new(size_of::<Descriptors>() as u32),
        flags: U32::new(HAS_FS_DESC | HAS_HS_DESC | HAS_SS_DESC | HAS_MS_OS_DESC),
        fs_count: U32::new(4),
        hs_count: U32::new(4),
        ss_count: U32::new(7),
        os_count: U32::new(1),
        fs: function(64),
        hs: function(512),
        ss: SsFunction {
            interface: interface(),
            sink: endpoint(1 | DIR_IN, XFER_BULK, 1024, 0),
            sink_comp: companion(6, 0),
            source: endpoint(2, XFER_BULK, 1024, 0),
            source_comp: companion(6, 0),
            event: endpoint(3 | DIR_IN, XFER_INT, EVENT_PACKET, EVENT_INTERVAL),
            event_comp: companion(0, EVENT_PACKET),
        },
        compat_header: OsDescHeader {
            interface: 1,
            length: U32::new((size_of::<OsDescHeader>() + size_of::<ExtCompatDesc>()) as u32),
            bcd_version: U16::new(1),
            index: U16::new(4),
            count: U16::new(1),
        },
        // Windows finds its MTP driver by this.
        compat: ExtCompatDesc { first_interface_number: 0, reserved1: 1, compatible_id: *b"MTP\0\0\0\0\0", sub_compatible_id: [0; 8], reserved2: [0; 6] },
    }
}

fn strings() -> Strings {
    Strings {
        magic: U32::new(STRINGS_MAGIC),
        length: U32::new(size_of::<Strings>() as u32),
        str_count: U32::new(1),
        lang_count: U32::new(1),
        lang: U16::new(0x0409),
        interface: *b"MTP\0",
    }
}

/// struct iocb, laid out little-endian.
#[derive(Debug, Default)]
#[repr(C)]
struct Iocb {
    data: u64,
    key: u32,
    rw_flags: i32,
    opcode: u16,
    reqprio: i16,
    fildes: u32,
    buf: u64,
    nbytes: u64,
    offset: i64,
    reserved2: u64,
    flags: u32,
    resfd: u32,
}

/// struct io_event.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
struct IoEvent {
    data: u64,
    obj: u64,
    res: i64,
    res2: i64,
}

const _: () = assert!(size_of::<Iocb>() == 64);
const _: () = assert!(size_of::<IoEvent>() == 32);

/// The kernel's asynchronous I/O: requests on the endpoints that end on
/// their own time, each end told through an eventfd.
struct Aio {
    ctx: libc::c_ulong,
    done: OwnedFd,
    /// The requests in flight, by the key their ends come back with.
    flying: HashMap<u64, Box<Iocb>>,
    next: u64,
}

impl Aio {
    fn new() -> io::Result<Aio> {
        let mut ctx: libc::c_ulong = 0;
        if unsafe { libc::syscall(libc::SYS_io_setup, 16, &mut ctx) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::syscall(libc::SYS_io_destroy, ctx) };
            return Err(e);
        }
        Ok(Aio { ctx, done: unsafe { OwnedFd::from_raw_fd(fd) }, flying: HashMap::new(), next: 1 })
    }

    /// Start moving `len` bytes at `buf` over `fd`; the key its end comes
    /// with.
    ///
    /// # Safety
    /// The bytes at `buf` stay in place until the request has ended.
    unsafe fn submit(&mut self, fd: RawFd, buf: *mut u8, len: usize, write: bool) -> io::Result<u64> {
        let key = self.next;
        self.next += 1;
        let mut iocb = Box::new(Iocb {
            data: key,
            opcode: if write { IOCB_CMD_PWRITE } else { IOCB_CMD_PREAD },
            fildes: fd as u32,
            buf: buf as u64,
            nbytes: len as u64,
            flags: IOCB_FLAG_RESFD,
            resfd: self.done.as_raw_fd() as u32,
            ..Iocb::default()
        });
        let mut list = [&mut *iocb as *mut Iocb];
        if unsafe { libc::syscall(libc::SYS_io_submit, self.ctx, 1, list.as_mut_ptr()) } != 1 {
            return Err(io::Error::last_os_error());
        }
        self.flying.insert(key, iocb);
        Ok(key)
    }

    /// The requests that ended since last asked: each key and its result,
    /// the bytes moved or an error number below zero.
    fn ended(&mut self) -> io::Result<Vec<(u64, i64)>> {
        let mut count = 0u64;
        unsafe { libc::read(self.done.as_raw_fd(), (&mut count as *mut u64).cast(), 8) };
        let mut out = Vec::new();
        let mut events = [IoEvent::default(); 16];
        loop {
            let zero = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            let n = unsafe { libc::syscall(libc::SYS_io_getevents, self.ctx, 0, events.len(), events.as_mut_ptr(), &zero) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            for ev in &events[..n as usize] {
                self.flying.remove(&ev.data);
                out.push((ev.data, ev.res));
            }
            if (n as usize) < events.len() {
                return Ok(out);
            }
        }
    }

    /// Ask the request to end at once; its end comes as any other's does.
    fn cancel(&mut self, key: u64) {
        let Some(iocb) = self.flying.get_mut(&key) else { return };
        let mut result = IoEvent::default();
        // The kernel answers that cancelling is under way; a request that
        // ended meanwhile is not found, and its end is on the way already.
        unsafe { libc::syscall(libc::SYS_io_cancel, self.ctx, &mut **iocb as *mut Iocb, &mut result) };
    }
}

impl Drop for Aio {
    /// Waits for every request still in flight.
    fn drop(&mut self) {
        unsafe { libc::syscall(libc::SYS_io_destroy, self.ctx) };
    }
}

/// Why a transfer did not end as asked.
#[derive(Debug)]
pub enum Halt {
    /// The computer cancelled the transaction.
    Cancelled,
    /// The function went away: the cable came out, or the computer
    /// configured the port again.
    Disconnected,
    /// The phone asked to stop the transfer.
    Stopped,
    /// The transaction was given up, its endpoints halted: the file it
    /// moved could not be read, or the computer broke off a transfer.
    Aborted(io::Error),
    /// The function itself failed.
    Failed(io::Error),
}

impl From<io::Error> for Halt {
    fn from(e: io::Error) -> Halt {
        Halt::Failed(e)
    }
}

/// Put `line` in the kernel log, where the recovery's own lines go; a line
/// that cannot go there is not worth stopping for.
fn note(line: &str) {
    let _ = skyward_sys::kmsg(&format!("mtp: {line}"));
}

/// How waiting for a request ended, short of the request ending.
enum Interrupted {
    Cancel,
    Disconnect,
    /// The phone's wake pipe has something.
    Woken,
    /// The other descriptor waited on has something.
    Changed,
}

/// What came while waiting for a command.
pub enum Next {
    Command(Vec<u8>),
    /// The folders followed changed.
    Changed,
    /// The phone has something to ask.
    Woken,
}

/// What came on the control endpoint.
enum Control {
    Enabled,
    Disabled,
    Cancel,
}

/// The MTP function: its control endpoint, its three others, and the
/// transfers on them.
pub struct Port {
    // The endpoints close before ep0, so the function can go.
    bulk_in: File,
    bulk_out: File,
    event_in: File,
    aio: Aio,
    ep0: File,
    /// Readable while the phone has something to ask of the function.
    wake: RawFd,
    /// The computer has configured the function, so its endpoints work.
    enabled: bool,
    /// A packet's size on the bulk endpoints at the speed connected at.
    packet: usize,
    /// The device status reports a cancelled transaction, once.
    cancelled: bool,
    /// The command being read, kept while the phone attends to something
    /// else.
    reading: Option<(u64, Box<[u8; COMMAND_MAX]>)>,
    /// Requests that ended while some other one was waited for, kept for
    /// whoever asks after them: a command that came during a transfer.
    spare: HashMap<u64, i64>,
    events: VecDeque<[u8; 16]>,
    /// The event going, which ends once the computer polls for it.
    event: Option<(u64, Box<[u8; 16]>)>,
}

impl Port {
    /// Hand the kernel the function's descriptors, which it needs before
    /// init can bind the function, and open its endpoints; `wake` is what
    /// the phone wakes the function's thread with.
    pub fn open(wake: RawFd) -> Result<Port> {
        let ep0 = OpenOptions::new().read(true).write(true).custom_flags(libc::O_CLOEXEC).open(EP0).with_context(|| format!("open {EP0}"))?;
        (&ep0).write_all(descriptors().as_bytes()).context("write the MTP descriptors")?;
        (&ep0).write_all(strings().as_bytes()).context("write the MTP strings")?;
        // Without blocking, a transfer started as the function goes fails
        // at once instead of waiting for it to come back.
        let open = |path: &str, write: bool| {
            OpenOptions::new()
                .read(!write)
                .write(write)
                .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(path)
                .with_context(|| format!("open {path}"))
        };
        Ok(Port {
            bulk_in: open(EP_IN, true)?,
            bulk_out: open(EP_OUT, false)?,
            event_in: open(EP_EVENT, true)?,
            aio: Aio::new().context("set up asynchronous I/O")?,
            ep0,
            wake,
            enabled: false,
            packet: 512,
            cancelled: false,
            reading: None,
            spare: HashMap::new(),
            events: VecDeque::new(),
            event: None,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Wait until the computer configures the function, or the phone has
    /// something to ask; whether the computer did.
    pub fn wait_enabled(&mut self) -> Result<bool> {
        while !self.enabled {
            let mut fds = [
                libc::pollfd { fd: self.ep0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.wake, events: libc::POLLIN, revents: 0 },
            ];
            if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(e).context("wait for the computer");
            }
            if fds[0].revents != 0 {
                self.control().context("answer the control endpoint")?;
            }
            if fds[1].revents != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Read one event from the control endpoint and answer it.
    fn control(&mut self) -> io::Result<Option<Control>> {
        let mut buf = [0u8; size_of::<Event>()];
        let n = (&self.ep0).read(&mut buf)?;
        if n != buf.len() {
            return Err(io::Error::new(ErrorKind::InvalidData, format!("a control event of {n} bytes")));
        }
        let ev = Event::read_from_bytes(&buf).expect("the buffer is one event");
        Ok(match ev.kind {
            ENABLE => {
                self.enabled = true;
                self.packet = self.packet_size()?;
                Some(Control::Enabled)
            }
            DISABLE | UNBIND => {
                self.enabled = false;
                Some(Control::Disabled)
            }
            SETUP => self.setup(&ev)?,
            BIND => None,
            // The computer leaves the bus alone for a while and comes
            // back: what was under way is lost, and the transfer starts
            // again once it is listening.
            SUSPEND => {
                self.enabled = false;
                note("the computer left the bus");
                Some(Control::Disabled)
            }
            RESUME => {
                self.packet = self.packet_size().unwrap_or(self.packet);
                self.enabled = true;
                note("the computer came back to the bus");
                Some(Control::Enabled)
            }
            other => {
                note(&format!("passed over control event {other}"));
                None
            }
        })
    }

    /// Answer a control request to the interface: a cancel, a reset, or how
    /// the device stands. Any other the computer sends is taken, and one
    /// that asks for data is answered with zeros, as the stock system
    /// answers it.
    fn setup(&mut self, ev: &Event) -> io::Result<Option<Control>> {
        let length = usize::from(ev.length.get());
        let class = ev.request_type & TYPE_MASK == TYPE_CLASS && ev.index.get() == 0 && ev.value.get() == 0;
        let into_device = ev.request_type & DIR_IN == 0;
        match (class, ev.request) {
            (true, request::CANCEL | request::RESET) if into_device => {
                note(&format!("control: the computer asks {:#04x}", ev.request));
                let mut data = vec![0u8; length];
                (&self.ep0).read_exact(&mut data)?;
                Ok(Some(Control::Cancel))
            }
            (true, request::GET_DEVICE_STATUS) if !into_device => {
                let mut status = Vec::new();
                if self.cancelled {
                    // Which endpoints the cancelling halted, for the
                    // computer to clear.
                    let ends = [&self.bulk_in, &self.bulk_out].map(|f| unsafe { libc::ioctl(f.as_raw_fd(), ENDPOINT_REVMAP as _) } as u16);
                    status.extend(8u16.to_le_bytes());
                    status.extend(response::TRANSACTION_CANCELLED.to_le_bytes());
                    for end in ends {
                        status.extend(end.to_le_bytes());
                    }
                    self.cancelled = false;
                } else {
                    status.extend(4u16.to_le_bytes());
                    status.extend(response::OK.to_le_bytes());
                }
                status.truncate(length);
                (&self.ep0).write_all(&status)?;
                Ok(None)
            }
            _ => {
                note(&format!("took control request {:#04x} of type {:#04x}, {length} bytes", ev.request, ev.request_type));
                let mut data = vec![0u8; length];
                if into_device {
                    (&self.ep0).read_exact(&mut data)?;
                } else {
                    (&self.ep0).write_all(&data)?;
                }
                Ok(None)
            }
        }
    }

    /// The bulk endpoints' packet size at the speed connected at.
    fn packet_size(&self) -> io::Result<usize> {
        let mut desc = [0u8; 9];
        if unsafe { libc::ioctl(self.bulk_in.as_raw_fd(), ENDPOINT_DESC as _, desc.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(usize::from(u16::from_le_bytes([desc[4], desc[5]]) & 0x7ff))
    }

    /// Start moving `len` bytes at `buf`, to the computer with `write`.
    ///
    /// # Safety
    /// The bytes at `buf` stay in place until the request has ended.
    unsafe fn start(&mut self, buf: *mut u8, len: usize, write: bool) -> Result<u64, Halt> {
        let fd = if write { self.bulk_in.as_raw_fd() } else { self.bulk_out.as_raw_fd() };
        match unsafe { self.aio.submit(fd, buf, len, write) } {
            Ok(key) => Ok(key),
            Err(e) if matches!(e.raw_os_error(), Some(libc::EAGAIN | libc::ESHUTDOWN | libc::ECONNRESET)) => {
                self.unconfigured();
                Err(Halt::Disconnected)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// What a request's result comes to: the bytes it moved, or why none
    /// were.
    fn moved(&mut self, res: i64) -> Result<usize, Halt> {
        match res {
            r if r >= 0 => Ok(r as usize),
            r if r == -i64::from(libc::ESHUTDOWN)
                || r == -i64::from(libc::EAGAIN)
                || r == -i64::from(libc::ECONNRESET) =>
            {
                self.unconfigured();
                Err(Halt::Disconnected)
            }
            r => Err(Halt::Aborted(io::Error::from_raw_os_error(-r as i32))),
        }
    }

    /// The endpoints say the computer no longer has the function
    /// configured, which the control endpoint says too, a moment later:
    /// the port waits for it to be configured again.
    fn unconfigured(&mut self) {
        self.enabled = false;
    }

    /// Wait until request `key` ends: its result. Meanwhile the control
    /// endpoint is answered and events go as the computer takes them; a
    /// cancel, the function going, the phone waking the thread or `also`
    /// having something ends the wait with the request still in flight.
    fn finish(&mut self, key: u64, also: Option<RawFd>) -> Result<Result<i64, Interrupted>, Halt> {
        if let Some(res) = self.spare.remove(&key) {
            return Ok(Ok(res));
        }
        loop {
            let mut result = None;
            for (k, res) in self.aio.ended()? {
                if k == key {
                    result = Some(res);
                } else if self.event.as_ref().is_some_and(|(e, _)| *e == k) {
                    self.event = None;
                    self.next_event()?;
                } else {
                    // Keep it: the command it answers is asked for later,
                    // and a result thrown away would leave that wait on a
                    // request that has already ended.
                    if k == self.reading.as_ref().map_or(u64::MAX, |(r, _)| *r) {
                        note(&format!("command read ended {res} while waiting for {key}"));
                    }
                    self.spare.insert(k, res);
                }
            }
            if let Some(res) = result {
                return Ok(Ok(res));
            }
            // poll passes over a descriptor below zero.
            let mut fds = [
                libc::pollfd { fd: self.aio.done.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.ep0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.wake, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: also.unwrap_or(-1), events: libc::POLLIN, revents: 0 },
            ];
            if unsafe { libc::poll(fds.as_mut_ptr(), 4, -1) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.into());
            }
            if fds[1].revents != 0 {
                match self.control() {
                    Ok(Some(Control::Cancel)) => return Ok(Err(Interrupted::Cancel)),
                    Ok(Some(Control::Disabled)) => return Ok(Err(Interrupted::Disconnect)),
                    Ok(Some(Control::Enabled)) | Ok(None) => {}
                    // The port went with the function while a transfer was
                    // under way: nothing of it is left to wait for.
                    Err(e) if matches!(e.raw_os_error(), Some(libc::EAGAIN | libc::ESHUTDOWN | libc::ECONNRESET)) => {
                        self.unconfigured();
                        return Ok(Err(Interrupted::Disconnect));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            if fds[2].revents != 0 {
                return Ok(Err(Interrupted::Woken));
            }
            if fds[3].revents != 0 {
                return Ok(Err(Interrupted::Changed));
            }
        }
    }

    /// Give up request `key`, and wait until it has ended, so what it
    /// moves can go. With `stall` the bulk endpoints halt after it, as
    /// cancelling a transaction asks, and the device status says so.
    fn abort(&mut self, key: u64, stall: bool) -> io::Result<()> {
        self.aio.cancel(key);
        while self.aio.flying.contains_key(&key) {
            let mut fds = [libc::pollfd { fd: self.aio.done.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
            if unsafe { libc::poll(fds.as_mut_ptr(), 1, -1) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != ErrorKind::Interrupted {
                    return Err(e);
                }
            }
            for (k, res) in self.aio.ended()? {
                if self.event.as_ref().is_some_and(|(e, _)| *e == k) {
                    self.event = None;
                } else if k != key {
                    self.spare.insert(k, res);
                }
            }
        }
        if stall {
            self.halt();
        }
        Ok(())
    }

    /// Halt both bulk endpoints, as cancelling a transaction asks, and say
    /// so in the device status. Reading the endpoint that sends, or writing
    /// the one that receives, halts it; one gone with the function needs
    /// no halting.
    fn halt(&mut self) {
        unsafe {
            libc::read(self.bulk_in.as_raw_fd(), std::ptr::null_mut(), 0);
            libc::write(self.bulk_out.as_raw_fd(), std::ptr::null(), 0);
        }
        self.cancelled = true;
    }

    /// Wait for request `key` of a transaction: the bytes it moved. A
    /// cancel gives it up, as does the function going, or the phone
    /// waking the thread when `stop` says that ends the transfer.
    fn wait(&mut self, key: u64, stop: &mut dyn FnMut() -> bool) -> Result<usize, Halt> {
        loop {
            let interrupted = match self.finish(key, None) {
                Ok(Ok(res)) => {
                    return match self.moved(res) {
                        Err(Halt::Aborted(e)) => {
                            self.abort(key, true)?;
                            Err(Halt::Aborted(e))
                        }
                        other => other,
                    };
                }
                Ok(Err(interrupted)) => interrupted,
                Err(halt) => {
                    self.abort(key, false)?;
                    return Err(halt);
                }
            };
            match interrupted {
                Interrupted::Changed => unreachable!("a transfer waits on nothing else"),
                Interrupted::Woken if !stop() => {}
                Interrupted::Woken => {
                    self.abort(key, true)?;
                    return Err(Halt::Stopped);
                }
                Interrupted::Cancel => {
                    self.abort(key, true)?;
                    return Err(Halt::Cancelled);
                }
                Interrupted::Disconnect => {
                    self.abort(key, false)?;
                    return Err(Halt::Disconnected);
                }
            }
        }
    }

    fn transfer(&mut self, buf: &mut [u8], write: bool, stop: &mut dyn FnMut() -> bool) -> Result<usize, Halt> {
        // Safety: `buf` is borrowed until the wait has seen the request end.
        let key = unsafe { self.start(buf.as_mut_ptr(), buf.len(), write)? };
        self.wait(key, stop)
    }

    /// The next command, or what came before it: the phone waking the
    /// thread, or `watch` having something. The read stays in flight for
    /// the next call. A packet of nothing, which ends the data before it,
    /// is passed over, as is a cancel while no transaction is under way.
    pub fn command(&mut self, watch: Option<RawFd>) -> Result<Next, Halt> {
        loop {
            let key = match &self.reading {
                Some((key, _)) => *key,
                None => {
                    let mut buf = Box::new([0u8; COMMAND_MAX]);
                    // Safety: the box is kept with the request until it ends.
                    let key = unsafe { self.start(buf.as_mut_ptr(), COMMAND_MAX, false)? };
                    self.reading = Some((key, buf));
                    key
                }
            };
            match self.finish(key, watch) {
                Ok(Ok(res)) => {
                    let (_, buf) = self.reading.take().expect("the command read was under way");
                    let n = self.moved(res)?;
                    if n > 0 {
                        return Ok(Next::Command(buf[..n].to_vec()));
                    }
                }
                Ok(Err(Interrupted::Cancel)) => {
                    // The computer gave up on what it was asking: the read
                    // ends with it and the endpoints halt, as a cancel
                    // asks, and the next command starts afresh.
                    self.abort(key, true)?;
                    self.reading = None;
                }
                Ok(Err(Interrupted::Woken)) => return Ok(Next::Woken),
                Ok(Err(Interrupted::Changed)) => return Ok(Next::Changed),
                Ok(Err(Interrupted::Disconnect)) => {
                    self.stop_reading()?;
                    return Err(Halt::Disconnected);
                }
                Err(halt) => {
                    self.stop_reading()?;
                    return Err(halt);
                }
            }
        }
    }

    /// Give up the command being read.
    pub fn stop_reading(&mut self) -> io::Result<()> {
        if let Some((key, _)) = &self.reading {
            let key = *key;
            self.abort(key, false)?;
        }
        self.reading = None;
        Ok(())
    }

    /// Send `bytes` whole, ending with a packet of nothing where they fill
    /// their last packet, so the computer knows the data is complete.
    fn send_whole(&mut self, bytes: &mut [u8], stop: &mut dyn FnMut() -> bool) -> Result<(), Halt> {
        for piece in bytes.chunks_mut(CHUNK) {
            self.transfer(piece, true, stop)?;
        }
        if bytes.len().is_multiple_of(self.packet) {
            self.transfer(&mut [], true, stop)?;
        }
        Ok(())
    }

    /// A data phase with `payload`.
    pub fn send_data(&mut self, code: u16, transaction: u32, payload: &[u8], stop: &mut dyn FnMut() -> bool) -> Result<(), Halt> {
        let mut bytes = dataset::data_header(code, transaction, payload.len() as u64).to_vec();
        bytes.extend_from_slice(payload);
        self.send_whole(&mut bytes, stop)
    }

    /// A response, which fits in a packet.
    pub fn send_response(&mut self, code: u16, transaction: u32, params: &[u32], stop: &mut dyn FnMut() -> bool) -> Result<(), Halt> {
        let mut bytes = dataset::short(container::RESPONSE, code, transaction, params);
        self.send_whole(&mut bytes, stop)
    }

    /// A data phase with `length` bytes of `file` from `offset`, each piece
    /// read from the file while the one before is on the bus. A file
    /// shorter than it was a moment ago halts the transfer.
    #[allow(clippy::too_many_arguments)]
    pub fn send_file(&mut self, code: u16, transaction: u32, file: &File, offset: u64, length: u64, stop: &mut dyn FnMut() -> bool) -> Result<(), Halt> {
        let mut bufs = [vec![0u8; FILE_CHUNK], vec![0u8; FILE_CHUNK]];
        let end = offset + length;
        let mut at = offset;
        // Fill a buffer from `start` with what comes next of the file: how
        // far it is full.
        let fill = |buf: &mut [u8], start: usize, at: &mut u64| -> io::Result<usize> {
            let n = (buf.len() - start).min((end - *at) as usize);
            file.read_exact_at(&mut buf[start..start + n], *at)?;
            *at += n as u64;
            Ok(start + n)
        };
        bufs[0][..HEADER].copy_from_slice(&dataset::data_header(code, transaction, length));
        // Nothing is on the bus yet: the computer waits for the data phase,
        // and halting the endpoints tells it there is none.
        let mut len = match fill(&mut bufs[0], HEADER, &mut at) {
            Ok(n) => n,
            Err(e) => {
                self.halt();
                return Err(Halt::Aborted(e));
            }
        };
        let mut cur = 0;
        // How much of the file went out, so that a transfer that stops part
        // way says where it stopped rather than only that it did.
        let mut sent: u64 = 0;
        loop {
            let (this, other) = match bufs.split_at_mut(1) {
                (a, b) if cur == 0 => (&mut a[0], &mut b[0]),
                (a, b) => (&mut b[0], &mut a[0]),
            };
            // Safety: `this` stays put until the wait, or giving the
            // request up, has seen it end.
            let key = unsafe { self.start(this.as_mut_ptr(), len, true)? };
            let next = if at < end {
                match fill(other, 0, &mut at) {
                    Ok(n) => n,
                    Err(e) => {
                        self.abort(key, true)?;
                        return Err(Halt::Aborted(e));
                    }
                }
            } else {
                0
            };
            if let Err(e) = self.wait(key, stop) {
                note(&format!("send {code:#06x} txn {transaction} {sent} bytes then {e:?}"));
                return Err(e);
            }
            sent += len as u64;
            if next == 0 {
                break;
            }
            len = next;
            cur = 1 - cur;
        }
        note(&format!("send {code:#06x} txn {transaction} done {sent}"));
        note(&format!("send {code:#06x} txn {transaction} done {sent}"));
        if (HEADER as u64 + length).is_multiple_of(self.packet as u64) {
            self.transfer(&mut [], true, stop)?;
        }
        Ok(())
    }

    /// Read a data phase from the computer: its header, then the payload,
    /// handed to `sink` piece by piece while the next is on the bus. The
    /// computer may send it in several transfers, the header alone in the
    /// first: it ends once it is as long as the header says, or, past 4 GiB
    /// or `unbounded`, with a short packet after the first transfer. Once
    /// `sink` fails, the rest is read and dropped: the header, the bytes
    /// `sink` took, and its error.
    pub fn receive(
        &mut self,
        stop: &mut dyn FnMut() -> bool,
        unbounded: bool,
        sink: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<(Header, u64, Option<io::Error>), Halt> {
        let mut bufs = [vec![0u8; CHUNK], vec![0u8; CHUNK]];
        // The first piece, past any packet of nothing that ended the data
        // before.
        let first = loop {
            let n = self.transfer(&mut bufs[0], false, stop)?;
            if n > 0 {
                break n;
            }
        };
        let Some(header) = Header::read(&bufs[0][..first]).filter(|h| h.kind == container::DATA) else {
            // What came is no data phase: the transaction is given up.
            self.halt();
            return Err(Halt::Aborted(io::Error::new(ErrorKind::InvalidData, format!("{first} bytes came where a data phase starts"))));
        };
        // How much payload is still to come, where the header says.
        let mut left = (!unbounded && header.length != u32::MAX).then(|| u64::from(header.length).saturating_sub(HEADER as u64));
        // The part of `got` bytes that is payload, and whether more comes.
        let take = |got: usize, full: bool, left: &mut Option<u64>| -> (usize, bool) {
            match left {
                Some(l) => {
                    let n = got.min(*l as usize);
                    *l -= n as u64;
                    (n, *l > 0)
                }
                None => (got, full),
            }
        };
        let (mut cur, mut from) = (0, HEADER);
        // Past 4 GiB the first transfer ends short whatever follows: the
        // computer sends the header with a little data in a packet of its
        // own, short at the faster speeds.
        let (mut len, mut more) = take(first - HEADER, true, &mut left);
        let (mut taken, mut failed) = (0u64, None);
        loop {
            // The next piece goes on the bus while this one goes to `sink`.
            let next = if more {
                let asked = match left {
                    Some(l) => (l as usize).min(CHUNK).next_multiple_of(self.packet).min(CHUNK),
                    None => CHUNK,
                };
                // Safety: the other buffer stays put until the wait below,
                // or giving the request up, has seen it end.
                Some((unsafe { self.start(bufs[1 - cur].as_mut_ptr(), asked, false)? }, asked))
            } else {
                None
            };
            if failed.is_none() {
                match sink(&bufs[cur][from..from + len]) {
                    Ok(()) => taken += len as u64,
                    Err(e) => failed = Some(e),
                }
            }
            let Some((key, asked)) = next else { break };
            let got = self.wait(key, stop)?;
            (len, more) = take(got, got == asked, &mut left);
            (cur, from) = (1 - cur, 0);
        }
        Ok((header, taken, failed))
    }

    /// Tell the computer `code` about `param`, once it polls; events wait
    /// their turn behind the one going.
    pub fn event(&mut self, code: u16, transaction: u32, param: u32) -> io::Result<()> {
        let bytes: [u8; 16] = dataset::short(container::EVENT, code, transaction, &[param]).try_into().expect("an event with one parameter is sixteen bytes");
        self.events.push_back(bytes);
        if self.event.is_none() {
            self.next_event()?;
        }
        Ok(())
    }

    fn next_event(&mut self) -> io::Result<()> {
        if !self.enabled {
            self.events.clear();
            return Ok(());
        }
        let Some(bytes) = self.events.pop_front() else { return Ok(()) };
        let mut boxed = Box::new(bytes);
        let fd = self.event_in.as_raw_fd();
        // Safety: the box goes with the request and is kept until it ends.
        match unsafe { self.aio.submit(fd, boxed.as_mut_ptr(), boxed.len(), true) } {
            Ok(key) => {
                self.event = Some((key, boxed));
                Ok(())
            }
            Err(e) if matches!(e.raw_os_error(), Some(libc::EAGAIN | libc::ESHUTDOWN)) => {
                self.events.clear();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// The connection ended: events waiting go, the one going and the
    /// command being read are given up.
    pub fn disconnected(&mut self) -> io::Result<()> {
        self.events.clear();
        if let Some((key, _)) = &self.event {
            let key = *key;
            self.abort(key, false)?;
        }
        self.event = None;
        self.stop_reading()?;
        self.cancelled = false;
        Ok(())
    }
}
