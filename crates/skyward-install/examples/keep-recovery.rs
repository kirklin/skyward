//! Retry post-install recovery preservation without applying the OTA again.
//! By default this only reports the slots that need preservation. Writing
//! requires --write and the SHA-256 of the booted slot's recovery image.

#[cfg(target_os = "android")]
fn main() -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use sha2::{Digest, Sha256};
    use skyward_install::{image, keep::Kept};
    use std::path::Path;

    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(args.is_empty() || (args.len() == 2 && args[0] == "--write"), "usage: keep-recovery [--write EXPECTED_SOURCE_SHA256]");
    ensure!(skyward_sys::prop("init.svc.recovery").as_deref() == Some("running"), "not running in recovery");
    let current = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let next = skyward_vab::boot::next_slot()?;
    let dir = Path::new(skyward_block::BY_NAME);
    let source = std::fs::read(image::device(dir, "recovery", current))?;
    let digest = format!("{:x}", Sha256::digest(&source));
    if !args.is_empty() {
        ensure!(args[1] == digest, "source recovery differs from the expected image");
        let engine = std::process::Command::new("/system/bin/pidof").arg("update_engine_sideload").status()?;
        ensure!(engine.code() == Some(1), "update_engine must have exited before recovery preservation is retried");
    }
    let kept = Kept::hold(dir, current)?.context("booted slot has no Skyward recovery to preserve")?;
    let slots = kept.wanted(next)?;
    println!("booted {current:?}, next {next:?}, source sha256 {digest}, preserve {slots:?}");
    if !args.is_empty() && !slots.is_empty() {
        kept.write(&slots, &mut |_, _| {})?;
        ensure!(kept.wanted(next)?.is_empty(), "recovery still needs preservation after write");
        println!("recovery preservation written and verified");
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn main() {
    eprintln!("keep-recovery runs only inside Android recovery");
}
