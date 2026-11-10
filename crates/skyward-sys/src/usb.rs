//! The functions the USB port offers, which init sets up as
//! `sys.usb.config` asks: adb while the recovery waits, with files over MTP
//! beside it while file transfer is on.

use crate::{prop, set_prop, wait_prop};
use anyhow::{ensure, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static MTP: AtomicBool = AtomicBool::new(false);

/// What the port offers while nothing else has it.
pub fn resting() -> &'static str {
    if MTP.load(Ordering::Relaxed) {
        "mtp,adb"
    } else {
        "adb"
    }
}

/// Whether MTP goes beside adb while the port rests: from when its function
/// has its descriptors until it goes.
pub fn set_mtp(on: bool) {
    MTP.store(on, Ordering::Relaxed);
}

/// Ask init for `config` and wait until it is in place. Going to "none"
/// also waits for adbd to be gone, so its function is free.
pub fn set(config: &str, timeout: Duration) -> Result<()> {
    // A port left with this set binds nothing at all, whatever asks for it:
    // adb alone does not come back and the computer finds nothing.
    if prop("sys.usb.config_disable").as_deref() == Some("1") {
        set_prop("sys.usb.config_disable", "0")?;
    }
    set_prop("sys.usb.config", config)?;
    if wait_prop("sys.usb.state", config, timeout).is_err() {
        // Init binds the functions once the programs behind them are
        // ready, which can take longer than the wait allows; asking again
        // costs nothing and wakes the rules that had given up.
        set_prop("sys.usb.config", config)?;
        wait_prop("sys.usb.state", config, timeout)?;
    };
    if config == "none" {
        let start = Instant::now();
        while !matches!(prop("init.svc.adbd").as_deref(), None | Some("stopped")) {
            ensure!(start.elapsed() < timeout, "adbd still runs {} ms after the port went to none", timeout.as_millis());
            thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

/// Hand the port to `config` through "none", which stops the programs
/// serving it and takes every function off; init then sets the port up
/// once the programs behind the new configuration are ready.
pub fn switch(config: &str, timeout: Duration) -> Result<()> {
    set("none", timeout)?;
    set(config, timeout)
}
