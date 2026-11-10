use abootimg_oxide::{
    binrw::io::Cursor, Header as StockHeader, HeaderV0, HeaderV0Versioned, HeaderV3, OsVersionPatch,
};
use anyhow::{anyhow, bail, Context, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const PAGE: usize = 4096;

/// The standard Android boot image's header, either the one versions 0 to 2
/// carry beside a kernel and a ramdisk, as every phone up to Android 10
/// boots, or the one versions 3 and 4 carry for the ramdisk alone.
pub enum Header {
    V0(HeaderV0),
    V3(HeaderV3),
}

impl Header {
    pub const fn header_version(&self) -> u32 {
        match self {
            Header::V0(h) => h.header_version(),
            Header::V3(h) => h.header_version(),
        }
    }

    pub const fn osversionpatch(&self) -> &OsVersionPatch {
        match self {
            Header::V0(h) => &h.osversionpatch,
            Header::V3(h) => &h.osversionpatch,
        }
    }

    /// The page size the image's parts are padded to.
    pub const fn page_size(&self) -> usize {
        match self {
            Header::V0(h) => h.page_size as usize,
            Header::V3(_) => PAGE,
        }
    }
}

/// A boot image with the header version it carries: `boot`, `init_boot` and
/// `recovery` are all such images. Versions 0 to 2 keep a kernel, the
/// ramdisk, and on some a second bootloader, a recovery DTBO and a DTB
/// beside them; versions 3 and 4 keep the kernel and ramdisk alone.
pub struct BootImage {
    pub header: Header,
    pub kernel: Vec<u8>,
    pub ramdisk: Vec<u8>,
    /// The second bootloader, which versions 0 to 2 may carry.
    pub second: Vec<u8>,
    /// The recovery DTBO, which versions 1 and 2 may carry.
    pub recovery_dtbo: Vec<u8>,
    /// The DTB, which version 2 may carry after the recovery DTBO.
    pub dtb: Vec<u8>,
}

fn section(bytes: &[u8], at: usize, len: u32, what: &str) -> Result<Vec<u8>> {
    let end = at + len as usize;
    bytes.get(at..end).map(<[u8]>::to_vec).ok_or_else(|| anyhow!("{what} runs past the end of the image"))
}

impl BootImage {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        BootImage::parse(&bytes).with_context(|| format!("read {}", path.display()))
    }

    /// The image `bytes` hold, as a partition holds it: the image, then
    /// whatever fills the partition after it.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        match StockHeader::parse(&mut Cursor::new(bytes)).context("header")? {
            StockHeader::V0(header) => {
                let page = header.page_size as usize;
                if !matches!(page, 2048 | 4096 | 8192 | 16384) {
                    bail!("the header's page size, {page}, is not one a boot image uses");
                }
                let kernel = section(bytes, header.kernel_position(), header.kernel_size, "kernel")?;
                let ramdisk = section(bytes, header.ramdisk_position(), header.ramdisk_size, "ramdisk")?;
                let second = section(bytes, header.second_bootloader_position(), header.second_bootloader_size, "second bootloader")?;
                let (recovery_dtbo_size, dtb_size) = match header.versioned {
                    HeaderV0Versioned::V0 => (0, 0),
                    HeaderV0Versioned::V1 { recovery_dtbo_size, .. } => (recovery_dtbo_size, 0),
                    HeaderV0Versioned::V2 { recovery_dtbo_size, dtb_size, .. } => (recovery_dtbo_size, dtb_size),
                };
                let recovery_dtbo = section(bytes, header.recovery_dtbo_position(), recovery_dtbo_size, "recovery DTBO")?;
                let dtb = match header.dtb_position() {
                    Some(at) => section(bytes, at, dtb_size, "DTB")?,
                    None => Vec::new(),
                };
                Ok(BootImage { header: Header::V0(header), kernel, ramdisk, second, recovery_dtbo, dtb })
            }
            StockHeader::V3(header) => {
                let kernel = section(bytes, HeaderV3::kernel_position(), header.kernel_size, "kernel")?;
                let ramdisk = section(bytes, header.ramdisk_position(), header.ramdisk_size, "ramdisk")?;
                Ok(BootImage { header: Header::V3(header), kernel, ramdisk, second: Vec::new(), recovery_dtbo: Vec::new(), dtb: Vec::new() })
            }
        }
    }

    /// The image with sizes taken from its parts. A version 4 header keeps its
    /// version but carries no boot signature; unlocked bootloaders do not ask
    /// for one.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        match &self.header {
            Header::V3(header) => {
                let mut header = header.clone();
                header.kernel_size = u32::try_from(self.kernel.len()).context("kernel size")?;
                header.ramdisk_size = u32::try_from(self.ramdisk.len()).context("ramdisk size")?;
                header.v4_signature_size = header.v4_signature_size.map(|_| 0);
                let mut out = Vec::new();
                StockHeader::V3(header).write(&mut out).context("write header")?;
                pad(&mut out, self.header.page_size());
                out.extend_from_slice(&self.kernel);
                pad(&mut out, self.header.page_size());
                out.extend_from_slice(&self.ramdisk);
                pad(&mut out, self.header.page_size());
                Ok(out)
            }
            Header::V0(header) => {
                let mut header = header.clone();
                header.kernel_size = u32::try_from(self.kernel.len()).context("kernel size")?;
                header.ramdisk_size = u32::try_from(self.ramdisk.len()).context("ramdisk size")?;
                header.second_bootloader_size = u32::try_from(self.second.len()).context("second bootloader size")?;
                let (has_dtbo, has_dtb) = match &mut header.versioned {
                    HeaderV0Versioned::V0 => (false, false),
                    HeaderV0Versioned::V1 { recovery_dtbo_size, .. } => {
                        *recovery_dtbo_size = u32::try_from(self.recovery_dtbo.len()).context("recovery DTBO size")?;
                        (true, false)
                    }
                    HeaderV0Versioned::V2 { recovery_dtbo_size, dtb_size, .. } => {
                        *recovery_dtbo_size = u32::try_from(self.recovery_dtbo.len()).context("recovery DTBO size")?;
                        *dtb_size = u32::try_from(self.dtb.len()).context("DTB size")?;
                        (true, true)
                    }
                };
                // The bootloader may check the header's digest of what it
                // starts; one that was written at all is written anew.
                if header.hash_digest.iter().any(|b| *b != 0) {
                    header.hash_digest = digest(header.hash_digest[20..].iter().all(|b| *b == 0), &self.parts());
                }
                let page = self.header.page_size();
                let mut out = Vec::new();
                StockHeader::V0(header).write(&mut out).context("write header")?;
                pad(&mut out, page);
                out.extend_from_slice(&self.kernel);
                pad(&mut out, page);
                out.extend_from_slice(&self.ramdisk);
                pad(&mut out, page);
                out.extend_from_slice(&self.second);
                pad(&mut out, page);
                if has_dtbo {
                    out.extend_from_slice(&self.recovery_dtbo);
                    pad(&mut out, page);
                }
                if has_dtb {
                    out.extend_from_slice(&self.dtb);
                    pad(&mut out, page);
                }
                Ok(out)
            }
        }
    }

    /// What a version 0 to 2 header's digest covers, in the order it covers
    /// them.
    fn parts(&self) -> [&[u8]; 5] {
        [&self.kernel, &self.ramdisk, &self.second, &self.recovery_dtbo, &self.dtb]
    }
}

/// The SHA-1 or SHA-256 digest a version 0 to 2 header keeps of its parts,
/// each hashed with its length as a 32-bit little-endian number, as mkbootimg
/// writes it.
fn digest(sha1: bool, parts: &[&[u8]; 5]) -> [u8; 32] {
    let mut out = [0u8; 32];
    if sha1 {
        let mut hasher = Sha1::new();
        for part in parts {
            hasher.update(part);
            hasher.update((part.len() as u32).to_le_bytes());
        }
        out[..20].copy_from_slice(&hasher.finalize());
    } else {
        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update(part);
            hasher.update((part.len() as u32).to_le_bytes());
        }
        out.copy_from_slice(&hasher.finalize());
    }
    out
}

fn pad(out: &mut Vec<u8>, page: usize) {
    out.resize(out.len().div_ceil(page) * page, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_v0(kernel: u32, ramdisk: u32) -> HeaderV0 {
        HeaderV0 {
            kernel_size: kernel,
            kernel_addr: 0x8000_8000,
            ramdisk_size: ramdisk,
            ramdisk_addr: 0x8100_0000,
            second_bootloader_size: 0,
            second_bootloader_addr: 0,
            tags_addr: 0x8000_0100,
            page_size: 2048,
            osversionpatch: OsVersionPatch(0),
            board_name: [0; 16],
            hash_digest: [0; 32],
            cmdline: Box::new([0; 1536]),
            versioned: HeaderV0Versioned::V0,
        }
    }

    #[test]
    fn a_version_0_image_keeps_every_part_across_a_round_trip() {
        let image = BootImage {
            header: Header::V0(header_v0(6, 7)),
            kernel: b"kernel".to_vec(),
            ramdisk: b"ramdisk".to_vec(),
            second: Vec::new(),
            recovery_dtbo: Vec::new(),
            dtb: Vec::new(),
        };
        let bytes = image.to_bytes().unwrap();
        assert_eq!(bytes.len(), 2048 * 3, "the header and both parts, each a page");
        let back = BootImage::parse(&bytes).unwrap();
        assert_eq!(back.header.header_version(), 0);
        assert_eq!(back.kernel, b"kernel");
        assert_eq!(back.ramdisk, b"ramdisk");
    }

    #[test]
    fn a_version_2_image_carries_its_dtbo_and_dtb() {
        let mut header = header_v0(4, 2);
        header.versioned = HeaderV0Versioned::V2 { recovery_dtbo_size: 0, recovery_dtbo_addr: 0, dtb_size: 0, dtb_addr: 0 };
        let image = BootImage {
            header: Header::V0(header),
            kernel: b"kern".to_vec(),
            ramdisk: b"rd".to_vec(),
            second: Vec::new(),
            recovery_dtbo: b"dtbo".to_vec(),
            dtb: b"dtb".to_vec(),
        };
        let bytes = image.to_bytes().unwrap();
        let back = BootImage::parse(&bytes).unwrap();
        assert_eq!(back.recovery_dtbo, b"dtbo");
        assert_eq!(back.dtb, b"dtb");
        match back.header {
            Header::V0(h) => match h.versioned {
                HeaderV0Versioned::V2 { recovery_dtbo_size, dtb_size, .. } => {
                    assert_eq!((recovery_dtbo_size, dtb_size), (4, 3));
                }
                _ => panic!("the version was lost"),
            },
            _ => panic!("the header was lost"),
        }
    }

    #[test]
    fn the_digest_covers_each_part_with_its_length() {
        let one = digest(true, &[b"a", b"", b"", b"", b""]);
        let mut hasher = Sha1::new();
        hasher.update(b"a");
        hasher.update(1u32.to_le_bytes());
        for _ in 0..4 {
            hasher.update([]);
            hasher.update(0u32.to_le_bytes());
        }
        let mut want = [0u8; 32];
        want[..20].copy_from_slice(&hasher.finalize());
        assert_eq!(one, want);
    }
}
