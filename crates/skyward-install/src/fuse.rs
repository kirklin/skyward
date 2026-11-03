//! The package as a file: a FUSE filesystem with one read-only file,
//! /sideload/package.zip, whose blocks are fetched from where the package
//! is as they are read. A block read again must hash as it did the first
//! time, so whoever reads the file sees one package even if the copy it
//! comes from changes underneath.

use crate::adb::{Connection, Link, Offer, MAX_BLOCKS};
use anyhow::{anyhow, Context, Result};
use polyfuse_kernel::{
    fuse_attr, fuse_attr_out, fuse_entry_out, fuse_in_header, fuse_init_in, fuse_init_out, fuse_opcode, fuse_open_out,
    fuse_out_header, fuse_read_in, FUSE_COMPAT_22_INIT_OUT_SIZE, FUSE_KERNEL_MINOR_VERSION, FUSE_KERNEL_VERSION,
};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::mem::size_of;
use std::os::unix::fs::FileExt;
use std::path::Path;
use zerocopy::{FromBytes, IntoBytes};

pub const MOUNT_POINT: &str = "/sideload";
pub const PACKAGE: &str = "/sideload/package.zip";
/// Looking this name up stops the filesystem; it is how the installer
/// ends it from another thread.
pub const EXIT: &str = "/sideload/exit";

const ROOT_ID: u64 = 1;
const PACKAGE_ID: u64 = 2;
const EXIT_ID: u64 = 3;

/// File types in a mode, as Linux numbers them.
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;

/// How long the kernel may keep names and attributes, in seconds.
const VALID: u64 = 10;

/// What a request comes to.
#[derive(Debug, PartialEq, Eq)]
pub enum Handled {
    Reply(Vec<u8>),
    /// The request takes no reply.
    Silent,
    /// Write the reply and stop serving.
    Exit(Vec<u8>),
}

/// Fetching a block from where the package is; the error says why it
/// failed.
pub type Fetch<'a> = dyn FnMut(u64, &mut [u8]) -> Result<(), String> + 'a;

/// Why the package stopped being readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Fetching a block failed: where the package is stopped giving it.
    Transfer(String),
    /// A block read again hashed differently: the package changed where
    /// it is while it was being read.
    Changed(u64),
}

/// Where the package's blocks come from.
pub trait Source {
    /// Block `index` into `out`, which is as long as the block; the error
    /// says why it could not be read.
    fn read_block(&mut self, index: u64, out: &mut [u8]) -> Result<(), String>;
}

/// The computer, sending the package with `adb sideload`.
impl<L: Link> Source for Connection<L> {
    fn read_block(&mut self, index: u64, out: &mut [u8]) -> Result<(), String> {
        self.fetch(index, out).map_err(|e| e.to_string())
    }
}

/// The block size a file on the phone is served in, as the stock recovery
/// serves one.
const FILE_BLOCK: u32 = 64 * 1024;

/// A package that is a file on the phone: in internal storage, or on a USB
/// drive.
pub struct LocalFile {
    file: File,
    block_size: u64,
}

impl LocalFile {
    /// Open the file at `path` to serve, with its size and the block size
    /// it is served in.
    pub fn open(path: &Path) -> Result<(LocalFile, Offer)> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let size = file.metadata().with_context(|| format!("look at {}", path.display()))?.len();
        let block_size = file_block(size);
        Ok((LocalFile { file, block_size: u64::from(block_size) }, Offer { size, block_size }))
    }
}

impl Source for LocalFile {
    fn read_block(&mut self, index: u64, out: &mut [u8]) -> Result<(), String> {
        self.file.read_exact_at(out, index * self.block_size).map_err(|e| e.to_string())
    }
}

/// The block size for a file of `size` bytes: the usual one, or a bigger
/// one when the file has more blocks of it than the package may.
pub(crate) fn file_block(size: u64) -> u32 {
    let fits = size.div_ceil(MAX_BLOCKS).next_power_of_two();
    u32::try_from(fits).map_or(u32::MAX, |b| b.max(FILE_BLOCK))
}

/// The filesystem's state: the package's blocks as served so far.
pub struct Server {
    offer: Offer,
    /// Each block's hash from the first time it was read.
    hashes: Vec<Option<[u8; 32]>>,
    /// The block in `block`, kept for the read that comes back for it.
    current: Option<u64>,
    block: Vec<u8>,
    /// Every read after a fault fails too.
    fault: Option<Fault>,
    /// The furthest byte any read reached.
    served: u64,
    /// The parts of the package read so far, merged, in order.
    runs: Vec<(u64, u64)>,
}

impl Server {
    pub fn new(offer: Offer) -> Server {
        Server {
            offer,
            hashes: vec![None; offer.blocks() as usize],
            current: None,
            block: vec![0; offer.block_size as usize],
            fault: None,
            served: 0,
            runs: Vec::new(),
        }
    }

    pub fn served(&self) -> u64 {
        self.served
    }

    /// How far into the package the reads that began at `base` have come. A
    /// read that lands past them, as a zip's own table of contents does at
    /// the end of the file, says nothing about them.
    pub fn progress_from(&self, base: u64) -> u64 {
        match self.runs.iter().find(|(a, b)| *a <= base && base < *b) {
            Some((_, b)) => *b,
            None => base,
        }
    }

    /// The parts of the package read so far, merged, in order.
    pub fn runs(&self) -> &[(u64, u64)] {
        &self.runs
    }

    /// Note that the package was read from `at` for `len` bytes.
    fn note(&mut self, at: u64, len: u64) {
        if len == 0 {
            return;
        }
        let (mut start, mut end) = (at, (at + len).min(self.offer.size));
        let mut runs: Vec<(u64, u64)> = Vec::with_capacity(self.runs.len() + 1);
        for &(a, b) in &self.runs {
            if b < start || a > end {
                runs.push((a, b));
            } else {
                start = start.min(a);
                end = end.max(b);
            }
        }
        runs.push((start, end));
        runs.sort();
        self.runs = runs;
    }

    pub fn fault(&self) -> Option<&Fault> {
        self.fault.as_ref()
    }

    pub fn handle(&mut self, request: &[u8], fetch: &mut Fetch) -> Result<Handled> {
        let (header, body) =
            fuse_in_header::read_from_prefix(request).map_err(|_| anyhow!("a request of {} bytes", request.len()))?;
        let unique = header.unique;
        let Ok(opcode) = fuse_opcode::try_from(header.opcode) else { return Ok(Handled::Reply(error(unique, libc::ENOSYS))) };
        Ok(match opcode {
            fuse_opcode::FUSE_INIT => {
                let (init, _) = fuse_init_in::read_from_prefix(body).map_err(|_| anyhow!("a short INIT"))?;
                Handled::Reply(self.init(unique, &init))
            }
            fuse_opcode::FUSE_LOOKUP => {
                let name = body.split(|&b| b == 0).next().unwrap_or_default();
                match (header.nodeid, name) {
                    (ROOT_ID, b"package.zip") => Handled::Reply(reply(unique, self.entry(PACKAGE_ID).as_bytes())),
                    (ROOT_ID, b"exit") => Handled::Exit(reply(unique, self.entry(EXIT_ID).as_bytes())),
                    _ => Handled::Reply(error(unique, libc::ENOENT)),
                }
            }
            fuse_opcode::FUSE_GETATTR => match self.attr(header.nodeid) {
                Some(attr) => {
                    let out = fuse_attr_out { attr_valid: VALID, attr, ..Default::default() };
                    let r = reply(unique, out.as_bytes());
                    if header.nodeid == EXIT_ID {
                        Handled::Exit(r)
                    } else {
                        Handled::Reply(r)
                    }
                }
                None => Handled::Reply(error(unique, libc::ENOENT)),
            },
            fuse_opcode::FUSE_OPEN => Handled::Reply(match header.nodeid {
                PACKAGE_ID => reply(unique, fuse_open_out::default().as_bytes()),
                EXIT_ID => error(unique, libc::EPERM),
                _ => error(unique, libc::ENOENT),
            }),
            fuse_opcode::FUSE_READ => {
                if header.nodeid != PACKAGE_ID {
                    return Ok(Handled::Reply(error(unique, libc::ENOENT)));
                }
                let (read, _) = fuse_read_in::read_from_prefix(body).map_err(|_| anyhow!("a short READ"))?;
                Handled::Reply(self.read(unique, read.offset, read.size as usize, fetch))
            }
            fuse_opcode::FUSE_FLUSH | fuse_opcode::FUSE_RELEASE => Handled::Reply(reply(unique, &[])),
            fuse_opcode::FUSE_DESTROY => Handled::Exit(reply(unique, &[])),
            fuse_opcode::FUSE_FORGET | fuse_opcode::FUSE_BATCH_FORGET | fuse_opcode::FUSE_INTERRUPT => Handled::Silent,
            _ => Handled::Reply(error(unique, libc::ENOSYS)),
        })
    }

    fn init(&self, unique: u64, init: &fuse_init_in) -> Vec<u8> {
        if init.major != FUSE_KERNEL_VERSION || init.minor < 6 {
            return error(unique, libc::EPROTO);
        }
        let out = fuse_init_out {
            minor: init.minor.min(FUSE_KERNEL_MINOR_VERSION),
            max_readahead: init.max_readahead,
            max_background: 32,
            congestion_threshold: 32,
            max_write: 4096,
            ..Default::default()
        };
        // Kernels before 7.23 take the reply's first fields only.
        let len = if init.minor <= 22 { FUSE_COMPAT_22_INIT_OUT_SIZE } else { size_of::<fuse_init_out>() };
        reply(unique, &out.as_bytes()[..len])
    }

    fn attr(&self, node: u64) -> Option<fuse_attr> {
        let (size, mode) = match node {
            ROOT_ID => (4096, S_IFDIR | 0o555),
            PACKAGE_ID => (self.offer.size, S_IFREG | 0o444),
            EXIT_ID => (0, S_IFREG),
            _ => return None,
        };
        Some(fuse_attr { ino: node, size, blocks: size.div_ceil(512), mode, nlink: 1, blksize: 4096, ..Default::default() })
    }

    fn entry(&self, node: u64) -> fuse_entry_out {
        let attr = self.attr(node).expect("entries are for nodes that exist");
        fuse_entry_out { nodeid: node, generation: node, entry_valid: VALID, attr_valid: VALID, attr, ..Default::default() }
    }

    /// `size` bytes from `offset`, zeros past the end of the package.
    fn read(&mut self, unique: u64, offset: u64, size: usize, fetch: &mut Fetch) -> Vec<u8> {
        if self.fault.is_some() {
            return error(unique, libc::EIO);
        }
        let block_size = u64::from(self.offer.block_size);
        let mut data = Vec::with_capacity(size);
        let mut at = offset;
        while data.len() < size {
            let index = at / block_size;
            let within = (at % block_size) as usize;
            if let Err(e) = self.load(index, fetch) {
                self.fault = Some(e);
                return error(unique, libc::EIO);
            }
            let take = (block_size as usize - within).min(size - data.len());
            data.extend_from_slice(&self.block[within..within + take]);
            at += take as u64;
        }
        self.served = self.served.max(at.min(self.offer.size));
        self.note(offset, data.len() as u64);
        reply(unique, &data)
    }

    /// Bring block `index` into `block`. The last block is padded with
    /// zeros and hashed padded, and blocks past the end are all zeros.
    fn load(&mut self, index: u64, fetch: &mut Fetch) -> Result<(), Fault> {
        if self.current == Some(index) {
            return Ok(());
        }
        self.current = None;
        let len = self.offer.block_len(index);
        self.block[len..].fill(0);
        if len > 0 {
            fetch(index, &mut self.block[..len]).map_err(Fault::Transfer)?;
            let hash: [u8; 32] = Sha256::digest(&self.block).into();
            let seen = &mut self.hashes[index as usize];
            if seen.is_some_and(|h| h != hash) {
                return Err(Fault::Changed(index));
            }
            *seen = Some(hash);
        }
        self.current = Some(index);
        Ok(())
    }
}

fn reply(unique: u64, body: &[u8]) -> Vec<u8> {
    let len = size_of::<fuse_out_header>() + body.len();
    let header = fuse_out_header { len: len as u32, error: 0, unique };
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

fn error(unique: u64, errno: i32) -> Vec<u8> {
    fuse_out_header { len: size_of::<fuse_out_header>() as u32, error: -errno, unique }.as_bytes().to_vec()
}

#[cfg(target_os = "android")]
pub use device::{mount, Mount, Mounted};

#[cfg(target_os = "android")]
mod device {
    use super::{Fault, Handled, Server, Source, EXIT, MOUNT_POINT};
    use crate::adb::Offer;
    use anyhow::{Context, Result};
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::{ErrorKind, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

    /// Big enough for any request: the kernel asks for at least 8 KiB, and
    /// writes never come.
    const REQUEST_BUFFER: usize = 64 * 1024;

    /// The filesystem, mounted and not served yet.
    pub struct Mounted {
        dev: File,
        offer: Offer,
    }

    /// The filesystem, served from a thread of its own out of `S`.
    pub struct Mount<S> {
        thread: JoinHandle<(S, Result<Option<Fault>>)>,
        served: Arc<AtomicU64>,
        runs: Arc<Mutex<Vec<(u64, u64)>>>,
    }

    /// Mount the package `offer` describes at /sideload, read-only.
    pub fn mount(offer: Offer) -> Result<Mounted> {
        let point = Path::new(MOUNT_POINT);
        if !point.try_exists().with_context(|| format!("look for {MOUNT_POINT}"))? {
            fs::create_dir(point).with_context(|| format!("create {MOUNT_POINT}"))?;
        }
        // An earlier recovery that died while installing leaves it mounted.
        if skyward_block::mount::is_mount_point(point)? {
            skyward_block::mount::umount_detach(point)?;
        }
        let dev = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open("/dev/fuse")
            .context("open /dev/fuse")?;
        let options = format!(
            "fd={},user_id=0,group_id=0,max_read={},allow_other,rootmode=040000",
            dev.as_raw_fd(),
            offer.block_size
        );
        let (source, target, kind, data) =
            (CString::new("/dev/fuse")?, CString::new(MOUNT_POINT)?, CString::new("fuse")?, CString::new(options)?);
        let flags = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_RDONLY | libc::MS_NOEXEC;
        if unsafe { libc::mount(source.as_ptr(), target.as_ptr(), kind.as_ptr(), flags, data.as_ptr().cast()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("mount the package at /sideload");
        }
        Ok(Mounted { dev, offer })
    }

    impl Mounted {
        /// Serve the package from `source`.
        pub fn serve<S: Source + Send + 'static>(self, source: S) -> Mount<S> {
            let served = Arc::new(AtomicU64::new(0));
            let reach = served.clone();
            let runs = Arc::new(Mutex::new(Vec::new()));
            let parts = runs.clone();
            let thread = thread::Builder::new()
                .name("sideload-fuse".into())
                .spawn(move || {
                    let mut source = source;
                    let mut server = Server::new(self.offer);
                    let result = serve(&self.dev, &mut server, &mut source, &reach, &parts);
                    (source, result.map(|()| server.fault().cloned()))
                })
                .expect("the FUSE thread starts");
            Mount { thread, served, runs }
        }
    }

    fn serve(
        dev: &File,
        server: &mut Server,
        source: &mut impl Source,
        served: &AtomicU64,
        runs: &Mutex<Vec<(u64, u64)>>,
    ) -> Result<()> {
        let mut buf = vec![0u8; REQUEST_BUFFER];
        loop {
            let n = match { dev }.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                // The filesystem was unmounted under it.
                Err(e) if e.raw_os_error() == Some(libc::ENODEV) => return Ok(()),
                Err(e) => return Err(e).context("read a FUSE request"),
            };
            let mut fetch = |index: u64, out: &mut [u8]| source.read_block(index, out);
            let handled = server.handle(&buf[..n], &mut fetch)?;
            served.store(server.served(), Ordering::Relaxed);
            if let Ok(mut held) = runs.lock() {
                held.clear();
                held.extend_from_slice(server.runs());
            }
            match handled {
                Handled::Reply(r) => answer(dev, &r)?,
                Handled::Silent => {}
                Handled::Exit(r) => return answer(dev, &r),
            }
        }
    }

    /// Write a reply; the kernel refuses one whose request was interrupted
    /// and gone.
    fn answer(dev: &File, reply: &[u8]) -> Result<()> {
        match { dev }.write(reply) {
            Ok(_) => Ok(()),
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(()),
            Err(e) => Err(e).context("write a FUSE reply"),
        }
    }

    impl<S> Mount<S> {
        /// How far into the package reads have reached.
        pub fn served(&self) -> u64 {
            self.served.load(Ordering::Relaxed)
        }

        /// How far into the package the reads that began at `base` have come,
        /// as [`Server::progress_from`] tells it.
        pub fn progress_from(&self, base: u64) -> u64 {
            match self.runs.lock() {
                Ok(held) => match held.iter().find(|(a, b)| *a <= base && base < *b) {
                    Some((_, b)) => *b,
                    None => base,
                },
                Err(_) => base,
            }
        }

        /// Stop serving and unmount. The source always comes back; the
        /// result says whether serving went well, and why the package
        /// stopped being readable if it did.
        pub fn unmount(self) -> (S, Result<Option<Fault>>) {
            let stopped = stop();
            let (source, served) = self.thread.join().expect("the FUSE thread does not panic");
            let unmounted = skyward_block::mount::umount_detach(Path::new(MOUNT_POINT));
            (source, stopped.and(unmounted).and(served))
        }
    }

    /// The server answers the lookup and stops. If it has stopped already,
    /// the connection is gone and the lookup fails.
    fn stop() -> Result<()> {
        match fs::symlink_metadata(EXIT) {
            Err(e) if e.raw_os_error() != Some(libc::ENOTCONN) => Err(e).context("stop the FUSE server"),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polyfuse_kernel::fuse_open_in;
    use std::cell::Cell;
    use zerocopy::Immutable;

    fn request<T: IntoBytes + Immutable>(opcode: fuse_opcode, node: u64, body: &T) -> Vec<u8> {
        request_raw(opcode, node, body.as_bytes())
    }

    fn request_raw(opcode: fuse_opcode, node: u64, body: &[u8]) -> Vec<u8> {
        let header = fuse_in_header {
            len: (size_of::<fuse_in_header>() + body.len()) as u32,
            opcode: opcode as u32,
            unique: 7,
            nodeid: node,
            ..Default::default()
        };
        [header.as_bytes(), body].concat()
    }

    fn read_request(offset: u64, size: u32) -> Vec<u8> {
        request(fuse_opcode::FUSE_READ, PACKAGE_ID, &fuse_read_in { offset, size, ..Default::default() })
    }

    /// The reply's header and body.
    fn split(handled: Handled) -> (fuse_out_header, Vec<u8>) {
        let Handled::Reply(bytes) = handled else { panic!("a reply") };
        let (header, body) = fuse_out_header::read_from_prefix(&bytes).unwrap();
        assert_eq!(header.len as usize, bytes.len());
        assert_eq!(header.unique, 7);
        (header, body.to_vec())
    }

    /// A package of 2.5 blocks of 4096 bytes, each byte telling its offset.
    fn package() -> Vec<u8> {
        (0..10_240u32).map(|i| (i % 253) as u8).collect()
    }

    fn server_for(package: &[u8]) -> Server {
        Server::new(Offer { size: package.len() as u64, block_size: 4096 })
    }

    /// Fetching the blocks of `package`, counting the fetches.
    fn fetch_from<'a>(package: &'a [u8], fetches: &'a Cell<u32>) -> impl FnMut(u64, &mut [u8]) -> Result<(), String> + 'a {
        move |index, out| {
            fetches.set(fetches.get() + 1);
            let start = index as usize * 4096;
            out.copy_from_slice(&package[start..start + out.len()]);
            Ok(())
        }
    }

    #[test]
    fn the_package_is_the_one_file_and_exit_stops_serving() {
        let (pkg, fetches) = (package(), Cell::new(0));
        let (mut server, mut fetch) = (server_for(&pkg), fetch_from(&pkg, &fetches));
        let (h, body) = split(server.handle(&request_raw(fuse_opcode::FUSE_LOOKUP, ROOT_ID, b"package.zip\0"), &mut fetch).unwrap());
        assert_eq!(h.error, 0);
        let (entry, _) = fuse_entry_out::read_from_prefix(&body).unwrap();
        assert_eq!((entry.nodeid, entry.attr.size, entry.attr.mode), (PACKAGE_ID, 10_240, S_IFREG | 0o444));
        let (h, _) = split(server.handle(&request_raw(fuse_opcode::FUSE_LOOKUP, ROOT_ID, b"other.zip\0"), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::ENOENT);
        let (h, _) = split(server.handle(&request(fuse_opcode::FUSE_OPEN, PACKAGE_ID, &fuse_open_in::default()), &mut fetch).unwrap());
        assert_eq!(h.error, 0);
        let (h, _) = split(server.handle(&request(fuse_opcode::FUSE_OPEN, EXIT_ID, &fuse_open_in::default()), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::EPERM);
        let exit = server.handle(&request_raw(fuse_opcode::FUSE_LOOKUP, ROOT_ID, b"exit\0"), &mut fetch).unwrap();
        assert!(matches!(exit, Handled::Exit(_)));
        let forget = server.handle(&request_raw(fuse_opcode::FUSE_FORGET, PACKAGE_ID, &[0; 8]), &mut fetch).unwrap();
        assert_eq!(forget, Handled::Silent);
    }

    #[test]
    fn init_answers_with_the_older_of_both_versions() {
        let (pkg, fetches) = (package(), Cell::new(0));
        let (mut server, mut fetch) = (server_for(&pkg), fetch_from(&pkg, &fetches));
        let init = fuse_init_in { major: 7, minor: 38, max_readahead: 131_072, flags: 0 };
        let (h, body) = split(server.handle(&request(fuse_opcode::FUSE_INIT, 0, &init), &mut fetch).unwrap());
        assert_eq!(h.error, 0);
        let (out, _) = fuse_init_out::read_from_prefix(&body).unwrap();
        assert_eq!((out.major, out.minor, out.max_readahead), (7, FUSE_KERNEL_MINOR_VERSION, 131_072));
        let old = fuse_init_in { major: 7, minor: 22, max_readahead: 0, flags: 0 };
        let (_, body) = split(server.handle(&request(fuse_opcode::FUSE_INIT, 0, &old), &mut fetch).unwrap());
        assert_eq!(body.len(), FUSE_COMPAT_22_INIT_OUT_SIZE);
        let wrong = fuse_init_in { major: 6, minor: 0, max_readahead: 0, flags: 0 };
        let (h, _) = split(server.handle(&request(fuse_opcode::FUSE_INIT, 0, &wrong), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::EPROTO);
    }

    #[test]
    fn reads_span_blocks_and_run_on_in_zeros_past_the_end() {
        let (pkg, fetches) = (package(), Cell::new(0));
        let (mut server, mut fetch) = (server_for(&pkg), fetch_from(&pkg, &fetches));
        let (_, body) = split(server.handle(&read_request(4000, 4096), &mut fetch).unwrap());
        assert_eq!(body, pkg[4000..8096]);
        assert_eq!(server.served(), 8096);
        let (_, body) = split(server.handle(&read_request(8192, 4096), &mut fetch).unwrap());
        assert_eq!(&body[..2048], &pkg[8192..]);
        assert!(body[2048..].iter().all(|&b| b == 0));
        assert_eq!(server.served(), 10_240);
        let (_, body) = split(server.handle(&read_request(12_288, 100), &mut fetch).unwrap());
        assert_eq!(body, vec![0; 100]);
        // Blocks 0, 1 and 2; the block past the end needs no fetch.
        assert_eq!(fetches.get(), 3);
    }

    #[test]
    fn a_read_at_the_end_of_the_package_is_not_progress_through_it() {
        let (pkg, fetches) = (package(), Cell::new(0));
        let (mut server, mut fetch) = (server_for(&pkg), fetch_from(&pkg, &fetches));
        // A zip's own table of contents sits at the end of the file, and
        // reading it says nothing about how far the payload has come.
        let last = pkg.len() as u64 - 100;
        split(server.handle(&read_request(last, 100), &mut fetch).unwrap());
        assert_eq!(server.progress_from(4096), 4096, "nothing of the payload is read yet");
        split(server.handle(&read_request(4096, 4096), &mut fetch).unwrap());
        assert_eq!(server.progress_from(4096), 8192);
        assert_eq!(server.progress_from(8192), 8192, "a byte no read began at");
    }

    #[test]
    fn a_block_that_changes_fails_every_read_after() {
        let (mut pkg, fetches) = (package(), Cell::new(0));
        let mut server = server_for(&pkg);
        let serve = |server: &mut Server, pkg: &[u8], offset| {
            let mut fetch = fetch_from(pkg, &fetches);
            split(server.handle(&read_request(offset, 4096), &mut fetch).unwrap()).0.error
        };
        assert_eq!(serve(&mut server, &pkg, 0), 0);
        assert_eq!(serve(&mut server, &pkg, 4096), 0);
        pkg[10] ^= 0xff;
        assert_eq!(serve(&mut server, &pkg, 0), -libc::EIO);
        assert_eq!(server.fault(), Some(&Fault::Changed(0)));
        assert_eq!(serve(&mut server, &pkg, 8192), -libc::EIO);
    }

    #[test]
    fn a_failed_fetch_fails_the_read() {
        let offer = Offer { size: 10_240, block_size: 4096 };
        let mut server = Server::new(offer);
        let mut fetch = |_: u64, _: &mut [u8]| Err("USB: the computer closed the stream".to_string());
        let (h, _) = split(server.handle(&read_request(0, 4096), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::EIO);
        assert_eq!(server.fault(), Some(&Fault::Transfer("USB: the computer closed the stream".into())));
    }

    #[test]
    fn unknown_and_unsupported_requests_get_enosys() {
        let (pkg, fetches) = (package(), Cell::new(0));
        let (mut server, mut fetch) = (server_for(&pkg), fetch_from(&pkg, &fetches));
        let (h, _) = split(server.handle(&request_raw(fuse_opcode::FUSE_STATFS, ROOT_ID, &[]), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::ENOSYS);
        let mut odd = request_raw(fuse_opcode::FUSE_STATFS, ROOT_ID, &[]);
        odd[4..8].copy_from_slice(&999u32.to_le_bytes());
        let (h, _) = split(server.handle(&odd, &mut fetch).unwrap());
        assert_eq!(h.error, -libc::ENOSYS);
        assert!(server.handle(&[0; 8], &mut fetch).is_err());
    }

    #[test]
    fn a_file_on_the_phone_is_served_in_blocks_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boot.img");
        let content: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let (mut file, offer) = LocalFile::open(&path).unwrap();
        assert_eq!(offer, Offer { size: 150_000, block_size: FILE_BLOCK });
        assert!(offer.check().is_ok());
        let mut server = Server::new(offer);
        let mut fetch = |index: u64, out: &mut [u8]| file.read_block(index, out);
        // Across the first block into the second, and the short last one.
        let (h, body) = split(server.handle(&read_request(65_000, 1_000), &mut fetch).unwrap());
        assert_eq!(h.error, 0);
        assert_eq!(body, content[65_000..66_000]);
        let (_, body) = split(server.handle(&read_request(131_072, 65_536), &mut fetch).unwrap());
        assert_eq!(body[..18_928], content[131_072..]);
        assert!(body[18_928..].iter().all(|&b| b == 0), "zeros past the end");
    }

    #[test]
    fn a_file_cut_short_while_served_fails_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.zip");
        std::fs::write(&path, vec![7u8; 200_000]).unwrap();
        let (mut file, offer) = LocalFile::open(&path).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_len(70_000).unwrap();
        let mut server = Server::new(offer);
        let mut fetch = |index: u64, out: &mut [u8]| file.read_block(index, out);
        let (h, _) = split(server.handle(&read_request(65_536, 4096), &mut fetch).unwrap());
        assert_eq!(h.error, -libc::EIO);
        assert!(matches!(server.fault(), Some(Fault::Transfer(_))));
    }

    #[test]
    fn a_file_too_big_for_the_usual_blocks_gets_bigger_ones() {
        assert_eq!(file_block(0), FILE_BLOCK);
        assert_eq!(file_block(7_103_041_280), FILE_BLOCK, "a whole system update");
        assert_eq!(file_block(16 << 30), FILE_BLOCK, "the most the usual blocks hold");
        assert_eq!(file_block((16 << 30) + 1), 2 * FILE_BLOCK);
        let size = 100 << 30;
        let offer = Offer { size, block_size: file_block(size) };
        assert!(offer.check().is_ok());
    }
}
