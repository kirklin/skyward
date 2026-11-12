use anyhow::{anyhow, bail, Result};

/// The vendor persist partition, where Android's time daemon keeps how far
/// the real time runs ahead of the phone's real-time clock.
pub const PERSIST: &str = "/dev/block/by-name/persist";

/// Where the recovery mounts persist while it reads, read-only, when it is
/// not mounted already.
pub const MOUNT: &str = "/skyward/persist";

/// The time daemon's offset for the time of day, relative to persist's root:
/// milliseconds as a little-endian u64.
pub const OFFSET: &str = "time/ats_2";

/// The real-time clock's count of seconds. On Qualcomm phones it runs from
/// whenever the PMIC last lost power, and Android never sets it.
pub const RTC: &str = "/sys/class/rtc/rtc0/since_epoch";

/// The offset an ats_ file holds, in milliseconds. A daemon that never had
/// the time stores none, and nothing can be told from the RTC alone.
pub fn offset(bytes: &[u8]) -> Result<u64> {
    let raw: [u8; 8] = bytes.try_into().map_err(|_| anyhow!("an offset is 8 bytes, this one is {}", bytes.len()))?;
    match u64::from_le_bytes(raw) {
        0 => bail!("the time daemon has stored no offset yet"),
        ms => Ok(ms),
    }
}

/// The real time, in Unix milliseconds, from the RTC's count of seconds and
/// the time daemon's offset.
pub fn wall(rtc_s: u64, offset_ms: u64) -> u64 {
    rtc_s * 1000 + offset_ms
}

/// Set the system clock to `unix` seconds, from a source that knows better
/// than the phone's own: a server's answer.
pub fn set_wall(unix: i64) -> Result<()> {
    use anyhow::Context;
    let ts = libc::timespec { tv_sec: unix as libc::time_t, tv_nsec: 0 };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set the system clock");
    }
    Ok(())
}

/// Set the system clock to the real time and return it, in Unix
/// milliseconds: the RTC's count plus the offset Android left on persist.
#[cfg(target_os = "android")]
pub fn set_from_rtc() -> Result<u64> {
    use anyhow::Context;
    use std::fs;
    use std::path::Path;
    // An earlier run of the recovery service may have left persist mounted
    // where the phone's own programs use it, writable, and then it cannot
    // be mounted read-only besides.
    let read = match skyward_block::mount::points_of(skyward_block::devnum(PERSIST)?)?.first() {
        Some(at) => fs::read(at.join(OFFSET)).with_context(|| format!("read persist's {OFFSET}")),
        None => {
            let mount = Path::new(MOUNT);
            fs::create_dir_all(mount).with_context(|| format!("make {}", mount.display()))?;
            skyward_block::mount::mount_ro(Path::new(PERSIST), mount)?;
            let read = fs::read(mount.join(OFFSET)).with_context(|| format!("read persist's {OFFSET}"));
            skyward_block::mount::umount(mount)?;
            read
        }
    };
    let offset_ms = offset(&read?)?;
    let rtc = fs::read_to_string(RTC).with_context(|| format!("read {RTC}"))?;
    let rtc_s: u64 = rtc.trim().parse().with_context(|| format!("{RTC} holds {rtc:?}"))?;
    let now = wall(rtc_s, offset_ms);
    let ts = libc::timespec { tv_sec: (now / 1000) as libc::time_t, tv_nsec: ((now % 1000) * 1_000_000) as libc::c_long };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set the system clock");
    }
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_and_the_rtc_make_the_time() {
        // A 13 Ultra on 2026-10-19: its RTC had counted 411912 seconds and
        // ats_2 held 1791968239417 ms; the computer's clock said
        // 1792380150 at that moment.
        let held = 1_791_968_239_417u64.to_le_bytes();
        let now = wall(411_912, offset(&held).unwrap());
        assert!(now / 1000 >= 1_792_380_150 && now / 1000 <= 1_792_380_152, "{now}");
    }

    #[test]
    fn an_offset_never_stored_or_cut_short_is_refused() {
        assert!(offset(&[0; 8]).is_err());
        assert!(offset(&[1, 2, 3]).is_err());
    }
}
