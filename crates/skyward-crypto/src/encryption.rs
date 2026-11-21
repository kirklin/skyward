//! What a phone's own table says of how its data partition is encrypted,
//! whole and file by file, and the device-mapper target that reads a
//! partition encrypted whole through the storage hardware's decryption. A
//! table that says neither keeps the partition as the plain filesystem it
//! is, which opens with no key at all.

use anyhow::{Context, Result};
use skyward_block::dm::Target;
use skyward_block::fstab::Entry;
use std::path::{Path, PathBuf};

/// The key is in a directory of this name under the one the phone names.
const KEY_DIR: &str = "key";

/// How the kernel knows the cipher a phone names.
const CIPHERS: [(&str, &str); 2] = [("aes-256-xts", "aes-xts-plain64"), ("adiantum", "xchacha12,aes-adiantum-plain64")];

/// The hardware reads and writes in 4096-byte units, so the partition is
/// mapped in whole multiples of eight 512-byte sectors.
const SECTORS_PER_UNIT: u64 = 8;

/// How a phone says its data partition is encrypted whole: the cipher, and
/// whether the key is one only the storage hardware can unwrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encryption {
    pub cipher: String,
    pub wrapped: bool,
}

impl Encryption {
    /// Read what an entry of the phone's own table says, which is nothing
    /// where the entry does not have the partition encrypted whole.
    pub fn of(entry: &Entry) -> Result<Option<Encryption>> {
        let Some(said) = entry.extra("metadata_encryption") else { return Ok(None) };
        let (name, rest) = said.split_once(':').unwrap_or((said, ""));
        let cipher = CIPHERS
            .iter()
            .find_map(|(theirs, kernels)| (*theirs == name).then_some(*kernels))
            .with_context(|| format!("no cipher here is the {name} the phone names"))?;
        Ok(Some(Encryption { cipher: cipher.to_string(), wrapped: rest == "wrappedkey_v0" }))
    }
}

/// The entry that has the partition encrypted whole, and how, among those
/// the phone's table has for it: a phone may name the partition once per
/// filesystem it might hold, and say how it is encrypted in one of them.
/// Nothing where none says so, and the filesystem is then read and written
/// on the partition itself.
pub fn whole(entries: &[Entry]) -> Result<Option<(&Entry, Encryption)>> {
    for entry in entries {
        if let Some(encryption) = Encryption::of(entry)? {
            return Ok(Some((entry, encryption)));
        }
    }
    Ok(None)
}

/// Whether the phone's table has the partition's files encrypted one by
/// one, each user's under keys of their own. Where it has not, every name
/// and every file reads as it is once the filesystem is mounted.
pub fn files(entries: &[Entry]) -> bool {
    entries.iter().any(|e| e.extra("fileencryption").is_some())
}

/// Where the phone keeps the key for the filesystem an entry describes.
pub fn key_dir(entry: &Entry) -> Result<PathBuf> {
    let dir = entry.extra("keydirectory").context("the phone says where the key for its data partition is not")?;
    Ok(Path::new(dir).join(KEY_DIR))
}

/// Build the device-mapper target that reads the partition through the
/// storage hardware's decryption.
pub fn target(device: &str, sectors: u64, key: &[u8], encryption: &Encryption) -> Target {
    Target::DefaultKey {
        sectors: sectors - sectors % SECTORS_PER_UNIT,
        device: device.to_string(),
        cipher: encryption.cipher.clone(),
        key: hex::encode(key),
        wrapped: encryption.wrapped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(extras: &str) -> Entry {
        Entry {
            device: "/dev/block/bootdevice/by-name/userdata".into(),
            mount_point: "/data".into(),
            filesystem: "f2fs".into(),
            options: vec!["noatime".into(), "inlinecrypt".into()],
            extras: extras.split(',').map(str::to_string).collect(),
        }
    }

    #[test]
    fn the_phones_own_words_for_the_cipher_become_the_kernels() {
        let e = entry("metadata_encryption=aes-256-xts:wrappedkey_v0,keydirectory=/metadata/vold/metadata_encryption");
        assert_eq!(Encryption::of(&e).unwrap(), Some(Encryption { cipher: "aes-xts-plain64".into(), wrapped: true }));
        let plain = entry("metadata_encryption=adiantum");
        assert_eq!(Encryption::of(&plain).unwrap(), Some(Encryption { cipher: "xchacha12,aes-adiantum-plain64".into(), wrapped: false }));
    }

    #[test]
    fn an_entry_that_names_no_whole_encryption_says_nothing_of_it() {
        assert_eq!(Encryption::of(&entry("quota")).unwrap(), None);
        assert_eq!(Encryption::of(&entry("fileencryption=aes-256-xts:aes-256-cts:v2")).unwrap(), None);
    }

    #[test]
    fn a_cipher_or_a_key_the_phone_does_not_name_is_an_error() {
        assert!(Encryption::of(&entry("metadata_encryption=rot13")).is_err());
        assert!(whole(&[entry("quota"), entry("metadata_encryption=rot13")]).is_err());
        assert!(key_dir(&entry("quota")).is_err());
    }

    /// What a phone's table says of its data partition, as the phone's
    /// maker wrote it: encrypted whole and file by file, and named a
    /// second time for another filesystem with neither said again.
    const STOCK: [&str; 2] = [
        "latemount,wait,check,formattable,fileencryption=aes-256-xts:aes-256-cts:v2,keydirectory=/metadata/vold/metadata_encryption,metadata_encryption=aes-256-xts:wrappedkey_v0,quota",
        "latemount,wait,keydirectory=/metadata/vold/metadata_encryption",
    ];
    /// The same with what encrypts the partition whole taken out, and with
    /// all that encrypts it taken out.
    const FILES_ONLY: &str = "latemount,wait,check,formattable,fileencryption=aes-256-xts:aes-256-cts:v2,quota";
    const PLAIN: &str = "latemount,wait,check,formattable,quota";

    #[test]
    fn a_table_is_read_for_both_ways_a_partition_is_encrypted() {
        let stock = STOCK.map(entry);
        let (said_in, encryption) = whole(&stock).unwrap().expect("encrypted whole");
        assert_eq!(said_in, &stock[0]);
        assert_eq!(encryption, Encryption { cipher: "aes-xts-plain64".into(), wrapped: true });
        assert!(files(&stock));

        let turned = [entry(STOCK[1]), entry(STOCK[0])];
        assert_eq!(whole(&turned).unwrap().expect("encrypted whole").0, &turned[1], "whichever entry says so");
        assert!(files(&turned));

        let files_only = [entry(FILES_ONLY)];
        assert_eq!(whole(&files_only).unwrap(), None);
        assert!(files(&files_only), "its files still open only under their keys");

        let plain = [entry(PLAIN), entry("latemount,wait")];
        assert_eq!(whole(&plain).unwrap(), None);
        assert!(!files(&plain));
    }

    #[test]
    fn the_key_is_in_a_directory_under_the_one_the_phone_names() {
        let e = entry("keydirectory=/metadata/vold/metadata_encryption");
        assert_eq!(key_dir(&e).unwrap(), Path::new("/metadata/vold/metadata_encryption/key"));
    }

    #[test]
    fn the_partition_is_mapped_in_whole_units_of_the_hardwares_reads() {
        let e = Encryption { cipher: "aes-xts-plain64".into(), wrapped: true };
        let t = target("/dev/block/by-name/userdata", 969_825_751, &[0xAB, 0x01], &e);
        let Target::DefaultKey { sectors, key, cipher, device, wrapped } = t else { panic!("a decrypting target") };
        assert_eq!(sectors, 969_825_744, "a whole number of 4096-byte units");
        assert_eq!(key, "ab01");
        assert_eq!(cipher, "aes-xts-plain64");
        assert_eq!(device, "/dev/block/by-name/userdata");
        assert!(wrapped);
    }
}
