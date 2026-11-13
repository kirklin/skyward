use crate::parcel::{Parcel, Payload, Reader};
use crate::serve::{Answer, Objects};
use crate::wire::{
    BC_ACQUIRE, BC_ACQUIRE_DONE, BC_ENTER_LOOPER, BC_FREE_BUFFER, BC_INCREFS_DONE, BC_RELEASE, BC_REPLY, BC_TRANSACTION, BC_TRANSACTION_SG,
    BINDER_SET_MAX_THREADS, BINDER_VERSION, BINDER_WRITE_READ, BR_ACQUIRE, BR_DEAD_REPLY, BR_DECREFS, BR_ERROR, BR_FAILED_REPLY, BR_FROZEN_REPLY,
    BR_INCREFS, BR_NOOP, BR_OK, BR_RELEASE, BR_REPLY, BR_SPAWN_LOOPER, BR_TRANSACTION, BR_TRANSACTION_COMPLETE,
};
use crate::wire::{TransactionData, WriteRead};
use anyhow::{bail, ensure, Context, Result};
use std::convert::Infallible;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::Arc;

/// The driver every program on the phone reaches its neighbours through.
const DEVICE: &str = "/dev/binder";

/// The second driver, which the phone's hardware services talk over. Older
/// phones have the node itself; newer ones have the driver's own filesystem
/// and the node inside it.
const HARDWARE: [&str; 2] = ["/dev/hwbinder", "/dev/binderfs/hwbinder"];

/// The layout this crate speaks. The driver refuses a program that knows a
/// different one.
const PROTOCOL: i32 = 8;

/// Room the driver fills with what arrives for us, the size libbinder asks
/// for: a megabyte less the two pages the driver keeps for itself.
const MAP_BYTES: usize = 1024 * 1024 - 8192;

/// The caller waits for no reply.
const TF_ONE_WAY: u32 = 0x01;
/// The call takes no file descriptors but is willing to receive them.
const TF_ACCEPT_FDS: u32 = 0x10;
/// The reply carries a bare error number in place of a payload.
const TF_STATUS_CODE: u32 = 0x08;

/// The error number libbinder answers a call with that reached no method:
/// the object has none numbered so, or is not there.
const UNKNOWN_TRANSACTION: i32 = -74;

/// The most the driver says in one go that is read at a time; what does
/// not fit comes the next time.
const SAID_BYTES: usize = 256;

fn bytes_of<T>(v: &T) -> &[u8] {
    // Every field is a plain integer, so the struct is its own bytes.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) }
}

/// This program's end of the driver.
pub struct Binder {
    file: File,
    map: *mut libc::c_void,
    /// The objects this end serves to other programs, where it serves any.
    served: Option<Arc<Objects<Binder>>>,
}

// The driver keeps what each thread is in the middle of by the thread that
// asks, so any thread may ask at any time, and the mapped room is only
// ever read here: one end is shared by the threads that take its calls.
unsafe impl Send for Binder {}
unsafe impl Sync for Binder {}

/// What is waiting to be told the driver the next time it is asked
/// anything, and the replies those words point at, which have to stay
/// where they are until then.
#[derive(Default)]
struct Waiting {
    words: Vec<u8>,
    replies: Vec<Parcel>,
}

impl Waiting {
    fn tell(&mut self, note: u32, payload: &[u8]) {
        self.words.extend_from_slice(&note.to_ne_bytes());
        self.words.extend_from_slice(payload);
    }
}

impl Binder {
    pub fn open() -> Result<Binder> {
        Binder::on(Path::new(DEVICE), None)
    }

    /// The driver the phone's hardware services talk over, which is a
    /// second one of the same kind.
    pub fn hardware() -> Result<Binder> {
        let node = HARDWARE
            .iter()
            .map(Path::new)
            .find(|p| p.exists())
            .with_context(|| format!("none of {} is here, so no hardware service can be reached", HARDWARE.join(" or ")))?;
        Binder::on(node, None)
    }

    /// An end of the driver that serves `objects` to other programs: an
    /// object written into a call or a reply made through it is one of
    /// these, by its number, and the calls made on them come to the
    /// threads that [`Binder::serve`] it.
    pub fn serving(objects: Arc<Objects<Binder>>) -> Result<Binder> {
        Binder::on(Path::new(DEVICE), Some(objects))
    }

    fn on(node: &Path, served: Option<Arc<Objects<Binder>>>) -> Result<Binder> {
        let file = File::options().read(true).write(true).open(node).with_context(|| format!("open {}", node.display()))?;
        let mut version: i32 = 0;
        unsafe { call(&file, BINDER_VERSION, &mut version as *mut i32 as *mut libc::c_void) }.context("ask the driver its protocol")?;
        if version != PROTOCOL {
            bail!("the driver speaks protocol {version}, Skyward speaks {PROTOCOL}");
        }
        // The threads that take calls say so themselves, as many as the
        // program starts, so the driver never has to ask for another.
        let mut threads: u32 = 0;
        unsafe { call(&file, BINDER_SET_MAX_THREADS, &mut threads as *mut u32 as *mut libc::c_void) }.context("keep the driver from asking for threads")?;
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), MAP_BYTES, libc::PROT_READ, libc::MAP_PRIVATE | libc::MAP_NORESERVE, file.as_raw_fd(), 0)
        };
        if map == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error()).context("map the driver's replies");
        }
        Ok(Binder { file, map, served })
    }

    /// The service directory, which every program reaches at handle 0. The
    /// driver carries a call to whichever process holds that place without
    /// being given a share of it first, so this takes none and gives none
    /// back: taking one and giving it back on every look-up would have the
    /// directory's own object counted up and down from outside while it is
    /// still starting.
    pub fn directory(&self) -> Result<Handle<'_>> {
        Ok(Handle { binder: self, handle: 0, held: false })
    }

    /// Call method `code` on `handle` and wait for what it answers.
    pub fn transact(&self, handle: u32, code: u32, data: &impl Payload) -> Result<Reply<'_>> {
        let payload = data.values();
        let addresses = data.addresses();
        let td = TransactionData {
            target: handle as u64,
            code,
            flags: TF_ACCEPT_FDS,
            data_size: payload.len() as u64,
            offsets_size: (addresses.len() * 8) as u64,
            buffer: payload.as_ptr() as u64,
            offsets: addresses.as_ptr() as u64,
            ..TransactionData::default()
        };
        // A call that hands over addresses is sent the way that tells the
        // driver how much room to set aside for what they point at.
        let mut write = match addresses.is_empty() {
            true => BC_TRANSACTION.to_ne_bytes().to_vec(),
            false => BC_TRANSACTION_SG.to_ne_bytes().to_vec(),
        };
        write.extend_from_slice(bytes_of(&td));
        if !addresses.is_empty() {
            write.extend_from_slice(&data.pointed().to_ne_bytes());
        }
        // The driver answers a call with a handful of short notices; what
        // the other side sent lands in the mapped room, not here. While the
        // reply is awaited the other side may call back into an object
        // served here, which is answered and the waiting gone on with.
        let mut waiting = Waiting { words: write, replies: Vec::new() };
        let mut said = [0u8; SAID_BYTES];
        loop {
            let heard = self.exchange(&mut waiting, &mut said).context("send a call")?;
            if let Some(reply) = self.take_in(&said[..heard], &mut waiting, true)? {
                // What is still waiting to be told is told now: nothing
                // asks the driver on this thread again for certain.
                if !waiting.words.is_empty() {
                    self.exchange(&mut waiting, &mut []).context("answer the driver")?;
                }
                return Ok(reply);
            }
        }
    }

    /// Take the calls made on the objects served through this end, on the
    /// thread that asks, for as long as the driver gives them out.
    pub fn serve(&self) -> Result<Infallible> {
        ensure!(self.served.is_some(), "this end of the driver serves no object");
        let mut waiting = Waiting::default();
        waiting.tell(BC_ENTER_LOOPER, &[]);
        let mut said = [0u8; SAID_BYTES];
        loop {
            let heard = self.exchange(&mut waiting, &mut said).context("wait for a call")?;
            if self.take_in(&said[..heard], &mut waiting, false)?.is_some() {
                bail!("the driver gave a reply to a thread that made no call");
            }
        }
    }

    /// Tell the driver what is waiting to be told and hear what it has to
    /// say into `said`, waiting for it to have something where `said` has
    /// room; gives how much it said.
    fn exchange(&self, waiting: &mut Waiting, said: &mut [u8]) -> Result<usize> {
        let mut bwr = WriteRead {
            write_size: waiting.words.len() as u64,
            write_buffer: waiting.words.as_ptr() as u64,
            read_size: said.len() as u64,
            read_buffer: said.as_mut_ptr() as u64,
            ..WriteRead::default()
        };
        unsafe { call(&self.file, BINDER_WRITE_READ, &mut bwr as *mut WriteRead as *mut libc::c_void) }?;
        ensure!(bwr.write_consumed == bwr.write_size, "the driver took {} of the {} bytes it was told", bwr.write_consumed, bwr.write_size);
        waiting.words.clear();
        waiting.replies.clear();
        Ok(bwr.read_consumed as usize)
    }

    /// Answer a call the driver brought for an object served here: the
    /// reply, and the room the call lay in given back, wait to be told.
    fn take_call(&self, td: &TransactionData, waiting: &mut Waiting) {
        let data = match td.data_size {
            0 => &[][..],
            size => unsafe { std::slice::from_raw_parts(td.buffer as *const u8, size as usize) },
        };
        let answer = match &self.served {
            Some(objects) => objects.answer(self, td.cookie, td.code, data),
            None => Answer::Unknown,
        };
        waiting.tell(BC_FREE_BUFFER, &td.buffer.to_ne_bytes());
        if td.flags & TF_ONE_WAY != 0 {
            return;
        }
        let (reply, flags) = match answer {
            Answer::Reply(reply) => (reply, 0),
            Answer::Unknown => {
                let mut status = Parcel::default();
                status.i32(UNKNOWN_TRANSACTION);
                (status, TF_STATUS_CODE)
            }
        };
        let back = TransactionData {
            flags,
            data_size: reply.values().len() as u64,
            offsets_size: (reply.addresses().len() * 8) as u64,
            buffer: reply.values().as_ptr() as u64,
            offsets: reply.addresses().as_ptr() as u64,
            ..TransactionData::default()
        };
        waiting.tell(BC_REPLY, bytes_of(&back));
        waiting.replies.push(reply);
    }

    /// What the driver said, taken in: the reply among it if it has come.
    /// What it says of the objects served here is answered, and a call on
    /// one of them is taken, each answer waiting to be told.
    ///
    /// `calling` says a call made on this thread awaits its reply. Without
    /// one, what the driver says of a reply that did not arrive is of a
    /// reply this thread gave, to a caller that has gone since: there is
    /// nobody left to tell.
    fn take_in(&self, mut said: &[u8], waiting: &mut Waiting, calling: bool) -> Result<Option<Reply<'_>>> {
        while said.len() >= 4 {
            let note = u32::from_ne_bytes(said[..4].try_into().expect("four bytes"));
            said = &said[4..];
            match note {
                BR_NOOP | BR_OK | BR_TRANSACTION_COMPLETE | BR_SPAWN_LOOPER => {}
                // Another program has come to hold an object served here,
                // which the driver wants acknowledged before it says so
                // again.
                BR_INCREFS => waiting.tell(BC_INCREFS_DONE, part(&mut said, 16)?),
                BR_ACQUIRE => waiting.tell(BC_ACQUIRE_DONE, part(&mut said, 16)?),
                // No other program can call the object any more.
                BR_RELEASE => {
                    let cookie = u64::from_ne_bytes(part(&mut said, 16)?[8..].try_into().expect("eight bytes"));
                    if let Some(objects) = &self.served {
                        objects.let_go(self, cookie);
                    }
                }
                BR_DECREFS => {
                    part(&mut said, 16)?;
                }
                BR_TRANSACTION => {
                    let raw = part(&mut said, size_of::<TransactionData>())?;
                    let td: TransactionData = unsafe { std::ptr::read_unaligned(raw.as_ptr().cast()) };
                    self.take_call(&td, waiting);
                }
                BR_REPLY => {
                    let raw = part(&mut said, size_of::<TransactionData>())?;
                    let td: TransactionData = unsafe { std::ptr::read_unaligned(raw.as_ptr().cast()) };
                    if td.flags & TF_STATUS_CODE != 0 {
                        let code = unsafe { std::ptr::read_unaligned(td.buffer as *const i32) };
                        self.free(td.buffer)?;
                        bail!("the call failed in the driver with error {code}");
                    }
                    let data = unsafe { std::slice::from_raw_parts(td.buffer as *const u8, td.data_size as usize) };
                    return Ok(Some(Reply { binder: self, buffer: td.buffer, data }));
                }
                BR_DEAD_REPLY | BR_FAILED_REPLY | BR_FROZEN_REPLY if !calling => {}
                BR_DEAD_REPLY => bail!("the other side is gone"),
                BR_FAILED_REPLY => bail!("the driver refused the call"),
                BR_FROZEN_REPLY => bail!("the other side is frozen"),
                BR_ERROR => {
                    let code = i32::from_ne_bytes(part(&mut said, 4)?.try_into().expect("four bytes"));
                    bail!("the driver reported error {code}");
                }
                other => bail!("the driver said {other:#x}, which Skyward does not know"),
            }
        }
        Ok(None)
    }

    /// Tell the driver something that needs no answer.
    fn tell(&self, note: u32, payload: &[u8]) -> Result<()> {
        let mut write = note.to_ne_bytes().to_vec();
        write.extend_from_slice(payload);
        let mut bwr = WriteRead {
            write_size: write.len() as u64,
            write_buffer: write.as_ptr() as u64,
            ..WriteRead::default()
        };
        unsafe { call(&self.file, BINDER_WRITE_READ, &mut bwr as *mut WriteRead as *mut libc::c_void) }.context("tell the driver")
    }

    fn free(&self, buffer: u64) -> Result<()> {
        self.tell(BC_FREE_BUFFER, &buffer.to_ne_bytes())
    }
}

impl Drop for Binder {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.map, MAP_BYTES) };
    }
}

/// The next `n` bytes of what the driver said, taken off its front.
fn part<'a>(said: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    ensure!(said.len() >= n, "the driver cut what it said short: {} bytes where {n} follow", said.len());
    let (head, rest) = said.split_at(n);
    *said = rest;
    Ok(head)
}

unsafe fn call(file: &File, request: u32, arg: *mut libc::c_void) -> Result<()> {
    // The driver answers a signal with EINTR and expects the call again.
    loop {
        if unsafe { libc::ioctl(file.as_raw_fd(), request as libc::Ioctl, arg) } >= 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
    }
}

/// An object in another process, held open until this goes out of scope.
pub struct Handle<'a> {
    binder: &'a Binder,
    handle: u32,
    /// Whether a share of the object was taken, and so has to be given
    /// back.
    held: bool,
}

impl<'a> Handle<'a> {
    /// Take a share of an object the driver has just named, so it outlives
    /// the reply that carried it.
    pub fn hold(binder: &'a Binder, handle: u32) -> Result<Handle<'a>> {
        binder.tell(BC_ACQUIRE, &handle.to_ne_bytes()).context("hold an object open")?;
        Ok(Handle { binder, handle, held: true })
    }

    pub fn transact(&self, code: u32, data: &impl Payload) -> Result<Reply<'a>> {
        self.binder.transact(self.handle, code, data)
    }
}

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        if self.held {
            let _ = self.binder.tell(BC_RELEASE, &self.handle.to_ne_bytes());
        }
    }
}

/// What the other side answered, in the driver's own room until it is read.
pub struct Reply<'a> {
    binder: &'a Binder,
    buffer: u64,
    data: &'a [u8],
}

impl<'a> Reply<'a> {
    pub fn read(&self) -> Reader<'_> {
        Reader::new(self.data)
    }

    /// The reply as it lies in the driver's room, for a layout the plain
    /// reader does not know.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    pub fn binder(&self) -> &'a Binder {
        self.binder
    }
}

impl Drop for Reply<'_> {
    fn drop(&mut self) {
        let _ = self.binder.free(self.buffer);
    }
}
