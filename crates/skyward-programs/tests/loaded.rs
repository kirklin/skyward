//! The libraries a program asks for that its loader finds in none of its
//! own folders, read from the program and the libraries in tests/loaded,
//! which the NDK's compiler linked as tests/loaded/build.sh says, laid out
//! as a phone has its own.

use skyward_programs::loaded::{self, Beyond};
use std::path::{Path, PathBuf};

fn linked(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/loaded").join(name)
}

/// Put the linked file `name` in `folder`.
fn put(folder: &Path, name: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::copy(linked(name), folder.join(name)).unwrap();
}

/// A recovery's folders under `root`: the program on the odm partition,
/// the library of the recovery's own it asks for with the one that names
/// beside it, and the installed system's, where one library is a link to
/// a file a recovery has none of and is there itself a folder further in.
struct Laid {
    program: PathBuf,
    own: PathBuf,
    system: PathBuf,
    bootstrap: PathBuf,
}

fn laid(root: &Path) -> Laid {
    let laid = Laid {
        program: root.join("odm/bin/hw/program"),
        own: root.join("system/lib64"),
        system: root.join("mnt/system/system/lib64"),
        bootstrap: root.join("mnt/system/system/lib64/bootstrap"),
    };
    put(laid.program.parent().unwrap(), "program");
    put(&laid.own, "libown.so");
    put(&laid.own.join("more"), "libmore.so");
    put(&laid.system, "libdeep.so");
    put(&laid.system, "libshared.so");
    put(&laid.bootstrap, "libbootstrap.so");
    std::os::unix::fs::symlink("/apex/com.android.runtime/lib64/bionic/libbootstrap.so", laid.system.join("libbootstrap.so")).unwrap();
    laid
}

/// The program asks for one library of the recovery's own, one only the
/// installed system has and one no folder has. The first asks for one in
/// the folder it names beside itself and one only the installed system
/// has; the second for the one that is a link to nothing where it is
/// looked for first, for one in a folder it names beside where it is to be
/// put, and for the loader itself.
#[test]
fn what_the_loader_finds_in_none_of_its_folders_is_found_in_the_others() {
    let root = tempfile::tempdir().unwrap();
    let laid = laid(root.path());
    put(&laid.own.join("extra/lib64"), "libextra.so");
    let loader = [laid.own.clone(), root.path().join("odm/lib64"), root.path().join("vendor/lib64")];
    let others = [laid.own.clone(), laid.system.clone(), laid.bootstrap.clone()];
    let found = loaded::beyond(&laid.program, &loader, &others, &laid.own).unwrap();
    let expected = Beyond { elsewhere: vec![laid.system.join("libshared.so"), laid.system.join("libdeep.so"), laid.bootstrap.join("libbootstrap.so")], nowhere: vec!["libgone.so".into()] };
    assert_eq!(found, expected);
}

/// Once what was found elsewhere is in the loader's own folder, the loader
/// finds all of it there.
#[test]
fn what_is_put_in_the_loader_s_folder_is_found_there() {
    let root = tempfile::tempdir().unwrap();
    let laid = laid(root.path());
    put(&laid.own.join("extra/lib64"), "libextra.so");
    let loader = [laid.own.clone()];
    let others = [laid.system.clone(), laid.bootstrap.clone()];
    for file in loaded::beyond(&laid.program, &loader, &others, &laid.own).unwrap().elsewhere {
        std::fs::copy(&file, laid.own.join(file.file_name().unwrap())).unwrap();
    }
    assert_eq!(loaded::beyond(&laid.program, &loader, &others, &laid.own).unwrap(), Beyond { elsewhere: Vec::new(), nowhere: vec!["libgone.so".into()] });
}

/// A library found elsewhere names its folders from where it is to be
/// put: one beside where it is found is not looked in.
#[test]
fn a_library_found_elsewhere_names_its_folders_from_where_it_is_put() {
    let root = tempfile::tempdir().unwrap();
    let laid = laid(root.path());
    put(&laid.system.join("extra/lib64"), "libextra.so");
    let found = loaded::beyond(&laid.program, std::slice::from_ref(&laid.own), &[laid.system.clone(), laid.bootstrap.clone()], &laid.own).unwrap();
    assert_eq!(found.nowhere, ["libgone.so", "libextra.so"]);
}

/// A file that is no program is refused by its name.
#[test]
fn a_file_that_is_no_program_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("program");
    std::fs::write(&file, "#!/system/bin/sh\n").unwrap();
    let refused = loaded::beyond(&file, &[], &[], root.path()).unwrap_err();
    assert!(format!("{refused:#}").contains(&file.display().to_string()), "{refused:#}");
}
