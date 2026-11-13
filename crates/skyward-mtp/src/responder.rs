//! What the phone answers each operation of an MTP session, apart from how
//! the answers travel: the device and its properties, the storage it
//! offers, and the files and folders in that, which the computer reads,
//! writes, renames, moves, copies and deletes.

use crate::codes::{device, event, form, format, kind, op, prop, response, storage, PARENT_ROOT};
use crate::dataset::{Command, Reader, Writer};
use crate::store::{self, Change, Store, TOP};
use crate::{format as formats, time};
use jiff::tz::TimeZone;
use skyward_block::mount;
use skyward_files::{Clash, Ownership};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The storage the phone offers, the first of the first.
pub const STORAGE_ID: u32 = 0x0001_0001;

/// The extensions the device info names: Microsoft's, which property
/// lists and the like follow, and Android's own operations.
const EXTENSIONS: &str = "microsoft.com: 1.0; android.com: 1.0;";

/// The standard's version, 1.00.
const STANDARD_VERSION: u16 = 100;

/// Microsoft's vendor extension, which the extensions above extend.
const MICROSOFT: u32 = 6;

/// A mobile handset, for the computer to show the phone as one.
const MOBILE_HANDSET: u32 = 3;

/// How many more objects the storage could take, which the standard asks
/// for and a filesystem does not count.
const FREE_OBJECTS: u32 = 1 << 30;

/// The longest dataset taken from the computer whole.
pub const DATASET_MAX: usize = 64 * 1024;

const OPERATIONS: &[u16] = &[
    op::GET_DEVICE_INFO,
    op::OPEN_SESSION,
    op::CLOSE_SESSION,
    op::GET_STORAGE_IDS,
    op::GET_STORAGE_INFO,
    op::GET_NUM_OBJECTS,
    op::GET_OBJECT_HANDLES,
    op::GET_OBJECT_INFO,
    op::GET_OBJECT,
    op::DELETE_OBJECT,
    op::SEND_OBJECT_INFO,
    op::SEND_OBJECT,
    op::RESET_DEVICE,
    op::GET_DEVICE_PROP_DESC,
    op::GET_DEVICE_PROP_VALUE,
    op::SET_DEVICE_PROP_VALUE,
    op::RESET_DEVICE_PROP_VALUE,
    op::MOVE_OBJECT,
    op::COPY_OBJECT,
    op::GET_PARTIAL_OBJECT,
    op::GET_OBJECT_PROPS_SUPPORTED,
    op::GET_OBJECT_PROP_DESC,
    op::GET_OBJECT_PROP_VALUE,
    op::SET_OBJECT_PROP_VALUE,
    op::GET_OBJECT_PROP_LIST,
    op::GET_PARTIAL_OBJECT_64,
    op::SEND_PARTIAL_OBJECT,
    op::TRUNCATE_OBJECT,
    op::BEGIN_EDIT_OBJECT,
    op::END_EDIT_OBJECT,
];

const EVENTS: &[u16] = &[
    event::OBJECT_ADDED,
    event::OBJECT_REMOVED,
    event::STORE_ADDED,
    event::STORE_REMOVED,
    event::DEVICE_PROP_CHANGED,
    event::OBJECT_INFO_CHANGED,
];

const DEVICE_PROPERTIES: &[u16] = &[
    device::SYNCHRONIZATION_PARTNER,
    device::DEVICE_FRIENDLY_NAME,
    device::BATTERY_LEVEL,
    device::PERCEIVED_DEVICE_TYPE,
    device::SESSION_INITIATOR_VERSION_INFO,
];

/// The properties every object has, file or folder.
const OBJECT_PROPERTIES: &[u16] = &[
    prop::STORAGE_ID,
    prop::OBJECT_FORMAT,
    prop::PROTECTION_STATUS,
    prop::OBJECT_SIZE,
    prop::OBJECT_FILE_NAME,
    prop::DATE_MODIFIED,
    prop::PERSISTENT_UID,
    prop::PARENT_OBJECT,
    prop::NAME,
    prop::DISPLAY_NAME,
    prop::DATE_ADDED,
];

/// What the phone tells the computer about itself.
pub struct Device {
    pub manufacturer: String,
    pub model: String,
    pub version: String,
    pub serial: String,
    /// The name the computer shows the phone by, until it gives it another.
    pub name: String,
    /// The battery's charge in percent, read when asked.
    pub battery: Box<dyn Fn() -> Option<u8> + Send>,
}

/// The storage offered: its top folder, and what the computer calls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    pub top: PathBuf,
    pub description: String,
}

/// What goes to the computer after a command: data, if the operation has
/// any, then the response.
#[derive(Debug)]
pub struct Reply {
    pub data: Option<Outgoing>,
    pub code: u16,
    pub params: Vec<u32>,
    /// Why the operation failed, for the log.
    pub why: Option<String>,
}

impl Reply {
    fn ok() -> Reply {
        Reply::code(response::OK)
    }

    fn code(code: u16) -> Reply {
        Reply { data: None, code, params: Vec::new(), why: None }
    }

    pub(crate) fn failed(code: u16, why: impl Display) -> Reply {
        Reply { why: Some(why.to_string()), ..Reply::code(code) }
    }

    fn dataset(w: Writer) -> Reply {
        Reply { data: Some(Outgoing::Dataset(w.into_bytes())), ..Reply::ok() }
    }

    fn params(mut self, params: &[u32]) -> Reply {
        self.params = params.to_vec();
        self
    }
}

#[derive(Debug)]
pub enum Outgoing {
    Dataset(Vec<u8>),
    /// `length` bytes of `file` from `offset`.
    File { file: File, offset: u64, length: u64 },
}

/// The data the computer sends with a command.
#[derive(Debug)]
pub enum Incoming {
    /// The operation has none.
    None,
    /// A dataset, read whole and handed to [`Responder::answer`].
    Dataset,
    /// A file's contents, written into `file` from `offset` as they come;
    /// [`Responder::received`] answers once they have. With `unbounded`,
    /// the object holds 4 GiB or more, and its contents run up to a short
    /// packet whatever length their header gives, as the stock system
    /// reads them: computers cut that length to 32 bits.
    File { file: File, offset: u64, unbounded: bool },
    /// The operation is refused: what comes is read and dropped, and this
    /// answers it.
    Refused(Reply),
}

/// How the contents of a file the computer sent went.
#[derive(Debug)]
pub enum Received {
    /// This many bytes came, every one written.
    Done(u64),
    /// Writing failed; what came after was read and dropped.
    Failed(io::Error),
    /// The computer cancelled, or the connection or the storage went: no
    /// answer goes.
    Stopped,
}

/// Something the computer is told without asking: its code and the object,
/// storage or property it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub code: u16,
    pub param: u32,
}

/// A file SendObjectInfo made, for the SendObject that fills it.
struct Sending {
    handle: u32,
    path: PathBuf,
    /// Open until its contents come; a folder has none.
    file: Option<File>,
    /// What SendObjectInfo said it holds; the largest size where it holds
    /// 4 GiB or more.
    size: u32,
    modified: Option<i64>,
}

/// A file opened to be changed in place.
struct Editing {
    file: File,
    size: u64,
}

pub struct Responder {
    device: Device,
    zone: TimeZone,
    offered: Option<Storage>,
    /// The storage's entries, while a session is open with it.
    store: Option<Store>,
    session: Option<u32>,
    sending: Option<Sending>,
    editing: HashMap<u32, Editing>,
    partner: String,
    name: String,
    initiator: String,
    /// Set when the phone needs the storage back: copying, moving and
    /// deleting stop between two files, or two chunks of one.
    stop: Arc<AtomicBool>,
}

/// What an object is, as read from the disk.
struct Object {
    name: OsString,
    folder: bool,
    parent: u32,
    meta: fs::Metadata,
    relative: PathBuf,
}

impl Object {
    fn format(&self) -> u16 {
        formats::of(&self.name, self.folder)
    }

    fn size(&self) -> u64 {
        if self.folder {
            0
        } else {
            self.meta.len()
        }
    }

    fn name(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }

    /// Unique to the entry and its last change, the same in every session.
    fn persistent_id(&self) -> u128 {
        use std::os::unix::ffi::OsStrExt;
        let hash = self.relative.as_os_str().as_bytes().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        u128::from(hash) << 64 | u128::from(self.meta.mtime() as u64)
    }
}

impl Responder {
    pub fn new(device: Device, zone: TimeZone, stop: Arc<AtomicBool>) -> Responder {
        let name = device.name.clone();
        Responder {
            device,
            zone,
            offered: None,
            store: None,
            session: None,
            sending: None,
            editing: HashMap::new(),
            partner: String::new(),
            name,
            initiator: String::new(),
            stop,
        }
    }

    /// The zone the times of files are told in.
    pub fn set_zone(&mut self, zone: TimeZone) {
        self.zone = zone;
    }

    /// Offer `storage`, or none: whatever is under way in the storage
    /// offered so far ends, and an open session is told. The entries of a
    /// storage keep their handles while it is offered, from one session,
    /// and one connection, to the next.
    pub fn offer(&mut self, storage: Option<Storage>) -> io::Result<Vec<Event>> {
        let had = self.offered.is_some();
        self.sending = None;
        self.editing.clear();
        self.store = storage.as_ref().map(|s| Store::new(s.top.clone())).transpose()?;
        self.offered = storage;
        let mut told = Vec::new();
        if self.session.is_none() {
            return Ok(told);
        }
        if had {
            told.push(Event { code: event::STORE_REMOVED, param: STORAGE_ID });
        }
        if self.offered.is_some() {
            told.push(Event { code: event::STORE_ADDED, param: STORAGE_ID });
        }
        Ok(told)
    }

    /// The connection ended, and with it the session.
    pub fn disconnected(&mut self) {
        self.close();
    }

    /// What to wait on for changes other programs make to the storage.
    pub fn fd(&self) -> Option<RawFd> {
        self.store.as_ref().and_then(Store::fd)
    }

    /// What other programs changed in the folders the computer has read,
    /// as events for an open session; with none open, the entries follow
    /// the changes all the same.
    pub fn changes(&mut self) -> Vec<Event> {
        let Some(store) = &mut self.store else { return Vec::new() };
        let changes = store.changes();
        let told: Vec<Event> = changes
            .into_iter()
            .map(|change| match change {
                Change::Added(h) => Event { code: event::OBJECT_ADDED, param: h },
                Change::Removed(h) => {
                    self.editing.remove(&h);
                    Event { code: event::OBJECT_REMOVED, param: h }
                }
                Change::Written(h) => Event { code: event::OBJECT_INFO_CHANGED, param: h },
            })
            .collect();
        if self.session.is_some() {
            told
        } else {
            Vec::new()
        }
    }

    /// End the session: files half sent stay as far as they came, files
    /// edited keep what was written.
    fn close(&mut self) {
        self.session = None;
        self.sending = None;
        self.editing.clear();
    }

    /// What data the computer sends with `cmd`. The file SendObjectInfo
    /// made waits only for the SendObject right after it.
    pub fn incoming(&mut self, cmd: &Command) -> Incoming {
        if cmd.code != op::SEND_OBJECT {
            self.sending = None;
        }
        match cmd.code {
            op::SEND_OBJECT_INFO | op::SET_OBJECT_PROP_VALUE | op::SET_DEVICE_PROP_VALUE => Incoming::Dataset,
            op::SET_OBJECT_REFERENCES => Incoming::Refused(Reply::code(response::OPERATION_NOT_SUPPORTED)),
            op::SEND_OBJECT => self.object_sink(),
            op::SEND_PARTIAL_OBJECT => self.partial_sink(cmd),
            _ => Incoming::None,
        }
    }

    fn object_sink(&mut self) -> Incoming {
        if self.session.is_none() {
            return Incoming::Refused(Reply::code(response::SESSION_NOT_OPEN));
        }
        let Some(sending) = &mut self.sending else {
            return Incoming::Refused(Reply::failed(response::NO_VALID_OBJECT_INFO, "SendObject came with no SendObjectInfo before it"));
        };
        match sending.file.take() {
            Some(file) => Incoming::File { file, offset: 0, unbounded: sending.size == u32::MAX },
            // A folder sends nothing, which is read as a dataset.
            None => Incoming::Dataset,
        }
    }

    fn partial_sink(&mut self, cmd: &Command) -> Incoming {
        let Some([h, low, high, _]) = cmd.take::<4>() else { return Incoming::Refused(Reply::code(response::INVALID_PARAMETER)) };
        let offset = u64::from(low) | u64::from(high) << 32;
        let Some(edit) = self.editing.get(&h) else {
            return Incoming::Refused(Reply::failed(response::GENERAL_ERROR, format!("object {h} is not open to be edited")));
        };
        if offset > edit.size {
            return Incoming::Refused(Reply::failed(response::GENERAL_ERROR, format!("writing at {offset}, past the end at {}", edit.size)));
        }
        match edit.file.try_clone() {
            Ok(file) => Incoming::File { file, offset, unbounded: false },
            Err(e) => Incoming::Refused(Reply::failed(response::GENERAL_ERROR, e)),
        }
    }

    /// Answer a command whose data went into a file.
    pub fn received(&mut self, cmd: &Command, outcome: Received) -> Option<Reply> {
        match cmd.code {
            op::SEND_OBJECT => self.object_received(outcome),
            op::SEND_PARTIAL_OBJECT => self.partial_received(cmd, outcome),
            other => unreachable!("only sending objects writes into files, not {other:#06x}"),
        }
    }

    /// SendObject's answer once the contents came, or did not. The storage
    /// going away while they came takes the file with it.
    fn object_received(&mut self, outcome: Received) -> Option<Reply> {
        let sending = self.sending.take()?;
        let mut failed = match outcome {
            Received::Done(n) if sending.size != u32::MAX && n != u64::from(sending.size) => {
                Some(Reply::failed(response::INCOMPLETE_TRANSFER, format!("{n} bytes came of the {} announced", sending.size)))
            }
            Received::Done(_) => {
                return Some(match sending.modified.map(|at| set_modified(&sending.path, at)) {
                    Some(Err(e)) => Reply::failed(response::GENERAL_ERROR, format!("give {} its time: {e}", sending.path.display())),
                    _ => Reply::ok(),
                });
            }
            Received::Failed(e) if e.raw_os_error() == Some(libc::ENOSPC) => Some(Reply::failed(response::STORAGE_FULL, e)),
            Received::Failed(e) => Some(Reply::failed(response::GENERAL_ERROR, format!("write {}: {e}", sending.path.display()))),
            Received::Stopped => None,
        };
        // A file not sent whole is not left behind.
        if let Err(e) = fs::remove_file(&sending.path)
            && let Some(reply) = &mut failed
        {
            reply.why = Some(format!("{}; then delete {}: {e}", reply.why.take().unwrap_or_default(), sending.path.display()));
        }
        if let Some(store) = &mut self.store {
            store.remove(sending.handle);
        }
        failed
    }

    fn partial_received(&mut self, cmd: &Command, outcome: Received) -> Option<Reply> {
        let [h, low, high] = cmd.take::<3>().expect("the command was taken with its parameters");
        let offset = u64::from(low) | u64::from(high) << 32;
        match outcome {
            Received::Done(n) => {
                if let Some(edit) = self.editing.get_mut(&h) {
                    edit.size = edit.size.max(offset + n);
                }
                Some(Reply::ok().params(&[u32::try_from(n).unwrap_or(u32::MAX)]))
            }
            Received::Failed(e) => Some(Reply::failed(response::GENERAL_ERROR, e).params(&[0])),
            Received::Stopped => None,
        }
    }

    /// Answer `cmd`, with the dataset that came with it.
    pub fn answer(&mut self, cmd: &Command, data: Option<&[u8]>) -> Reply {
        match cmd.code {
            op::GET_DEVICE_INFO => self.device_info(),
            op::OPEN_SESSION => match (cmd.take::<1>(), self.session) {
                (None, _) => Reply::code(response::INVALID_PARAMETER),
                (_, Some(open)) => Reply::code(response::SESSION_ALREADY_OPEN).params(&[open]),
                (Some([id]), None) => {
                    self.session = Some(id);
                    Reply::ok()
                }
            },
            op::CLOSE_SESSION | op::RESET_DEVICE => {
                if self.session.is_none() {
                    return Reply::code(response::SESSION_NOT_OPEN);
                }
                self.close();
                Reply::ok()
            }
            op::GET_DEVICE_PROP_DESC => self.device_prop(cmd, true),
            op::GET_DEVICE_PROP_VALUE => self.device_prop(cmd, false),
            op::SET_DEVICE_PROP_VALUE => self.set_device_prop(cmd, data.unwrap_or_default()),
            op::RESET_DEVICE_PROP_VALUE => self.reset_device_prop(cmd),
            op::GET_OBJECT_PROP_DESC => object_prop_desc(cmd),
            _ if self.session.is_none() => Reply::code(response::SESSION_NOT_OPEN),
            op::GET_STORAGE_IDS => {
                let ids: &[u32] = if self.offered.is_some() { &[STORAGE_ID] } else { &[] };
                let mut w = Writer::new();
                w.u32s(ids);
                Reply::dataset(w)
            }
            op::GET_STORAGE_INFO => self.storage_info(cmd),
            op::GET_OBJECT_PROPS_SUPPORTED => match cmd.take::<1>() {
                None => Reply::code(response::INVALID_PARAMETER),
                Some(_) => {
                    let mut w = Writer::new();
                    w.u16s(OBJECT_PROPERTIES);
                    Reply::dataset(w)
                }
            },
            op::GET_OBJECT_HANDLES => self.object_handles(cmd, false),
            op::GET_NUM_OBJECTS => self.object_handles(cmd, true),
            op::GET_OBJECT_REFERENCES => match cmd.take::<1>() {
                None => Reply::code(response::INVALID_PARAMETER),
                Some([h]) => match self.store.as_ref().is_some_and(|s| s.contains(h)) {
                    false => Reply::code(response::INVALID_OBJECT_HANDLE),
                    true => {
                        let mut w = Writer::new();
                        w.u32s(&[]);
                        Reply::dataset(w)
                    }
                },
            },
            op::GET_OBJECT_INFO => self.object_info(cmd),
            op::GET_OBJECT => self.get_object(cmd),
            op::GET_PARTIAL_OBJECT | op::GET_PARTIAL_OBJECT_64 => self.partial_object(cmd),
            op::GET_OBJECT_PROP_VALUE => self.object_prop_value(cmd),
            op::SET_OBJECT_PROP_VALUE => self.set_object_prop(cmd, data.unwrap_or_default()),
            op::GET_OBJECT_PROP_LIST => self.object_prop_list(cmd),
            op::SEND_OBJECT_INFO => self.send_object_info(cmd, data.unwrap_or_default()),
            // A folder's SendObject: its info made it already.
            op::SEND_OBJECT => {
                self.sending = None;
                Reply::ok()
            }
            op::DELETE_OBJECT => self.delete_object(cmd),
            op::MOVE_OBJECT => self.move_object(cmd),
            op::COPY_OBJECT => self.copy_object(cmd),
            op::BEGIN_EDIT_OBJECT => self.begin_edit(cmd),
            op::TRUNCATE_OBJECT => self.truncate(cmd),
            op::END_EDIT_OBJECT => match cmd.take::<1>() {
                None => Reply::code(response::INVALID_PARAMETER),
                Some([h]) => match self.editing.remove(&h) {
                    Some(_) => Reply::ok(),
                    None => Reply::failed(response::GENERAL_ERROR, format!("object {h} is not open to be edited")),
                },
            },
            other => Reply::failed(response::OPERATION_NOT_SUPPORTED, format!("operation {other:#06x}")),
        }
    }

    fn device_info(&self) -> Reply {
        let mut w = Writer::new();
        w.u16(STANDARD_VERSION).u32(MICROSOFT).u16(STANDARD_VERSION).string(EXTENSIONS).u16(0);
        w.u16s(OPERATIONS).u16s(EVENTS).u16s(DEVICE_PROPERTIES).u16s(&[]).u16s(formats::ACCEPTED);
        w.string(&self.device.manufacturer).string(&self.device.model).string(&self.device.version).string(&self.device.serial);
        Reply::dataset(w)
    }

    fn device_prop(&self, cmd: &Command, desc: bool) -> Reply {
        let Some([code]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        let Ok(code) = u16::try_from(code) else { return Reply::code(response::DEVICE_PROP_NOT_SUPPORTED) };
        let mut w = Writer::new();
        let string = |w: &mut Writer, value: &str| {
            if desc {
                // Writable, with no value of its own to go back to.
                w.u16(code).u16(kind::STR).u8(1).string("").string(value).u8(form::NONE);
            } else {
                w.string(value);
            }
        };
        match code {
            device::SYNCHRONIZATION_PARTNER => string(&mut w, &self.partner),
            device::DEVICE_FRIENDLY_NAME => string(&mut w, &self.name),
            device::SESSION_INITIATOR_VERSION_INFO => string(&mut w, &self.initiator),
            device::BATTERY_LEVEL => {
                let level = (self.device.battery)().unwrap_or(0);
                if desc {
                    w.u16(code).u16(kind::UINT8).u8(0).u8(0).u8(level).u8(form::RANGE).u8(0).u8(100).u8(1);
                } else {
                    w.u8(level);
                }
            }
            device::PERCEIVED_DEVICE_TYPE => {
                if desc {
                    w.u16(code).u16(kind::UINT32).u8(0).u32(0).u32(MOBILE_HANDSET).u8(form::NONE);
                } else {
                    w.u32(MOBILE_HANDSET);
                }
            }
            _ => return Reply::code(response::DEVICE_PROP_NOT_SUPPORTED),
        }
        Reply::dataset(w)
    }

    fn set_device_prop(&mut self, cmd: &Command, data: &[u8]) -> Reply {
        let Some([code]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        let target = match u16::try_from(code) {
            Ok(device::SYNCHRONIZATION_PARTNER) => &mut self.partner,
            Ok(device::DEVICE_FRIENDLY_NAME) => &mut self.name,
            Ok(device::SESSION_INITIATOR_VERSION_INFO) => &mut self.initiator,
            _ => return Reply::code(response::DEVICE_PROP_NOT_SUPPORTED),
        };
        match Reader::new(data).string() {
            Some(value) => {
                *target = value;
                Reply::ok()
            }
            None => Reply::code(response::INVALID_DEVICE_PROP_VALUE),
        }
    }

    fn reset_device_prop(&mut self, cmd: &Command) -> Reply {
        let Some([code]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        match u16::try_from(code) {
            Ok(device::SYNCHRONIZATION_PARTNER) => self.partner.clear(),
            Ok(device::DEVICE_FRIENDLY_NAME) => self.name = self.device.name.clone(),
            Ok(device::SESSION_INITIATOR_VERSION_INFO) => self.initiator.clear(),
            _ => return Reply::code(response::DEVICE_PROP_NOT_SUPPORTED),
        }
        Reply::ok()
    }

    fn storage_info(&self, cmd: &Command) -> Reply {
        let Some([id]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        let Some(s) = self.offered.as_ref().filter(|_| id == STORAGE_ID) else { return Reply::code(response::INVALID_STORAGE_ID) };
        // What is free is what is free for files other than the system's
        // own.
        let (total, free) = match mount::space(&s.top) {
            Ok(room) => (room.total, room.available),
            Err(e) => return Reply::failed(response::GENERAL_ERROR, format!("look at {}: {e}", s.top.display())),
        };
        let mut w = Writer::new();
        w.u16(storage::FIXED_RAM).u16(storage::HIERARCHICAL).u16(storage::READ_WRITE);
        w.u64(total).u64(free).u32(FREE_OBJECTS).string(&s.description).string("");
        Reply::dataset(w)
    }

    /// Whether `id` names the storage offered: itself, or every storage.
    fn storage_named(&self, id: u32) -> bool {
        self.offered.is_some() && (id == STORAGE_ID || id == 0 || id == 0xFFFF_FFFF)
    }

    /// The handles GetObjectHandles asks for, or how many: in the top
    /// folder, in another, or with a parent of 0 every object.
    fn object_handles(&mut self, cmd: &Command, count: bool) -> Reply {
        let Some([id, wanted, parent]) = cmd.take::<3>() else { return Reply::code(response::INVALID_PARAMETER) };
        if !self.storage_named(id) {
            return Reply::code(response::INVALID_STORAGE_ID);
        }
        let store = self.store.as_mut().expect("a session with a storage follows it");
        let found = match parent {
            0 => store.descendants(TOP),
            PARENT_ROOT => store.children(TOP),
            folder => store.children(folder),
        };
        let mut handles = match found {
            Ok(handles) => handles,
            Err(e) => return store_failed(e, response::INVALID_PARENT_OBJECT),
        };
        if wanted != 0 {
            handles.retain(|&h| u32::from(formats::of(store.name(h).unwrap_or_default(), store.is_folder(h))) == wanted);
        }
        if count {
            return Reply::ok().params(&[handles.len() as u32]);
        }
        let mut w = Writer::new();
        w.u32s(&handles);
        Reply::dataset(w)
    }

    /// The object `h` as it is on the disk.
    fn object(&self, h: u32) -> Result<Object, Reply> {
        let store = self.store.as_ref().ok_or_else(|| Reply::code(response::INVALID_OBJECT_HANDLE))?;
        let (Some(path), Some(name), Some(parent)) = (store.path(h), store.name(h), store.parent(h)) else {
            return Err(Reply::code(response::INVALID_OBJECT_HANDLE));
        };
        let meta = fs::metadata(&path).map_err(|e| Reply::failed(response::INVALID_OBJECT_HANDLE, format!("look at {}: {e}", path.display())))?;
        let relative = store.relative(h).expect("a known entry has a path");
        Ok(Object { name: name.to_os_string(), folder: store.is_folder(h), parent, meta, relative })
    }

    fn object_info(&self, cmd: &Command) -> Reply {
        let Some([h]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        let o = match self.object(h) {
            Ok(o) => o,
            Err(reply) => return reply,
        };
        let modified = time::format(o.meta.mtime(), &self.zone);
        let mut w = Writer::new();
        w.u32(STORAGE_ID).u16(o.format()).u16(0).u32(u32::try_from(o.size()).unwrap_or(u32::MAX));
        // No thumbnail, and no image size, colour depth or sequence.
        w.u16(0).u32(0).u32(0).u32(0).u32(0).u32(0).u32(0);
        w.u32(o.parent).u16(if o.folder { 1 } else { 0 }).u32(0).u32(0);
        w.string(&o.name()).string(&modified).string(&modified).string("");
        Reply::dataset(w)
    }

    fn get_object(&self, cmd: &Command) -> Reply {
        let Some([h]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        self.read_file(h, 0, u64::MAX).map_or_else(|reply| reply, |(reply, _)| reply)
    }

    /// GetPartialObject's piece of a file, or GetPartialObject64's, which
    /// may start past 4 GiB; the response says how much came.
    fn partial_object(&self, cmd: &Command) -> Reply {
        let (h, offset, length) = if cmd.code == op::GET_PARTIAL_OBJECT_64 {
            let Some([h, low, high, length]) = cmd.take::<4>() else { return Reply::code(response::INVALID_PARAMETER) };
            (h, u64::from(low) | u64::from(high) << 32, length)
        } else {
            let Some([h, offset, length]) = cmd.take::<3>() else { return Reply::code(response::INVALID_PARAMETER) };
            (h, u64::from(offset), length)
        };
        match self.read_file(h, offset, u64::from(length)) {
            Ok((reply, sent)) => reply.params(&[sent as u32]),
            Err(reply) => reply,
        }
    }

    /// Up to `most` bytes of the file `h` from `offset`, and how many.
    fn read_file(&self, h: u32, offset: u64, most: u64) -> Result<(Reply, u64), Reply> {
        let o = self.object(h)?;
        if o.folder {
            return Err(Reply::failed(response::INVALID_OBJECT_HANDLE, format!("object {h} is a folder")));
        }
        let path = self.store.as_ref().and_then(|s| s.path(h)).expect("a known object has a path");
        let file = File::open(&path).map_err(|e| Reply::failed(access_failed(&e), format!("open {}: {e}", path.display())))?;
        let size = file.metadata().map_err(|e| Reply::failed(response::GENERAL_ERROR, e))?.len();
        let length = most.min(size.saturating_sub(offset));
        Ok((Reply { data: Some(Outgoing::File { file, offset, length }), ..Reply::ok() }, length))
    }

    fn object_prop_value(&self, cmd: &Command) -> Reply {
        let Some([h, code]) = cmd.take::<2>() else { return Reply::code(response::INVALID_PARAMETER) };
        let o = match self.object(h) {
            Ok(o) => o,
            Err(reply) => return reply,
        };
        let mut w = Writer::new();
        match u16::try_from(code).ok().filter(|c| OBJECT_PROPERTIES.contains(c)) {
            Some(code) => self.prop_value(&mut w, &o, code),
            None => return Reply::code(response::OBJECT_PROP_NOT_SUPPORTED),
        }
        Reply::dataset(w)
    }

    /// One property's value, in the type its description gives.
    fn prop_value(&self, w: &mut Writer, o: &Object, code: u16) {
        match code {
            prop::STORAGE_ID => w.u32(STORAGE_ID),
            prop::OBJECT_FORMAT => w.u16(o.format()),
            prop::PROTECTION_STATUS => w.u16(0),
            prop::OBJECT_SIZE => w.u64(o.size()),
            prop::OBJECT_FILE_NAME | prop::NAME | prop::DISPLAY_NAME => w.string(&o.name()),
            prop::DATE_MODIFIED | prop::DATE_ADDED => w.string(&time::format(o.meta.mtime(), &self.zone)),
            prop::PERSISTENT_UID => w.u128(o.persistent_id()),
            prop::PARENT_OBJECT => w.u32(o.parent),
            other => unreachable!("{other:#06x} is not among the properties objects have"),
        };
    }

    /// GetObjectPropList, as the stock system answers it: one object, its
    /// entries with it at a depth of 1, the top folder's entries for an
    /// object of 0, and every object for all objects at any depth; a
    /// property of all of them, or one.
    fn object_prop_list(&mut self, cmd: &Command) -> Reply {
        let Some([h, wanted, code, group, depth]) = cmd.take::<5>() else { return Reply::code(response::INVALID_PARAMETER) };
        if code == 0 {
            return Reply::code(if group == 0 { response::PARAMETER_NOT_SUPPORTED } else { response::SPECIFICATION_BY_GROUP_UNSUPPORTED });
        }
        let codes: Vec<u16> = match code {
            0xFFFF_FFFF => OBJECT_PROPERTIES.to_vec(),
            one => match u16::try_from(one).ok().filter(|c| OBJECT_PROPERTIES.contains(c)) {
                Some(c) => vec![c],
                None => return Reply::code(response::OBJECT_PROP_NOT_SUPPORTED),
            },
        };
        let (h, depth) = if depth == 0xFFFF_FFFF && (h == 0 || h == 0xFFFF_FFFF) { (0xFFFF_FFFF, 0) } else { (h, depth) };
        if depth > 1 {
            return Reply::code(response::SPECIFICATION_BY_DEPTH_UNSUPPORTED);
        }
        let Some(store) = self.store.as_mut() else { return Reply::code(response::INVALID_OBJECT_HANDLE) };
        let mut handles = Vec::new();
        let listed = match h {
            0xFFFF_FFFF => store.descendants(TOP),
            0 => store.children(TOP),
            _ if !store.contains(h) => return Reply::code(response::INVALID_OBJECT_HANDLE),
            _ if depth == 1 => store.children(h),
            _ => Ok(Vec::new()),
        };
        match listed {
            Ok(found) => handles.extend(found),
            Err(store::Error::NotFolder) => {}
            Err(e) => return store_failed(e, response::INVALID_OBJECT_HANDLE),
        }
        if h != 0 && h != 0xFFFF_FFFF {
            handles.push(h);
        }
        let mut entries = Writer::new();
        let mut count = 0u32;
        for h in handles {
            // One gone since it was listed is no longer there to describe.
            let Ok(o) = self.object(h) else { continue };
            if wanted != 0 && u32::from(o.format()) != wanted {
                continue;
            }
            for &c in &codes {
                entries.u32(h).u16(c).u16(prop_kind(c));
                self.prop_value(&mut entries, &o, c);
                count += 1;
            }
        }
        let mut w = Writer::new();
        w.u32(count);
        let mut bytes = w.into_bytes();
        bytes.extend(entries.into_bytes());
        Reply { data: Some(Outgoing::Dataset(bytes)), ..Reply::ok() }
    }

    fn set_object_prop(&mut self, cmd: &Command, data: &[u8]) -> Reply {
        let Some([h, code]) = cmd.take::<2>() else { return Reply::code(response::INVALID_PARAMETER) };
        if let Err(reply) = self.object(h) {
            return reply;
        }
        if code != u32::from(prop::OBJECT_FILE_NAME) {
            return Reply::code(response::OBJECT_PROP_NOT_SUPPORTED);
        }
        let Some(name) = Reader::new(data).string() else { return Reply::code(response::INVALID_OBJECT_PROP_VALUE) };
        let store = self.store.as_mut().expect("a known object has a store");
        let path = store.path(h).expect("a known object has a path");
        match skyward_files::rename(&path, &name) {
            Ok(_) => {
                store.rename(h, name.into());
                Reply::ok()
            }
            Err(e) => Reply::failed(response::GENERAL_ERROR, format!("{e:#}")),
        }
    }

    fn send_object_info(&mut self, cmd: &Command, data: &[u8]) -> Reply {
        let Some([id, parent]) = cmd.take::<2>() else { return Reply::code(response::INVALID_PARAMETER) };
        if !self.storage_named(id) || id == 0xFFFF_FFFF {
            return Reply::code(response::INVALID_STORAGE_ID);
        }
        let folder = if parent == PARENT_ROOT { TOP } else { parent };
        let store = self.store.as_mut().expect("a session with a storage follows it");
        if folder != TOP && !store.contains(folder) {
            return Reply::code(response::INVALID_OBJECT_HANDLE);
        }
        if !store.is_folder(folder) {
            return Reply::code(response::INVALID_PARENT_OBJECT);
        }
        let Some(info) = ObjectInfo::read(data) else { return Reply::failed(response::INVALID_PARAMETER, "the object info is cut short") };
        if info.name.is_empty() || info.name == "." || info.name == ".." || info.name.contains('/') {
            return Reply::failed(response::INVALID_PARAMETER, format!("{:?} cannot be a name", info.name));
        }
        let into = store.path(folder).expect("a known folder has a path");
        let path = into.join(&info.name);
        if info.size != u32::MAX {
            match mount::space(&into) {
                Ok(room) if u64::from(info.size) > room.available => return Reply::code(response::STORAGE_FULL),
                Ok(_) => {}
                Err(e) => return Reply::failed(response::GENERAL_ERROR, format!("look at {}: {e}", into.display())),
            }
        }
        // The folder is read first, so the new entry joins the others.
        if let Err(e) = store.children(folder) {
            return store_failed(e, response::INVALID_PARENT_OBJECT);
        }
        if fs::symlink_metadata(&path).is_ok() {
            return Reply::failed(response::GENERAL_ERROR, format!("{} is there already", path.display()));
        }
        let is_folder = info.format == format::ASSOCIATION;
        let file = if is_folder {
            skyward_files::make_folders(&path, Ownership::Shared).map(|()| None)
        } else {
            skyward_files::make_file(&path, Ownership::Shared).map(Some)
        };
        let file = match file {
            Ok(file) => file,
            Err(e) => return Reply::failed(response::GENERAL_ERROR, format!("{e:#}")),
        };
        let handle = store.add(folder, info.name.into(), is_folder);
        let modified = time::parse(&info.modified, &self.zone);
        self.sending = Some(Sending { handle, path, file, size: info.size, modified });
        Reply::ok().params(&[STORAGE_ID, parent, handle])
    }

    fn delete_object(&mut self, cmd: &Command) -> Reply {
        let Some([h]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        if let Err(reply) = self.object(h) {
            return reply;
        }
        let store = self.store.as_mut().expect("a known object has a store");
        let path = store.path(h).expect("a known object has a path");
        self.editing.remove(&h);
        match skyward_files::delete(std::slice::from_ref(&path), &self.stop, &mut |_| {}) {
            Ok(()) => {
                store.remove(h);
                Reply::ok()
            }
            // What could not go stays, and the folders followed tell the
            // computer of what did.
            Err(e) if fs::symlink_metadata(&path).is_ok() => Reply::failed(response::PARTIAL_DELETION, format!("{e:#}")),
            Err(e) => {
                store.remove(h);
                Reply::failed(response::GENERAL_ERROR, format!("{e:#}"))
            }
        }
    }

    /// Where MoveObject or CopyObject puts an object: the folder, which a
    /// parent of 0 makes the top one.
    fn destination(&self, id: u32, parent: u32) -> Result<(u32, PathBuf), Reply> {
        if !self.storage_named(id) || id == 0xFFFF_FFFF {
            return Err(Reply::code(response::INVALID_STORAGE_ID));
        }
        let folder = if parent == 0 || parent == PARENT_ROOT { TOP } else { parent };
        let store = self.store.as_ref().expect("a session with a storage follows it");
        if folder != TOP && !store.contains(folder) {
            return Err(Reply::code(response::INVALID_OBJECT_HANDLE));
        }
        if !store.is_folder(folder) {
            return Err(Reply::code(response::INVALID_PARENT_OBJECT));
        }
        Ok((folder, store.path(folder).expect("a known folder has a path")))
    }

    fn move_object(&mut self, cmd: &Command) -> Reply {
        let Some([h, id, parent]) = cmd.take::<3>() else { return Reply::code(response::INVALID_PARAMETER) };
        let o = match self.object(h) {
            Ok(o) => o,
            Err(reply) => return reply,
        };
        let (folder, into) = match self.destination(id, parent) {
            Ok(to) => to,
            Err(reply) => return reply,
        };
        let store = self.store.as_mut().expect("a known object has a store");
        let path = store.path(h).expect("a known object has a path");
        match skyward_files::move_into(&[path], &into, Clash::Refuse, Ownership::Shared, &self.stop, &mut |_| {}) {
            Ok(()) => {
                store.relocate(h, folder, o.name);
                Reply::ok()
            }
            Err(e) => Reply::failed(response::GENERAL_ERROR, format!("{e:#}")),
        }
    }

    /// CopyObject, which a name taken where the copy goes refuses, as the
    /// stock system does.
    fn copy_object(&mut self, cmd: &Command) -> Reply {
        let Some([h, id, parent]) = cmd.take::<3>() else { return Reply::code(response::INVALID_PARAMETER) };
        let o = match self.object(h) {
            Ok(o) => o,
            Err(reply) => return reply,
        };
        let (folder, into) = match self.destination(id, parent) {
            Ok(to) => to,
            Err(reply) => return reply,
        };
        let target = into.join(&o.name);
        if fs::symlink_metadata(&target).is_ok() {
            return Reply::failed(response::GENERAL_ERROR, format!("{} is there already", target.display()));
        }
        match mount::space(&into) {
            Ok(room) if o.size() > room.available => return Reply::code(response::STORAGE_FULL),
            Ok(_) => {}
            Err(e) => return Reply::failed(response::GENERAL_ERROR, format!("look at {}: {e}", into.display())),
        }
        let store = self.store.as_mut().expect("a known object has a store");
        if let Err(e) = store.children(folder) {
            return store_failed(e, response::INVALID_PARENT_OBJECT);
        }
        let path = store.path(h).expect("a known object has a path");
        match skyward_files::copy(&[path], &into, Clash::Refuse, Ownership::Shared, &self.stop, &mut |_| {}) {
            Ok(()) => {
                let made = store.add(folder, o.name, o.folder);
                Reply::ok().params(&[made])
            }
            Err(e) => Reply::failed(response::GENERAL_ERROR, format!("{e:#}")),
        }
    }

    fn begin_edit(&mut self, cmd: &Command) -> Reply {
        let Some([h]) = cmd.take::<1>() else { return Reply::code(response::INVALID_PARAMETER) };
        if self.editing.contains_key(&h) {
            return Reply::failed(response::GENERAL_ERROR, format!("object {h} is open to be edited already"));
        }
        let o = match self.object(h) {
            Ok(o) => o,
            Err(reply) => return reply,
        };
        if o.folder {
            return Reply::failed(response::GENERAL_ERROR, format!("object {h} is a folder"));
        }
        let path = self.store.as_ref().and_then(|s| s.path(h)).expect("a known object has a path");
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => {
                self.editing.insert(h, Editing { file, size: o.size() });
                Reply::ok()
            }
            Err(e) => Reply::failed(access_failed(&e), format!("open {}: {e}", path.display())),
        }
    }

    fn truncate(&mut self, cmd: &Command) -> Reply {
        let Some([h, low, high]) = cmd.take::<3>() else { return Reply::code(response::INVALID_PARAMETER) };
        let Some(edit) = self.editing.get_mut(&h) else {
            return Reply::failed(response::GENERAL_ERROR, format!("object {h} is not open to be edited"));
        };
        let size = u64::from(low) | u64::from(high) << 32;
        match edit.file.set_len(size) {
            Ok(()) => {
                edit.size = size;
                Reply::ok()
            }
            Err(e) => Reply::failed(response::GENERAL_ERROR, e),
        }
    }
}

/// GetObjectPropDesc: the type of each property, whether it can be set, and
/// for the times, that they are dates and times.
fn object_prop_desc(cmd: &Command) -> Reply {
    let Some([code, of_format]) = cmd.take::<2>() else { return Reply::code(response::INVALID_PARAMETER) };
    let Some(code) = u16::try_from(code).ok().filter(|c| OBJECT_PROPERTIES.contains(c)) else {
        return Reply::code(response::OBJECT_PROP_NOT_SUPPORTED);
    };
    let mut w = Writer::new();
    let writable = code == prop::OBJECT_FILE_NAME;
    w.u16(code).u16(prop_kind(code)).u8(u8::from(writable));
    match prop_kind(code) {
        kind::UINT16 if code == prop::OBJECT_FORMAT => w.u16(of_format as u16),
        kind::UINT16 => w.u16(0),
        kind::UINT32 => w.u32(0),
        kind::UINT64 => w.u64(0),
        kind::UINT128 => w.u128(0),
        _ => w.string(""),
    };
    w.u32(0);
    w.u8(if matches!(code, prop::DATE_MODIFIED | prop::DATE_ADDED) { form::DATE_TIME } else { form::NONE });
    Reply::dataset(w)
}

/// The type a property's value is written in.
fn prop_kind(code: u16) -> u16 {
    match code {
        prop::STORAGE_ID | prop::PARENT_OBJECT => kind::UINT32,
        prop::OBJECT_FORMAT | prop::PROTECTION_STATUS => kind::UINT16,
        prop::OBJECT_SIZE => kind::UINT64,
        prop::PERSISTENT_UID => kind::UINT128,
        _ => kind::STR,
    }
}

/// What SendObjectInfo tells of the object to come.
struct ObjectInfo {
    format: u16,
    /// The largest size for 4 GiB or more.
    size: u32,
    name: String,
    modified: String,
}

impl ObjectInfo {
    fn read(data: &[u8]) -> Option<ObjectInfo> {
        let mut r = Reader::new(data);
        r.u32()?;
        let format = r.u16()?;
        r.u16()?;
        let size = r.u32()?;
        // The thumbnail's format, size and dimensions, the image's
        // dimensions and depth, the parent, the association's type and
        // description, and the sequence number.
        r.u16()?;
        for _ in 0..7 {
            r.u32()?;
        }
        r.u16()?;
        r.u32()?;
        r.u32()?;
        let name = r.string()?;
        let _created = r.string()?;
        let modified = r.string()?;
        Some(ObjectInfo { format, size, name, modified })
    }
}

/// Give the entry at `path` the time it last changed.
fn set_modified(path: &Path, unix: i64) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let times = [libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_OMIT }, libc::timespec { tv_sec: unix as _, tv_nsec: 0 }];
    if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The response for a file that would not open.
fn access_failed(e: &io::Error) -> u16 {
    match e.kind() {
        io::ErrorKind::PermissionDenied => response::ACCESS_DENIED,
        _ => response::GENERAL_ERROR,
    }
}

/// The response for an entry the store does not have as asked.
fn store_failed(e: store::Error, not_folder: u16) -> Reply {
    match e {
        store::Error::Unknown => Reply::code(response::INVALID_OBJECT_HANDLE),
        store::Error::NotFolder => Reply::code(not_folder),
        store::Error::Io(e) => Reply::failed(response::GENERAL_ERROR, e),
    }
}

#[cfg(test)]
mod tests;
