//! The files a part of a backup is kept in: what the part holds, compressed
//! or not, cut into files of a size every filesystem a backup may go to
//! takes, with a digest of what it holds to check it by.

use anyhow::{ensure, Context, Result};
use lz4_flex::frame::{BlockSize, FrameDecoder, FrameEncoder, FrameInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skyward_files::{hex, Ownership};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// The most one file holds: FAT, which USB drives often hold, takes no file
/// of 4 GiB or more.
pub const FILE_BYTES: u64 = 1 << 30;

/// A part as its files keep it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// The files, in order, in the backup's folder.
    pub files: Vec<String>,
    /// The bytes the part holds, and the bytes its files take.
    pub size: u64,
    pub stored: u64,
    pub compressed: bool,
    /// The SHA-256 of what the part holds, in hex.
    pub sha256: String,
}

/// Files in `folder` named `<name>.000` on, each cut at `limit` bytes and
/// given what `ownership` says.
struct Files {
    folder: PathBuf,
    name: String,
    ownership: Ownership,
    limit: u64,
    open: Option<File>,
    in_open: u64,
    names: Vec<String>,
    written: u64,
}

impl Files {
    /// Write the file open to the storage, and make the next one.
    fn next(&mut self) -> io::Result<()> {
        if let Some(full) = self.open.take() {
            full.sync_all()?;
        }
        let name = format!("{}.{:03}", self.name, self.names.len());
        let file = skyward_files::make_file(&self.folder.join(&name), self.ownership).map_err(io::Error::other)?;
        self.names.push(name);
        self.open = Some(file);
        self.in_open = 0;
        Ok(())
    }

    /// Write what is left to the storage, and give the names of the files.
    fn finish(mut self) -> io::Result<(Vec<String>, u64)> {
        if self.open.is_none() {
            self.next()?;
        }
        if let Some(last) = self.open.take() {
            last.sync_all()?;
        }
        Ok((self.names, self.written))
    }
}

impl Write for Files {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.open.is_none() || self.in_open == self.limit {
            self.next()?;
        }
        let room = (self.limit - self.in_open).min(buf.len() as u64) as usize;
        let n = self.open.as_mut().expect("a file is open").write(&buf[..room])?;
        self.in_open += n as u64;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.open.as_mut().map_or(Ok(()), |f| f.flush())
    }
}

enum Sink {
    Plain(Files),
    Lz4(FrameEncoder<Files>),
}

/// A part being written to its files.
pub struct Writer {
    sink: Sink,
    digest: Sha256,
    size: u64,
}

impl Writer {
    /// Begin the part called `name` in the backup's `folder`, its files
    /// given what `ownership` says.
    pub fn create(folder: &Path, name: &str, compress: bool, ownership: Ownership) -> Writer {
        Writer::cut_at(folder, name, compress, ownership, FILE_BYTES)
    }

    fn cut_at(folder: &Path, name: &str, compress: bool, ownership: Ownership, limit: u64) -> Writer {
        let files =
            Files { folder: folder.to_path_buf(), name: name.into(), ownership, limit, open: None, in_open: 0, names: Vec::new(), written: 0 };
        let sink = match compress {
            true => {
                let info = FrameInfo::new().block_size(BlockSize::Max4MB).content_checksum(true);
                Sink::Lz4(FrameEncoder::with_frame_info(info, files))
            }
            false => Sink::Plain(files),
        };
        Writer { sink, digest: Sha256::new(), size: 0 }
    }

    /// Write the rest to the storage, and give what the part's files keep.
    pub fn finish(self) -> Result<Stored> {
        let (files, compressed) = match self.sink {
            Sink::Plain(files) => (files, false),
            Sink::Lz4(encoder) => (encoder.finish().context("finish compressing")?, true),
        };
        let (files, stored) = files.finish().context("write the backup's files")?;
        Ok(Stored { files, size: self.size, stored, compressed, sha256: hex(&self.digest.finalize()) })
    }
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = match &mut self.sink {
            Sink::Plain(files) => files.write(buf)?,
            Sink::Lz4(encoder) => encoder.write(buf)?,
        };
        self.digest.update(&buf[..n]);
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.sink {
            Sink::Plain(files) => files.flush(),
            Sink::Lz4(encoder) => encoder.flush(),
        }
    }
}

/// A part's files read one after another, as one.
struct Joined {
    folder: PathBuf,
    names: Vec<String>,
    next: usize,
    open: Option<BufReader<File>>,
}

impl Read for Joined {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(file) = &mut self.open {
                let n = file.read(buf)?;
                if n > 0 || buf.is_empty() {
                    return Ok(n);
                }
            }
            let Some(name) = self.names.get(self.next) else { return Ok(0) };
            let path = self.folder.join(name);
            let file = File::open(&path).map_err(|e| io::Error::new(e.kind(), format!("open {}: {e}", path.display())))?;
            self.open = Some(BufReader::with_capacity(1 << 20, file));
            self.next += 1;
        }
    }
}

/// A part read back from its files, what it holds counted and digested as
/// it goes.
pub struct Reader {
    source: Box<dyn Read + Send>,
    digest: Sha256,
    size: u64,
}

impl Reader {
    pub fn open(folder: &Path, stored: &Stored) -> Reader {
        let joined = Joined { folder: folder.to_path_buf(), names: stored.files.clone(), next: 0, open: None };
        let source: Box<dyn Read + Send> = match stored.compressed {
            true => Box::new(FrameDecoder::new(joined)),
            false => Box::new(joined),
        };
        Reader { source, digest: Sha256::new(), size: 0 }
    }

    /// Once all of it is read: whether it is what was stored.
    pub fn check(self, stored: &Stored) -> Result<()> {
        ensure!(self.size == stored.size, "the backup holds {} bytes where {} were stored", self.size, stored.size);
        let digest = hex(&self.digest.finalize());
        ensure!(digest == stored.sha256, "the backup does not hold what was stored: its digest is {digest}, {} was stored", stored.sha256);
        Ok(())
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.source.read(buf)?;
        self.digest.update(&buf[..n]);
        self.size += n as u64;
        Ok(n)
    }
}

/// The SHA-256 of the first `len` bytes `from` gives, in hex as a part's is
/// kept: a partition read back once a part is written to it, to see that
/// it holds the part. Tells how many bytes are read so far.
pub fn digest(mut from: impl Read, len: u64, progress: &mut dyn FnMut(u64)) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    let mut done = 0u64;
    while done < len {
        let want = (len - done).min(buf.len() as u64) as usize;
        from.read_exact(&mut buf[..want])?;
        digest.update(&buf[..want]);
        done += want as u64;
        progress(done);
    }
    Ok(hex(&digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(test: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-store-{test}-{}", std::process::id()));
        if at.exists() {
            fs::remove_dir_all(&at).unwrap();
        }
        fs::create_dir_all(&at).unwrap();
        at
    }

    /// Bytes that compress some way but not all the way.
    fn sample(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8 ^ (i / 997) as u8).collect()
    }

    #[test]
    fn a_part_is_cut_into_files_and_read_back_whole() {
        let at = scratch("cut");
        for compress in [false, true] {
            let name = if compress { "packed" } else { "plain" };
            let held = sample(2_500);
            let mut w = Writer::cut_at(&at, name, compress, Ownership::Kept, 1_000);
            w.write_all(&held).unwrap();
            let stored = w.finish().unwrap();
            assert_eq!((stored.size, stored.compressed), (2_500, compress));
            if !compress {
                assert_eq!(stored.files, ["plain.000", "plain.001", "plain.002"], "cut at the limit");
                assert_eq!(stored.stored, 2_500);
            }
            let mut r = Reader::open(&at, &stored);
            let mut back = Vec::new();
            r.read_to_end(&mut back).unwrap();
            assert_eq!(back, held);
            r.check(&stored).unwrap();
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_part_that_changed_after_it_was_stored_fails_its_check() {
        let at = scratch("changed");
        let mut w = Writer::cut_at(&at, "boot", false, Ownership::Kept, 1_000);
        w.write_all(&sample(1_500)).unwrap();
        let stored = w.finish().unwrap();
        let second = at.join("boot.001");
        let mut bytes = fs::read(&second).unwrap();
        bytes[10] ^= 1;
        fs::write(&second, bytes).unwrap();
        let mut r = Reader::open(&at, &stored);
        io::copy(&mut r, &mut io::sink()).unwrap();
        assert!(r.check(&stored).is_err());
        fs::remove_file(&second).unwrap();
        let mut r = Reader::open(&at, &stored);
        assert!(io::copy(&mut r, &mut io::sink()).is_err(), "a file gone is an error");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn what_is_read_back_digests_as_the_part_was_stored() {
        let at = scratch("digest");
        let held = sample(3_000);
        let mut w = Writer::cut_at(&at, "dtbo", true, Ownership::Kept, 1_000);
        w.write_all(&held).unwrap();
        let stored = w.finish().unwrap();
        // A partition is larger than what went into it; only the part's
        // bytes count.
        let mut partition = held.clone();
        partition.extend([0xff; 512]);
        let mut told = 0;
        assert_eq!(digest(partition.as_slice(), stored.size, &mut |n| told = n).unwrap(), stored.sha256);
        assert_eq!(told, 3_000);
        partition[2_999] ^= 1;
        assert_ne!(digest(partition.as_slice(), stored.size, &mut |_| {}).unwrap(), stored.sha256);
        assert!(digest(&held[..10], stored.size, &mut |_| {}).is_err(), "a partition too small to hold the part");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn an_empty_part_still_has_a_file() {
        let at = scratch("empty");
        let stored = Writer::cut_at(&at, "fsc", true, Ownership::Kept, 1_000).finish().unwrap();
        assert_eq!(stored.files, ["fsc.000"]);
        let mut r = Reader::open(&at, &stored);
        assert_eq!(io::copy(&mut r, &mut io::sink()).unwrap(), 0);
        r.check(&stored).unwrap();
        fs::remove_dir_all(&at).unwrap();
    }
}
