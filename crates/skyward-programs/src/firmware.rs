//! Firmware files read from a recovery or a temporarily mounted partition.

use anyhow::{Context, Result, bail};
use skyward_config::Firmware;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Try the configured locations in order. Only an absent file is skipped;
/// an unreadable file is an error even when the firmware is optional.
pub fn read(root: &Path, file: &Firmware) -> Result<Option<(PathBuf, Vec<u8>)>> {
    let paths = std::iter::once(&file.path).chain(&file.alternatives);
    let mut tried = Vec::new();
    for path in paths {
        let at = root.join(path);
        match std::fs::read(&at) {
            Ok(bytes) => return Ok(Some((at, bytes))),
            Err(error) if error.kind() == ErrorKind::NotFound => tried.push(at),
            Err(error) => return Err(error).with_context(|| format!("read {}", at.display())),
        }
    }
    if file.optional {
        return Ok(None);
    }
    bail!(
        "none of the firmware sources exists: {}",
        tried
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(optional: bool) -> Firmware {
        Firmware {
            name: "wlan/qca_cld/kiwi_v2/wlan_mac.bin".into(),
            path: "wlan/wlan_mac.bin".into(),
            partition: Some("persist".into()),
            alternatives: vec!["kiwi_v2/wlan_mac.bin".into(), "wlan_mac.bin".into()],
            optional,
        }
    }

    #[test]
    fn finds_the_factory_mac_in_the_chip_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("kiwi_v2")).unwrap();
        let at = root.path().join("kiwi_v2/wlan_mac.bin");
        std::fs::write(&at, b"factory address").unwrap();
        std::fs::write(root.path().join("wlan_mac.bin"), b"later fallback").unwrap();
        assert_eq!(
            read(root.path(), &file(false)).unwrap(),
            Some((at, b"factory address".to_vec()))
        );
    }

    #[test]
    fn an_existing_primary_file_takes_precedence() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("wlan")).unwrap();
        let at = root.path().join("wlan/wlan_mac.bin");
        std::fs::write(&at, b"primary").unwrap();
        std::fs::write(root.path().join("wlan_mac.bin"), b"fallback").unwrap();
        assert_eq!(
            read(root.path(), &file(true)).unwrap().unwrap().1,
            b"primary"
        );
    }

    #[test]
    fn only_an_optional_file_may_be_absent() {
        let root = tempfile::tempdir().unwrap();
        assert!(read(root.path(), &file(true)).unwrap().is_none());
        assert!(read(root.path(), &file(false)).is_err());
    }

    #[test]
    fn an_optional_file_does_not_hide_other_read_errors() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("wlan/wlan_mac.bin")).unwrap();
        std::fs::write(root.path().join("wlan_mac.bin"), b"fallback").unwrap();
        assert!(read(root.path(), &file(true)).is_err());
    }
}
