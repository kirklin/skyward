//! Whole partitions, kept as they are on the block device: read into one
//! file each, and written back the same way.
//!
//! A partition's files can be backed up as files; this keeps the partition
//! itself, which is what a phone that will not start is brought back with:
//! the image puts back every block, the filesystem, its labels and what
//! sits in the gaps alike.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{ensure, Context, Result};
use lz4_flex::frame::{FrameDecoder, FrameEncoder};
use sha2::{Digest, Sha256};

/// Where images go, from the top of the place they are kept in.
pub const FOLDER: &str = "Skyward/partitions";

/// The bytes read or written between two reports of how far it has come.
const CHUNK: u64 = 1 << 20;

/// The file an image of the partition `name` is kept in.
pub fn file_name(name: &str) -> String {
    format!("{name}.img")
}

/// The file a compressed image of the partition `name` is kept in.
pub fn packed_name(name: &str) -> String {
    format!("{name}.img.lz4")
}

/// The bytes `path` holds; a partition's are the bytes of its device.
pub fn bytes_of(path: &Path) -> Result<u64> {
    let meta = fs::metadata(path).with_context(|| format!("measure {}", path.display()))?;
    match meta.is_file() {
        true => Ok(meta.len()),
        false => skyward_block::size_bytes(path).with_context(|| format!("measure {}", path.display())),
    }
}

/// Read the whole of `from` into `to`, telling how far it has come through
/// `done`, in bytes written.
pub fn read_out(from: &Path, to: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<u64> {
    let len = bytes_of(from)?;
    let mut src = File::open(from).with_context(|| format!("read {}", from.display()))?;
    let mut dst = File::create(to).with_context(|| format!("write {}", to.display()))?;
    let mut buf = vec![0u8; CHUNK as usize];
    let mut left = len;
    let mut at = 0u64;
    while left > 0 {
        if stop.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("stopped as asked"));
        }
        let want = buf.len().min(left as usize);
        // A block device may hand back less than asked for in one go, and
        // its end of file is the end of the partition, not of the read.
        src.read_exact(&mut buf[..want]).with_context(|| format!("read {} at {at}", from.display()))?;
        dst.write_all(&buf[..want]).with_context(|| format!("write {} at {at}", to.display()))?;
        at += want as u64;
        left -= want as u64;
        done.store(at, Ordering::Relaxed);
    }
    dst.sync_all().with_context(|| format!("write {}", to.display()))?;
    Ok(at)
}

/// Read the whole of `from` into `to` compressed with LZ4, telling how far
/// it has come through `done`, and give the bytes it read and their SHA-256.
///
/// Compression is what makes a backup of every partition fit in one phone's
/// storage: the images are read once and never expanded again until one is
/// written back.
pub fn read_out_packed(from: &Path, to: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<(u64, String)> {
    let len = bytes_of(from)?;
    let mut src = File::open(from).with_context(|| format!("read {}", from.display()))?;
    let mut enc = FrameEncoder::new(File::create(to).with_context(|| format!("write {}", to.display()))?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK as usize];
    let mut left = len;
    let mut at = 0u64;
    while left > 0 {
        if stop.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("stopped as asked"));
        }
        let want = buf.len().min(left as usize);
        src.read_exact(&mut buf[..want]).with_context(|| format!("read {} at {at}", from.display()))?;
        hasher.update(&buf[..want]);
        enc.write_all(&buf[..want]).with_context(|| format!("write {} at {at}", to.display()))?;
        at += want as u64;
        left -= want as u64;
        done.store(at, Ordering::Relaxed);
    }
    let out = enc.finish().with_context(|| format!("finish {}", to.display()))?;
    out.sync_all().with_context(|| format!("write {}", to.display()))?;
    Ok((at, hex(&hasher.finalize())))
}

/// Write the compressed image at `from` back into the partition at `to`,
/// telling how far it has come through `done`, and give the bytes it wrote
/// and their SHA-256, which the caller compares with what was taken.
///
/// An image that unpacks to more than the partition holds stops before the
/// end of it is reached, so the partition is left short rather than
/// another's.
pub fn write_back_packed(from: &Path, to: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<(u64, String)> {
    let room = bytes_of(to)?;
    let mut dec = FrameDecoder::new(File::open(from).with_context(|| format!("read {}", from.display()))?);
    let mut dst = File::open(to).with_context(|| format!("write {}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK as usize];
    let mut at = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("stopped as asked"));
        }
        let want = dec.read(&mut buf).with_context(|| format!("unpack {} at {at}", from.display()))?;
        if want == 0 {
            break;
        }
        ensure!(
            at + want as u64 <= room,
            "{} unpacks past the {room} bytes {} holds",
            from.display(),
            to.display()
        );
        dst.write_all(&buf[..want]).with_context(|| format!("write {} at {at}", to.display()))?;
        hasher.update(&buf[..want]);
        at += want as u64;
        done.store(at, Ordering::Relaxed);
    }
    dst.sync_all().with_context(|| format!("write {}", to.display()))?;
    Ok((at, hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write the image at `from` back into the partition at `to`, telling how
/// far it has come through `done`, then read `to` back and compare it whole
/// with the image.
///
/// An image larger than the partition it is meant for is refused before
/// anything is written: written anyway, it would leave the partition half
/// another phone's, which is the one state nothing here can undo.
pub fn write_back(from: &Path, to: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<()> {
    let len = bytes_of(from)?;
    let room = bytes_of(to)?;
    ensure!(
        len <= room,
        "{} holds {len} bytes and {} holds {room}: the image is not for this partition",
        from.display(),
        to.display()
    );
    let mut src = File::open(from).with_context(|| format!("read {}", from.display()))?;
    let mut dst = File::open(to).with_context(|| format!("write {}", to.display()))?;
    let mut buf = vec![0u8; CHUNK as usize];
    let mut left = len;
    let mut at = 0u64;
    while left > 0 {
        if stop.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("stopped as asked"));
        }
        let want = buf.len().min(left as usize);
        src.read_exact(&mut buf[..want]).with_context(|| format!("read {} at {at}", from.display()))?;
        dst.write_all(&buf[..want]).with_context(|| format!("write {} at {at}", to.display()))?;
        at += want as u64;
        left -= want as u64;
        done.store(at, Ordering::Relaxed);
    }
    dst.sync_all().with_context(|| format!("write {}", to.display()))?;
    drop(dst);
    // Read back what was just written: a block device that took the bytes
    // without keeping them reads differently, and nothing else here would
    // say so.
    let wrote = digest(from, len, &AtomicU64::new(0))?;
    let back = digest(to, len, done)?;
    ensure!(
        wrote == back,
        "{} read back as {back}, not the {wrote} written",
        to.display()
    );
    Ok(())
}

/// The SHA-256 of the first `len` bytes of `path`, telling how far it has
/// come through `done`.
pub fn digest(path: &Path, len: u64, done: &AtomicU64) -> Result<String> {
    let f = File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut at = 0u64;
    let mut told = 0u64;
    crate::store::digest(f.take(len), len, &mut |n| {
        at += n;
        // One report a megabyte keeps a page's counter from taking the
        // thread's time.
        if at - told >= CHUNK || at == len {
            told = at;
            done.store(at, Ordering::Relaxed);
        }
    })
    .with_context(|| format!("read {}", path.display()))
}

/// The images kept in `folder`, by file name and bytes, biggest first: a
/// folder that is not there keeps none.
pub fn kept(folder: &Path) -> Result<Vec<(String, u64)>> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", folder.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata().with_context(|| format!("measure {}", entry.path().display()))?;
        if name.starts_with('.') || !meta.is_file() {
            continue;
        }
        // The sidecars that name what an image checks as are not images.
        if name.ends_with(".sha256") {
            continue;
        }
        found.push((name, meta.len()));
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(found)
}

/// The file beside an image that says which partition it was read from, how
/// many bytes it holds and what it checks as.
pub fn sidecar(image: &Path) -> std::path::PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(".sha256");
    std::path::PathBuf::from(name)
}

/// Write what `image` holds and where it came from, beside it.
pub fn write_sidecar(image: &Path, partition: &str, bytes: u64, sha256: &str) -> Result<()> {
    let path = sidecar(image);
    let text = format!("partition={partition}\nbytes={bytes}\nsha256={sha256}\n");
    fs::write(&path, text).with_context(|| format!("write {}", path.display()))
}

/// Write what a compressed `image` holds and where it came from, beside it,
/// with the word that says it is packed: written back as it stands, it
/// would put LZ4's bytes into the partition rather than the partition's.
pub fn write_sidecar_packed(image: &Path, partition: &str, bytes: u64, sha256: &str) -> Result<()> {
    let path = sidecar(image);
    let text = format!("partition={partition}\nbytes={bytes}\nsha256={sha256}\npacked=lz4\n");
    fs::write(&path, text).with_context(|| format!("write {}", path.display()))
}

/// Whether the image is kept compressed, as its sidecar says.
pub fn packed(image: &Path) -> Result<bool> {
    let path = sidecar(image);
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(text.lines().any(|line| line.trim() == "packed=lz4"))
}

/// What the file beside `image` says: the partition it came from, its bytes
/// and its digest. An image without one cannot be written back, as which
/// partition it belongs to would be a guess.
pub fn read_sidecar(image: &Path) -> Result<(String, u64, String)> {
    let path = sidecar(image);
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")) )
            .map(|v| v.trim().to_string())
            .with_context(|| format!("{} says nothing of {key}", path.display()))
    };
    let partition = value("partition")?;
    let bytes: u64 = value("bytes")?.parse().with_context(|| format!("{} names bytes", path.display()))?;
    let sha256 = value("sha256")?;
    Ok((partition, bytes, sha256))
}
