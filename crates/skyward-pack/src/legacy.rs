//! Boot images whose header is version 0, 1 or 2: the layout phones from
//! before vendor_boot boot their recovery from. The kernel stays, the ramdisk
//! is replaced, and the header is written back with the sizes and the SHA-1
//! the bootloader expects.

use abootimg_oxide::{binrw::io::Cursor, Header, HeaderV0, HeaderV0Versioned};
use anyhow::{bail, ensure, Context, Result};
use sha1::{Digest, Sha1};

/// The space a version 0-2 header takes before the kernel, padded to a page:
/// the longest of the three layouts, so every version finds its kernel there.
const HEADER_SPACE: usize = 1660;

pub struct LegacyImage {
    pub header: HeaderV0,
    pub kernel: Vec<u8>,
    pub ramdisk: Vec<u8>,
    second: Vec<u8>,
    dtbo: Vec<u8>,
    dtb: Vec<u8>,
}

fn section(bytes: &[u8], at: usize, len: u32, what: &str) -> Result<Vec<u8>> {
    let end = at + len as usize;
    bytes.get(at..end).map(<[u8]>::to_vec).ok_or_else(|| anyhow::anyhow!("{what} runs past the end of the image"))
}

fn page_size(header: &HeaderV0) -> Result<usize> {
    let page = header.page_size as usize;
    ensure!(page.is_power_of_two() && page >= 512, "a {page}-byte page is not a power of two");
    Ok(page)
}

impl LegacyImage {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = match Header::parse(&mut Cursor::new(bytes)).context("header")? {
            Header::V0(h) => h,
            Header::V3(_) => bail!("not a header of version 0 to 2"),
        };
        let page = page_size(&header)?;
        let kernel = section(bytes, header.kernel_position(), header.kernel_size, "kernel")?;
        let ramdisk = section(bytes, header.ramdisk_position(), header.ramdisk_size, "ramdisk")?;
        let second = section(bytes, header.second_bootloader_position(), header.second_bootloader_size, "second bootloader")?;
        let (dtbo, dtb) = match header.versioned {
            HeaderV0Versioned::V0 => (Vec::new(), Vec::new()),
            HeaderV0Versioned::V1 { recovery_dtbo_size, .. } => {
                (section(bytes, header.recovery_dtbo_position(), recovery_dtbo_size, "recovery dtbo")?, Vec::new())
            }
            HeaderV0Versioned::V2 { recovery_dtbo_size, dtb_size, .. } => {
                let dtbo = section(bytes, header.recovery_dtbo_position(), recovery_dtbo_size, "recovery dtbo")?;
                let at = header.recovery_dtbo_position() + aligned(recovery_dtbo_size as usize, page);
                (dtbo, section(bytes, at, dtb_size, "dtb")?)
            }
        };
        Ok(LegacyImage { header, kernel, ramdisk, second, dtbo, dtb })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let page = page_size(&self.header)?;
        let mut header = self.header.clone();
        header.kernel_size = u32::try_from(self.kernel.len()).context("kernel size")?;
        header.ramdisk_size = u32::try_from(self.ramdisk.len()).context("ramdisk size")?;
        header.second_bootloader_size = u32::try_from(self.second.len()).context("second bootloader size")?;

        let kernel_at = aligned(HEADER_SPACE, page);
        let ramdisk_at = aligned(kernel_at + self.kernel.len(), page);
        let second_at = aligned(ramdisk_at + self.ramdisk.len(), page);
        let dtbo_at = aligned(second_at + self.second.len(), page);
        match &mut header.versioned {
            HeaderV0Versioned::V0 => {}
            HeaderV0Versioned::V1 { recovery_dtbo_size, recovery_dtbo_addr } => {
                *recovery_dtbo_size = u32::try_from(self.dtbo.len()).context("recovery dtbo size")?;
                *recovery_dtbo_addr = dtbo_at as u64;
            }
            HeaderV0Versioned::V2 { recovery_dtbo_size, recovery_dtbo_addr, dtb_size, .. } => {
                *recovery_dtbo_size = u32::try_from(self.dtbo.len()).context("recovery dtbo size")?;
                *recovery_dtbo_addr = dtbo_at as u64;
                *dtb_size = u32::try_from(self.dtb.len()).context("dtb size")?;
            }
        }
        // The header's own sum of what follows it, as mkbootimg lays it: each
        // section padded to a page.
        let mut digest = Sha1::new();
        for part in [&self.kernel, &self.ramdisk, &self.second, &self.dtbo, &self.dtb] {
            digest.update(part);
            digest.update(vec![0u8; aligned(part.len(), page) - part.len()]);
        }
        header.hash_digest = [0u8; 32];
        header.hash_digest[..20].copy_from_slice(&digest.finalize());

        let mut out = Vec::new();
        Header::V0(header).write(&mut out).context("write header")?;
        pad(&mut out, page);
        for part in [&self.kernel, &self.ramdisk, &self.second, &self.dtbo, &self.dtb] {
            out.extend_from_slice(part);
            pad(&mut out, page);
        }
        Ok(out)
    }
}

/// `n` rounded up to the next multiple of `page`.
fn aligned(n: usize, page: usize) -> usize {
    n.div_ceil(page) * page
}

fn pad(out: &mut Vec<u8>, page: usize) {
    out.resize(aligned(out.len(), page), 0);
}
