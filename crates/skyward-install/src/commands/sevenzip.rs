//! 7za as install scripts run it to take files out of the zip they came
//! in: `e` and `x`, which write the files their names pick to standard
//! output with `-so`, or into a folder, `x` keeping the folders the files
//! are in. Names pick files as 7-Zip's do; the archive is read as a zip.

use super::{copy, Failed, Io, CHUNK};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use zip::read::ZipFile;
use zip::ZipArchive;

/// How 7-Zip ends: having done all it was asked, stopped by an error, or
/// refusing its command line.
const DONE: i32 = 0;
const FATAL: i32 = 2;
const USAGE: i32 = 7;

pub(super) fn run(args: &[OsString], io: &mut Io) -> i32 {
    let request = match parse(args) {
        Ok(request) => request,
        Err(e) => {
            let _ = writeln!(io.errors, "7za: {e}");
            return USAGE;
        }
    };
    match extract(&request, io) {
        Ok(()) => DONE,
        Err(e) => {
            let _ = writeln!(io.errors, "7za: {e:#}");
            FATAL
        }
    }
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
struct Request {
    /// `x` keeps the folders a file is in inside the archive; `e` puts each
    /// file straight into the folder it is taken out to.
    folders: bool,
    archive: PathBuf,
    /// No names pick every file.
    names: Vec<Name>,
    to: To,
    existing: Existing,
}

impl Request {
    /// Whether the file or folder at `path`, given as its parts, is taken
    /// out; `folder` when the path is a folder's own.
    fn picks(&self, path: &[&str], folder: bool) -> bool {
        self.names.is_empty() || self.names.iter().any(|name| name.picks(path, folder))
    }
}

/// Where the files go.
#[derive(Debug, PartialEq)]
enum To {
    /// Standard output, one file's data after another's: `-so`.
    Output,
    /// This folder, made where missing: the one `-o` names, or the current
    /// one.
    Folder(PathBuf),
}

/// What becomes of a file already where one is taken out to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Existing {
    /// 7-Zip asks whether to replace it, which no one answers in a script,
    /// so the command stops.
    Ask,
    /// `-y` or `-aoa`.
    Replace,
    /// `-aos`.
    Skip,
}

/// Where a name's last part matches.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Recurse {
    /// Right under the folders the name gives: without `-r`, or with `-r-`.
    Never,
    /// At any depth below them: `-r`.
    Always,
    /// At any depth for a last part with a wildcard, right under for any
    /// other: `-r0`.
    Wildcards,
}

/// Read the command line: the command, the archive and the names, among
/// switches anywhere before a `--`.
fn parse(args: &[OsString]) -> Result<Request, String> {
    let (mut words, mut switches) = (Vec::new(), true);
    let (mut output, mut folder, mut yes, mut overwrite, mut recurse) = (false, None, false, None, Recurse::Never);
    for arg in args {
        let arg = arg.to_str().ok_or_else(|| format!("{} is not UTF-8", arg.to_string_lossy()))?;
        let switch = match arg.strip_prefix('-') {
            Some("-") if switches => {
                switches = false;
                continue;
            }
            Some(switch) if switches && !switch.is_empty() => switch,
            _ => {
                words.push(arg);
                continue;
            }
        };
        let lower = switch.to_ascii_lowercase();
        let bytes = lower.as_bytes();
        match lower.as_str() {
            "so" => output = true,
            "y" => yes = true,
            "aoa" => overwrite = Some(Existing::Replace),
            "aos" => overwrite = Some(Existing::Skip),
            "r" => recurse = Recurse::Always,
            "r-" => recurse = Recurse::Never,
            "r0" => recurse = Recurse::Wildcards,
            "tzip" | "t*" => {}
            // How much is told, and where: progress, the log level, the
            // times, and which stream each kind of message goes to.
            "bd" | "bt" | "bb" | "bb0" | "bb1" | "bb2" | "bb3" => {}
            _ if bytes.len() == 4 && bytes.starts_with(b"bs") && b"oep".contains(&bytes[2]) && b"012".contains(&bytes[3]) => {}
            // How many threads take the files out.
            _ if lower.starts_with("mmt") => {}
            // The folder follows the switch's letter, as it was written.
            _ if lower.starts_with('o') => match &switch[1..] {
                "" => return Err("-o names no folder".into()),
                named => folder = Some(PathBuf::from(named)),
            },
            _ => return Err(format!("unsupported switch -{switch}")),
        }
    }
    let mut words = words.into_iter();
    let folders = match words.next().map(str::to_ascii_lowercase).as_deref() {
        Some("x") => true,
        Some("e") => false,
        Some(other) => return Err(format!("unsupported command {other}; e and x take files out of zip archives")),
        None => return Err("no command; e and x take files out of zip archives".into()),
    };
    let archive = PathBuf::from(words.next().ok_or("no archive named")?);
    let names = words.map(|name| Name::new(name, recurse)).collect::<Result<_, _>>()?;
    let to = match output {
        true => To::Output,
        false => To::Folder(folder.unwrap_or_default()),
    };
    let existing = overwrite.unwrap_or(if yes { Existing::Replace } else { Existing::Ask });
    Ok(Request { folders, archive, names, to, existing })
}

/// A name on the command line, which picks files as 7-Zip's names pick
/// them: the folders it gives match from the top of the archive down, and
/// its last part the next folder or file, or any below them where it
/// recurses. Whatever is in a folder a name picks is picked with it. In
/// each part, `*` stands for any run of characters and `?` for any one;
/// the parts are taken as written, so an empty one, `.` or `..` matches
/// nothing in the archive.
#[derive(Debug, PartialEq)]
struct Name {
    folders: Vec<String>,
    last: String,
    /// Written with a slash at its end, the name picks folders alone.
    folder: bool,
    recursive: bool,
}

impl Name {
    fn new(name: &str, recurse: Recurse) -> Result<Name, String> {
        if name.is_empty() {
            return Err("an empty name".into());
        }
        if name.starts_with('@') {
            return Err(format!("{name}: lists of names are not supported"));
        }
        let (name, folder) = match name.strip_suffix('/') {
            Some(name) => (name, true),
            None => (name, false),
        };
        let mut folders: Vec<String> = name.split('/').map(str::to_string).collect();
        let last = folders.pop().expect("a split gives a part at least");
        let recursive = match recurse {
            Recurse::Never => false,
            Recurse::Always => true,
            Recurse::Wildcards => last.contains(['*', '?']),
        };
        Ok(Name { folders, last, folder, recursive })
    }

    /// Whether this name picks the file or folder at `path`, given as its
    /// parts; `folder` when the path is a folder's own.
    fn picks(&self, path: &[&str], folder: bool) -> bool {
        let depth = self.folders.len();
        if path.len() <= depth || !self.folders.iter().zip(path).all(|(name, part)| fits(name, part)) {
            return false;
        }
        // A part is a folder when more of the path follows it.
        let last = |at: usize| fits(&self.last, path[at]) && (!self.folder || folder || at + 1 < path.len());
        match self.recursive {
            true => (depth..path.len()).any(last),
            false => last(depth),
        }
    }
}

/// The folders and file a path in the archive goes through.
fn parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|part| !part.is_empty()).collect()
}

/// Whether `text` fits `pattern`, where `*` stands for any run of
/// characters and `?` for any one.
fn fits(pattern: &str, text: &str) -> bool {
    let (pattern, text): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut p, mut t) = (0, 0);
    // The last `*` met, and where in the text it stopped standing in.
    let mut star = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                // The `*` stands in for one character more.
                Some((at, from)) => {
                    star = Some((at, from + 1));
                    p = at + 1;
                    t = from + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Take the files the request picks out of its archive, in the order the
/// archive lists them.
fn extract(request: &Request, io: &mut Io) -> Result<()> {
    let archive = &request.archive;
    let file = File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let reader = BufReader::with_capacity(CHUNK, file);
    let mut zip = ZipArchive::new(reader).with_context(|| format!("read {} as a zip archive", archive.display()))?;
    for index in 0..zip.len() {
        let name = zip.name_for_index(index).with_context(|| format!("name file {index} of {}", archive.display()))?.to_string();
        if !request.picks(&parts(&name), name.ends_with('/')) {
            continue;
        }
        let mut entry = zip.by_index(index).with_context(|| format!("open {name} in {}", archive.display()))?;
        match &request.to {
            To::Output if entry.is_dir() => {}
            To::Output => pour(&mut entry, io.output, &name, "standard output")?,
            To::Folder(folder) => take_out(&mut entry, folder, request)?,
        }
    }
    io.output.flush().context("write to standard output")
}

/// Take `entry` out into `folder`, under the folders it is in inside the
/// archive where the request keeps them.
fn take_out<R: Read>(entry: &mut ZipFile<'_, R>, folder: &Path, request: &Request) -> Result<()> {
    let name = entry.name().to_string();
    let inside = entry.enclosed_name().with_context(|| format!("{name} leads out of the folder it would be taken out to"))?;
    let path = match (request.folders, entry.is_dir()) {
        (true, _) => folder.join(&inside),
        (false, true) => return Ok(()),
        (false, false) => folder.join(inside.file_name().with_context(|| format!("{name} names no file"))?),
    };
    if entry.is_dir() {
        return fs::create_dir_all(&path).with_context(|| format!("make {}", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => match request.existing {
            Existing::Ask => bail!("{} already exists; -y or -aoa replaces it, -aos keeps it", path.display()),
            Existing::Skip => return Ok(()),
            Existing::Replace => fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("look at {}", path.display())),
    }
    let mut out = OpenOptions::new().write(true).create_new(true).open(&path).with_context(|| format!("create {}", path.display()))?;
    pour(entry, &mut out, &name, &path.display().to_string())?;
    if let Some(mode) = entry.unix_mode() {
        out.set_permissions(Permissions::from_mode(mode & 0o777)).with_context(|| format!("set the permissions of {}", path.display()))?;
    }
    Ok(())
}

/// Copy the file `name` out of the archive into `to`, which `target`
/// names.
fn pour(entry: &mut dyn Read, to: &mut dyn Write, name: &str, target: &str) -> Result<()> {
    match copy(entry, to) {
        Ok(_) => Ok(()),
        Err(Failed::Reading(e)) => Err(anyhow!(e).context(format!("read {name} from the archive"))),
        Err(Failed::Writing(e)) => Err(anyhow!(e).context(format!("write {name} to {target}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// An entry of a test zip: a folder where it ends in `/`.
    struct Entry<'a> {
        name: &'a str,
        data: &'a [u8],
        method: CompressionMethod,
        mode: Option<u32>,
        large: bool,
    }

    fn stored<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
        Entry { name, data, method: CompressionMethod::Stored, mode: None, large: false }
    }

    fn zip(entries: &[Entry]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for e in entries {
            let mut options = SimpleFileOptions::default().compression_method(e.method).large_file(e.large);
            if let Some(mode) = e.mode {
                options = options.unix_permissions(mode);
            }
            match e.name.strip_suffix('/') {
                Some(folder) => w.add_directory(folder, options).unwrap(),
                None => {
                    w.start_file(e.name, options).unwrap();
                    w.write_all(e.data).unwrap();
                }
            }
        }
        w.finish().unwrap().into_inner()
    }

    /// Bytes that do not repeat, so no method shrinks them.
    fn noise(len: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    /// Run 7za with `args`, the archive's path standing for `{zip}`: how it
    /// ended, what it wrote to standard output, and what it said went
    /// wrong.
    fn seven(archive: &Path, args: &[&str]) -> (i32, Vec<u8>, String) {
        let args: Vec<OsString> = args.iter().map(|a| a.replace("{zip}", &archive.display().to_string()).into()).collect();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let code = run(&args, &mut Io { input: &mut io::empty(), output: &mut output, errors: &mut errors });
        (code, output, String::from_utf8(errors).unwrap())
    }

    fn written(dir: &Path, entries: &[Entry]) -> PathBuf {
        let path = dir.join("rom.zip");
        fs::write(&path, zip(entries)).unwrap();
        path
    }

    #[test]
    fn names_pick_files_as_7_zip_picks_them() {
        // What 7-Zip 26.05 takes out of this archive with `e -so`, by name.
        let listed = ["images/", "images/a.txt", "t.txt", "d.txt/", "d.txt/inner.bin", "x/", "x/images/", "x/images/a.txt"];
        let picked = |names: &[&str], recurse: Recurse| -> Vec<&str> {
            let names: Vec<Name> = names.iter().map(|n| Name::new(n, recurse).unwrap()).collect();
            let request = Request { folders: true, archive: PathBuf::new(), names, to: To::Output, existing: Existing::Ask };
            listed.iter().copied().filter(|path| !path.ends_with('/') && request.picks(&parts(path), false)).collect()
        };
        let none = Vec::<&str>::new();
        let every = ["images/a.txt", "t.txt", "d.txt/inner.bin", "x/images/a.txt"];
        assert_eq!(picked(&["images/a.txt"], Recurse::Never), ["images/a.txt"]);
        assert_eq!(picked(&["images/a.txt"], Recurse::Always), ["images/a.txt"], "the folders a name gives start at the top");
        assert_eq!(picked(&["*.txt"], Recurse::Never), ["t.txt", "d.txt/inner.bin"], "a folder picked brings what is in it");
        assert_eq!(picked(&["*.txt"], Recurse::Always), every);
        assert_eq!(picked(&["images"], Recurse::Always), ["images/a.txt", "x/images/a.txt"]);
        assert_eq!(picked(&["a.txt"], Recurse::Never), none);
        assert_eq!(picked(&["a.txt"], Recurse::Always), ["images/a.txt", "x/images/a.txt"]);
        assert_eq!(picked(&["a.txt"], Recurse::Wildcards), none);
        assert_eq!(picked(&["*.txt"], Recurse::Wildcards), every);
        assert_eq!(picked(&[], Recurse::Never), every, "no names pick every file");
        assert_eq!(picked(&["*"], Recurse::Never), every);
        assert_eq!(picked(&["x/*/a.txt"], Recurse::Never), ["x/images/a.txt"]);
        assert_eq!(picked(&["i*s"], Recurse::Never), ["images/a.txt"]);
        assert_eq!(picked(&["images/A.TXT"], Recurse::Never), none, "case counts");
        assert_eq!(picked(&["t.txt", "images/a.txt"], Recurse::Never), ["images/a.txt", "t.txt"], "in the archive's order");
        // A slash at the end picks folders alone.
        assert_eq!(picked(&["images/"], Recurse::Never), ["images/a.txt"]);
        assert_eq!(picked(&["d.txt/"], Recurse::Never), ["d.txt/inner.bin"]);
        assert_eq!(picked(&["t.txt/"], Recurse::Never), none);
        assert_eq!(picked(&["images/a.txt/"], Recurse::Never), none);
        // Every part counts as written.
        for odd in ["./images/a.txt", "/images/a.txt", "images//a.txt", "images/./a.txt", "images/../images/a.txt", "/", "//", "\\images\\a.txt"] {
            assert_eq!(picked(&[odd], Recurse::Never), none, "{odd}");
            assert_eq!(picked(&[odd], Recurse::Always), none, "{odd}");
        }
    }

    #[test]
    fn wildcards_stand_for_runs_and_single_characters() {
        assert!(fits("super.img.zst", "super.img.zst"));
        assert!(fits("*.zst", "super.img.zst"));
        assert!(fits("s*r*.zst", "super.img.zst"));
        assert!(fits("sup?r.img*", "super.img.zst"));
        assert!(fits("*", ""));
        assert!(fits("**a*", "banana"));
        assert!(!fits("*.zst", "super.img"));
        assert!(!fits("?", ""));
        assert!(!fits("super", "super.img"));
        assert!(!fits("*.*", "README"), "a name without a dot does not fit *.*, as with 7-Zip on Linux");
        assert!(fits("镜像?.img", "镜像一.img"), "a wildcard stands for a character, not a byte");
    }

    #[test]
    fn a_file_goes_to_standard_output_as_the_archive_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let image = noise(CHUNK * 2 + 4321);
        let script = b"#!/sbin/sh\nexit 0\n".repeat(100);
        for (method, large) in [(CompressionMethod::Stored, false), (CompressionMethod::Deflated, false), (CompressionMethod::Stored, true)] {
            let archive = written(
                dir.path(),
                &[
                    Entry { method: CompressionMethod::Deflated, ..stored("META-INF/com/google/android/update-binary", &script) },
                    Entry { method, large, ..stored("images/super.img.zst", &image) },
                ],
            );
            let (code, output, errors) = seven(&archive, &["e", "-so", "{zip}", "images/super.img.zst"]);
            assert_eq!((code, errors.as_str()), (DONE, ""), "{method:?}, zip64 {large}");
            assert!(output == image, "{method:?}, zip64 {large}");
        }
    }

    #[test]
    fn the_files_picked_go_out_one_after_another_in_the_archive_s_order() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("images/", b""), stored("images/b.img", b"bbb"), stored("images/a.img", b"aa")]);
        let (code, output, _) = seven(&archive, &["e", "-so", "{zip}", "images/a.img", "images/b.img"]);
        assert_eq!((code, output.as_slice()), (DONE, b"bbbaa".as_slice()));
        let (code, output, _) = seven(&archive, &["x", "-so", "{zip}"]);
        assert_eq!((code, output.as_slice()), (DONE, b"bbbaa".as_slice()), "a folder brings no data of its own");
    }

    #[test]
    fn a_name_that_picks_nothing_takes_nothing_out_and_ends_well() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("super.img.zst", b"data")]);
        assert_eq!(seven(&archive, &["e", "-so", "{zip}", "images/super.img.zst"]), (DONE, Vec::new(), String::new()));
    }

    #[test]
    fn x_keeps_the_folders_files_are_in_and_e_puts_them_straight_into_one() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(
            dir.path(),
            &[
                stored("bin/", b""),
                Entry { mode: Some(0o755), ..stored("bin/zstd", b"\x7fELF") },
                stored("images/", b""),
                stored("images/boot.img", b"ANDROID!"),
                stored("empty/", b""),
            ],
        );
        let kept = dir.path().join("kept");
        let switch = format!("-o{}", kept.display());
        assert_eq!(seven(&archive, &["x", "{zip}", &switch]).0, DONE);
        assert_eq!(fs::read(kept.join("bin/zstd")).unwrap(), b"\x7fELF");
        assert_eq!(fs::metadata(kept.join("bin/zstd")).unwrap().permissions().mode() & 0o777, 0o755, "its permissions come with it");
        assert_eq!(fs::read(kept.join("images/boot.img")).unwrap(), b"ANDROID!");
        assert!(kept.join("empty").is_dir(), "a folder is made though nothing is in it");
        let flat = dir.path().join("flat");
        let switch = format!("-o{}", flat.display());
        assert_eq!(seven(&archive, &["e", "{zip}", &switch, "bin/zstd", "images/*"]).0, DONE);
        let mut names: Vec<_> = fs::read_dir(&flat).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["boot.img", "zstd"]);
    }

    #[test]
    fn a_file_already_there_stops_the_command_unless_a_switch_says_what_becomes_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("images/boot.img", b"new")]);
        let out = dir.path().join("out");
        let switch = format!("-o{}", out.display());
        let boot = out.join("images/boot.img");
        fs::create_dir_all(boot.parent().unwrap()).unwrap();
        fs::write(&boot, b"old").unwrap();
        let (code, _, errors) = seven(&archive, &["x", "{zip}", &switch]);
        assert_eq!(code, FATAL);
        assert!(errors.contains("already exists"), "{errors}");
        assert_eq!(fs::read(&boot).unwrap(), b"old");
        assert_eq!(seven(&archive, &["x", "{zip}", &switch, "-aos"]).0, DONE);
        assert_eq!(fs::read(&boot).unwrap(), b"old");
        assert_eq!(seven(&archive, &["x", "{zip}", &switch, "-y"]).0, DONE);
        assert_eq!(fs::read(&boot).unwrap(), b"new");
        fs::write(&boot, b"old").unwrap();
        assert_eq!(seven(&archive, &["x", "{zip}", &switch, "-aoa"]).0, DONE);
        assert_eq!(fs::read(&boot).unwrap(), b"new");
        fs::write(&boot, b"old").unwrap();
        assert_eq!(seven(&archive, &["x", "{zip}", &switch, "-y", "-aos"]).0, DONE, "-aos says it, whatever -y says");
        assert_eq!(fs::read(&boot).unwrap(), b"old");
    }

    #[test]
    fn data_that_does_not_match_its_checksum_stops_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let image = noise(100_000);
        let mut bytes = zip(&[stored("images/super.img.zst", &image)]);
        let start = ZipArchive::new(Cursor::new(&bytes)).unwrap().by_index(0).unwrap().data_start().unwrap() as usize;
        bytes[start + 50_000] ^= 0xff;
        let archive = dir.path().join("damaged.zip");
        fs::write(&archive, &bytes).unwrap();
        let (code, _, errors) = seven(&archive, &["e", "-so", "{zip}", "images/super.img.zst"]);
        assert_eq!(code, FATAL);
        assert!(errors.contains("images/super.img.zst"), "{errors}");
    }

    #[test]
    fn an_archive_missing_or_not_a_zip_stops_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let (code, _, errors) = seven(&dir.path().join("none.zip"), &["e", "-so", "{zip}", "x"]);
        assert_eq!(code, FATAL);
        assert!(errors.contains("none.zip"), "{errors}");
        let text = dir.path().join("text.zip");
        fs::write(&text, b"not a zip").unwrap();
        assert_eq!(seven(&text, &["e", "-so", "{zip}", "x"]).0, FATAL);
    }

    #[test]
    fn a_file_that_would_land_outside_the_folder_stops_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("../escaped", b"out")]);
        let switch = format!("-o{}", dir.path().join("in").display());
        let (code, _, errors) = seven(&archive, &["x", "{zip}", &switch]);
        assert_eq!(code, FATAL);
        assert!(errors.contains("leads out"), "{errors}");
        assert!(!dir.path().join("escaped").exists());
        let (code, output, _) = seven(&archive, &["e", "-so", "{zip}"]);
        assert_eq!((code, output.as_slice()), (DONE, b"out".as_slice()), "standard output is no folder to leave");
    }

    #[test]
    fn switches_go_anywhere_before_two_dashes_in_either_case() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("images/super.img.zst", b"zst"), stored("-odd", b"odd")]);
        let args = ["-bso0", "E", "-bd", "{zip}", "-SO", "images/super.img.zst", "-bsp1", "-mmt4", "-bb0", "-y", "-tzip"];
        assert_eq!(seven(&archive, &args), (DONE, b"zst".to_vec(), String::new()));
        let (code, output, _) = seven(&archive, &["e", "-so", "{zip}", "--", "-odd"]);
        assert_eq!((code, output.as_slice()), (DONE, b"odd".as_slice()), "after -- a dash starts a name");
    }

    #[test]
    fn commands_and_switches_it_does_not_have_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), &[stored("a", b"a")]);
        for args in [
            &[][..],
            &["e"],
            &["l", "{zip}"],
            &["a", "{zip}", "file"],
            &["e", "-p1234", "{zip}"],
            &["e", "-x!a", "{zip}"],
            &["e", "-aou", "{zip}"],
            &["e", "-t7z", "{zip}"],
            &["e", "-o", "{zip}"],
            &["e", "{zip}", "@names.txt"],
            &["e", "{zip}", ""],
        ] {
            let (code, output, errors) = seven(&archive, args);
            assert_eq!((code, output.as_slice()), (USAGE, b"".as_slice()), "{args:?}");
            assert!(errors.starts_with("7za: "), "{args:?}: {errors}");
        }
    }
}
