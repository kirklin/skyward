//! Filesystem mounts retained by services across recovery detach.

use anyhow::{Context, Result};
use procfs::process::MountInfos;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

/// The devices a running service keeps mounted in a mount namespace of
/// its own, from its mount table: none while it shares the recovery's
/// namespace, whose mounts the recovery takes away with its own. A
/// namespace or table that cannot be read, or a line of it that cannot
/// be, is an error, not a guess.
pub(crate) fn kept_mounted(namespace: io::Result<PathBuf>, ours: &Path, table: impl FnOnce() -> Result<MountInfos>) -> Result<BTreeSet<u64>> {
    let theirs = namespace.context("read the service's mount namespace")?;
    if theirs.as_path() == ours {
        return Ok(BTreeSet::new());
    }
    let mut devices = BTreeSet::new();
    for mount in table().context("read the service's mount table")? {
        let (major, minor) = mount.majmin.split_once(':').context("invalid service mount device number")?;
        let major = major.parse::<u32>().context("invalid service mount major number")?;
        let minor = minor.parse::<u32>().context("invalid service mount minor number")?;
        devices.insert(libc::makedev(major, minor));
    }
    Ok(devices)
}

/// Which of the mounted partitions, by device, still have a mount
/// somewhere: their mount points, each once, in the order mounted.
pub(crate) fn retained_mounts(devices: &BTreeSet<u64>, mounted: &[(u64, PathBuf)]) -> Vec<PathBuf> {
    let mut busy = Vec::new();
    for (device, point) in mounted {
        if devices.contains(device) && !busy.contains(point) {
            busy.push(point.clone());
        }
    }
    busy
}

#[cfg(test)]
mod tests {
    use super::*;
    use procfs::FromBufRead;

    /// mi_product as mounted: its device and where the recovery has it.
    fn mi_product() -> (u64, PathBuf) {
        (libc::makedev(259, 5), PathBuf::from("/mi_product"))
    }

    /// One line of a service's mount table, mi_product's device mounted
    /// at `point`.
    fn mountinfo(point: &str) -> MountInfos {
        MountInfos::from_buf_read(io::Cursor::new(format!("963 40 259:5 / {point} ro,relatime - erofs /dev/block/dm-9 ro\n"))).unwrap()
    }

    #[test]
    fn a_mount_kept_in_a_service_s_own_namespace_retains_its_partition() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026532622]")), Path::new("mnt:[4026531840]"), || Ok(mountinfo("/mi_product"))).unwrap();
        assert_eq!(retained_mounts(&devices, &[mi_product()]), [PathBuf::from("/mi_product")], "no file of it is mapped or open anywhere");
    }

    #[test]
    fn a_service_sharing_the_recovery_s_namespace_keeps_no_mounts() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026531840]")), Path::new("mnt:[4026531840]"), || anyhow::bail!("a shared namespace must not read a private mount table")).unwrap();
        assert!(devices.is_empty(), "what it has mounted the recovery takes away with its own");
    }

    #[test]
    fn a_device_no_partition_has_keeps_none_of_them() {
        let devices = BTreeSet::from([libc::makedev(259, 6)]);
        assert!(retained_mounts(&devices, &[mi_product()]).is_empty(), "matched by device, not by path");
    }

    #[test]
    fn a_bind_at_another_path_keeps_the_device() {
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[4026532622]")), Path::new("mnt:[4026531840]"), || Ok(mountinfo("/vendor/firmware_mnt/bind"))).unwrap();
        assert_eq!(retained_mounts(&devices, &[mi_product()]), [PathBuf::from("/mi_product")], "the bind shows nowhere the recovery looks");
    }

    #[test]
    fn a_partition_kept_twice_is_told_once() {
        let mut table = mountinfo("/mi_product");
        table.0.extend(mountinfo("/private/bind"));
        let devices = kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Ok(table)).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(retained_mounts(&devices, &[mi_product(), mi_product()]), [PathBuf::from("/mi_product")]);
    }

    #[test]
    fn a_namespace_or_table_that_cannot_be_read_is_an_error() {
        assert!(kept_mounted(Err(io::Error::from(io::ErrorKind::PermissionDenied)), Path::new("mnt:[1]"), || Ok(mountinfo("/mi_product"))).is_err());
        assert!(kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Err(io::Error::from(io::ErrorKind::PermissionDenied).into())).is_err());
        assert!(MountInfos::from_buf_read(io::Cursor::new("nonsense\n")).is_err());
    }

    #[test]
    fn invalid_service_mount_device_number_is_refused() {
        let mut table = mountinfo("/mi_product");
        table.0[0].majmin = "invalid".into();
        assert!(kept_mounted(Ok(PathBuf::from("mnt:[2]")), Path::new("mnt:[1]"), || Ok(table)).is_err());
    }
}
