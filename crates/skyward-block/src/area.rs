//! A fixed place in a partition, held by a file in the filesystem on it:
//! the filesystem is told never to move the file's blocks or let it be
//! deleted, and the place is read and written through the partition
//! itself, around the filesystem and the encryption under it, as Android
//! keeps the images of updates and of dynamic systems in the data
//! partition.

use crate::dm::Target;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What starts every area: what it is, and which one.
pub const HEADER: u64 = 4096;
const MAGIC: &[u8; 16] = b"SKYWARD AREA\0\0\0\x01";
const SECTOR: u64 = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    /// The partition the area lies in, by its name, such as `userdata`.
    pub partition: String,
    /// The file that holds the area's blocks.
    pub file: PathBuf,
    /// Made up when the area is, 32 hexadecimal digits, and written in its
    /// header.
    pub id: String,
    /// Bytes, the header's among them.
    pub size: u64,
    /// Where the area lies in the partition, in the order of its own bytes:
    /// each piece's offset and length, in bytes.
    pub extents: Vec<(u64, u64)>,
}

impl Area {
    /// Where the area's bytes `from..from + len` lie in the partition, as
    /// pieces of offset and length.
    pub fn pieces(&self, from: u64, len: u64) -> Result<Vec<(u64, u64)>> {
        let end = from.checked_add(len).context("an area's range runs past what a u64 counts")?;
        ensure!(end <= self.size, "bytes {from}..{end} run past the {} the area holds", self.size);
        let mut out = Vec::new();
        let mut at = 0;
        for &(offset, length) in &self.extents {
            let (lo, hi) = (from.max(at), end.min(at + length));
            if lo < hi {
                out.push((offset + (lo - at), hi - lo));
            }
            at += length;
        }
        ensure!(at >= self.size, "the area's {} bytes lie in only {at} bytes of the partition", self.size);
        Ok(out)
    }

    /// The table that lays the area's bytes `from..from + len` out as one
    /// device over the partition, which is block device `device`.
    pub fn targets(&self, from: u64, len: u64, device: (u32, u32)) -> Result<Vec<Target>> {
        self.pieces(from, len)?
            .into_iter()
            .map(|(offset, length)| {
                ensure!(offset % SECTOR == 0 && length % SECTOR == 0, "a piece at {offset} of {length} bytes is not in whole sectors");
                Ok(Target::Linear { sectors: length / SECTOR, device, offset: offset / SECTOR })
            })
            .collect()
    }

    /// What the area's header holds: what it is, its id, and its size.
    pub fn header(&self) -> Result<Vec<u8>> {
        let mut out = vec![0u8; HEADER as usize];
        out[..16].copy_from_slice(MAGIC);
        out[16..32].copy_from_slice(&unhex(&self.id)?);
        out[32..40].copy_from_slice(&self.size.to_le_bytes());
        Ok(out)
    }

    /// Check that `header`, read from the area's start, is the area's own:
    /// that the place still holds the area, and not what the filesystem
    /// gave the blocks to once the file was gone.
    pub fn check(&self, header: &[u8]) -> Result<()> {
        ensure!(header.len() >= HEADER as usize, "an area's header takes {HEADER} bytes, not {}", header.len());
        ensure!(header[..16] == MAGIC[..], "{} holds no area where it was", self.partition);
        ensure!(header[16..32] == unhex(&self.id)?[..], "{} holds another area where this one was", self.partition);
        let size = u64::from_le_bytes(header[32..40].try_into().expect("eight bytes"));
        ensure!(size == self.size, "the area's header says {size} bytes, and the area is {}", self.size);
        Ok(())
    }
}

/// A new area's id: 16 random bytes in hexadecimal.
pub fn new_id() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).context("read /dev/urandom")?;
    Ok(hex::encode(bytes))
}

fn unhex(id: &str) -> Result<[u8; 16]> {
    ensure!(id.len() == 32 && id.is_ascii(), "an area's id is 32 hexadecimal digits, not {id:?}");
    let mut out = [0u8; 16];
    for (k, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&id[k * 2..k * 2 + 2], 16).with_context(|| format!("an area's id is hexadecimal, not {id:?}"))?;
    }
    Ok(out)
}

#[cfg(target_os = "android")]
mod sys {
    use super::{Area, HEADER};
    use crate::dm;
    use anyhow::{bail, ensure, Context, Result};
    use std::fs::{self, File, OpenOptions};
    use std::io::{ErrorKind, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    const F2FS_MAGIC: i64 = 0xF2F5_2010;
    const EXT4_MAGIC: i64 = 0xEF53;
    // From <linux/fs.h>, <linux/fiemap.h> and <linux/f2fs.h>, on a 64-bit
    // kernel. bionic's ioctl request is a c_int; the bits are reinterpreted.
    const FS_IOC_GETFLAGS: u32 = 0x8008_6601;
    const FS_IOC_SETFLAGS: u32 = 0x4008_6602;
    const FS_IOC_FIEMAP: u32 = 0xC020_660B;
    const F2FS_IOC_SET_PIN_FILE: u32 = 0x4004_F50D;
    const F2FS_IOC_GET_PIN_FILE: u32 = 0x8004_F50E;
    const FS_IMMUTABLE_FL: libc::c_int = 0x0000_0010;
    const FS_NOCOW_FL: libc::c_int = 0x0080_0000;
    const FIEMAP_FLAG_SYNC: u32 = 0x1;
    const FIEMAP_EXTENT_LAST: u32 = 0x1;
    /// What makes an extent no fixed place on the device: not known yet,
    /// not written, not aligned, kept in the inode or another file's tail,
    /// or shared with another file.
    const UNFIT: u32 = 0x2 | 0x4 | 0x8 | 0x100 | 0x200 | 0x400 | 0x800 | 0x2000;
    /// As many extents as Android's own images may have.
    const MAX_EXTENTS: usize = 50_000;
    const FIEMAP_LEN: usize = 32;
    const EXTENT_LEN: usize = 56;

    fn ioctl(f: &File, request: u32, arg: *mut libc::c_void) -> std::io::Result<()> {
        if unsafe { libc::ioctl(f.as_raw_fd(), request as libc::c_int, arg) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn flags(f: &File) -> Result<libc::c_int> {
        let mut flags: libc::c_int = 0;
        ioctl(f, FS_IOC_GETFLAGS, &mut flags as *mut _ as *mut libc::c_void).context("read the file's flags")?;
        Ok(flags)
    }

    fn set_flags(f: &File, flags: libc::c_int) -> Result<()> {
        let mut flags = flags;
        ioctl(f, FS_IOC_SETFLAGS, &mut flags as *mut _ as *mut libc::c_void).context("set the file's flags")
    }

    /// The filesystem `path` is on: its kind and its block size.
    fn filesystem(path: &Path) -> Result<(i64, u64)> {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).context("a path with a NUL in it")?;
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("look at the filesystem of {}", path.display()));
        }
        Ok((s.f_type as i64, s.f_bsize as u64))
    }

    /// The name the kernel gives block device `dev`, such as dm-8.
    fn block_name(dev: (u32, u32)) -> Result<String> {
        let link = format!("/sys/dev/block/{}:{}", dev.0, dev.1);
        let target = fs::read_link(&link).with_context(|| format!("read {link}"))?;
        Ok(target.file_name().context("a block device's link names it")?.to_string_lossy().into_owned())
    }

    /// The partition under block device `name`, through the device-mapper
    /// devices that lay it out one to one, as the data partition's
    /// encryption does: its name and its own (major, minor). A layer that
    /// moves blocks about is no way down to the partition.
    fn partition_under(name: &str) -> Result<(String, (u32, u32))> {
        if let Some(dm_name) = name.strip_prefix("dm-").map(|_| fs::read_to_string(format!("/sys/block/{name}/dm/name"))).transpose()? {
            let dm_name = dm_name.trim();
            let lines = dm::table(dm_name)?;
            let [line] = lines.as_slice() else { bail!("{dm_name} lays its device out in {} pieces", lines.len()) };
            ensure!(line.start == 0, "{dm_name} starts its device at sector {}", line.start);
            match line.kind.as_str() {
                "default-key" | "crypt" | "bow" => {}
                "linear" => ensure!(line.text.split(' ').nth(1) == Some("0"), "{dm_name} starts part way into its device"),
                other => bail!("{dm_name} is a {other} device, which moves blocks about"),
            }
            let slaves = format!("/sys/block/{name}/slaves");
            let under: Vec<String> =
                fs::read_dir(&slaves).with_context(|| format!("read {slaves}"))?.map(|e| Ok(e?.file_name().to_string_lossy().into_owned())).collect::<Result<_>>()?;
            let [under] = under.as_slice() else { bail!("{dm_name} lies over {} devices", under.len()) };
            return partition_under(under);
        }
        let uevent = format!("/sys/class/block/{name}/uevent");
        let text = fs::read_to_string(&uevent).with_context(|| format!("read {uevent}"))?;
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(str::to_string);
        let partition = field("PARTNAME=").with_context(|| format!("{name} is no named partition"))?;
        let number = |key: &str| -> Result<u32> { field(key).with_context(|| format!("{uevent} has no {key}"))?.parse().with_context(|| format!("{uevent}: {key}")) };
        Ok((partition, (number("MAJOR=")?, number("MINOR=")?)))
    }

    /// Where the file `f` lies on the device its filesystem is on, pieces
    /// next to each other joined.
    fn extents(f: &File) -> Result<Vec<(u64, u64)>> {
        let mut buf = vec![0u8; FIEMAP_LEN + MAX_EXTENTS * EXTENT_LEN];
        buf[8..16].copy_from_slice(&u64::MAX.to_ne_bytes());
        buf[16..20].copy_from_slice(&FIEMAP_FLAG_SYNC.to_ne_bytes());
        buf[24..28].copy_from_slice(&(MAX_EXTENTS as u32).to_ne_bytes());
        ioctl(f, FS_IOC_FIEMAP, buf.as_mut_ptr() as *mut libc::c_void).context("ask where the file lies")?;
        let u64_at = |at: usize| u64::from_ne_bytes(buf[at..at + 8].try_into().expect("eight bytes"));
        let u32_at = |at: usize| u32::from_ne_bytes(buf[at..at + 4].try_into().expect("four bytes"));
        let count = u32_at(20) as usize;
        ensure!(count > 0, "the file lies nowhere yet");
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut next_logical = 0;
        for k in 0..count {
            let at = FIEMAP_LEN + k * EXTENT_LEN;
            let (logical, physical, length, flags) = (u64_at(at), u64_at(at + 8), u64_at(at + 16), u32_at(at + 40));
            ensure!(flags & UNFIT == 0, "the file's bytes at {logical} have no fixed place (flags {flags:#x})");
            ensure!((k + 1 == count) == (flags & FIEMAP_EXTENT_LAST != 0), "the filesystem told of the file's pieces out of order");
            ensure!(logical == next_logical, "the file has a hole at {next_logical}");
            next_logical = logical + length;
            match out.last_mut() {
                Some((offset, len)) if *offset + *len == physical => *len += length,
                _ => out.push((physical, length)),
            }
        }
        Ok(out)
    }

    /// Whether f2fs holds the file's blocks where they are: the file is
    /// pinned, and no block of it has been moved.
    fn pinned(f: &File) -> Result<bool> {
        if flags(f)? & FS_NOCOW_FL == 0 {
            return Ok(false);
        }
        let mut moved: u32 = 0;
        ioctl(f, F2FS_IOC_GET_PIN_FILE, &mut moved as *mut _ as *mut libc::c_void).context("ask whether the file is pinned")?;
        Ok(moved == 0)
    }

    fn pin(f: &File) -> Result<()> {
        let mut on: u32 = 1;
        ioctl(f, F2FS_IOC_SET_PIN_FILE, &mut on as *mut _ as *mut libc::c_void).context("pin the file")
    }

    /// Whether f2fs on the device the file is on pins a file before any
    /// of it is allocated, which it then allocates where it stays.
    fn pins_first(f: &File) -> Result<bool> {
        let dev = f.metadata()?.dev() as libc::dev_t;
        let name = block_name((libc::major(dev) as u32, libc::minor(dev) as u32))?;
        let features = fs::read_to_string(format!("/sys/fs/f2fs/{name}/features")).with_context(|| format!("read the features of f2fs on {name}"))?;
        Ok(features.trim().split(", ").any(|f| f == "pin_file"))
    }

    /// Make an area of `size` bytes, the header's among them, held by a
    /// new file at `path`, as Android's own libfiemap makes its images:
    /// pinned before it is allocated where f2fs can, written through once
    /// where it cannot and on ext4, so that no block of it stays unwritten,
    /// then pinned and told where it lies. The file is then made immutable,
    /// so that nothing deletes it while the area is in use, and the header
    /// is written at the area's start. `progress` hears how much is
    /// allocated, of `size`.
    pub fn create(path: &Path, size: u64, progress: &mut dyn FnMut(u64, u64)) -> Result<Area> {
        let (kind, block) = filesystem(path.parent().context("an area's file is in a folder")?)?;
        ensure!(kind == F2FS_MAGIC || kind == EXT4_MAGIC, "{} is on a filesystem of kind {kind:#x}, neither f2fs nor ext4", path.display());
        let size = size.div_ceil(block) * block;
        ensure!(size > HEADER, "an area of {size} bytes holds nothing past its header");
        let mut f = OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(path).with_context(|| format!("make {}", path.display()))?;
        let made = (|| -> Result<Area> {
            let first = kind == F2FS_MAGIC && pins_first(&f)?;
            if first {
                pin(&f)?;
            }
            // f2fs may allocate part and ask to be asked again; it has run
            // out of room once a try gets no further.
            let mut got = 0;
            loop {
                if unsafe { libc::fallocate(f.as_raw_fd(), 0, 0, size as libc::off_t) } == 0 {
                    break;
                }
                let e = std::io::Error::last_os_error();
                ensure!(e.raw_os_error() == Some(libc::EAGAIN), "allocate {size} bytes for {}: {e}", path.display());
                let now = f.metadata()?.len();
                ensure!(now > got, "the data partition has no room for {size} bytes, and {now} were allocated");
                got = now;
                progress(got, size);
            }
            if !first {
                let zeros = vec![0u8; 1 << 20];
                let mut at = 0;
                while at < size {
                    let n = (size - at).min(zeros.len() as u64) as usize;
                    f.write_all(&zeros[..n]).with_context(|| format!("write {}", path.display()))?;
                    at += n as u64;
                    progress(at, size);
                }
                if kind == F2FS_MAGIC {
                    pin(&f)?;
                }
            }
            f.sync_all().with_context(|| format!("sync {}", path.display()))?;
            ensure!(kind != F2FS_MAGIC || pinned(&f)?, "f2fs did not pin {}", path.display());
            let extents = extents(&f)?;
            let dev = f.metadata()?.dev() as libc::dev_t;
            let (partition, raw) = partition_under(&block_name((libc::major(dev) as u32, libc::minor(dev) as u32))?)?;
            let area = Area { partition, file: path.to_path_buf(), id: super::new_id()?, size, extents };
            area.pieces(0, size)?;
            set_flags(&f, flags(&f)? | FS_IMMUTABLE_FL)?;
            let device = Path::new(crate::BY_NAME).join(&area.partition);
            ensure!(crate::devnum(&device)? == raw, "{} is not the partition under {}", device.display(), path.display());
            // Written through a device of its own, as everything in the
            // area is: a kernel may keep the partition from being written
            // while the filesystem on it is mounted.
            let node = area.map(NEW)?;
            let header = (|| -> Result<()> {
                use std::os::unix::fs::FileExt;
                let out = OpenOptions::new().write(true).open(&node)?;
                out.write_all_at(&area.header()?, 0)?;
                out.sync_all()?;
                Ok(())
            })();
            dm::remove(NEW)?;
            header.with_context(|| format!("write the header of {}", path.display()))?;
            progress(size, size);
            Ok(area)
        })();
        if made.is_err() {
            let _ = set_flags(&f, flags(&f).unwrap_or(0) & !FS_IMMUTABLE_FL);
            let _ = fs::remove_file(path);
        }
        made
    }

    /// The device a new area's header is written through.
    const NEW: &str = "skyward-area-new";

    impl Area {
        /// Check that the area is still where it was made: its file is
        /// there, pinned and immutable, lies where it did, and the place
        /// still starts with the area's header.
        pub fn verify(&self) -> Result<()> {
            let f = File::open(&self.file).with_context(|| format!("open {}", self.file.display()))?;
            let flags = flags(&f)?;
            ensure!(flags & FS_IMMUTABLE_FL != 0, "{} can be deleted", self.file.display());
            let (kind, _) = filesystem(&self.file)?;
            ensure!(kind != F2FS_MAGIC || pinned(&f)?, "{} is no longer pinned", self.file.display());
            ensure!(extents(&f)? == self.extents, "{} has moved", self.file.display());
            self.check_header()
        }

        /// Check, through the partition, that the place starts with the
        /// area's header, which needs no filesystem.
        pub fn check_header(&self) -> Result<()> {
            use std::os::unix::fs::FileExt;
            let device = Path::new(crate::BY_NAME).join(&self.partition);
            let f = File::open(&device).with_context(|| format!("open {}", device.display()))?;
            crate::drop_cache(&f)?;
            let mut header = vec![0u8; HEADER as usize];
            f.read_exact_at(&mut header, self.extents[0].0).with_context(|| format!("read {}", device.display()))?;
            self.check(&header)
        }

        /// Lay the whole area out as device-mapper device `name` over its
        /// partition; returns the device's node.
        pub fn map(&self, name: &str) -> Result<PathBuf> {
            let device = crate::devnum(Path::new(crate::BY_NAME).join(&self.partition))?;
            dm::create(name, &self.targets(0, self.size, device)?, false)
        }

        /// Let the file go: no longer immutable, then deleted, and its
        /// blocks are the filesystem's again.
        pub fn remove(&self) -> Result<()> {
            let f = match File::open(&self.file) {
                Ok(f) => f,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e).with_context(|| format!("open {}", self.file.display())),
            };
            set_flags(&f, flags(&f)? & !FS_IMMUTABLE_FL)?;
            fs::remove_file(&self.file).with_context(|| format!("delete {}", self.file.display()))
        }
    }
}

#[cfg(target_os = "android")]
pub use sys::create;

#[cfg(test)]
mod tests {
    use super::*;

    fn area(extents: &[(u64, u64)], size: u64) -> Area {
        Area { partition: "userdata".into(), file: "/data/skyward/a".into(), id: "00112233445566778899aabbccddeeff".into(), size, extents: extents.to_vec() }
    }

    #[test]
    fn a_range_of_the_area_is_found_across_the_pieces_it_lies_in() {
        let a = area(&[(1 << 20, 8192), (64 << 20, 4096), (8 << 20, 16384)], 28672);
        assert_eq!(a.pieces(0, 28672).unwrap(), [(1 << 20, 8192), (64 << 20, 4096), (8 << 20, 16384)]);
        assert_eq!(a.pieces(4096, 8192).unwrap(), [((1 << 20) + 4096, 4096), (64 << 20, 4096)], "the header left out");
        assert_eq!(a.pieces(13000, 100).unwrap(), [((8 << 20) + 712, 100)]);
        assert!(a.pieces(28000, 1000).is_err(), "past the area's end");
        assert!(area(&[(0, 4096)], 8192).pieces(0, 8192).is_err(), "an area its pieces do not hold");
    }

    #[test]
    fn a_range_is_laid_out_as_a_linear_table_in_sectors() {
        let a = area(&[(1 << 20, 8192), (64 << 20, 8192)], 16384);
        let t = a.targets(HEADER, 12288, (259, 14)).unwrap();
        assert_eq!(
            t,
            [
                Target::Linear { sectors: 8, device: (259, 14), offset: ((1 << 20) + 4096) / 512 },
                Target::Linear { sectors: 16, device: (259, 14), offset: (64 << 20) / 512 },
            ]
        );
        assert!(area(&[(1000, 16384)], 16384).targets(0, 16384, (259, 14)).is_err(), "a piece not in whole sectors");
    }

    #[test]
    fn the_header_tells_the_area_from_any_other_and_from_what_took_its_place() {
        let a = area(&[(0, 8192)], 8192);
        let header = a.header().unwrap();
        a.check(&header).unwrap();
        let other = Area { id: "ffeeddccbbaa99887766554433221100".into(), ..a.clone() };
        assert!(other.check(&header).is_err(), "another area's id");
        assert!(Area { size: 4096 * 3, ..a.clone() }.check(&header).is_err(), "another size");
        assert!(a.check(&vec![0u8; HEADER as usize]).is_err(), "blocks given to something else");
        assert!(Area { id: "not hex".into(), ..a }.header().is_err());
        assert_eq!(new_id().unwrap().len(), 32);
    }
}
