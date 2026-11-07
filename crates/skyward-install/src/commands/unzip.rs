//! unzip as the stock recovery carries it, AOSP's ziptool, which install
//! scripts run to take files out of the zip they came in: its options, its
//! glob patterns, its messages and its statuses. What `-p` takes out goes to
//! standard output as it is read, where ziptool first holds the whole file
//! in memory, as much as a super image takes; and every file's data is
//! checked against its checksum as it is read, which ziptool does for `-t`
//! alone.

use super::{copy, Failed, Io, CHUNK};
use flate2::Crc;
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsString};
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::read::ZipFile;
use zip::{CompressionMethod, ZipArchive};

/// How ziptool ends: well, stopped by an error, or having tested files
/// whose data does not match their checksums.
const DONE: i32 = 0;
const FAILED: i32 = 1;
const BAD_CRC: i32 = 2;

const USAGE: &str = "usage: unzip [-d DIR] [-jlnopqtv] ZIP [FILE...] [-x FILE...]";

const HELP: &str = "\
usage: unzip [-d DIR] [-jlnopqtv] ZIP [FILE...] [-x FILE...]

Takes FILEs out of the ZIP archive, every file when none is named. FILEs and
the names after -x are shell glob patterns, where * and ? stand for slashes too.

-d DIR  take the files out into DIR, made if missing
-j      leave out the folders the files are in
-l      list the files (-lq without the archive's name, -lv in full)
-n      never replace a file already there (the default is to ask)
-o      always replace a file already there
-p      write the files to standard output as they are read
-q      tell less
-t      check the files' data, taking nothing out
-v      list the files in full
-x      leave out the files the names after it match
-Z      run zipinfo with the arguments that follow it
";

/// The program `unzip -Z` becomes, given the arguments after `-Z`.
const ZIPINFO: &str = "zipinfo";

/// The modes of what an archive gives none for: a file, and a folder or a
/// folder a file is in.
const FILE_MODE: u32 = 0o644;
const FOLDER_MODE: u32 = 0o755;
const PARENT_MODE: u32 = 0o777;

pub(super) fn run(args: &[OsString], io: &mut Io) -> i32 {
    match parse(args) {
        Ok(Asked::Unzip(request)) => match unzip(&request, io) {
            Ok(status) => status,
            Err(e) => {
                // What was told before the error comes before it.
                let _ = io.output.flush();
                let _ = writeln!(io.errors, "unzip: {e}");
                FAILED
            }
        },
        Ok(Asked::Help) => match io.output.write_all(HELP.as_bytes()).and_then(|()| io.output.flush()) {
            Ok(()) => DONE,
            Err(_) => FAILED,
        },
        Ok(Asked::Zipinfo(args)) => {
            let e = zipinfo(&args).exec();
            let _ = writeln!(io.errors, "unzip: run {ZIPINFO}: {e}");
            FAILED
        }
        Err(e) => {
            let _ = writeln!(io.errors, "unzip: {e}");
            FAILED
        }
    }
}

/// The stock zipinfo, which `unzip -Z` hands the arguments after `-Z` to,
/// as ziptool does.
fn zipinfo(args: &[OsString]) -> Command {
    let mut command = Command::new(ZIPINFO);
    command.args(args);
    command
}

/// What the command line asks of unzip.
#[derive(Debug, PartialEq)]
enum Asked {
    Unzip(Request),
    Help,
    Zipinfo(Vec<OsString>),
}

#[derive(Debug, PartialEq)]
struct Request {
    archive: PathBuf,
    /// The patterns that pick files; none picks every file.
    includes: BTreeSet<CString>,
    /// The patterns that leave files out whatever picks them.
    excludes: BTreeSet<CString>,
    /// The folder `-d` names, with a slash at its end as the messages show
    /// it.
    folder: Option<OsString>,
    junk: bool,
    overwrite: Overwrite,
    quiet: bool,
    // What to do, as ziptool picks: testing over listing, listing over
    // writing to standard output, any of them over taking files out.
    test: bool,
    list: bool,
    verbose: bool,
    pipe: bool,
}

impl Request {
    /// Whether the file `name` is one to work on.
    fn takes(&self, name: &str) -> bool {
        let Ok(name) = CString::new(name) else { return self.includes.is_empty() };
        !self.excludes.iter().any(|pattern| matches(pattern, &name))
            && (self.includes.is_empty() || self.includes.iter().any(|pattern| matches(pattern, &name)))
    }
}

/// What becomes of a file already where one is taken out to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Overwrite {
    /// Ask, reading the answer from standard input.
    Ask,
    /// `-o`.
    Always,
    /// `-n`.
    Never,
}

/// Whether `name` fits the shell glob `pattern` as ziptool matches it: by
/// the C library's fnmatch with no flags, so `*` and `?` stand for slashes
/// too and a backslash takes the next character as it is.
fn matches(pattern: &CStr, name: &CStr) -> bool {
    unsafe { libc::fnmatch(pattern.as_ptr(), name.as_ptr(), 0) == 0 }
}

/// Read the command line as ziptool reads it: options one letter each,
/// grouped behind one dash or not, among the archive and the names, which
/// are names to leave out once a `-x` has come; `-Z` first hands the rest
/// to zipinfo.
fn parse(args: &[OsString]) -> Result<Asked, String> {
    if args.first().is_some_and(|first| first == "-Z") {
        return Ok(Asked::Zipinfo(args[1..].to_vec()));
    }
    let (mut archive, mut includes, mut excludes) = (None, BTreeSet::new(), BTreeSet::new());
    let (mut folder, mut junk, mut overwrite, mut quiet) = (None, false, Overwrite::Ask, false);
    let (mut test, mut list, mut verbose, mut pipe) = (false, false, false, false);
    let (mut leaving_out, mut options) = (false, true);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let bytes = arg.as_bytes();
        if options && bytes == b"--" {
            options = false;
            continue;
        }
        if options && bytes.starts_with(b"--") {
            return match bytes {
                b"--help" => Ok(Asked::Help),
                _ => Err(format!("unrecognized option '{}'\n{USAGE}", arg.to_string_lossy())),
            };
        }
        if options && bytes.len() > 1 && bytes[0] == b'-' {
            for (at, &letter) in bytes.iter().enumerate().skip(1) {
                match letter {
                    // Taken by the original unzip; ziptool sets no times anyway.
                    b'D' => {}
                    // The folder is the rest of the group, or the next argument.
                    b'd' => {
                        let mut named = match &bytes[at + 1..] {
                            [] => args.next().ok_or_else(|| format!("option requires an argument -- d\n{USAGE}"))?.as_bytes().to_vec(),
                            rest => rest.to_vec(),
                        };
                        if !named.ends_with(b"/") {
                            named.push(b'/');
                        }
                        folder = Some(OsString::from_vec(named));
                        break;
                    }
                    b'h' => return Ok(Asked::Help),
                    b'j' => junk = true,
                    b'l' => list = true,
                    b'n' => overwrite = Overwrite::Never,
                    b'o' => overwrite = Overwrite::Always,
                    b'p' => (pipe, quiet) = (true, true),
                    b'q' => quiet = true,
                    b't' => test = true,
                    b'v' => verbose = true,
                    b'x' => leaving_out = true,
                    _ => {
                        let letter = String::from_utf8_lossy(&bytes[at..]).chars().next().expect("a letter follows the dash");
                        return Err(format!("invalid option -- {letter}\n{USAGE}"));
                    }
                }
            }
            continue;
        }
        if archive.is_none() {
            archive = Some(PathBuf::from(arg));
            continue;
        }
        let pattern = CString::new(bytes).expect("an argument holds no NUL");
        match leaving_out {
            true => excludes.insert(pattern),
            false => includes.insert(pattern),
        };
    }
    let archive = archive.ok_or("missing archive filename")?;
    Ok(Asked::Unzip(Request { archive, includes, excludes, folder, junk, overwrite, quiet, test, list, verbose, pipe }))
}

/// How much the files worked on held, and how many there were.
#[derive(Default)]
struct Totals {
    size: u64,
    compressed: u64,
    files: usize,
}

/// Work on the files the request picks, in the order the archive lists
/// them, and give the status unzip ends with.
fn unzip(request: &Request, io: &mut Io) -> Result<i32, String> {
    let shown = request.archive.display();
    let file = File::open(&request.archive).map_err(|e| format!("couldn't open {shown}: {e}"))?;
    let mut zip = ZipArchive::new(BufReader::with_capacity(CHUNK, file)).map_err(|e| format!("couldn't open {shown}: {e}"))?;
    // The folder itself is made, whatever is done; the folders it is in
    // have to be there already, as with the original unzip.
    if let Some(folder) = &request.folder {
        match DirBuilder::new().mode(PARENT_MODE).create(folder) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && Path::new(folder).is_dir() => {}
            Err(e) => return Err(format!("couldn't create {}: {e}", folder.to_string_lossy())),
        }
    }
    if !request.quiet {
        tell(io, &format!("Archive:  {shown}\n"))?;
    }
    if request.verbose {
        tell(io, " Length   Method    Size  Cmpr    Date    Time   CRC-32   Name\n--------  ------  ------- ---- ---------- ----- --------  ----\n")?;
    } else if request.list {
        tell(io, "  Length      Date    Time    Name\n---------  ---------- -----   ----\n")?;
    }
    let (mut totals, mut bad, mut overwrite) = (Totals::default(), 0, request.overwrite);
    for index in 0..zip.len() {
        let name = zip.name_for_index(index).ok_or_else(|| format!("couldn't read file {index} of {shown}"))?.to_string();
        if !request.takes(&name) {
            continue;
        }
        let mut entry = zip.by_index(index).map_err(|e| format!("failed to extract {name}: {e}"))?;
        if request.test {
            bad += usize::from(!test(&mut entry, &name, request.quiet, io)?);
        } else if request.list || request.verbose {
            list(&entry, &name, request.verbose, io)?;
        } else if request.pipe {
            copy(&mut entry, io.output).map_err(|e| match e {
                Failed::Reading(e) => format!("failed to extract {name}: {e}"),
                Failed::Writing(e) => format!("failed to write {name} to stdout: {e}"),
            })?;
        } else {
            take_out(&mut entry, &name, request, &mut overwrite, io)?;
        }
        totals.size += entry.size();
        totals.compressed += entry.compressed_size();
        totals.files += 1;
    }
    footer(request, &totals, bad, io)?;
    io.output.flush().map_err(|e| format!("write to stdout: {e}"))?;
    Ok(if bad > 0 { BAD_CRC } else { DONE })
}

fn tell(io: &mut Io, text: &str) -> Result<(), String> {
    io.output.write_all(text.as_bytes()).map_err(|e| format!("write to stdout: {e}"))
}

/// The share of a file its compression saved, in percent, as ziptool
/// works it out.
fn saved(size: u64, compressed: u64) -> f32 {
    match size {
        0 => 0.0,
        _ => 100.0 * (size as f32 - compressed as f32) / size as f32,
    }
}

/// When a file was last changed, as the archive keeps it, to the minute.
fn changed<R: Read>(entry: &ZipFile<'_, R>) -> String {
    let (year, month, day, hour, minute) = match entry.last_modified() {
        Some(t) => (t.year(), t.month(), t.day(), t.hour(), t.minute()),
        // The earliest time an archive can keep.
        None => (1980, 1, 1, 0, 0),
    };
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// One line of `-l` or `-v`.
fn list<R: Read>(entry: &ZipFile<'_, R>, name: &str, verbose: bool, io: &mut Io) -> Result<(), String> {
    let (size, compressed) = (entry.size(), entry.compressed_size());
    let line = match verbose {
        true => {
            let method = if entry.compression() == CompressionMethod::Stored { "Stored" } else { "Defl:N" };
            let crc = entry.crc32();
            format!("{size:8}  {method} {compressed:8} {:3.0}% {} {crc:08x}  {name}\n", saved(size, compressed), changed(entry))
        }
        false => format!("{size:9}  {}   {name}\n", changed(entry)),
    };
    tell(io, &line)
}

/// What testing writes a file's data to: its checksum, and how much there
/// was.
#[derive(Default)]
struct Checked {
    crc: Crc,
    length: u64,
}

impl Write for Checked {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.crc.update(data);
        self.length += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Check the data of `entry` against its checksum, telling how it went as
/// ziptool tells it, and give whether it matched.
fn test<R: Read>(entry: &mut ZipFile<'_, R>, name: &str, quiet: bool, io: &mut Io) -> Result<bool, String> {
    if !quiet {
        tell(io, &format!("    testing: {name:<24} "))?;
    }
    let mut checked = Checked::default();
    let read = copy(entry, &mut checked);
    let (sum, should) = (checked.crc.sum(), entry.crc32());
    match read {
        Ok(_) => {}
        // The archive's reader finds the checksum wrong once all the data
        // is through it; anything else wrong stops the test.
        Err(Failed::Reading(_)) if checked.length == entry.size() && sum != should => {}
        Err(Failed::Reading(e) | Failed::Writing(e)) => return Err(format!("failed to extract {name}: {e}")),
    }
    if sum == should {
        if !quiet {
            tell(io, "OK\n")?;
        }
        return Ok(true);
    }
    if quiet {
        tell(io, &format!("{name:<23} "))?;
    }
    tell(io, &format!("bad CRC {sum:08x}  (should be {should:08x})\n"))?;
    Ok(false)
}

/// The lines after the files: totals for a listing, a verdict for a test.
fn footer(request: &Request, totals: &Totals, bad: usize, io: &mut Io) -> Result<(), String> {
    let Totals { size, compressed, files } = *totals;
    let plural = if files == 1 { "" } else { "s" };
    let archive = request.archive.display();
    let text = if request.verbose {
        format!(
            "--------          -------  ---                            -------\n{size:8}         {compressed:8} {:3.0}%                            {files} file{plural}\n",
            saved(size, compressed)
        )
    } else if request.list {
        format!("---------                     -------\n{size:9}                     {files} file{plural}\n")
    } else if !request.test {
        return Ok(());
    } else if bad > 0 {
        format!("At least one error was detected in {archive}.\n")
    } else if request.includes.is_empty() && request.excludes.is_empty() {
        format!("No errors detected in compressed data of {archive}.\n")
    } else {
        format!("No errors detected in {archive} for the {files} file{plural} tested.\n")
    };
    tell(io, &text)
}

/// Take `entry` out under its name, into the folder `-d` named or the
/// current one, as ziptool takes it out.
fn take_out<R: Read>(entry: &mut ZipFile<'_, R>, name: &str, request: &Request, overwrite: &mut Overwrite, io: &mut Io) -> Result<(), String> {
    if name.starts_with('/') || name.split('/').any(|part| part == "..") {
        return Err(format!("bad filename {name}"));
    }
    let folder_entry = name.ends_with('/');
    // Without their paths, folders are nothing to take out.
    if request.junk && folder_entry {
        return Ok(());
    }
    let name = match request.junk {
        true => name.rsplit('/').next().expect("a split gives a part at least"),
        false => name,
    };
    let base = request.folder.as_deref().map(Path::new).unwrap_or(Path::new(""));
    let (path, shown) = (base.join(name), format!("{}{name}", base.display()));
    if let Some(parent) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(PARENT_MODE)
            .create(parent)
            .map_err(|e| format!("couldn't create directory hierarchy for {shown}: {e}"))?;
    }
    if folder_entry {
        let mode = entry.unix_mode().unwrap_or(FOLDER_MODE) & 0o777;
        return match DirBuilder::new().mode(mode).create(&path) {
            Err(e) if !(e.kind() == io::ErrorKind::AlreadyExists && path.is_dir()) => Err(format!("couldn't extract directory {shown}: {e}")),
            _ => Ok(()),
        };
    }
    let mode = entry.unix_mode().unwrap_or(FILE_MODE) & 0o777;
    let mut out = match OpenOptions::new().write(true).create_new(true).mode(mode).open(&path) {
        Ok(out) => out,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let replace = match *overwrite {
                Overwrite::Never => false,
                Overwrite::Always => true,
                Overwrite::Ask => ask(&shown, overwrite, io)?,
            };
            if !replace {
                return Ok(());
            }
            OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&path).map_err(|e| format!("couldn't create file {shown}: {e}"))?
        }
        Err(e) => return Err(format!("couldn't create file {shown}: {e}")),
    };
    if !request.quiet {
        tell(io, &format!("  inflating: {shown}\n"))?;
    }
    copy(entry, &mut out).map(drop).map_err(|e| match e {
        Failed::Reading(e) | Failed::Writing(e) => format!("failed to extract {shown}: {e}"),
    })
}

/// Ask whether to replace the file `shown`, as ziptool asks, reading the
/// answer from standard input; an answer for every file sets `overwrite`
/// for the rest. Standard input ending first stops the command.
fn ask(shown: &str, overwrite: &mut Overwrite, io: &mut Io) -> Result<bool, String> {
    tell(io, &format!("replace {shown}? [y]es, [n]o, [A]ll, [N]one: "))?;
    io.output.flush().map_err(|e| format!("write to stdout: {e}"))?;
    loop {
        let mut line = Vec::new();
        let mut byte = [0];
        loop {
            match io.input.read(&mut byte) {
                Ok(0) => break,
                Ok(_) if byte[0] == b'\n' => {
                    line.push(byte[0]);
                    break;
                }
                Ok(_) => line.push(byte[0]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        match line.first() {
            None => return Err("(EOF/read error; assuming [N]one...)".into()),
            Some(b'y') => return Ok(true),
            Some(b'n') => return Ok(false),
            Some(b'A') => {
                *overwrite = Overwrite::Always;
                return Ok(true);
            }
            Some(b'N') => {
                *overwrite = Overwrite::Never;
                return Ok(false);
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use zip::write::SimpleFileOptions;
    use zip::DateTime;

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

    /// A zip of `entries`, all last changed at 20:41 on 6 November 2026.
    fn zip(entries: &[Entry]) -> Vec<u8> {
        let time = DateTime::from_date_and_time(2026, 11, 6, 20, 41, 0).unwrap();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for e in entries {
            let mut options = SimpleFileOptions::default().compression_method(e.method).large_file(e.large).last_modified_time(time);
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

    fn written(dir: &Path, name: &str, entries: &[Entry]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, zip(entries)).unwrap();
        path
    }

    /// Bytes that do not repeat, so no method shrinks them.
    fn noise(len: usize) -> Vec<u8> {
        let mut state = 0xbb67_ae85_84ca_a73bu64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    /// Run unzip with `args`, `{zip}` standing for the archive's path and
    /// `{dir}` for the test's folder, and `input` as standard input: how it
    /// ended, what it wrote to standard output, and what to standard error.
    fn ran(args: &[&str], zip: &Path, dir: &Path, input: &[u8]) -> (i32, Vec<u8>, String) {
        let fill = |a: &&str| OsString::from(a.replace("{zip}", &zip.display().to_string()).replace("{dir}", &dir.display().to_string()));
        let args: Vec<OsString> = args.iter().map(fill).collect();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let code = run(&args, &mut Io { input: &mut Cursor::new(input), output: &mut output, errors: &mut errors });
        (code, output, String::from_utf8(errors).unwrap())
    }

    fn text(output: Vec<u8>) -> String {
        String::from_utf8(output).unwrap()
    }

    fn parsed(args: &[&str]) -> Result<Asked, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    fn request(args: &[&str]) -> Request {
        match parsed(args) {
            Ok(Asked::Unzip(request)) => request,
            other => panic!("{args:?} gave {other:?}"),
        }
    }

    fn patterns(list: &[&str]) -> BTreeSet<CString> {
        list.iter().map(|p| CString::new(*p).unwrap()).collect()
    }

    fn crc(data: &[u8]) -> u32 {
        let mut crc = Crc::new();
        crc.update(data);
        crc.sum()
    }

    #[test]
    fn options_are_read_as_ziptool_reads_them() {
        let r = request(&["-qo", "rom.zip", "bin/*", "-d", "/tmp", "-x", "*.txt", "-j", "META-INF/*"]);
        assert_eq!(r.archive, PathBuf::from("rom.zip"));
        assert_eq!(r.includes, patterns(&["bin/*"]));
        assert_eq!(r.excludes, patterns(&["*.txt", "META-INF/*"]), "every name after -x is one to leave out");
        assert_eq!(r.folder, Some("/tmp/".into()));
        assert!(r.quiet && r.junk);
        assert_eq!(r.overwrite, Overwrite::Always);
        assert_eq!(request(&["-od/data/x", "rom.zip"]).folder, Some("/data/x/".into()), "-d takes the rest of its group");
        let r = request(&["-p", "-o", "-n", "rom.zip"]);
        assert!(r.pipe && r.quiet, "-p is quiet");
        assert_eq!(r.overwrite, Overwrite::Never, "the last of -n and -o counts");
        let r = request(&["--", "-o.zip", "-q"]);
        assert_eq!((r.archive, r.includes, r.quiet), (PathBuf::from("-o.zip"), patterns(&["-q"]), false), "after -- a dash starts a name");
        assert_eq!(parsed(&["--help"]), Ok(Asked::Help));
        assert_eq!(parsed(&["-h", "rom.zip"]), Ok(Asked::Help));
        assert_eq!(parsed(&["-Z", "-1", "rom.zip"]), Ok(Asked::Zipinfo(vec!["-1".into(), "rom.zip".into()])));
        assert_eq!(parsed(&["-K", "rom.zip"]), Err(format!("invalid option -- K\n{USAGE}")));
        assert_eq!(parsed(&["--list", "rom.zip"]), Err(format!("unrecognized option '--list'\n{USAGE}")));
        assert_eq!(parsed(&["rom.zip", "-d"]), Err(format!("option requires an argument -- d\n{USAGE}")));
        assert_eq!(parsed(&[]), Err("missing archive filename".into()));
        assert_eq!(parsed(&["-q"]), Err("missing archive filename".into()));
    }

    #[test]
    fn names_match_as_the_c_library_s_fnmatch_matches_them_with_no_flags() {
        let takes = |args: &[&str], name: &str| request(&[&["rom.zip"][..], args].concat()).takes(name);
        assert!(takes(&[], "images/super.img"), "no names take every file");
        assert!(takes(&["images/*"], "images/super.img"));
        assert!(takes(&["*.img"], "images/super.img"), "* stands for slashes too");
        assert!(takes(&["images?super.img"], "images/super.img"), "and so does ?");
        assert!(takes(&["[it]*.txt"], "top.txt") && takes(&["[it]*.txt"], "images/a.txt"));
        assert!(!takes(&["[!it]*"], "top.txt"));
        assert!(takes(&["a\\*b"], "a*b") && !takes(&["a\\*b"], "axb"), "a backslash takes the next character as it is");
        assert!(!takes(&["images/*"], "bin/tool"));
        assert!(!takes(&["-x", "*.txt"], "a.txt"));
        assert!(!takes(&["*", "-x", "*/updater-script"], "META-INF/com/google/android/updater-script"), "a name left out stays out");
        assert!(takes(&["*", "-x", "*/updater-script"], "META-INF/com/google/android/update-binary"));
    }

    #[test]
    fn p_pours_each_file_picked_out_as_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let image = noise(CHUNK * 2 + 99);
        let script = b"#!/sbin/sh\n";
        for (method, large) in [(CompressionMethod::Stored, false), (CompressionMethod::Deflated, false), (CompressionMethod::Stored, true)] {
            let archive = written(
                dir.path(),
                "rom.zip",
                &[stored("META-INF/com/google/android/update-binary", script), Entry { method, large, ..stored("super.zst", &image) }],
            );
            let (code, output, errors) = ran(&["-p", "{zip}", "super.zst"], &archive, dir.path(), b"");
            assert_eq!((code, errors.as_str()), (DONE, ""), "{method:?}, zip64 {large}");
            assert!(output == image, "{method:?}, zip64 {large}");
        }
        let archive = written(dir.path(), "several.zip", &[stored("images/", b""), stored("images/b.txt", b"b"), stored("images/a.txt", b"a"), stored("top.txt", b"top")]);
        let (code, output, _) = ran(&["-p", "{zip}", "top.txt", "images/*"], &archive, dir.path(), b"");
        assert_eq!((code, output.as_slice()), (DONE, b"batop".as_slice()), "in the archive's order, a folder giving nothing");
        assert_eq!(ran(&["-p", "{zip}", "none.txt"], &archive, dir.path(), b""), (DONE, Vec::new(), String::new()), "nothing picked is no error");
    }

    #[test]
    fn data_that_does_not_match_its_checksum_stops_p_and_fails_t() {
        let dir = tempfile::tempdir().unwrap();
        let data = noise(1000);
        let mut bytes = zip(&[stored("x.bin", &data)]);
        let start = ZipArchive::new(Cursor::new(&bytes)).unwrap().by_index(0).unwrap().data_start().unwrap() as usize;
        bytes[start + 500] ^= 0xff;
        let mut damaged = data.clone();
        damaged[500] ^= 0xff;
        let archive = dir.path().join("damaged.zip");
        fs::write(&archive, &bytes).unwrap();
        let (code, _, errors) = ran(&["-p", "{zip}"], &archive, dir.path(), b"");
        assert_eq!(code, FAILED);
        assert!(errors.starts_with("unzip: failed to extract x.bin: "), "{errors}");
        let shown = archive.display();
        let (sum, should) = (crc(&damaged), crc(&data));
        let (code, output, _) = ran(&["-t", "{zip}"], &archive, dir.path(), b"");
        let told = format!("Archive:  {shown}\n    testing: x.bin                    bad CRC {sum:08x}  (should be {should:08x})\nAt least one error was detected in {shown}.\n");
        assert_eq!((code, text(output)), (BAD_CRC, told));
        let (code, output, _) = ran(&["-tq", "{zip}"], &archive, dir.path(), b"");
        let told = format!("x.bin                   bad CRC {sum:08x}  (should be {should:08x})\nAt least one error was detected in {shown}.\n");
        assert_eq!((code, text(output)), (BAD_CRC, told));
    }

    #[test]
    fn t_tells_of_each_file_and_of_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(
            dir.path(),
            "rom.zip",
            &[stored("images/", b""), stored("images/a.txt", b"hello\n"), stored("META-INF/com/google/android/updater-script", b"ui_print(\"x\");\n")],
        );
        let shown = archive.display();
        let (code, output, _) = ran(&["-t", "{zip}"], &archive, dir.path(), b"");
        let told = format!(
            "Archive:  {shown}\n    testing: images/                  OK\n    testing: images/a.txt             OK\n    testing: META-INF/com/google/android/updater-script OK\nNo errors detected in compressed data of {shown}.\n"
        );
        assert_eq!((code, text(output)), (DONE, told));
        let (code, output, _) = ran(&["-tq", "{zip}"], &archive, dir.path(), b"");
        assert_eq!((code, text(output)), (DONE, format!("No errors detected in compressed data of {shown}.\n")));
        let (code, output, _) = ran(&["-tq", "{zip}", "images/*"], &archive, dir.path(), b"");
        assert_eq!((code, text(output)), (DONE, format!("No errors detected in {shown} for the 2 files tested.\n")), "the folder counts too");
        let (code, output, _) = ran(&["-tq", "{zip}", "images/a.txt"], &archive, dir.path(), b"");
        assert_eq!((code, text(output)), (DONE, format!("No errors detected in {shown} for the 1 file tested.\n")));
    }

    #[test]
    fn l_and_v_list_as_ziptool_lists() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), "rom.zip", &[stored("images/", b""), stored("images/a.txt", b"hello\n"), stored("bin/tool", b"\x7fELF")]);
        let shown = archive.display();
        let (code, output, _) = ran(&["-l", "{zip}"], &archive, dir.path(), b"");
        let listed = "  Length      Date    Time    Name\n\
                      ---------  ---------- -----   ----\n        \
                      0  2026-11-06 20:41   images/\n        \
                      6  2026-11-06 20:41   images/a.txt\n        \
                      4  2026-11-06 20:41   bin/tool\n\
                      ---------                     -------\n       \
                      10                     3 files\n";
        assert_eq!((code, text(output)), (DONE, format!("Archive:  {shown}\n{listed}")));
        assert_eq!(text(ran(&["-lq", "{zip}"], &archive, dir.path(), b"").1), listed, "-q leaves out the archive's name");
        let (code, output, _) = ran(&["-v", "{zip}", "*a*"], &archive, dir.path(), b"");
        let listed = format!(
            "Archive:  {shown}\n Length   Method    Size  Cmpr    Date    Time   CRC-32   Name\n\
             --------  ------  ------- ---- ---------- ----- --------  ----\n       \
             0  Stored        0   0% 2026-11-06 20:41 00000000  images/\n       \
             6  Stored        6   0% 2026-11-06 20:41 363a3020  images/a.txt\n\
             --------          -------  ---                            -------\n       \
             6                6   0%                            2 files\n"
        );
        assert_eq!((code, text(output)), (DONE, listed));
        let deflated = written(dir.path(), "deflated.zip", &[Entry { method: CompressionMethod::Deflated, ..stored("zeros.img", &[0; 100_000]) }]);
        let packed = ZipArchive::new(File::open(&deflated).unwrap()).unwrap().by_index(0).unwrap().compressed_size();
        let line = text(ran(&["-vq", "{zip}"], &deflated, dir.path(), b"").1).lines().nth(2).unwrap().to_string();
        let saved = 100.0 * (100_000.0 - packed as f32) / 100_000.0;
        assert_eq!(line, format!("  100000  Defl:N {packed:8} {saved:3.0}% 2026-11-06 20:41 {:08x}  zeros.img", crc(&[0; 100_000])));
    }

    #[test]
    fn files_go_into_the_folder_under_their_folders_with_their_modes() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(
            dir.path(),
            "rom.zip",
            &[
                Entry { mode: Some(0o755), ..stored("bin/", b"") },
                Entry { mode: Some(0o755), ..stored("bin/tool", b"\x7fELF") },
                Entry { mode: Some(0o644), ..stored("images/a.txt", b"hello\n") },
                stored("META-INF/com/google/android/update-binary", b"#!/sbin/sh\n"),
                stored("empty/", b""),
            ],
        );
        let (code, output, errors) = ran(&["{zip}", "-d", "{dir}/out"], &archive, dir.path(), b"");
        let out = dir.path().join("out");
        let told = format!(
            "Archive:  {}\n  inflating: {o}/bin/tool\n  inflating: {o}/images/a.txt\n  inflating: {o}/META-INF/com/google/android/update-binary\n",
            archive.display(),
            o = out.display()
        );
        assert_eq!((code, text(output), errors.as_str()), (DONE, told, ""));
        assert_eq!(fs::read(out.join("bin/tool")).unwrap(), b"\x7fELF");
        assert_eq!(fs::read(out.join("META-INF/com/google/android/update-binary")).unwrap(), b"#!/sbin/sh\n");
        let mode = |path: &str| fs::metadata(out.join(path)).unwrap().permissions().mode();
        assert!(mode("bin/tool") & 0o100 != 0 && mode("images/a.txt") & 0o100 == 0, "each file keeps its mode");
        assert!(out.join("empty").is_dir());
        let (code, _, _) = ran(&["-jqo", "{zip}", "-d", "{dir}/flat", "bin/*", "META-INF/*"], &archive, dir.path(), b"");
        assert_eq!(code, DONE);
        let mut names: Vec<_> = fs::read_dir(dir.path().join("flat")).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["tool", "update-binary"], "-j takes the files out without their folders, and no folder as a file");
    }

    #[test]
    fn d_makes_its_folder_but_not_the_folders_it_is_in() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), "rom.zip", &[stored("top.txt", b"top")]);
        let (code, _, errors) = ran(&["-q", "{zip}", "-d", "{dir}/x/y"], &archive, dir.path(), b"");
        assert_eq!(code, FAILED);
        assert!(errors.starts_with(&format!("unzip: couldn't create {}/x/y/: ", dir.path().display())), "{errors}");
        assert_eq!(ran(&["-q", "{zip}", "-d", "{dir}/x"], &archive, dir.path(), b"").0, DONE);
        assert_eq!(fs::read(dir.path().join("x/top.txt")).unwrap(), b"top");
    }

    #[test]
    fn a_file_already_there_is_kept_replaced_or_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let archive = written(dir.path(), "rom.zip", &[stored("a.txt", b"new a"), stored("b.txt", b"new b")]);
        let out = dir.path().join("out");
        let before = || {
            fs::create_dir_all(&out).unwrap();
            fs::write(out.join("a.txt"), b"old a").unwrap();
            fs::write(out.join("b.txt"), b"old b").unwrap();
        };
        let now = |name: &str| fs::read(out.join(name)).unwrap();
        let prompt = |name: &str| format!("replace {}/{name}? [y]es, [n]o, [A]ll, [N]one: ", out.display());
        before();
        let (code, output, errors) = ran(&["-q", "{zip}", "-d", "{dir}/out"], &archive, dir.path(), b"");
        assert_eq!((code, text(output), errors.as_str()), (FAILED, prompt("a.txt"), "unzip: (EOF/read error; assuming [N]one...)\n"));
        assert_eq!(now("a.txt"), b"old a");
        assert_eq!(ran(&["-qn", "{zip}", "-d", "{dir}/out"], &archive, dir.path(), b"").0, DONE);
        assert_eq!((now("a.txt"), now("b.txt")), (b"old a".to_vec(), b"old b".to_vec()));
        assert_eq!(ran(&["-qo", "{zip}", "-d", "{dir}/out"], &archive, dir.path(), b"").0, DONE);
        assert_eq!((now("a.txt"), now("b.txt")), (b"new a".to_vec(), b"new b".to_vec()));
        for (answers, a, b, asked) in [
            (&b"y\nn\n"[..], "new a", "old b", 2),
            (b"?\nn\ny\n", "old a", "new b", 2),
            (b"A\n", "new a", "new b", 1),
            (b"N\n", "old a", "old b", 1),
        ] {
            before();
            let (code, output, _) = ran(&["-q", "{zip}", "-d", "{dir}/out"], &archive, dir.path(), answers);
            assert_eq!(code, DONE, "{answers:?}");
            assert_eq!((now("a.txt"), now("b.txt")), (a.as_bytes().to_vec(), b.as_bytes().to_vec()), "{answers:?}");
            assert_eq!(text(output).matches("replace ").count(), asked, "{answers:?}");
        }
    }

    #[test]
    fn a_name_leading_out_of_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["../escaped", "a/../../escaped"] {
            let archive = written(dir.path(), "rom.zip", &[stored(name, b"out")]);
            let (code, _, errors) = ran(&["-q", "{zip}", "-d", "{dir}/in"], &archive, dir.path(), b"");
            assert_eq!((code, errors), (FAILED, format!("unzip: bad filename {name}\n")));
            assert!(!dir.path().join("escaped").exists());
            assert_eq!(ran(&["-p", "{zip}"], &archive, dir.path(), b"").1, b"out", "standard output is no folder to leave");
        }
    }

    #[test]
    fn an_archive_missing_or_not_a_zip_stops_unzip() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.zip");
        let (code, _, errors) = ran(&["-p", "{zip}"], &missing, dir.path(), b"");
        assert_eq!(code, FAILED);
        assert!(errors.starts_with(&format!("unzip: couldn't open {}: ", missing.display())), "{errors}");
        let text_file = dir.path().join("text.zip");
        fs::write(&text_file, b"not a zip").unwrap();
        assert_eq!(ran(&["-l", "{zip}"], &text_file, dir.path(), b"").0, FAILED);
    }

    #[test]
    fn z_hands_what_follows_it_to_zipinfo() {
        let command = zipinfo(&["-1".into(), "rom.zip".into()]);
        assert_eq!(command.get_program(), ZIPINFO);
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["-1", "rom.zip"]);
    }
}
