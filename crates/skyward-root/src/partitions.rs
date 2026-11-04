//! The partitions a phone has, as the links under its block devices name
//! them: a page that keeps partitions lists these rather than a list of names
//! remembered from one phone.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Where a phone keeps its partitions by name.
pub const BY_NAME: &str = "/dev/block/by-name";

/// The partitions `dir` names that a page keeps, without the slot a phone's
/// two copies carry: `boot_a` and `boot_b` are both `boot`. The ones a root
/// manager patches, and a system update writes over, come first.
pub fn names(dir: &Path) -> Result<Vec<String>> {
    /// The partitions a page keeps.
    const KEEP: [&str; 54] = [
        "abl", "aop", "aop_config", "bluetooth", "boot", "countrycode",
        "cpucp", "cpucp_dtb", "dcp", "devcfg", "dsp", "dtbo",
        "featenabler", "fsg", "hyp", "hyp_ac_config", "idmanager", "imagefv",
        "init_boot", "keymaster", "modem", "modemfirmware", "modemst1", "modemst2",
        "multiimgoem", "multiimgqti", "pdp", "pdp_cdb", "persist", "pvmfw",
        "pvmfw_signed", "qtvm_dtbo", "qupfw", "qweslicstore", "recovery", "secretkeeper",
        "soccp", "soccp_dcd", "soccp_debug", "spuservice", "tme_fw", "tme_seq_patch",
        "tz", "tz_ac_config", "uefi", "uefisecapp", "vbmeta", "vbmeta_system",
        "vendor_boot", "vm-bootsys", "xbl", "xbl_ac_config", "xbl_config", "xbl_ramdump",
    ];

    // The ones the page lists first: a root manager patches these, and a
    // system update writes them over.
    const ORDER: [&str; 14] = ["boot", "init_boot", "vendor_boot", "dtbo", "vbmeta", "vbmeta_system", "vbmeta_vendor", "recovery", "persist", "modem", "modemst1", "modemst2", "fsg", "bluetooth"];
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let base = name.strip_suffix("_a").or_else(|| name.strip_suffix("_b")).unwrap_or(&name);
        // Only the ones a page keeps: a phone lists many partitions that no
        // update writes and no page should offer.
        if !KEEP.contains(&base) || found.iter().any(|seen| seen == base) {
            continue;
        }
        found.push(base.to_string());
    }
    found.sort_by_key(|name| {
        let first = ORDER.iter().position(|known| known == name).unwrap_or(ORDER.len());
        (first, name.to_lowercase())
    });
    Ok(found)
}

/// The partitions the phone under this recovery has.
pub fn here() -> Vec<String> {
    names(Path::new(BY_NAME)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn link(dir: &Path, name: &str) {
        File::create(dir.join(name)).unwrap();
    }

    #[test]
    fn the_two_copies_of_a_partition_are_one_name() {
        let dir = tempfile::tempdir().unwrap();
        link(dir.path(), "boot_a");
        link(dir.path(), "boot_b");
        link(dir.path(), "init_boot_a");
        link(dir.path(), "init_boot_b");
        let found = names(dir.path()).unwrap();
        assert_eq!(found, ["boot", "init_boot"]);
    }

    #[test]
    fn every_partition_shows_with_the_ones_a_page_keeps_first() {
        let dir = tempfile::tempdir().unwrap();
        link(dir.path(), "xbl_a");
        link(dir.path(), "persist_a");
        link(dir.path(), "boot_a");
        assert_eq!(names(dir.path()).unwrap(), ["boot", "persist", "xbl"]);
    }

    #[test]
    fn the_data_partition_is_not_one_a_page_keeps() {
        let dir = tempfile::tempdir().unwrap();
        link(dir.path(), "userdata");
        link(dir.path(), "super");
        link(dir.path(), "persist_a");
        assert_eq!(names(dir.path()).unwrap(), ["persist"]);
    }
}
