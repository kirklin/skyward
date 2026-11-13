//! All a phone's own partitions say of its programs, read together as its
//! init and its service directories read it: every script init takes in,
//! with the scripts those have it read as well, and every description of
//! the services the phone has.

use crate::description::{self, Described, Kind};
use crate::script::{self, Action, Program};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The partitions that carry the programs a recovery starts and say how,
/// in the order init reads them: the one the installed system keeps its
/// directory for hardware services of the older kind on, and the phone's
/// own.
pub const PARTITIONS: [&str; 3] = ["system_ext", "vendor", "odm"];

/// The script of the phone's hardware, which the system's own first script
/// has init read; the scripts of the phone's partitions hang off it.
const HARDWARE_SCRIPT: &str = "/vendor/etc/init/hw/init.${ro.hardware}.rc";

/// The properties init makes of the bootloader's before it reads any
/// script, each with the bootloader's it is made of and what init sets it
/// to where the bootloader passes none; it leaves one with nothing to set
/// it to unset.
const FROM_BOOTLOADER: [(&str, &str, Option<&str>); 6] = [
    ("ro.serialno", "ro.boot.serialno", None),
    ("ro.bootmode", "ro.boot.mode", Some("unknown")),
    ("ro.baseband", "ro.boot.baseband", Some("unknown")),
    ("ro.bootloader", "ro.boot.bootloader", Some("unknown")),
    ("ro.hardware", "ro.boot.hardware", Some("unknown")),
    ("ro.revision", "ro.boot.revision", Some("0")),
];

/// Where each partition keeps the scripts init reads without being told
/// to, and its descriptions.
const SCRIPTS: &str = "etc/init";
const DESCRIPTIONS: &str = "etc/vintf";

/// The partitions whose descriptions tell of the phone's own services, and
/// the property that names the build of the hardware each describes, where
/// one partition serves several.
pub(crate) const DESCRIBING: [(&str, &str); 2] = [("vendor", "ro.boot.product.vendor.sku"), ("odm", "ro.boot.product.hardware.sku")];

/// What happens as every phone starts, in init's own words and in the
/// order it happens; an `on` section that waits for anything else waits
/// for something a recovery cannot tell will happen.
pub(crate) const STARTING: [&str; 11] = ["early-init", "init", "late-init", "early-fs", "fs", "post-fs", "late-fs", "post-fs-data", "zygote-start", "early-boot", "boot"];

/// What the names of the properties begin with that are set once, as the
/// phone starts, and stay as they are.
const SET_FOR_GOOD: &str = "ro.";

/// One of the phone's partitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The name the scripts know it under, at the top of the tree: vendor
    /// for /vendor.
    pub name: String,
    /// Where it is mounted while the phone's programs run, which is where
    /// they are started from.
    pub mounted: PathBuf,
    /// Where its files are read now.
    pub at: PathBuf,
}

impl Partition {
    /// A partition read where it is mounted.
    pub fn mounted(name: &str, mounted: impl Into<PathBuf>) -> Partition {
        let mounted = mounted.into();
        Partition { name: name.into(), at: mounted.clone(), mounted }
    }
}

/// The one of `partitions` the path `path` of a script is on, and the path
/// within it; none where it is on none of them. A partition is named at
/// the top of the tree, or under another that keeps a link to it there:
/// the installed system keeps one to each of the others under /system, and
/// the vendor partition one to the odm partition.
pub fn partition_of<'p, 'a>(partitions: impl IntoIterator<Item = &'p str> + Clone, path: &'a str) -> Option<(&'p str, &'a str)> {
    let mut rest = path.strip_prefix('/')?;
    let named = |top: &str| partitions.clone().into_iter().find(|name| *name == top);
    loop {
        let (top, within) = rest.split_once('/').unwrap_or((rest, ""));
        let under = within.split('/').next().unwrap_or_default();
        if (top == SYSTEM || named(top).is_some()) && named(under).is_some() {
            rest = within;
            continue;
        }
        return Some((named(top)?, within));
    }
}

/// The name of the installed system's own partition at the top of the
/// tree.
const SYSTEM: &str = "system";

/// `text` as init reads a word of a script: with each `${name}` in it
/// replaced by what `property` gives for that name, or by what follows
/// `:-` in it where it gives nothing; none where there is neither.
pub fn expand(text: &str, property: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let end = at + rest[at..].find('}')?;
        let inside = &rest[at + 2..end];
        let (name, default) = match inside.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (inside, None),
        };
        out.push_str(&property(name).or(default.map(str::to_string))?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// A service the phone has, and the file that describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub described: Described,
    /// The description, as the phone's scripts would name it. Where the
    /// phone's init puts another file in its place as the phone starts,
    /// the service is one that file tells of.
    pub file: String,
}

/// Whether something a script says happens on the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Happens {
    Yes,
    No,
    /// It does once a property a recovery cannot know has a value.
    Maybe,
}

/// What a phone's own partitions say of its programs.
#[derive(Debug, Clone)]
pub struct Phone {
    pub partitions: Vec<Partition>,
    /// The properties init has when it reads the scripts: the bootloader's,
    /// those it makes of them, and the build's.
    pub properties: HashMap<String, String>,
    /// Every program init knows, each once: a later section of the same
    /// name takes an earlier one's place only where it says it overrides.
    pub programs: Vec<Program>,
    pub actions: Vec<Action>,
    pub services: Vec<Service>,
}

impl Phone {
    /// Read the scripts and the descriptions of `partitions`, in the order
    /// init reads its partitions, as a phone reads them whose bootloader
    /// and whose build give it `properties`.
    pub fn read(partitions: Vec<Partition>, mut properties: HashMap<String, String>) -> Result<Phone> {
        for (name, from, otherwise) in FROM_BOOTLOADER {
            match properties.get(from).cloned().or(otherwise.map(str::to_string)) {
                Some(value) => properties.insert(name.to_string(), value),
                None => properties.remove(name),
            };
        }
        let mut phone = Phone { partitions, properties, programs: Vec::new(), actions: Vec::new(), services: Vec::new() };
        phone.read_scripts()?;
        phone.read_descriptions()?;
        Ok(phone)
    }

    /// The partition the path `path` of a script is on, and the path within
    /// it; none where it is on none of the partitions read.
    fn within<'a>(&self, path: &'a str) -> Option<(&Partition, &'a str)> {
        let (name, within) = partition_of(self.partitions.iter().map(|p| p.name.as_str()), path)?;
        let partition = self.partitions.iter().find(|p| p.name == name)?;
        Some((partition, within))
    }

    /// Whether the file of `program` is on the partition its script names
    /// it on. A script may name a program its partition does not carry,
    /// and init leaves such a one unstarted and goes on.
    pub fn carries(&self, program: &Program) -> bool {
        self.locate(&program.path).is_some_and(|file| file.symlink_metadata().is_ok())
    }

    /// The names of the services of the newer kind the phone describes,
    /// each once, in the order they are told of: what the phone's own
    /// service directory takes from the programs that register them.
    pub fn told_of(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for service in self.services.iter().filter(|s| s.described.kind == Kind::Aidl) {
            let name = service.described.name();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    /// Where the file a script names `path` is read now; none where it is
    /// on none of the partitions read.
    pub fn locate(&self, path: &str) -> Option<PathBuf> {
        self.within(path).map(|(partition, within)| partition.at.join(within))
    }

    /// Where the program a script names `path` is started from.
    pub fn started_from(&self, path: &str) -> String {
        match self.within(path) {
            Some((partition, within)) => partition.mounted.join(within).to_string_lossy().into_owned(),
            None => path.to_string(),
        }
    }

    /// `text` with each `${name}` in it replaced by the property of that
    /// name, or by what follows `:-` in it where the phone has no such
    /// property; none where it has neither.
    pub fn expand(&self, text: &str) -> Option<String> {
        expand(text, |name| self.properties.get(name).cloned())
    }

    /// Whether all of `triggers` hold as the phone starts: what happens as
    /// every phone starts does, and a property the phone has holds where it
    /// has the value asked for, or any where `*` is asked for. A property
    /// the phone does not have never holds where it is one set for good as
    /// the phone starts, which is what `ro.` names, and may come to hold
    /// where it is any other, as one a program of the phone's sets while it
    /// runs.
    pub fn holds(&self, triggers: &[String]) -> Happens {
        let mut all = Happens::Yes;
        for trigger in triggers {
            let one = match trigger.strip_prefix("property:").map(|p| p.split_once('=').unwrap_or((p, ""))) {
                None => match STARTING.contains(&trigger.as_str()) {
                    true => Happens::Yes,
                    false => Happens::No,
                },
                Some((name, value)) => match self.properties.get(name) {
                    Some(has) if value == "*" || has == value => Happens::Yes,
                    Some(_) => Happens::No,
                    None if name.starts_with(SET_FOR_GOOD) => Happens::No,
                    None => Happens::Maybe,
                },
            };
            match one {
                Happens::No => return Happens::No,
                Happens::Maybe => all = Happens::Maybe,
                Happens::Yes => {}
            }
        }
        all
    }

    /// Whether the phone runs `program`, which its partition has to carry
    /// first: one its classes start, unless an `on` section that holds
    /// stops it; one they do not, where an `on` section that holds starts
    /// it or lets its classes start it, or where its section says which
    /// services it registers, as the service directory then has init start
    /// it when one of them is asked for.
    pub fn runs(&self, program: &Program) -> Happens {
        if !self.carries(program) {
            return Happens::No;
        }
        let told = |commands: &[&str]| -> Happens {
            let mut told = Happens::No;
            for action in &self.actions {
                if !action.commands.iter().any(|c| c.len() == 2 && commands.contains(&c[0].as_str()) && c[1] == program.name) {
                    continue;
                }
                match self.holds(&action.triggers) {
                    Happens::Yes => return Happens::Yes,
                    Happens::Maybe => told = Happens::Maybe,
                    Happens::No => {}
                }
            }
            told
        };
        match program.disabled {
            true if !program.interfaces.is_empty() => Happens::Yes,
            true => told(&["start", "enable"]),
            false => match told(&["stop"]) {
                Happens::Yes => Happens::No,
                Happens::Maybe => Happens::Maybe,
                Happens::No => Happens::Yes,
            },
        }
    }

    /// Read the hardware's script and the scripts in each partition's init
    /// folder, then whatever each script read names, once each. A script
    /// named and not there is passed over, as init passes over it, and so
    /// is one named with a property the phone does not have.
    fn read_scripts(&mut self) -> Result<()> {
        let mut waiting: VecDeque<String> = VecDeque::new();
        waiting.extend(self.expand(HARDWARE_SCRIPT));
        for partition in &self.partitions {
            let folder = format!("/{}/{SCRIPTS}", partition.name);
            waiting.extend(self.scripts_in(&folder)?);
        }
        let mut read = HashSet::new();
        while let Some(path) = waiting.pop_front() {
            if !read.insert(path.clone()) {
                continue;
            }
            let Some(file) = self.locate(&path) else { continue };
            if file.is_dir() {
                waiting.extend(self.scripts_in(&path)?);
                continue;
            }
            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("read {}", file.display())),
            };
            let script = script::read(&text).with_context(|| format!("read {path}"))?;
            waiting.extend(script.imports.iter().filter_map(|import| self.expand(import)));
            self.actions.extend(script.actions);
            for mut program in script.programs {
                program.script = path.clone();
                match self.programs.iter_mut().find(|p| p.name == program.name) {
                    Some(earlier) if program.overrides => *earlier = program,
                    Some(_) => {}
                    None => self.programs.push(program),
                }
            }
        }
        Ok(())
    }

    /// The scripts directly in `folder`, as the phone's scripts would name
    /// them, in the order of their names; none where the partition has no
    /// such folder.
    fn scripts_in(&self, folder: &str) -> Result<Vec<String>> {
        let names = match self.locate(folder) {
            Some(at) => files_in(&at, "rc")?,
            None => Vec::new(),
        };
        Ok(names.into_iter().map(|name| format!("{folder}/{name}")).collect())
    }

    /// The files the phone's init puts in the place of others as the phone
    /// starts, each with the file whose place it takes: a script does so by
    /// mounting the one on the other, in an `on` section that holds.
    pub fn put_in_place(&self) -> HashMap<String, String> {
        let mut put = HashMap::new();
        for action in self.actions.iter().filter(|a| self.holds(&a.triggers) == Happens::Yes) {
            for command in &action.commands {
                if let [mount, _, from, onto, options @ ..] = &command[..]
                    && mount == "mount"
                    && options.iter().any(|o| o == "bind")
                {
                    put.insert(onto.clone(), from.clone());
                }
            }
        }
        put
    }

    /// Read the descriptions of the partitions that describe the phone's
    /// own services: each one's main description, which is the one named
    /// after the build of the hardware where the phone says which build it
    /// is, and every description in its folder of them, each read from the
    /// file init puts in its place where it puts one there.
    fn read_descriptions(&mut self) -> Result<()> {
        let put = self.put_in_place();
        for (name, build) in DESCRIBING {
            let Some(partition) = self.partitions.iter().find(|p| p.name == name) else { continue };
            let folder = partition.at.join(DESCRIPTIONS);
            let mut files = Vec::new();
            let main = match self.properties.get(build) {
                Some(build) if !build.is_empty() => format!("manifest_{build}.xml"),
                _ => "manifest.xml".to_string(),
            };
            if folder.join(&main).is_file() {
                files.push(main);
            }
            files.extend(files_in(&folder.join("manifest"), "xml")?.into_iter().map(|file| format!("manifest/{file}")));
            for file in files {
                let named = format!("/{name}/{DESCRIPTIONS}/{file}");
                let in_its_place = put.get(&named).cloned();
                let at = match &in_its_place {
                    Some(other) => self.locate(other).with_context(|| format!("{other}, which the phone puts in the place of {named}, is on no partition read"))?,
                    None => folder.join(&file),
                };
                let text = fs::read_to_string(&at).with_context(|| format!("read {}", at.display()))?;
                for described in description::read(&text).with_context(|| format!("read {}", in_its_place.as_deref().unwrap_or(&named)))? {
                    self.services.push(Service { described, file: named.clone() });
                }
            }
        }
        Ok(())
    }
}

/// The names of the files directly in `folder` that end in `.extension`,
/// in order; none where there is no such folder.
fn files_in(folder: &Path, extension: &str) -> Result<Vec<String>> {
    let listing = match fs::read_dir(folder) {
        Ok(listing) => listing,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("list {}", folder.display())),
    };
    let mut names = Vec::new();
    for entry in listing {
        let entry = entry.with_context(|| format!("list {}", folder.display()))?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == extension) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_on_the_partition_named_at_its_top_or_linked_under_system() {
        let on = |path| partition_of(PARTITIONS, path);
        assert_eq!(on("/vendor/bin/hw/android.hardware.boot-service.qti"), Some(("vendor", "bin/hw/android.hardware.boot-service.qti")));
        assert_eq!(on("/odm/etc/init"), Some(("odm", "etc/init")));
        assert_eq!(on("/system/system_ext/bin/hwservicemanager"), Some(("system_ext", "bin/hwservicemanager")), "the installed system links to it");
        assert_eq!(on("/system/vendor/bin/sh"), Some(("vendor", "bin/sh")));
        assert_eq!(on("/vendor/odm/bin/init.mi.factory_id.sh"), Some(("odm", "bin/init.mi.factory_id.sh")), "the vendor partition links to it");
        assert_eq!(on("/system/vendor/odm/etc"), Some(("odm", "etc")));
        assert_eq!(on("/vendor"), Some(("vendor", "")));
        assert_eq!(on("/system/bin/sh"), None, "the installed system's own partition is not read");
        assert_eq!(on("/vendor_dlkm/lib/modules"), None, "a longer name is another partition");
        assert_eq!(on("vendor/bin/sh"), None, "a path from nowhere is on none");
    }

    #[test]
    fn a_phone_expands_the_properties_it_has_and_falls_back_where_a_script_says_how() {
        let properties = [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", "sun")].map(|(name, value)| (name.to_string(), value.to_string()));
        let phone = Phone::read(Vec::new(), properties.into_iter().collect()).unwrap();
        assert_eq!(phone.expand("/vendor/etc/init/hw/init.${ro.hardware}.rc").as_deref(), Some("/vendor/etc/init/hw/init.qcom.rc"), "init makes it of the bootloader's");
        assert_eq!(phone.expand("init.${ro.boot.product.vendor.sku}.${ro.boot.mode:-normal}.rc").as_deref(), Some("init.sun.normal.rc"));
        assert_eq!(phone.expand("init.${ro.boot.hwc}.rc"), None, "a property the phone does not have, and nothing to put in its place");
        let triggers = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(phone.holds(&triggers(&["boot", "property:ro.boot.product.vendor.sku=sun"])), Happens::Yes);
        assert_eq!(phone.holds(&triggers(&["boot", "property:ro.boot.product.vendor.sku=kera"])), Happens::No);
        assert_eq!(phone.holds(&triggers(&["property:ro.boot.hwc=CN"])), Happens::No, "one set for good as the phone starts, and not set");
        assert_eq!(phone.holds(&triggers(&["init", "property:vendor.ese.strongbox=NXP"])), Happens::Maybe, "one a program may set while it runs");
        assert_eq!(phone.holds(&triggers(&["property:sys.boot_completed=1", "property:ro.boot.hwc=CN"])), Happens::No);
        assert_eq!(phone.holds(&triggers(&["charger"])), Happens::No, "nothing that happens as every phone starts");
    }
}
