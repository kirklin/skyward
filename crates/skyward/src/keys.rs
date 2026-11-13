//! The phone's own programs that use the keys its secure world keeps.
//! Which they are and how each is started is read from the installed
//! system's own partitions, as its init and its service directories read
//! them, so no phone is told of them by hand, and a system that names them
//! otherwise after an update is followed as it is.

use anyhow::{Context, Result};
use skyward_config::Service;
use skyward_programs::naming::{self, Does};
use skyward_programs::phone::{self, Partition, Phone, PARTITIONS};
use skyward_programs::secure::{self, Step};
use std::path::Path;

/// What the partitions of an installed system say that the recovery acts
/// on.
pub struct Found {
    /// Every service of the newer kind the phone describes: what its own
    /// service directory takes from the programs that register them, and
    /// what the recovery's is told of, so that each of the phone's
    /// programs registers there all it registers on the phone.
    pub described: Vec<String>,
    /// What to do to use the phone's keys, in order, or why the partitions
    /// do not say.
    pub steps: Result<Vec<Step>>,
}

/// What the partitions of the slot's installed system say. Which build of
/// the hardware a partition describes the phone's init may name itself as
/// the phone starts; where it does and the build has no name yet, the
/// recovery names it first, as the phone's scripts have init name it, and
/// reads the partitions as the phone so named.
///
/// `libraries` is a directory of shared libraries a program run for that
/// may need, searched after whatever it names itself.
pub fn found(slot: skyward_vab::Slot, libraries: Option<&Path>) -> Result<Found> {
    let mut phone = read(slot)?;
    let unnamed = naming::steps(&phone)?;
    if !unnamed.is_empty() {
        name(&unnamed, libraries)?;
        phone = read(slot)?;
    }
    Ok(Found { described: phone.told_of(), steps: secure::steps(&phone) })
}

/// The phone as the partitions of the slot's installed system describe it,
/// read with the properties its init has as it reads them: the
/// bootloader's, with those the recovery has set beside them as the
/// phone's init sets them, and the installed build's.
fn read(slot: skyward_vab::Slot) -> Result<Phone> {
    let properties = skyward_info::installed_props(slot)?;
    let mounted = skyward_vab::mounted_partitions(slot)?;
    let mut partitions = Vec::new();
    for name in PARTITIONS {
        let named = format!("{name}{}", slot.suffix());
        let partition = mounted.iter().find(|m| m.partition == named).with_context(|| format!("{named} is not mounted"))?;
        partitions.push(Partition::mounted(name, &partition.mount_point));
    }
    Phone::read(partitions, properties)
}

/// Do what the phone's init does to name the builds its partitions
/// describe: run each program to its end, and set each property to its
/// value as the properties stand by then, the programs having set theirs.
/// Init goes on however a program ends, and a property whose value names
/// one nobody has set stays unset, as init leaves it.
fn name(does: &[Does], libraries: Option<&Path>) -> Result<()> {
    for one in does {
        match one {
            Does::Run(program) => {
                let status = crate::services::once(program, libraries)?;
                skyward_sys::kmsg(&format!("ran {} {}, which ended: {status}", program.service.path, program.service.args.join(" ")))?;
            }
            Does::Set { name, value } => match phone::expand(value, skyward_sys::prop) {
                Some(value) => {
                    skyward_sys::set_prop(name, &value)?;
                    skyward_sys::kmsg(&format!("{name} is {value}, as the phone's init sets it"))?;
                }
                None => skyward_sys::kmsg(&format!("{name} stays unset, as the phone's init leaves it: nothing has set what {value} names"))?,
            },
        }
    }
    Ok(())
}

/// The programs among `steps`, as the recovery keeps them running.
pub fn programs(steps: &[Step]) -> Vec<Service> {
    secure::programs(steps).into_iter().map(|started| started.service.clone()).collect()
}

/// Whether the recovery serves the secure element service among `steps`.
pub fn serves_elements(steps: &[Step]) -> bool {
    steps.contains(&Step::ServeElements)
}

/// Do `steps` in order: start each program and wait until it is ready,
/// and serve the secure element service where it stands among them, which
/// is after the secure elements' own services have registered and before
/// the weaver that reaches its secure element through it starts.
///
/// `libraries` is a directory of shared libraries every program may need,
/// searched after whatever it names itself.
pub fn start(steps: &[Step], libraries: Option<&Path>) -> Result<()> {
    for step in steps {
        match step {
            Step::Start(program) => crate::services::start(std::slice::from_ref(program.as_ref()), libraries)?,
            Step::ServeElements => {
                let readers = skyward_omapi::start()?;
                skyward_sys::kmsg(&format!("{} is served, with a reader for each of {} secure elements: {}", skyward_omapi::SERVICE, readers.len(), readers.join(", ")))?;
            }
        }
    }
    Ok(())
}

/// What `steps` are, a line each, as the `keys` command prints them.
pub fn describe(steps: &[Step]) -> Vec<String> {
    let mut lines = Vec::new();
    for step in steps {
        let Step::Start(program) = step else {
            lines.push(format!("serve {}", skyward_omapi::SERVICE));
            continue;
        };
        let s = &program.service;
        let mut line = format!("start {}", s.path);
        for word in &s.args {
            line.push_str(&format!(" {word}"));
        }
        line.push_str(&format!("\n  as {} in {}", s.user.as_deref().unwrap_or("root"), if s.groups.is_empty() { "root".to_string() } else { s.groups.join(" ") }));
        if !s.capabilities.is_empty() {
            line.push_str(&format!(", keeping {}", s.capabilities.join(" ")));
        }
        for socket in &program.sockets {
            let owner = format!("{}:{}", socket.user.as_deref().unwrap_or("root"), socket.group.as_deref().unwrap_or("root"));
            line.push_str(&format!("\n  with the socket {} ({:?}, {:o}, {owner})", socket.name, socket.kind, socket.mode));
        }
        for (name, value) in &program.environment {
            line.push_str(&format!("\n  with {name}={value}"));
        }
        for ready in s.ready_property.iter().chain(&s.ready_service).chain(&s.ready_hardware_service) {
            line.push_str(&format!("\n  ready with {ready}"));
        }
        for name in &s.registers {
            line.push_str(&format!("\n  registering {name} as well"));
        }
        if program.anew {
            line.push_str("\n  started anew where an earlier recovery service left it running");
        }
        lines.push(line);
    }
    lines
}
