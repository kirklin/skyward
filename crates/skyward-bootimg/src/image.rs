use abootimg_oxide::{binrw::io::Cursor, Header, HeaderV3};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;

const PAGE: usize = 4096;

/// A boot image with header version 3 or 4 (`boot`, `init_boot`, `recovery`)
/// split into its header, kernel and ramdisk.
pub struct BootImage {
    pub header: HeaderV3,
    pub kernel: Vec<u8>,
    pub ramdisk: Vec<u8>,
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
        let header = match Header::parse(&mut Cursor::new(bytes)).context("header")? {
            Header::V3(h) => h,
            Header::V0(h) => bail!("header version {} predates the ramdisk-only layout", h.header_version()),
        };
        let kernel = section(bytes, HeaderV3::kernel_position(), header.kernel_size, "kernel")?;
        let ramdisk = section(bytes, header.ramdisk_position(), header.ramdisk_size, "ramdisk")?;
        Ok(BootImage { header, kernel, ramdisk })
    }

    /// The image with sizes taken from its parts. A version 4 header keeps its
    /// version but carries no boot signature; unlocked bootloaders do not ask
    /// for one.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut header = self.header.clone();
        header.kernel_size = u32::try_from(self.kernel.len()).context("kernel size")?;
        header.ramdisk_size = u32::try_from(self.ramdisk.len()).context("ramdisk size")?;
        header.v4_signature_size = header.v4_signature_size.map(|_| 0);
        let mut out = Vec::new();
        Header::V3(header).write(&mut out).context("write header")?;
        pad(&mut out);
        out.extend_from_slice(&self.kernel);
        pad(&mut out);
        out.extend_from_slice(&self.ramdisk);
        pad(&mut out);
        Ok(out)
    }
}

fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().div_ceil(PAGE) * PAGE, 0);
}
