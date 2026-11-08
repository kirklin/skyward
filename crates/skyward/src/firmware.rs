//! Start processors before init starts the health and recovery services.

use anyhow::{ensure, Context, Result};
use skyward_config::FirmwarePartition;
use std::fs;
use std::path::Path;

pub fn start(config: &FirmwarePartition) -> Result<()> {
    let shown = Path::new("/firmware");
    if !skyward_block::mount::is_mount_point(shown)? {
        // The installed vendor is not mounted yet. Use the same table the
        // stock recovery uses to mount firmware during its fs action.
        let table = fs::read_to_string("/vendor/etc/charger_fstab.qti")?;
        let entry = skyward_block::fstab::entries(&table).into_iter()
            .find(|entry| Path::new(&entry.device).file_name().is_some_and(|n| n == config.partition.as_str()))
            .context("the recovery's charger table does not mount the firmware partition")?;
        let at = Path::new(&entry.mount_point);
        fs::create_dir_all(at)?;
        if !skyward_block::mount::is_mount_point(at)? {
            ensure!(entry.options.iter().any(|o| o == "ro"), "firmware must be mounted read-only");
            skyward_block::mount::mount(Path::new(&skyward_block::fstab::device(&entry)?), at, &entry.filesystem, &entry.options)?;
        }
        fs::create_dir_all(shown)?;
        skyward_block::mount::bind(at, shown)?;
    }
    // A later vendor mount must not hide firmware while remoteproc is
    // requesting another segment of its image.
    fs::write("/sys/module/firmware_class/parameters/path", "/firmware/image")?;
    if let Some(name) = &config.remoteproc {
        let mut found = false;
        for entry in fs::read_dir("/sys/class/remoteproc")? {
            let at = entry?.path();
            let kernel_name = fs::read_to_string(at.join("name"))?;
            if kernel_name.trim() != name && !kernel_name.trim().ends_with(&format!(".remoteproc-{name}")) {
                continue;
            }
            found = true;
            let state = fs::read_to_string(at.join("state"))?;
            match state.trim() {
                "offline" => fs::write(at.join("state"), "start")?,
                "running" => {},
                state => anyhow::bail!("{name} is {state}; refusing to repeat a partial processor boot"),
            }
            skyward_sys::kmsg(&format!("{name}: {}", fs::read_to_string(at.join("state"))?.trim()))?;
            break;
        }
        ensure!(found, "no remote processor named {name}");
    }
    Ok(())
}
