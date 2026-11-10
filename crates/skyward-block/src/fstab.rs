//! The table a phone keeps of its own filesystems: where each one is, what
//! it holds, and how it is encrypted.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Where the phone's own partitions keep the table. Android 10 and later
/// keep it in the vendor or odm partition; Android 9 and earlier keep it in
/// the recovery ramdisk as /etc/recovery.fstab, whose fifth column holds
/// the same flags.
pub const FILES: [&str; 3] = ["/vendor/etc/fstab.qcom", "/odm/etc/fstab.qcom", "/etc/recovery.fstab"];

/// One filesystem the phone knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub device: String,
    pub mount_point: String,
    pub filesystem: String,
    /// How the phone mounts it: the kernel's flags and the filesystem's
    /// own options, as one list.
    pub options: Vec<String>,
    /// What the phone says about the filesystem beyond how to mount it,
    /// such as how it is encrypted.
    pub extras: Vec<String>,
}

impl Entry {
    /// The value of an extra written as `name=value`.
    pub fn extra(&self, name: &str) -> Option<&str> {
        let prefix = format!("{name}=");
        self.extras.iter().find_map(|e| e.strip_prefix(&prefix))
    }
}

/// Every filesystem a table names. A phone may name the same mount point
/// more than once, once per filesystem it might hold.
pub fn entries(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [device, mount_point, filesystem, options, extras] = fields[..] else { continue };
        out.push(Entry {
            device: device.to_string(),
            mount_point: mount_point.to_string(),
            filesystem: filesystem.to_string(),
            options: options.split(',').map(str::to_string).collect(),
            extras: extras.split(',').map(str::to_string).collect(),
        });
    }
    out
}

/// Every entry of the phone's own table, read from wherever the table is.
fn table() -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for file in FILES {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {file}")),
        };
        out.extend(entries(&text));
    }
    Ok(out)
}

/// What the phone's own table says about `mount_point`.
pub fn read(mount_point: &str) -> Result<Vec<Entry>> {
    Ok(table()?.into_iter().filter(|e| e.mount_point == mount_point).collect())
}

/// What the phone's own table says about the partition `name`: the first
/// entry naming it, by whichever path the table reaches it.
pub fn partition(name: &str) -> Result<Option<Entry>> {
    Ok(table()?.into_iter().find(|e| names(e, name)))
}

/// Whether an entry is of the partition `name`.
fn names(entry: &Entry, name: &str) -> bool {
    Path::new(&entry.device).file_name().is_some_and(|n| n == name)
}

/// The partition an entry names, as it is reachable now: of the booted
/// slot, where the phone keeps one of it per slot. A phone's table points
/// at its storage controller by a path a recovery need not have.
pub fn device(entry: &Entry) -> Result<String> {
    let path = match entry.extras.iter().any(|e| e == "slotselect") {
        true => format!("{}{}", entry.device, crate::slot_suffix()?.with_context(|| format!("{} is of a slot, and the phone has none", entry.device))?),
        false => entry.device.clone(),
    };
    if Path::new(&path).exists() {
        return Ok(path);
    }
    let name = Path::new(&path).file_name().with_context(|| format!("{path} names no partition"))?;
    let by_name = Path::new(crate::BY_NAME).join(name);
    if by_name.exists() {
        return Ok(by_name.to_string_lossy().into_owned());
    }
    anyhow::bail!("neither {path} nor {} is here", by_name.display())
}

/// Mount the partition `name` where and as the phone's own table mounts
/// it, and say where. Whatever shows there already, as what an earlier run
/// of the recovery service mounted does, is taken as it is.
pub fn mount(name: &str) -> Result<PathBuf> {
    let entry = partition(name)?.with_context(|| format!("the phone's own table does not mount {name}"))?;
    let at = PathBuf::from(&entry.mount_point);
    if crate::mount::is_mount_point(&at)? {
        return Ok(at);
    }
    std::fs::create_dir_all(&at).with_context(|| format!("make {}", at.display()))?;
    crate::mount::mount(Path::new(&device(&entry)?), &at, &entry.filesystem, &entry.options)?;
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
# a comment
/dev/block/bootdevice/by-name/metadata /metadata f2fs noatime,nosuid wait,check,formattable
/dev/block/bootdevice/by-name/userdata /data f2fs noatime,discard latemount,wait,fileencryption=aes-256-xts:aes-256-cts:v2,keydirectory=/metadata/vold/metadata_encryption,metadata_encryption=aes-256-xts:wrappedkey_v0,quota
/dev/block/bootdevice/by-name/userdata /data mifs noatime,discard latemount,wait,keydirectory=/metadata/vold/metadata_encryption
";

    #[test]
    fn a_mount_point_can_be_named_once_per_filesystem_it_might_hold() {
        let data: Vec<Entry> = entries(TABLE).into_iter().filter(|e| e.mount_point == "/data").collect();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].filesystem, "f2fs");
        assert_eq!(data[1].filesystem, "mifs");
        assert_eq!(data[0].device, "/dev/block/bootdevice/by-name/userdata");
    }

    #[test]
    fn how_the_phone_mounts_a_filesystem_is_kept_apart_from_the_rest() {
        let data = &entries(TABLE)[1];
        assert_eq!(data.options, ["noatime", "discard"]);
        assert!(data.extras.iter().any(|e| e == "latemount") && !data.extras.iter().any(|e| e == "noatime"));
    }

    #[test]
    fn what_the_phone_says_about_encryption_is_read_off_the_entry() {
        let data = &entries(TABLE)[1];
        assert_eq!(data.extra("metadata_encryption"), Some("aes-256-xts:wrappedkey_v0"));
        assert_eq!(data.extra("keydirectory"), Some("/metadata/vold/metadata_encryption"));
        assert_eq!(data.extra("quota"), None, "a word on its own has no value");
        assert_eq!(data.extra("keydir"), None, "a shorter name is another extra");
    }

    #[test]
    fn an_entry_is_of_the_partition_its_path_ends_in() {
        let modem = &entries(
            "/dev/block/bootdevice/by-name/modem /vendor/firmware_mnt vfat ro,uid=1000,context=u:object_r:firmware_file:s0 wait,slotselect\n",
        )[0];
        assert!(names(modem, "modem"));
        assert!(!names(modem, "modemfirmware") && !names(modem, "mode"), "a name is matched whole");
        assert_eq!(modem.options[0], "ro");
        assert!(modem.extras.iter().any(|e| e == "slotselect"), "one of each slot");
    }

    #[test]
    fn comments_and_short_lines_are_passed_over() {
        assert!(entries("# nothing\n\n/dev/x /y f2fs rw\n").is_empty());
        assert_eq!(entries(TABLE).len(), 3);
    }
}
