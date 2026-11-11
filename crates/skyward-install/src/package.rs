//! What arrives over sideload: an A/B update package, a zip with an
//! install script, a partition image, a system to install beside the
//! installed one, or something this recovery cannot install; and whether
//! an update package may go on this phone, checked the way Android's
//! recovery checks it.

use crate::image::{self, ImageKind};
use crate::Refusal;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use zip::{CompressionMethod, ZipArchive};

const PAYLOAD: &str = "payload.bin";
const PROPERTIES: &str = "payload_properties.txt";
const METADATA: &str = "META-INF/com/android/metadata";
/// The install script of a zip that installs itself.
pub const UPDATE_BINARY: &str = "META-INF/com/google/android/update-binary";

/// The most read to tell an image's kind: every header this recovery
/// writes fits in one page.
const HEAD: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Package {
    Update(Update),
    Image(ImageKind),
    /// A zip that installs itself by running the script inside it.
    Script,
    /// A system to install beside the installed one.
    System(skyward_systems::package::Contents),
    Unknown,
}

/// An A/B update package: the payload update_engine applies, where it
/// starts in the zip, and the package's own description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub payload_offset: u64,
    pub payload_size: u64,
    /// payload_properties.txt, handed to update_engine as its headers.
    pub properties: String,
    pub metadata: Metadata,
}

impl Update {
    /// The version the package installs, as its build names it.
    pub fn version(&self) -> Option<&str> {
        self.metadata.get("post-build-incremental")
    }

    /// Whether the system it installs needs all user data erased before it
    /// starts, as the package says in its description or in what it hands
    /// update_engine.
    pub fn wipes(&self) -> bool {
        self.metadata.get("ota-wipe") == Some("yes") || self.properties.lines().any(|l| l.trim() == "POWERWASH=1")
    }
}

/// The package's META-INF/com/android/metadata: one `key=value` a line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata(BTreeMap<String, String>);

impl Metadata {
    pub fn parse(text: &str) -> Metadata {
        Metadata(
            text.lines()
                .filter_map(|line| line.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect(),
        )
    }

    /// A value, unless it is missing or empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }
}

/// Tell what `file` holds from its first bytes and, for a zip, its
/// directory.
pub fn identify<R: Read + Seek>(mut file: R) -> Result<Package> {
    let mut head = Vec::with_capacity(HEAD);
    (&mut file).take(HEAD as u64).read_to_end(&mut head).context("read the package's first bytes")?;
    if !head.starts_with(b"PK\x03\x04") {
        return Ok(image::detect(&mut file, &head)?.map_or(Package::Unknown, Package::Image));
    }
    file.seek(SeekFrom::Start(0)).context("rewind the package")?;
    let mut zip = ZipArchive::new(file).context("read the zip directory")?;
    if zip.index_for_name(PAYLOAD).is_some() {
        return update(&mut zip).map(Package::Update);
    }
    if skyward_systems::package::is_system(&zip) {
        return skyward_systems::package::read(&mut zip).map(Package::System);
    }
    if zip.index_for_name(UPDATE_BINARY).is_some() {
        return Ok(Package::Script);
    }
    Ok(Package::Unknown)
}

fn update<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<Update> {
    let properties = text(zip, PROPERTIES)?;
    let metadata = Metadata::parse(&text(zip, METADATA)?);
    let payload = zip.by_name(PAYLOAD).context("open payload.bin")?;
    // update_engine reads the payload straight out of the zip, so it has
    // to be stored as is.
    if payload.compression() != CompressionMethod::Stored {
        bail!("payload.bin is compressed ({:?})", payload.compression());
    }
    let payload_offset = payload.data_start().context("payload.bin has no data offset")?;
    Ok(Update { payload_offset, payload_size: payload.size(), properties, metadata })
}

fn text<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<String> {
    let mut entry = zip.by_name(name).with_context(|| format!("open {name}"))?;
    let mut s = String::new();
    entry.read_to_string(&mut s).with_context(|| format!("read {name}"))?;
    Ok(s)
}

/// What the phone is, and the system installed on it now when its
/// partitions could be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Phone {
    /// ro.product.device
    pub device: String,
    /// ro.serialno
    pub serial: String,
    pub installed: Option<Build>,
}

/// The installed system, from its build.prop files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// ro.build.date.utc
    pub timestamp: i64,
    /// ro.build.version.security_patch, as yyyy-mm-dd
    pub security_patch: String,
}

/// Whether `update` may go on `phone`.
pub fn check(update: &Update, phone: &Phone) -> Result<(), Refusal> {
    let m = &update.metadata;
    let kind = m.get("ota-type").unwrap_or_default();
    if kind != "AB" {
        return Err(Refusal::NotAb(kind.to_string()));
    }
    let devices = m.get("pre-device").unwrap_or_default();
    if !listed(&phone.device, devices) {
        return Err(Refusal::OtherDevice(devices.to_string()));
    }
    if let Some(serials) = m.get("serialno") {
        if !listed(&phone.serial, serials) {
            return Err(Refusal::OtherSerial);
        }
    }
    if m.get("pre-build").is_some() || m.get("pre-build-incremental").is_some() {
        return Err(Refusal::Incremental);
    }
    let Some(build) = &phone.installed else { return Ok(()) };
    // A full package may install the version already there again; going
    // back needs a package made for the build it leaves, an incremental
    // one.
    let timestamp = m.get("post-timestamp").and_then(|t| t.parse().ok()).unwrap_or(0);
    if timestamp < build.timestamp {
        return Err(Refusal::Older { package: timestamp, installed: build.timestamp });
    }
    // Patch levels are yyyy-mm-dd, so they order as text. Data that is
    // erased before the older system starts is never read under it.
    if let Some(patch) = m.get("post-security-patch-level") {
        let allowed = m.get("spl-downgrade") == Some("yes") || m.get("ota-downgrade") == Some("yes") || update.wipes();
        if patch < build.security_patch.as_str() && !allowed {
            return Err(Refusal::OlderPatch { package: patch.to_string(), installed: build.security_patch.clone() });
        }
    }
    Ok(())
}

/// Whether `name` is one of the `|`-separated names in `list`.
fn listed(name: &str, list: &str) -> bool {
    list.split('|').any(|n| n.trim() == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    const METADATA_TEXT: &str = "\
ota-property-files=payload_metadata.bin:2365:94208,payload.bin:2365:7103041280,payload_properties.txt:7103043714:154,metadata:69:637,metadata.pb:773:1529
ota-required-cache=0
ota-type=AB
post-build=Xiaomi/ishtar/ishtar:16/BP2A.250605.031.A3/OS3.0.306.0.WMACNXM:user/release-keys
post-build-incremental=OS3.0.306.0.WMACNXM
post-sdk-level=36
post-security-patch-level=2026-06-01
post-timestamp=1781000000
pre-device=ishtar
";

    const PROPERTIES_TEXT: &str = "\
FILE_HASH=7yP3vJ8M1qfAeMIuSv8Bq6P8xHkZ8eCwI0G2vC9q8Y4=
FILE_SIZE=4096
METADATA_HASH=sV9V3cOa1f8T5Nn+Yt0e8b1wqM3k9yvX2hR6J4iQ7nI=
METADATA_SIZE=1024
";

    fn phone() -> Phone {
        Phone {
            device: "ishtar".into(),
            serial: "2674dc3e".into(),
            installed: Some(Build { timestamp: 1_778_243_655, security_patch: "2026-04-01".into() }),
        }
    }

    fn update_with(metadata: &str, properties: &str) -> Update {
        Update { payload_offset: 0, payload_size: 0, properties: properties.into(), metadata: Metadata::parse(metadata) }
    }

    fn zip_with(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data, method) in entries {
            w.start_file(*name, SimpleFileOptions::default().compression_method(*method)).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn a_full_package_for_this_phone_is_accepted() {
        assert_eq!(check(&update_with(METADATA_TEXT, PROPERTIES_TEXT), &phone()), Ok(()));
    }

    #[test]
    fn the_same_build_may_be_installed_again() {
        let same = METADATA_TEXT.replace("1781000000", "1778243655");
        assert_eq!(check(&update_with(&same, PROPERTIES_TEXT), &phone()), Ok(()));
    }

    #[test]
    fn packages_for_other_phones_are_refused() {
        let other = METADATA_TEXT.replace("pre-device=ishtar", "pre-device=nezha");
        assert_eq!(check(&update_with(&other, PROPERTIES_TEXT), &phone()), Err(Refusal::OtherDevice("nezha".into())));
        let listed = METADATA_TEXT.replace("pre-device=ishtar", "pre-device=nezha|ishtar");
        assert_eq!(check(&update_with(&listed, PROPERTIES_TEXT), &phone()), Ok(()));
        let serial = format!("{METADATA_TEXT}serialno=abc|def\n");
        assert_eq!(check(&update_with(&serial, PROPERTIES_TEXT), &phone()), Err(Refusal::OtherSerial));
        let ours = format!("{METADATA_TEXT}serialno=abc|2674dc3e\n");
        assert_eq!(check(&update_with(&ours, PROPERTIES_TEXT), &phone()), Ok(()));
        let block = METADATA_TEXT.replace("ota-type=AB", "ota-type=BLOCK");
        assert_eq!(check(&update_with(&block, PROPERTIES_TEXT), &phone()), Err(Refusal::NotAb("BLOCK".into())));
    }

    #[test]
    fn incremental_and_older_packages_are_refused() {
        let incremental = format!(
            "{METADATA_TEXT}pre-build=Xiaomi/ishtar/ishtar:16/X/OS3.0.305.0.WMACNXM:user/release-keys\npre-build-incremental=OS3.0.305.0.WMACNXM\n"
        );
        assert_eq!(check(&update_with(&incremental, PROPERTIES_TEXT), &phone()), Err(Refusal::Incremental));
        let older = METADATA_TEXT.replace("1781000000", "1770000000");
        assert_eq!(
            check(&update_with(&older, PROPERTIES_TEXT), &phone()),
            Err(Refusal::Older { package: 1_770_000_000, installed: 1_778_243_655 })
        );
        let patch = METADATA_TEXT.replace("2026-06-01", "2026-03-01");
        assert_eq!(
            check(&update_with(&patch, PROPERTIES_TEXT), &phone()),
            Err(Refusal::OlderPatch { package: "2026-03-01".into(), installed: "2026-04-01".into() })
        );
        let allowed = format!("{}spl-downgrade=yes\n", METADATA_TEXT.replace("2026-06-01", "2026-03-01"));
        assert_eq!(check(&update_with(&allowed, PROPERTIES_TEXT), &phone()), Ok(()));
    }

    #[test]
    fn a_package_may_ask_for_all_user_data_to_be_erased_either_way() {
        let plain = update_with(METADATA_TEXT, PROPERTIES_TEXT);
        assert!(!plain.wipes());
        let powerwash = update_with(METADATA_TEXT, &format!("{PROPERTIES_TEXT}POWERWASH=1\n"));
        assert!(powerwash.wipes());
        assert_eq!(check(&powerwash, &phone()), Ok(()), "erasing is asked of the user, not refused");
        let wipe = update_with(&format!("{METADATA_TEXT}ota-wipe=yes\n"), PROPERTIES_TEXT);
        assert!(wipe.wipes());
        assert!(!update_with(METADATA_TEXT, &format!("{PROPERTIES_TEXT}POWERWASH=0\n")).wipes());
        let older_patch = format!("{}ota-wipe=yes\n", METADATA_TEXT.replace("2026-06-01", "2026-03-01"));
        assert_eq!(check(&update_with(&older_patch, PROPERTIES_TEXT), &phone()), Ok(()), "the data it could not read is erased");
    }

    #[test]
    fn without_an_installed_system_only_the_package_itself_is_checked() {
        let older = METADATA_TEXT.replace("1781000000", "1770000000");
        let bare = Phone { installed: None, ..phone() };
        assert_eq!(check(&update_with(&older, PROPERTIES_TEXT), &bare), Ok(()));
    }

    #[test]
    fn an_update_zip_gives_where_its_stored_payload_starts() {
        let payload = b"CrAU\0\0\0\0\0\0\0\x02rest of the payload";
        let bytes = zip_with(&[
            ("payload.bin", payload, CompressionMethod::Stored),
            ("payload_properties.txt", PROPERTIES_TEXT.as_bytes(), CompressionMethod::Stored),
            ("META-INF/com/android/metadata", METADATA_TEXT.as_bytes(), CompressionMethod::Deflated),
        ]);
        let Package::Update(u) = identify(Cursor::new(&bytes)).unwrap() else { panic!("an update package") };
        let at = u.payload_offset as usize;
        assert_eq!(&bytes[at..at + payload.len()], payload);
        assert_eq!(u.payload_size, payload.len() as u64);
        assert_eq!(u.properties, PROPERTIES_TEXT);
        assert_eq!(u.version(), Some("OS3.0.306.0.WMACNXM"));
    }

    #[test]
    fn script_zips_compressed_payloads_and_other_files_are_told_apart() {
        let script = zip_with(&[("META-INF/com/google/android/update-binary", b"#!/sbin/sh\n", CompressionMethod::Deflated)]);
        assert_eq!(identify(Cursor::new(script)).unwrap(), Package::Script);
        let squeezed = zip_with(&[
            ("payload.bin", &[7u8; 4096], CompressionMethod::Deflated),
            ("payload_properties.txt", PROPERTIES_TEXT.as_bytes(), CompressionMethod::Stored),
            ("META-INF/com/android/metadata", METADATA_TEXT.as_bytes(), CompressionMethod::Stored),
        ]);
        assert!(identify(Cursor::new(squeezed)).is_err());
        let other = zip_with(&[("readme.txt", b"hello", CompressionMethod::Stored)]);
        assert_eq!(identify(Cursor::new(other)).unwrap(), Package::Unknown);
        assert_eq!(identify(Cursor::new(b"just some text".to_vec())).unwrap(), Package::Unknown);
    }
}
