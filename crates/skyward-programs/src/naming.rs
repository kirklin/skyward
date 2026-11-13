//! The name of the build of the hardware each of a phone's partitions
//! describes, where the phone's own init gives it. A partition that serves
//! several builds keeps a main description of its services for each, named
//! after the build, and a property says which of them the phone is. The
//! bootloader passes some of these properties; the phone's init sets the
//! others as the phone starts, in an `on` section of its scripts that first
//! runs a program of the phone's to make the hardware out. A recovery's own
//! init reads none of the phone's scripts, so the recovery does what such a
//! section does before it reads the descriptions.

use crate::phone::{Happens, Phone, DESCRIBING, STARTING};
use crate::secure::{libraries_of, Started};
use anyhow::{bail, ensure, Context, Result};
use skyward_config::Service;

/// One thing the phone's init does to name a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Does {
    /// Run a program of the phone's to its end: it makes the hardware out
    /// and sets properties to what it found.
    Run(Box<Started>),
    /// Set the property `name` to `value`, in which each `${name}` stands
    /// for a property as it is by then; where one of them is not set, init
    /// leaves `name` unset as well.
    Set { name: String, value: String },
}

/// The commands a section gives properties their values with: one runs a
/// program, the other sets a property.
const RUN: &str = "exec";
const SET: &str = "setprop";

/// What ends, in a line that runs a program, the words that say as whom:
/// the SELinux label, the user, the group and the other groups, each there
/// only where the one before it is.
const PROGRAM_FOLLOWS: &str = "--";

/// What the phone's init does as the phone starts to name the builds its
/// partitions describe, in the order it does it, for each build `phone` has
/// no name of: of the first `on` section to set the property that names
/// it, the programs it runs and the properties it sets, up to that one. A
/// phone whose builds are all named, or whose scripts name none, gives
/// nothing to do.
pub fn steps(phone: &Phone) -> Result<Vec<Does>> {
    // Each section that names a build, with the last of its commands that
    // names one.
    let mut naming: Vec<(usize, usize)> = Vec::new();
    for (_, property) in DESCRIBING {
        if phone.properties.contains_key(property) {
            continue;
        }
        let Some((section, at)) = first_to_set(phone, property) else { continue };
        match naming.iter_mut().find(|(named, _)| *named == section) {
            Some((_, last)) => *last = (*last).max(at),
            None => naming.push((section, at)),
        }
    }
    naming.sort_by_key(|(section, _)| turn(phone, *section));
    let mut does = Vec::new();
    for (section, last) in naming {
        for command in &phone.actions[section].commands[..=last] {
            match command.first().map(String::as_str) {
                Some(RUN) => does.push(Does::Run(Box::new(run(phone, command)?))),
                Some(SET) => {
                    let [_, name, value] = &command[..] else { bail!("`{}` is not a property and its value", command.join(" ")) };
                    does.push(Does::Set { name: name.clone(), value: value.clone() });
                }
                _ => {}
            }
        }
    }
    Ok(does)
}

/// When init comes to the section numbered `section`: with what happens
/// first as the phone starts, and among the sections of one such moment in
/// the order it read them. One that names no such moment waits for its
/// properties alone, which init looks at once all of those have passed.
fn turn(phone: &Phone, section: usize) -> (usize, usize) {
    let moment = phone.actions[section].triggers.iter().find_map(|t| STARTING.iter().position(|s| s == t)).unwrap_or(STARTING.len());
    (moment, section)
}

/// The section that holds as the phone starts and sets `property` before
/// any other does, with which of its commands sets it. Init takes the
/// first value a property of this kind is given and turns away the rest.
fn first_to_set(phone: &Phone, property: &str) -> Option<(usize, usize)> {
    let sets = |command: &Vec<String>| matches!(&command[..], [set, name, _] if set == SET && name == property);
    let holding = phone.actions.iter().enumerate().filter(|(_, action)| phone.holds(&action.triggers) == Happens::Yes);
    let setting = holding.filter_map(|(section, action)| Some((section, action.commands.iter().position(sets)?)));
    setting.min_by_key(|(section, _)| turn(phone, *section))
}

/// The program the line `command` has init run, as the recovery runs it:
/// once, to its end, from where its partition is mounted, as the user and
/// in the groups the line names. The label the line names is passed over,
/// as a recovery runs the phone's programs under its own.
fn run(phone: &Phone, command: &[String]) -> Result<Started> {
    let line = || command.join(" ");
    let words = command.iter().map(|word| phone.expand(word).with_context(|| format!("`{}` names a property the phone has not", line()))).collect::<Result<Vec<_>>>()?;
    let (as_whom, program) = match words.iter().position(|word| word == PROGRAM_FOLLOWS) {
        Some(at) => (&words[1..at], &words[at + 1..]),
        None => (&words[..0], &words[1..]),
    };
    ensure!(!program.is_empty(), "`{}` names no program", line());
    let service = Service {
        path: phone.started_from(&program[0]),
        args: program[1..].to_vec(),
        libraries: libraries_of(&program[0]),
        oneshot: true,
        ready_property: None,
        ready_service: None,
        registers: Vec::new(),
        ready_hardware_service: None,
        when: None,
        user: as_whom.get(1).cloned(),
        groups: as_whom.iter().skip(2).cloned().collect(),
        capabilities: Vec::new(),
    };
    Ok(Started::from(service))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::Partition;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;

    /// A phone whose vendor and odm partitions lie under `at`, each with
    /// one script, read with `properties`.
    fn phone(at: &Path, vendor: &str, odm: &str, properties: &[(&str, &str)]) -> Phone {
        let mut partitions = Vec::new();
        for (name, script) in [("vendor", vendor), ("odm", odm)] {
            let folder = at.join(name).join("etc/init");
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("names.rc"), script).unwrap();
            partitions.push(Partition { name: name.to_string(), mounted: Path::new("/").join(name), at: at.join(name) });
        }
        let properties: HashMap<String, String> = properties.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
        Phone::read(partitions, properties).unwrap()
    }

    fn once(path: &str, args: &[&str], libraries: &[&str], user: Option<&str>, groups: &[&str]) -> Does {
        let words = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        Does::Run(Box::new(Started::from(Service {
            path: path.to_string(),
            args: words(args),
            libraries: words(libraries),
            oneshot: true,
            ready_property: None,
            ready_service: None,
            registers: Vec::new(),
            ready_hardware_service: None,
            when: None,
            user: user.map(str::to_string),
            groups: words(groups),
            capabilities: Vec::new(),
        })))
    }

    fn set(name: &str, value: &str) -> Does {
        Does::Set { name: name.to_string(), value: value.to_string() }
    }

    const BY_THE_CHIP: &str = "on early-init\n    exec u:r:vendor_qti_init_shell:s0 -- /vendor/bin/init.qti.qcv.sh\n    setprop ro.boot.product.vendor.sku ${ro.vendor.qti.soc_name}\n    setprop ro.soc.model ${ro.vendor.qti.soc_model}\n";

    #[test]
    fn a_build_the_bootloader_does_not_name_is_named_as_the_phone_s_init_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let named_by_none = phone(dir.path(), BY_THE_CHIP, "", &[("ro.boot.hardware", "qcom"), ("ro.boot.product.hardware.sku", "ishtar")]);
        assert_eq!(
            steps(&named_by_none).unwrap(),
            [
                once("/vendor/bin/init.qti.qcv.sh", &[], &["/system/lib64", "/vendor/lib64"], None, &[]),
                set("ro.boot.product.vendor.sku", "${ro.vendor.qti.soc_name}"),
            ],
            "the section's commands up to the one that names the build, and none after it"
        );
        let named = phone(dir.path(), BY_THE_CHIP, "", &[("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", "kalama")]);
        assert_eq!(steps(&named).unwrap(), [], "a build that has its name is not named again, and one no script names stays as it is");
    }

    #[test]
    fn the_sections_that_name_builds_are_done_in_init_s_order_each_once() {
        let vendor = "on boot\n    setprop ro.boot.product.vendor.sku late\n\n\
                      on init && property:ro.boot.hwc=GL\n    setprop ro.boot.product.vendor.sku abroad\n\n\
                      on init && property:ro.boot.hwc=CN\n    write /dev/kmsg naming\n    exec u:r:shell:s0 system system input -- /odm/bin/which.sh --of ${ro.boot.hwc}\n    setprop ro.boot.product.vendor.sku ${ro.vendor.which}\n    setprop ro.boot.product.hardware.sku ${ro.vendor.which}_${ro.boot.hwc}\n    setprop ro.vendor.after 1\n";
        let odm = "on charger\n    setprop ro.boot.product.hardware.sku charging\n";
        let dir = tempfile::tempdir().unwrap();
        let phone = phone(dir.path(), vendor, odm, &[("ro.boot.hardware", "qcom"), ("ro.boot.hwc", "CN")]);
        let odm_libraries = ["/system/lib64", "/vendor/lib64", "/odm/lib64", "/mnt/system/system/lib64", "/mnt/system/system/lib64/bootstrap"];
        assert_eq!(
            steps(&phone).unwrap(),
            [
                once("/odm/bin/which.sh", &["--of", "CN"], &odm_libraries, Some("system"), &["system", "input"]),
                set("ro.boot.product.vendor.sku", "${ro.vendor.which}"),
                set("ro.boot.product.hardware.sku", "${ro.vendor.which}_${ro.boot.hwc}"),
            ],
            "the section that holds and comes first as the phone starts, once for both builds it names"
        );
    }

    #[test]
    fn a_line_that_runs_nothing_or_names_a_property_the_phone_has_not_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for exec in ["exec u:r:shell:s0 --", "exec -- /vendor/bin/which.sh ${ro.boot.hwc}"] {
            let script = format!("on early-init\n    {exec}\n    setprop ro.boot.product.vendor.sku ${{ro.vendor.which}}\n");
            let phone = phone(dir.path(), &script, "", &[("ro.boot.hardware", "qcom")]);
            assert!(steps(&phone).is_err(), "{exec}");
        }
    }
}
