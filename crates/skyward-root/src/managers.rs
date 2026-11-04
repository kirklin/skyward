//! The root managers the recovery knows: what each one is, where it keeps its
//! patcher, which of them the phone has, and how a package of theirs is read.

pub mod apatch {
//! Patching a boot image the way APatch does: its own `boot_patch.sh`, which
//! unpacks the image, swaps the superkey the kernel patches for, and repacks.
//! What APatch also carries — the KPM modules embedded in the boot image, and
//! the superkey hash of the user — is read from the image being patched.

use crate::{magisk::run_boot_patch, Options};
use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The tools APatch patches with, and where they are: `/data/adb/ap` holds
/// `boot_patch.sh` and `bin/kp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apatch {
    pub dir: PathBuf,
    pub version: Option<String>,
}

impl Apatch {
    /// Patch `stock` into `out`.
    pub fn patch(&self, stock: &Path, out: &Path, options: &Options) -> Result<()> {
        // The patcher APatch ships takes the superkey the kernel patches for
        // before the image; `su` is the one it uses when none was chosen.
        let script = self.dir.join("kptools");
        if !script.is_file() {
            return run_boot_patch(&self.dir, stock, out, options);
        }
        let busybox = self.dir.join("busybox");
        let (program, mut args): (PathBuf, Vec<&OsStr>) = if busybox.is_file() {
            (busybox, vec![OsStr::new("sh"), OsStr::new("boot_patch.sh")])
        } else {
            (PathBuf::from("/system/bin/sh"), vec![OsStr::new("boot_patch.sh")])
        };
        args.push(OsStr::new("su"));
        args.push(stock.as_os_str());
        crate::cmd::run(&program, &args, &self.dir, &[]).context("run APatch's boot patcher")?;
        let made = self.dir.join("new-boot.img");
        if !made.is_file() {
            bail!("APatch's patcher made no new-boot.img in {}", self.dir.display());
        }
        crate::magisk::move_into(&made, out)
    }
}
}

pub mod apk {
//! The files inside a Magisk APK that its own boot patcher runs: it ships
//! them as libraries, so that the package manager installs them, and calls
//! them by the names it does.

use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

/// What a Magisk APK holds for its patcher, and the name each takes where the
/// patcher runs: the first is what the archive entry ends with, the second is
/// the file `boot_patch.sh` looks for.
const TOOLS: [(&str, &str); 8] = [
    ("libmagiskboot.so", "magiskboot"),
    ("libmagiskinit.so", "magiskinit"),
    ("libmagisk.so", "magisk"),
    ("libinit-ld.so", "init-ld"),
    ("libbusybox.so", "busybox"),
    ("assets/boot_patch.sh", "boot_patch.sh"),
    ("assets/util_functions.sh", "util_functions.sh"),
    ("assets/stub.apk", "stub.apk"),
];

/// Which root manager a package is: the three of them ship their patcher as
/// a library of its own, and the name of that library says which it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Magisk,
    KernelSu,
    Apatch,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Magisk => "Magisk",
            Kind::KernelSu => "KernelSU",
            Kind::Apatch => "APatch",
        }
    }

    /// The library whose name says the package is this one.
    fn marker(self) -> &'static str {
        match self {
            Kind::Magisk => "libmagiskboot.so",
            Kind::KernelSu => "libksud.so",
            Kind::Apatch => "libapd.so",
        }
    }
}

/// Which manager the package at `apk` is, by the library it ships.
pub fn kind(apk: &Path) -> Result<Kind> {
    let mut zip = ZipArchive::new(File::open(apk).with_context(|| format!("open {}", apk.display()))?)
        .with_context(|| format!("read {}", apk.display()))?;
    for kind in [Kind::Magisk, Kind::KernelSu, Kind::Apatch] {
        if entry(&mut zip, kind.marker()).is_some() {
            return Ok(kind);
        }
    }
    bail!("{} is not a root manager's package: it carries no patcher", apk.display())
}

/// The files `kind`'s patcher runs, unpacked into `into`, which is made: the
/// package carries them as libraries, and they run under the names the
/// patcher looks for.
pub fn unpack(apk: &Path, into: &Path, kind: Kind) -> Result<()> {
    match kind {
        Kind::Magisk => {
            unpack_magisk(apk, into)?;
        }
        Kind::KernelSu => {
            // `ksud` patches the boot image itself: nothing else comes with it.
            let mut zip = ZipArchive::new(File::open(apk).with_context(|| format!("open {}", apk.display()))?)?;
            let Some(entry) = entry(&mut zip, "libksud.so") else {
                bail!("{} carries no ksud", apk.display());
            };
            let mut bytes = Vec::new();
            zip.by_name(&entry)?.read_to_end(&mut bytes)?;
            fs::create_dir_all(into).with_context(|| format!("create {}", into.display()))?;
            let at = into.join("ksud");
            fs::write(&at, &bytes).with_context(|| format!("write {}", at.display()))?;
            fs::set_permissions(&at, fs::Permissions::from_mode(0o755))?;
        }
        // APatch patches the kernel in the image with `kptools`, which takes
        // `kpimg` to inject; its own script runs the files by name from its
        // folder.
        Kind::Apatch => {
            const APATCH: [(&str, &str); 5] = [
                ("libkptools.so", "kptools"),
                ("kpimg", "kpimg"),
                ("assets/boot_patch.sh", "boot_patch.sh"),
                ("assets/util_functions.sh", "util_functions.sh"),
                ("libbusybox.so", "busybox"),
            ];
            let mut zip = ZipArchive::new(File::open(apk).with_context(|| format!("open {}", apk.display()))?)?;
            fs::create_dir_all(into).with_context(|| format!("create {}", into.display()))?;
            for (suffix, name) in APATCH {
                let Some(entry) = entry(&mut zip, suffix) else {
                    bail!("{} holds no {suffix}", apk.display());
                };
                let mut bytes = Vec::new();
                zip.by_name(&entry)?.read_to_end(&mut bytes)?;
                let at = into.join(name);
                fs::write(&at, &bytes).with_context(|| format!("write {}", at.display()))?;
                fs::set_permissions(&at, fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

/// The entry of `zip` whose name ends with `suffix`, the phone's own
/// architecture first.
fn entry(zip: &mut ZipArchive<File>, suffix: &str) -> Option<String> {
    let mut found: Vec<String> = zip.file_names().filter(|name| name.ends_with(suffix)).map(str::to_string).collect();
    found.sort_by_key(|name| u8::from(!name.contains("/arm64")));
    found.into_iter().next()
}

/// Unpack the tools of the Magisk APK at `apk` into `into`, which is made, and
/// give back what was written. The tools come out runnable.
pub fn unpack_magisk(apk: &Path, into: &Path) -> Result<Vec<PathBuf>> {
    let mut zip = ZipArchive::new(File::open(apk).with_context(|| format!("open {}", apk.display()))?)
        .with_context(|| format!("read {}", apk.display()))?;
    fs::create_dir_all(into).with_context(|| format!("create {}", into.display()))?;
    let mut written = Vec::with_capacity(TOOLS.len());
    for (suffix, name) in TOOLS {
        let Some(entry) = entry(&mut zip, suffix) else {
            bail!("{} holds no {suffix}", apk.display());
        };
        let mut bytes = Vec::new();
        zip.by_name(&entry)
            .with_context(|| format!("find {entry}"))?
            .read_to_end(&mut bytes)
            .with_context(|| format!("read {entry}"))?;
        let at = into.join(name);
        fs::write(&at, &bytes).with_context(|| format!("write {}", at.display()))?;
        // The patcher is a program, whatever the package called it.
        fs::set_permissions(&at, fs::Permissions::from_mode(0o755)).with_context(|| format!("make {} run", at.display()))?;
        written.push(at);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn apk(at: &Path, entries: &[(&str, &[u8])]) {
        let mut file = File::create(at).unwrap();
        let mut zip = zip::ZipWriter::new(&mut file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn the_phoness_own_architecture_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("magisk.apk");
        apk(
            &at,
            &[
                ("lib/armeabi-v7a/libmagiskboot.so", b"32"),
                ("lib/arm64-v8a/libmagiskboot.so", b"64"),
            ],
        );
        let mut zip = ZipArchive::new(File::open(&at).unwrap()).unwrap();
        assert_eq!(entry(&mut zip, "libmagiskboot.so").as_deref(), Some("lib/arm64-v8a/libmagiskboot.so"));
    }

    #[test]
    fn unpacking_names_the_tools_as_the_patcher_looks_for_them() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("magisk.apk");
        let names: Vec<(&str, &[u8])> = TOOLS.iter().map(|(suffix, _)| (*suffix, b"tool".as_slice())).collect();
        apk(&at, &names);
        let into = dir.path().join("env");
        let written = unpack_magisk(&at, &into).unwrap();
        assert_eq!(written.len(), TOOLS.len());
        for (_, name) in TOOLS {
            let at = into.join(name);
            assert!(at.is_file(), "{name} is there");
            assert!(at.metadata().unwrap().permissions().mode() & 0o111 != 0, "{name} runs");
        }
    }

    #[test]
    fn a_package_without_the_tools_says_which_it_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("magisk.apk");
        apk(&at, &[("lib/arm64-v8a/libmagiskboot.so", b"64")]);
        let said = unpack_magisk(&at, &dir.path().join("env")).unwrap_err().to_string();
        assert!(said.contains("libmagiskinit.so"), "{said}");
    }
}
}

pub mod cmd {
//! Running a manager's own patcher, and what it says when it fails.

use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Run `program` with `args` in `dir`, with `env` set besides what it inherits,
/// and give back everything it wrote. A patcher that fails is read from what
/// it wrote, which is what says why.
pub fn run(program: &Path, args: &[&OsStr], dir: &Path, env: &[(&str, String)]) -> Result<String> {
    let done = Command::new(program)
        .args(args)
        .current_dir(dir)
        .envs(env.iter().map(|(key, value)| (key, value.as_str())))
        .output()
        .with_context(|| format!("run {}", program.display()))?;
    let mut said = String::from_utf8_lossy(&done.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&done.stderr));
    if !done.status.success() {
        bail!("{} {}: {said}", program.display(), args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    }
    Ok(said)
}
}

pub mod detect {
//! Which root manager the phone has, by what it leaves under the data
//! partition, and where its own patcher is.

use crate::{apatch::Apatch, ksu::KernelSu, magisk::Magisk};
use std::path::{Path, PathBuf};

/// Where managers keep what they need, on the data partition.
pub const ADB: &str = "/data/adb";

/// A root manager the phone has, with the directory holding its tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manager {
    Magisk(Magisk),
    KernelSu(KernelSu),
    Apatch(Apatch),
}

impl Manager {
    pub fn name(&self) -> &'static str {
        match self {
            Manager::Magisk(_) => "Magisk",
            Manager::KernelSu(_) => "KernelSU",
            Manager::Apatch(_) => "APatch",
        }
    }

    /// The version it says it is, where it says one.
    pub fn version(&self) -> Option<&str> {
        match self {
            Manager::Magisk(m) => m.version.as_deref(),
            Manager::KernelSu(k) => k.version.as_deref(),
            Manager::Apatch(a) => a.version.as_deref(),
        }
    }

    /// The directory its patcher runs in.
    pub fn dir(&self) -> &Path {
        match self {
            Manager::Magisk(m) => &m.dir,
            // `ksud` is a file; its directory is where it runs.
            Manager::KernelSu(k) => k.ksud.parent().unwrap_or(Path::new("/")),
            Manager::Apatch(a) => &a.dir,
        }
    }
}

/// Files a manager leaves where [`detect_in`] looks, which is also what it
/// needs to patch.
fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// The manager installed under `dir`, or none: Magisk keeps `boot_patch.sh`
/// and `magiskboot` in a directory of its own, KernelSU keeps `ksud` at the
/// top, and APatch keeps `boot_patch.sh` and `bin/kp` in one of its own.
pub fn detect_in(dir: &Path) -> Option<Manager> {
    // Magisk keeps its tools in a folder of its own, and its patcher with
    // them.
    let magisk = dir.join("magisk");
    if is_file(&magisk.join("magiskboot")) {
        return Some(Manager::Magisk(Magisk { dir: magisk, version: None }));
    }
    // KernelSU keeps `ksud` at the top as a link to its own folder, where its
    // tools are.
    let ksud = {
        let in_folder = dir.join("ksu/bin/ksud");
        if is_file(&in_folder) {
            in_folder
        } else {
            dir.join("ksud")
        }
    };
    if is_file(&ksud) {
        return Some(Manager::KernelSu(KernelSu { ksud, version: None }));
    }
    // APatch keeps its patcher, and the `kp` it runs, in a folder of its own.
    let apatch = dir.join("ap");
    if is_file(&apatch.join("boot_patch.sh")) || is_file(&apatch.join("bin/kp")) || is_file(&dir.join("apd")) {
        return Some(Manager::Apatch(Apatch { dir: apatch, version: None }));
    }
    None
}

/// The manager the phone runs, if it has one.
pub fn detect() -> Option<Manager> {
    detect_in(Path::new(ADB))
}

/// Every manager under `dir`: a phone keeps one, but a page shows what it
/// finds rather than assuming which.
pub fn all_in(dir: &Path) -> Vec<Manager> {
    let mut found = Vec::new();
    let magisk = dir.join("magisk");
    if magisk.join("magiskboot").is_file() {
        found.push(Manager::Magisk(Magisk { dir: magisk, version: None }));
    }
    let ksud = {
        let in_folder = dir.join("ksu/bin/ksud");
        if in_folder.is_file() {
            in_folder
        } else {
            dir.join("ksud")
        }
    };
    if ksud.is_file() {
        found.push(Manager::KernelSu(KernelSu { ksud, version: None }));
    }
    let apatch = dir.join("ap");
    if apatch.join("boot_patch.sh").is_file() || apatch.join("bin/kp").is_file() || dir.join("apd").is_file() {
        found.push(Manager::Apatch(Apatch { dir: apatch, version: None }));
    }
    found
}

/// Every manager on the phone.
pub fn all() -> Vec<Manager> {
    all_in(Path::new(ADB))
}

/// Where a manager's directory is, for a phone whose data partition is open.
pub fn adb_dir() -> PathBuf {
    PathBuf::from(ADB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn each_manager_is_known_by_the_patcher_it_keeps() {
        let dir = tempfile::tempdir().unwrap();
        let adb = dir.path();
        assert_eq!(detect_in(adb), None, "a phone with no manager");

        touch(&adb.join("magisk/boot_patch.sh"));
        touch(&adb.join("magisk/magiskboot"));
        assert!(matches!(detect_in(adb), Some(Manager::Magisk(_))));

        let other = tempfile::tempdir().unwrap();
        touch(&other.path().join("ksud"));
        assert!(matches!(detect_in(other.path()), Some(Manager::KernelSu(_))));

        let third = tempfile::tempdir().unwrap();
        touch(&third.path().join("ap/boot_patch.sh"));
        assert!(matches!(detect_in(third.path()), Some(Manager::Apatch(_))));
    }

    #[test]
    fn the_patcher_is_what_a_manager_needs_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("magisk")).unwrap();
        assert_eq!(detect_in(dir.path()), None, "an empty directory of a manager");
    }
}
}

pub mod installed {
//! The root managers installed as apps: each is a package under the data
//! partition, and its `base.apk` carries the patcher of the manager that made
//! it. Looking for them is how a package on the phone is offered without the
//! user having to find it.

use crate::apk::{self, Kind};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Where the packages installed for the user are kept.
pub const APPS: &str = "/data/app";

/// The packages the managers are installed as. A folder named after one of
/// them holds it, with the hash the system puts after it.
const KNOWN: [&str; 8] = [
    "com.topjohnwu.magisk",
    "me.weishu.kernelsu",
    "com.rifsxd.ksunext",
    "com.sukisu.ultra",
    "io.github.sukisu.ultra",
    "com.baka.su",
    "me.bmax.apatch",
    "com.kernelsu.next",
];

/// A manager installed as an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// The package's own name, as Android knows it.
    pub package: String,
    /// The package installed, `base.apk`.
    pub apk: PathBuf,
    /// Which manager it is, by the library its package carries.
    pub kind: Kind,
}

impl App {
    /// What a page calls it: the manager it is, and the package.
    pub fn label(&self) -> String {
        format!("{} — {}", self.kind.name(), self.package)
    }
}

/// The managers installed under `root`, which is the data partition's folder.
pub fn installed_in(root: &Path) -> Result<Vec<App>> {
    let apps = root.join("app");
    let entries = match fs::read_dir(&apps) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", apps.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", apps.display()))?;
        let at = entry.path();
        if !at.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // A package sits in a folder named after it, or in one of the
        // `~~<hash>` folders the system groups them in.
        if name.starts_with("~~") {
            collect(&at, &mut found)?;
        } else if let Some(package) = package_of(&name) {
            if let Some(app) = app_of(&at, package) {
                found.push(app);
            }
        }
    }
    found.sort_by(|a, b| a.package.cmp(&b.package));
    found.dedup_by(|a, b| a.package == b.package);
    Ok(found)
}

/// The managers installed under `/data`.
pub fn installed() -> Result<Vec<App>> {
    installed_in(Path::new("/data"))
}

fn collect(dir: &Path, found: &mut Vec<App>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        let at = entry.path();
        if !at.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(package) = package_of(&name) {
            if let Some(app) = app_of(&at, package) {
                found.push(app);
            }
        }
    }
    Ok(())
}

/// The package a folder is named after: `com.topjohnwu.magisk-<hash>`, or the
/// name with the hash after a dash.
fn package_of(folder: &str) -> Option<&str> {
    // The folder is the package's name with the hash the system keeps after a
    // dash, and the hash holds dashes of its own, so the name is matched from
    // its start rather than split off.
    let lowered = folder.to_ascii_lowercase();
    KNOWN.iter().find(|known| lowered.starts_with(*known)).copied()
}

/// The app a package's folder holds, where its own package carries the library
/// of a root manager: an app that ships no such library is not one, whatever
/// it is called.
fn app_of(at: &Path, package: &str) -> Option<App> {
    let apk = at.join("base.apk");
    if !apk.is_file() {
        return None;
    }
    let kind = apk::kind(&apk).ok()?;
    Some(App { package: package.to_string(), apk, kind })
}

/// Why the managers installed cannot be read, if they cannot: a page says so
/// rather than showing a list with none of them in it.
pub fn unreadable() -> Option<String> {
    installed().err().map(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An app installed as Android installs one: a folder named for the
    /// package, holding `base.apk`, which carries `marker` as a library.
    fn app_at(root: &Path, folder: &str, marker: Option<&str>) {
        let at = root.join("app").join(folder);
        fs::create_dir_all(&at).unwrap();
        let file = fs::File::create(at.join("base.apk")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        if let Some(marker) = marker {
            zip.start_file(format!("lib/arm64-v8a/{marker}"), options).unwrap();
            zip.write_all(b"so").unwrap();
        } else {
            zip.start_file("classes.dex", options).unwrap();
            zip.write_all(b"dex").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_manager_is_an_app_whose_package_carries_its_library() {
        let root = tempfile::tempdir().unwrap();
        app_at(root.path(), "com.topjohnwu.magisk-abc==", Some("libmagiskboot.so"));
        app_at(root.path(), "com.example.other-def==", None);
        let found = installed_in(root.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].package, "com.topjohnwu.magisk");
        assert_eq!(found[0].kind, Kind::Magisk);
    }

    #[test]
    fn the_folders_the_system_groups_them_in_are_looked_into() {
        let root = tempfile::tempdir().unwrap();
        let grouped = root.path().join("app").join("~~a1b2c3");
        fs::create_dir_all(grouped.join("me.weishu.kernelsu-xyz==")).unwrap();
        let file = fs::File::create(grouped.join("me.weishu.kernelsu-xyz==/base.apk")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("lib/arm64-v8a/libksud.so", options).unwrap();
        zip.write_all(b"so").unwrap();
        zip.finish().unwrap();
        let found = installed_in(root.path()).unwrap();
        assert_eq!(found[0].package, "me.weishu.kernelsu");
    }

    #[test]
    fn an_app_named_after_a_manager_but_carrying_nothing_is_not_one() {
        let root = tempfile::tempdir().unwrap();
        app_at(root.path(), "com.topjohnwu.magisk-abc==", None);
        assert!(installed_in(root.path()).unwrap().is_empty());
    }
}
}

pub mod ksu {
//! Patching a boot image the way KernelSU does, with the `ksud` the manager
//! installs: it takes the stock image and writes the one it patches.

use crate::{cmd, magisk::move_into};
use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The `ksud` a KernelSU of any name — KernelSU, SukiSU, BakaSU — keeps on the
/// data partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelSu {
    pub ksud: PathBuf,
    pub version: Option<String>,
}

impl KernelSu {
    /// Patch `stock` into `out`, as the manager's own installer would.
    pub fn patch(&self, stock: &Path, out: &Path) -> Result<()> {
        let dir = out.parent().context("a place to patch in")?;
        let name = "new-boot.img";
        let args = [
            OsStr::new("boot-patch"),
            OsStr::new("--boot"),
            stock.as_os_str(),
            OsStr::new("--out-name"),
            OsStr::new(name),
        ];
        cmd::run(&self.ksud, &args, dir, &[]).context("run ksud's boot patcher")?;
        let made = dir.join(name);
        if !made.is_file() {
            bail!("ksud made no {name} in {}", dir.display());
        }
        move_into(&made, out)
    }
}
}

pub mod magisk {
//! Patching a boot image the way Magisk does: its own `boot_patch.sh`, run in
//! the directory holding its tools, over the stock image.

use crate::{apk, cmd, Options};
use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// The tools Magisk patches with, and where they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magisk {
    /// The directory holding `boot_patch.sh` and the tools it runs.
    pub dir: PathBuf,
    pub version: Option<String>,
}

impl Magisk {
    /// The Magisk APK at `apk`, unpacked into `into`, as a patcher: what a
    /// phone with no Magisk installed still needs to run one.
    pub fn from_apk(apk: &Path, into: &Path) -> Result<Magisk> {
        apk::unpack_magisk(apk, into)?;
        Ok(Magisk { dir: into.to_path_buf(), version: None })
    }

    /// Patch `stock` into `out`.
    pub fn patch(&self, stock: &Path, out: &Path, options: &Options) -> Result<()> {
        run_boot_patch(&self.dir, stock, out, options)
    }
}

/// Run the `boot_patch.sh` in `dir` over `stock`, leaving the patched image at
/// `out`: `dir` holds the patcher, the tools it runs, and the stub app it
/// embeds, and the image it makes is `new-boot.img` beside them.
pub fn run_boot_patch(dir: &Path, stock: &Path, out: &Path, options: &Options) -> Result<()> {
    let script = dir.join("boot_patch.sh");
    if !script.is_file() {
        bail!("{} holds no boot_patch.sh", dir.display());
    }
    // The patcher wants a shell it can count on; the package's own busybox is
    // one, and the recovery's shell is the next best.
    let busybox = dir.join("busybox");
    let (program, mut args): (PathBuf, Vec<&OsStr>) = if busybox.is_file() {
        (busybox, vec![OsStr::new("sh"), OsStr::new("boot_patch.sh")])
    } else {
        (PathBuf::from("/system/bin/sh"), vec![OsStr::new("boot_patch.sh")])
    };
    args.push(stock.as_os_str());
    let env = [
        ("KEEPVERITY", options.keep_verity),
        ("KEEPFORCEENCRYPT", options.keep_forceencrypt),
        ("PATCHVBMETAFLAG", options.patch_vbmeta),
        ("RECOVERYMODE", false),
        ("LEGACYSAR", false),
    ]
    .map(|(key, value)| (key, value.to_string()));
    cmd::run(&program, &args, dir, &env).context("run the boot patcher")?;
    let made = dir.join("new-boot.img");
    if !made.is_file() {
        bail!("the boot patcher made no new-boot.img in {}", dir.display());
    }
    move_into(&made, out)
}

/// Move `from` to `to`, copying where the two are on different filesystems.
pub fn move_into(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).with_context(|| format!("write {}", to.display()))?;
    fs::remove_file(from).with_context(|| format!("remove {}", from.display()))
}
}
