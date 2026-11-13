//! Switching which system the bootloader starts: the boot set of one put
//! into the boot partitions of a slot in place of the other's, with every
//! step kept in the state first, so a switch cut off part way is finished
//! when the recovery next starts, and the installed system's boot set is
//! never lost: it is saved before anything is written over it, into the
//! entry its last save is not in, and that save counts only once whole.
//! The other system starts with the program of the Skyward that switches
//! to it, put into its init_boot image as it is written.

use crate::boot;
use crate::state::{State, Switching, Who};
use crate::store;
use anyhow::{bail, ensure, Context, Result};
use skyward_vab::Slot;
use std::fs;
use std::path::Path;

/// Where a switch reads and writes: the state, the partitions among the
/// links in `by_name`, the store, at `store` once laid out as a device,
/// and the program the other system starts with.
pub struct Places<'a> {
    pub state: &'a Path,
    pub by_name: &'a Path,
    pub store: &'a Path,
    pub program: &'a Path,
}

/// Have the bootloader start `to`: its boot set written into the boot
/// partitions of `next`, the slot that starts next, or, for the installed
/// system, of the slot the other system's boot set is in.
pub fn switch(places: &Places, next: Slot, to: Who) -> Result<()> {
    let mut state = State::load(places.state)?;
    ensure!(state.switching.is_none(), "a switch begun before is not done yet");
    ensure!(state.system.is_some(), "no other system is installed");
    let slot = match (to, &state.occupied) {
        (Who::Other, Some(s)) => bail!("the other system's boot set is in the partitions of slot {s} already"),
        (Who::Other, None) => next.suffix().to_string(),
        (Who::Installed, Some(s)) => s.clone(),
        (Who::Installed, None) => bail!("the installed system's boot set is in the partitions already"),
    };
    // Made before the switch begins, so that a switch begun is one that
    // can be finished.
    let init_boot = match to {
        Who::Other => Some(other_init_boot(places, &state)?),
        Who::Installed => None,
    };
    state.switching = Some(Switching { to, slot, saved: false });
    state.save(places.state)?;
    finish(places, &mut state, init_boot)
}

/// Finish a switch begun before and cut off, where one was; says whether
/// there was one.
pub fn resume(places: &Places) -> Result<bool> {
    let mut state = State::load(places.state)?;
    if state.switching.is_none() {
        return Ok(false);
    }
    finish(places, &mut state, None)?;
    Ok(true)
}

/// The other system's init_boot image as stored, made again with the
/// program at `places.program`.
fn other_init_boot(places: &Places, state: &State) -> Result<Vec<u8>> {
    let store = state.store.as_ref().context("no store holds the boot sets")?;
    let system = state.system.as_ref().context("no other system is installed")?;
    let stored = store::init_boot(store, places.store, &system.set)?;
    let program = fs::read(places.program).with_context(|| format!("read {}", places.program.display()))?;
    boot::with_program(&stored, system.own_ramdisk, &program, &system.start())
}

/// Finish the switch the state holds; `init_boot` is the other system's
/// init_boot image where a switch to it made it already.
fn finish(places: &Places, state: &mut State, init_boot: Option<Vec<u8>>) -> Result<()> {
    let switching = state.switching.clone().context("no switch to finish")?;
    let slot = Slot::from_suffix(&switching.slot).with_context(|| format!("the switch is to slot {:?}", switching.slot))?;
    let store = state.store.clone().context("no store holds the boot sets")?;
    let init_boot = match (switching.to, init_boot) {
        (Who::Other, Some(made)) => Some(made),
        (Who::Other, None) => Some(other_init_boot(places, state)?),
        (Who::Installed, _) => None,
    };
    // While the installed system's boot set is the one in the partitions,
    // it is saved before the other's is written over it.
    if state.occupant(&switching.slot) == Who::Installed && !switching.saved {
        let entry = crate::state::Store::next_save(state.saved.as_ref());
        let saved = store::save(&store, places.store, entry, places.by_name, slot).context("save the installed system's boot set")?;
        state.saved = Some(saved);
        state.switching = Some(Switching { saved: true, ..switching.clone() });
        state.save(places.state)?;
    }
    let set = match switching.to {
        Who::Installed => state.saved.clone().context("the installed system's boot set was never saved")?,
        Who::Other => state.system.as_ref().context("no other system is installed")?.set.clone(),
    };
    let written = store::restore(&store, places.store, &set, places.by_name, slot, init_boot.as_deref()).with_context(|| format!("write the {:?} system's boot set", switching.to))?;
    (state.occupied, state.written) = match switching.to {
        Who::Other => (Some(switching.slot.clone()), Some(written)),
        Who::Installed => (None, None),
    };
    state.switching = None;
    state.save(places.state)
}

/// What a slot's boot partitions hold, set against what the state says is
/// in them. Skyward is not the only one that writes them: a computer does
/// over fastboot, and erasing /metadata there takes the state away while
/// the partitions stay as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// What the state says.
    Recorded,
    /// The other system's boot set by the state, and something another
    /// hand wrote since in fact: the installed system's saved boot set is
    /// not written over that unasked.
    Rewritten,
    /// The installed system's boot set by the state, and in fact an
    /// init_boot image that starts a system from the data partition: the
    /// state that said so is gone, and the installed system's saved boot
    /// set with it where the data partition was erased as well.
    Unrecorded,
}

/// What the boot partitions hold, among the links in `by_name`: those of
/// the slot the state says the other system's boot set is in, or, where it
/// says of none, those of `starts`, the slot the bootloader starts. A
/// switch begun and not done is one the partitions are part way through,
/// which is as the state says.
pub fn held(state: &State, by_name: &Path, starts: Slot) -> Result<Held> {
    if state.switching.is_some() {
        return Ok(Held::Recorded);
    }
    let Some(occupied) = &state.occupied else {
        let init_boot = store::partition(by_name, store::INIT_BOOT, starts);
        return Ok(match fs::read(&init_boot) {
            Ok(image) if boot::starts_from_data(&image) => Held::Unrecorded,
            Ok(_) => Held::Recorded,
            // A phone without an init_boot partition holds none to tell by.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Held::Recorded,
            Err(e) => return Err(e).with_context(|| format!("read {}", init_boot.display())),
        });
    };
    let slot = Slot::from_suffix(occupied).with_context(|| format!("the other system is in slot {occupied:?}"))?;
    let store = state.store.as_ref().context("no store holds the boot sets")?;
    let system = state.system.as_ref().context("no other system is installed")?;
    let held = store::held(store, by_name, slot)?;
    // A state kept before what was written was kept tells of the boot
    // image alone, which goes in as the store holds it.
    let as_written = match &state.written {
        Some(written) => held == *written,
        None => held.boot == system.set.boot,
    };
    Ok(if as_written { Held::Recorded } else { Held::Rewritten })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::tests::{entries, start, template};
    use crate::boot::{INIT, START};
    use crate::state::tests::area;
    use crate::state::{Saved, System};
    use crate::store::tests::bench;
    use crate::store::{put, OTHER};
    use skyward_bootimg::BootImage;
    use std::path::PathBuf;

    struct Bench {
        _dir: tempfile::TempDir,
        dir: PathBuf,
        state: PathBuf,
        store: PathBuf,
        program: PathBuf,
    }

    impl Bench {
        fn places(&self) -> Places<'_> {
            Places { state: &self.state, by_name: &self.dir, store: &self.store, program: &self.program }
        }

        fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.dir.join(name)).unwrap()
        }

        /// The program the init_boot image in partition `name` starts.
        fn program_in(&self, name: &str) -> Vec<u8> {
            let found = entries(&BootImage::parse(&self.read(name)).unwrap().ramdisk);
            let start_text = &found.iter().find(|(n, _)| n == START).unwrap().1;
            assert_eq!(crate::boot::Start::parse(std::str::from_utf8(start_text).unwrap()).unwrap(), start());
            found.into_iter().find(|(n, _)| n == INIT).unwrap().1
        }
    }

    /// A phone whose boot partitions hold the installed system's boot set,
    /// with the other system's boot set put in the store as it was
    /// installed, with an older program than the one at hand now.
    fn installed() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let (store, device) = bench(&path);
        let init_boot = crate::boot::init_boot(&template(b"stock"), b"old program", &start()).unwrap();
        let set = put(&store, &device, OTHER, &mut &[0x5au8; 8192][..], 8192, &init_boot).unwrap();
        let state = State {
            store: Some(store),
            system: Some(System {
                id: "test".into(),
                name: "Test".into(),
                version: "1".into(),
                filesystem: "ext4".into(),
                init: "/sbin/init".into(),
                root: area("/data/skyward/systems/test", 1 << 20),
                set,
                own_ramdisk: None,
                vendor_boot: String::new(),
                dtbo: String::new(),
            }),
            ..State::default()
        };
        let state_path = path.join("systems.toml");
        state.save(&state_path).unwrap();
        let program = path.join("system-init");
        std::fs::write(&program, b"new program").unwrap();
        Bench { _dir: dir, dir: path, state: state_path, store: device, program }
    }

    #[test]
    fn switching_there_and_back_puts_each_systems_boot_set_in_place() {
        let b = installed();
        switch(&b.places(), Slot::A, Who::Other).unwrap();
        assert_eq!(b.read("boot_a"), vec![0x5a; 8192]);
        assert_eq!(b.program_in("init_boot_a"), b"new program", "the program at hand, in place of the one installed with it");
        let s = State::load(&b.state).unwrap();
        assert_eq!((s.occupied.as_deref(), s.switching, s.saved.as_ref().map(|x| x.entry)), (Some("_a"), None, Some(0)));
        assert_eq!(b.read("boot_b"), vec![0xb1; 8192], "the other slot left alone");
        assert!(switch(&b.places(), Slot::A, Who::Other).is_err(), "already there");
        switch(&b.places(), Slot::B, Who::Installed).unwrap();
        assert_eq!((b.read("boot_a"), b.read("init_boot_a")), (vec![0xa1; 8192], vec![0xa2; 16384]), "back in the slot it left");
        assert_eq!(State::load(&b.state).unwrap().occupied, None);
        // The next save goes to the other entry, the last one kept whole.
        switch(&b.places(), Slot::A, Who::Other).unwrap();
        assert_eq!(State::load(&b.state).unwrap().saved.unwrap().entry, 1);
    }

    #[test]
    fn a_switch_whose_init_boot_image_cannot_be_made_never_begins() {
        let b = installed();
        std::fs::remove_file(&b.program).unwrap();
        assert!(switch(&b.places(), Slot::A, Who::Other).is_err());
        let s = State::load(&b.state).unwrap();
        assert_eq!((s.switching, s.saved, s.occupied), (None, None, None), "nothing begun, nothing saved");
        assert_eq!((b.read("boot_a"), b.read("init_boot_a")), (vec![0xa1; 8192], vec![0xa2; 16384]));
    }

    #[test]
    fn a_switch_to_the_other_system_cut_off_is_finished_with_the_program_at_hand() {
        let b = installed();
        let mut s = State::load(&b.state).unwrap();
        s.switching = Some(Switching { to: Who::Other, slot: "_a".into(), saved: false });
        s.save(&b.state).unwrap();
        std::fs::write(&b.program, b"newer program").unwrap();
        assert!(resume(&b.places()).unwrap());
        assert_eq!((b.read("boot_a"), b.program_in("init_boot_a")), (vec![0x5a; 8192], b"newer program".to_vec()));
    }

    #[test]
    fn a_switch_cut_off_after_the_save_is_finished_without_saving_again() {
        let b = installed();
        // Cut off while the other system's boot set was being written: the
        // partitions hold half of it, and the save is kept.
        let mut s = State::load(&b.state).unwrap();
        let store = s.store.clone().unwrap();
        s.saved = Some(store::save(&store, &b.store, 0, &b.dir, Slot::A).unwrap());
        s.switching = Some(Switching { to: Who::Other, slot: "_a".into(), saved: true });
        s.save(&b.state).unwrap();
        std::fs::write(b.dir.join("boot_a"), [vec![0x5a; 4096], vec![0xa1; 4096]].concat()).unwrap();
        assert!(resume(&b.places()).unwrap());
        assert_eq!(b.read("boot_a"), vec![0x5a; 8192]);
        assert_eq!(State::load(&b.state).unwrap().saved.unwrap().boot, s.saved.unwrap().boot, "the half-written partition was not saved");
        switch(&b.places(), Slot::A, Who::Installed).unwrap();
        assert_eq!((b.read("boot_a"), b.read("init_boot_a")), (vec![0xa1; 8192], vec![0xa2; 16384]));
        assert!(!resume(&b.places()).unwrap(), "nothing left to finish");
    }

    #[test]
    fn a_switch_cut_off_before_the_save_saves_first() {
        let b = installed();
        let mut s = State::load(&b.state).unwrap();
        s.switching = Some(Switching { to: Who::Other, slot: "_b".into(), saved: false });
        s.save(&b.state).unwrap();
        assert!(resume(&b.places()).unwrap());
        assert_eq!(b.read("boot_b"), vec![0x5a; 8192]);
        let s = State::load(&b.state).unwrap();
        assert_eq!(s.occupied.as_deref(), Some("_b"));
        switch(&b.places(), Slot::A, Who::Installed).unwrap();
        assert_eq!(b.read("boot_b"), vec![0xb1; 8192], "the save was of slot B's own");
    }

    #[test]
    fn what_the_partitions_hold_is_set_against_what_the_state_says() {
        let b = installed();
        let load = || State::load(&b.state).unwrap();
        assert_eq!(held(&load(), &b.dir, Slot::A).unwrap(), Held::Recorded, "the installed system's, as the state says");
        switch(&b.places(), Slot::A, Who::Other).unwrap();
        let written = load().written.expect("what went into the partitions is kept");
        assert_eq!(written, store::held(&load().store.unwrap(), &b.dir, Slot::A).unwrap());
        assert_eq!(held(&load(), &b.dir, Slot::B).unwrap(), Held::Recorded, "whichever slot starts, the slot looked at is the one written");

        // A computer writes another boot image over fastboot.
        let ours = b.read("boot_a");
        std::fs::write(b.dir.join("boot_a"), vec![0xf1; 8192]).unwrap();
        assert_eq!(held(&load(), &b.dir, Slot::A).unwrap(), Held::Rewritten);
        std::fs::write(b.dir.join("boot_a"), &ours).unwrap();
        assert_eq!(held(&load(), &b.dir, Slot::A).unwrap(), Held::Recorded);
        // Or another init_boot image.
        let ours = b.read("init_boot_a");
        std::fs::write(b.dir.join("init_boot_a"), vec![0xf2; 16384]).unwrap();
        assert_eq!(held(&load(), &b.dir, Slot::A).unwrap(), Held::Rewritten);
        // A state from before what was written was kept tells by the boot
        // image alone.
        let earlier = State { written: None, ..load() };
        assert_eq!(held(&earlier, &b.dir, Slot::A).unwrap(), Held::Recorded);
        std::fs::write(b.dir.join("boot_a"), vec![0xf1; 8192]).unwrap();
        assert_eq!(held(&earlier, &b.dir, Slot::A).unwrap(), Held::Rewritten);
        std::fs::write(b.dir.join("boot_a"), vec![0x5a; 8192]).unwrap();
        std::fs::write(b.dir.join("init_boot_a"), &ours).unwrap();

        // /metadata erased: the state is gone and the partitions stay.
        assert_eq!(held(&State::default(), &b.dir, Slot::A).unwrap(), Held::Unrecorded);
        assert_eq!(held(&State::default(), &b.dir, Slot::B).unwrap(), Held::Recorded, "the other slot holds its own");

        switch(&b.places(), Slot::A, Who::Installed).unwrap();
        assert_eq!((load().written, held(&load(), &b.dir, Slot::A).unwrap()), (None, Held::Recorded));
    }

    #[test]
    fn a_switch_part_way_is_as_the_state_says() {
        let b = installed();
        switch(&b.places(), Slot::A, Who::Other).unwrap();
        std::fs::write(b.dir.join("boot_a"), [vec![0xa1; 4096], vec![0x5a; 4096]].concat()).unwrap();
        let mut s = State::load(&b.state).unwrap();
        s.switching = Some(Switching { to: Who::Installed, slot: "_a".into(), saved: false });
        assert_eq!(held(&s, &b.dir, Slot::A).unwrap(), Held::Recorded);
    }

    #[test]
    fn a_damaged_save_leaves_the_partitions_as_they_are() {
        let b = installed();
        switch(&b.places(), Slot::A, Who::Other).unwrap();
        let mut s = State::load(&b.state).unwrap();
        s.saved = Some(Saved { boot: "0".repeat(64), ..s.saved.unwrap() });
        s.save(&b.state).unwrap();
        assert!(switch(&b.places(), Slot::A, Who::Installed).is_err());
        assert_eq!(b.read("boot_a"), vec![0x5a; 8192], "nothing written from a save that does not check");
        assert!(State::load(&b.state).unwrap().switching.is_some(), "the switch waits to be finished");
    }
}
