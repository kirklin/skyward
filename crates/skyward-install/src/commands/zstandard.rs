//! zstd as install scripts run it to decompress a partition image on its
//! way to the partition: from files or standard input, to files, standard
//! output or nowhere, with the zstd command's switches that bear on
//! decompressing. It does not compress.

use super::{copy, Failed, Io, CHUNK};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// How zstd ends.
const DONE: i32 = 0;
const FAILED: i32 = 1;

/// The largest window, as a power of two, a frame may need for zstd to
/// decompress it unless told it may take more memory; and the bounds of
/// any.
const WINDOW_LOG: u32 = 27;
const WINDOW_LOG_MIN: u32 = 10;
const WINDOW_LOG_MAX: u32 = 31;

/// How a frame starts: a zstd frame, and a skippable one, whose last four
/// bits may be anything.
const FRAME: u32 = 0xfd2f_b528;
const SKIPPABLE: u32 = 0x184d_2a50;

const HELP: &str = "\
Usage: zstd -d [options] [file...]

Decompresses each file, or standard input when none is given or for -.
A file decompresses into one named as it is without .zst, or .tzst for .tar.

  -d, --decompress    decompress
  -t, --test          only check that each input decompresses
  -c, --stdout        write to standard output
  -o FILE             write to FILE; for one input
  -f, --force         replace a file already there; with -c, pass on
                      what is not zstd as it is
  -k, --keep          keep each input file (the default)
      --rm            remove each input file once it is decompressed into a file
  -q, --quiet         tell only of errors; twice, of nothing
  -v, --verbose       tell more
  -M#, --memory=#     allow a frame a window of up to # bytes, given with K, M or G
      --long[=#]      allow a frame a window of up to 2^# bytes (27 without #)
";

pub(super) fn run(args: &[OsString], io: &mut Io) -> i32 {
    let request = match parse(args) {
        Ok(Asked::Decompress(request)) => request,
        Ok(Asked::Version) => {
            let library = zstd::zstd_safe::version_string();
            return say(io, &format!("zstd of Skyward {}, decompressing with libzstd {library}\n", env!("CARGO_PKG_VERSION")));
        }
        Ok(Asked::Help) => return say(io, HELP),
        Err(e) => {
            let _ = writeln!(io.errors, "zstd: {e}");
            return FAILED;
        }
    };
    let mut failed = false;
    for input in &request.inputs {
        if let Err(e) = decompress(&request, input, io) {
            failed = true;
            if request.verbosity >= 1 {
                let _ = writeln!(io.errors, "zstd: {e}");
            }
        }
    }
    if let Err(e) = io.output.flush() {
        failed = true;
        let _ = writeln!(io.errors, "zstd: standard output: {e}");
    }
    match failed {
        true => FAILED,
        false => DONE,
    }
}

fn say(io: &mut Io, text: &str) -> i32 {
    match io.output.write_all(text.as_bytes()).and_then(|()| io.output.flush()) {
        Ok(()) => DONE,
        Err(_) => FAILED,
    }
}

/// What the command line asks of zstd.
#[derive(Debug, PartialEq)]
enum Asked {
    Decompress(Request),
    Version,
    Help,
}

#[derive(Debug, PartialEq)]
struct Request {
    inputs: Vec<Input>,
    to: To,
    /// Replace a file already where an output goes.
    force: bool,
    /// Remove each input file once it is decompressed into a file.
    remove: bool,
    /// Pass on an input that does not start as zstd as it is.
    pass_through: bool,
    window_log: u32,
    /// From 2, tell how much each file decompressed into; from 1, tell of
    /// errors.
    verbosity: i32,
}

#[derive(Debug, PartialEq)]
enum Input {
    Standard,
    File(PathBuf),
}

/// Where what an input decompresses into goes.
#[derive(Debug, PartialEq)]
enum To {
    /// A file beside each input file, named for it; standard input's goes
    /// to standard output.
    Own,
    /// Standard output, one input's after another's: `-c`.
    Output,
    /// The file `-o` names, from the one input.
    File(PathBuf),
    /// Nowhere, each input only checked: `-t`.
    Nowhere,
}

/// Read the command line as zstd reads it: options grouped behind one dash
/// or spelled out behind two, and the inputs, until a `--`.
fn parse(args: &[OsString]) -> Result<Asked, String> {
    let (mut decompress, mut test, mut stdout, mut force, mut remove) = (false, false, false, false, false);
    let (mut output, mut memory, mut long, mut pass_through) = (None, None, None, None);
    let (mut verbosity, mut inputs, mut options) = (2, Vec::new(), true);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg.to_str().filter(|text| options && text.len() > 1 && text.starts_with('-'));
        let Some(text) = text else {
            inputs.push(if arg == "-" { Input::Standard } else { Input::File(arg.into()) });
            continue;
        };
        if let Some(word) = text.strip_prefix("--") {
            match word {
                "" => options = false,
                "decompress" | "uncompress" => decompress = true,
                "test" => test = true,
                "stdout" => stdout = true,
                "force" => force = true,
                "keep" => remove = false,
                "rm" => remove = true,
                "quiet" => verbosity -= 1,
                "verbose" => verbosity += 1,
                "pass-through" => pass_through = Some(true),
                "no-pass-through" => pass_through = Some(false),
                "long" => long = Some(WINDOW_LOG),
                "version" => return Ok(Asked::Version),
                "help" => return Ok(Asked::Help),
                // Every byte is written, as a partition needs it, whether or
                // not runs of zeros could have been left out.
                "sparse" | "no-sparse" | "no-progress" | "progress" | "format=zstd" => {}
                _ => match word.split_once('=') {
                    Some(("long", log)) => long = Some(window_log(log)?),
                    Some(("memory" | "memlimit" | "memlimit-decompress", size)) => memory = Some(bytes(size)?),
                    // Decompressing takes one thread whatever is asked.
                    Some(("threads", n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {}
                    _ => return Err(format!("unsupported option --{word}")),
                },
            }
            continue;
        }
        let flags: Vec<char> = text[1..].chars().collect();
        let mut at = 0;
        while at < flags.len() {
            match flags[at] {
                'd' => decompress = true,
                't' => test = true,
                'c' => stdout = true,
                'f' => force = true,
                'k' => remove = false,
                'q' => verbosity -= 1,
                'v' => verbosity += 1,
                'V' => return Ok(Asked::Version),
                'h' | 'H' => return Ok(Asked::Help),
                // A compression level, or how many threads to take, both
                // given with digits and neither heeded in decompressing.
                'T' | '0'..='9' => {
                    at += 1;
                    while flags.get(at).is_some_and(char::is_ascii_digit) {
                        at += 1;
                    }
                    continue;
                }
                // The size is the rest of the group.
                'M' => {
                    memory = Some(bytes(&flags[at + 1..].iter().collect::<String>())?);
                    break;
                }
                // The file is the next argument.
                'o' if at + 1 == flags.len() => output = Some(PathBuf::from(args.next().ok_or("-o names no file")?)),
                'o' => return Err(format!("-o ends a group of options, before the file it names, not as in {text}")),
                other => return Err(format!("unsupported option -{other}")),
            }
            at += 1;
        }
    }
    if !decompress && !test {
        return Err("only decompresses; give -d, or -t to check files".into());
    }
    let to = match (test, stdout, output) {
        (true, _, _) => To::Nowhere,
        (false, true, _) => To::Output,
        (false, false, Some(file)) => To::File(file),
        (false, false, None) => To::Own,
    };
    if inputs.is_empty() {
        inputs.push(Input::Standard);
    }
    if matches!(to, To::File(_)) && inputs.len() > 1 {
        return Err("-o takes one input".into());
    }
    let window_log = match (memory, long) {
        // A limit between two powers of two allows the larger, as no frame
        // zstd itself would decompress under it is refused.
        (Some(bytes), _) => bytes.clamp(1, 1 << WINDOW_LOG_MAX).next_power_of_two().trailing_zeros().max(WINDOW_LOG_MIN),
        (None, Some(log)) => log,
        (None, None) => WINDOW_LOG,
    };
    Ok(Asked::Decompress(Request {
        inputs,
        to,
        force,
        remove,
        pass_through: pass_through.unwrap_or(stdout && force),
        window_log,
        verbosity,
    }))
}

/// A window size given as its power of two.
fn window_log(text: &str) -> Result<u32, String> {
    text.parse()
        .ok()
        .filter(|log| (WINDOW_LOG_MIN..=WINDOW_LOG_MAX).contains(log))
        .ok_or_else(|| format!("--long={text} is outside {WINDOW_LOG_MIN} to {WINDOW_LOG_MAX}"))
}

/// A number of bytes as zstd's options take it: digits, then K, M or G for
/// kibibytes, mebibytes or gibibytes, written as K, KB or KiB.
fn bytes(text: &str) -> Result<u64, String> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let number: u64 = text[..digits].parse().map_err(|_| format!("{text:?} is not a size"))?;
    let shift = match &text[digits..] {
        "" => 0,
        "K" | "KB" | "KiB" => 10,
        "M" | "MB" | "MiB" => 20,
        "G" | "GB" | "GiB" => 30,
        _ => return Err(format!("{text:?} is not a size")),
    };
    number.checked_mul(1 << shift).ok_or_else(|| format!("{text:?} is too large"))
}

/// Decompress one input where the request sends it.
fn decompress(request: &Request, input: &Input, io: &mut Io) -> Result<(), String> {
    let (name, mut opened) = match input {
        Input::Standard => ("stdin".to_string(), None),
        Input::File(path) => {
            let name = path.display().to_string();
            match fs::metadata(path) {
                Ok(found) if found.is_dir() => return Err(format!("{name} is a folder")),
                Ok(_) => {}
                Err(e) => return Err(format!("{name}: {e}")),
            }
            let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
            (name, Some(file))
        }
    };
    let from: &mut dyn Read = match &mut opened {
        Some(file) => file,
        None => &mut *io.input,
    };
    let into = match (&request.to, input) {
        (To::Nowhere, _) => None,
        (To::Output, _) | (To::Own, Input::Standard) => {
            return decode(from, io.output, request.window_log, request.pass_through).map(drop).map_err(|e| match e {
                Failed::Reading(e) => format!("{name}: {e}"),
                Failed::Writing(e) => format!("standard output: {e}"),
            });
        }
        (To::File(path), _) => Some(path.clone()),
        (To::Own, Input::File(path)) => Some(own_name(path)?),
    };
    let Some(into) = into else {
        let decompressed = decode(from, &mut io::sink(), request.window_log, false).map_err(|e| match e {
            Failed::Reading(e) | Failed::Writing(e) => format!("{name}: {e}"),
        })?;
        tell(request, io, &name, decompressed);
        return Ok(());
    };
    let input_path = match input {
        Input::File(path) => Some(path.as_path()),
        Input::Standard => None,
    };
    let (mut out, made) = create(&into, request.force, input_path)?;
    let decompressed = match decode(from, &mut out, request.window_log, request.pass_through) {
        Ok(decompressed) => decompressed,
        Err(e) => {
            // What was written of a file made for it is no use, as zstd
            // itself has it.
            if made {
                let _ = fs::remove_file(&into);
            }
            return Err(match e {
                Failed::Reading(e) => format!("{name}: {e}"),
                Failed::Writing(e) => format!("{}: {e}", into.display()),
            });
        }
    };
    if let (true, Some(path)) = (request.remove, input_path) {
        fs::remove_file(path).map_err(|e| format!("remove {name}: {e}"))?;
    }
    tell(request, io, &name, decompressed);
    Ok(())
}

/// Say how much `name` decompressed into, at the verbosity that tells it.
fn tell(request: &Request, io: &mut Io, name: &str, decompressed: u64) {
    if request.verbosity >= 2 {
        let _ = writeln!(io.errors, "{name:<20}: {decompressed} bytes");
    }
}

/// The file a compressed file decompresses into, beside it: its name
/// without .zst, or with .tar for .tzst.
fn own_name(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().map(OsStrExt::as_bytes).unwrap_or_default();
    let plain = match (name.strip_suffix(b".zst"), name.strip_suffix(b".tzst")) {
        (Some(stem), _) if !stem.is_empty() => stem.to_vec(),
        (_, Some(stem)) if !stem.is_empty() => [stem, &b".tar"[..]].concat(),
        _ => return Err(format!("{}: unknown suffix (.zst or .tzst expected); give the output with -o", path.display())),
    };
    Ok(path.with_file_name(OsString::from_vec(plain)))
}

/// Open `path` to decompress into. A file already there is replaced only
/// when forced, and never when it is the input itself; anything else
/// there, such as a partition, is written over where it is. Gives whether
/// a file was made for it.
fn create(path: &Path, force: bool, input: Option<&Path>) -> Result<(File, bool), String> {
    let shown = path.display();
    match fs::metadata(path) {
        Ok(there) => {
            let same = |input: &Path| fs::metadata(input).is_ok_and(|i| (i.dev(), i.ino()) == (there.dev(), there.ino()));
            if input.is_some_and(same) {
                return Err(format!("{shown} is the input itself"));
            }
            if !there.is_file() {
                let file = OpenOptions::new().write(true).open(path).map_err(|e| format!("{shown}: {e}"))?;
                return Ok((file, false));
            }
            if !force {
                return Err(format!("{shown} already exists; -f replaces it"));
            }
            fs::remove_file(path).map_err(|e| format!("remove {shown}: {e}"))?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{shown}: {e}")),
    }
    let file = OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("{shown}: {e}"))?;
    Ok((file, true))
}

/// Decompress the frames `input` holds into `output`, allowing windows of
/// up to 2^`window_log` bytes, and give how much they held. With
/// `pass_through`, an input that does not start as zstd goes out as it is.
fn decode(input: &mut dyn Read, output: &mut dyn Write, window_log: u32, pass_through: bool) -> Result<u64, Failed> {
    let mut head = [0; 4];
    let mut held = 0;
    while held < head.len() {
        match input.read(&mut head[held..]) {
            Ok(0) => break,
            Ok(read) => held += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Failed::Reading(e)),
        }
    }
    if held == 0 {
        return Err(Failed::Reading(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected end of file")));
    }
    let mut whole = (&head[..held]).chain(input);
    if pass_through && !is_zstd(&head[..held]) {
        return copy(&mut whole, output);
    }
    let mut decoder = zstd::stream::read::Decoder::with_buffer(BufReader::with_capacity(CHUNK, whole)).map_err(Failed::Reading)?;
    decoder.window_log_max(window_log).map_err(Failed::Reading)?;
    copy(&mut decoder, output)
}

/// Whether `head`, the first bytes of an input, starts a frame.
fn is_zstd(head: &[u8]) -> bool {
    let Ok(magic) = <[u8; 4]>::try_from(head).map(u32::from_le_bytes) else { return false };
    magic == FRAME || magic & !0xf == SKIPPABLE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn compressed(data: &[u8]) -> Vec<u8> {
        zstd::encode_all(data, 3).unwrap()
    }

    /// Bytes that do not repeat, so they keep their size compressed.
    fn noise(len: usize) -> Vec<u8> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    /// Run zstd with `args` and `input` as standard input: how it ended,
    /// what it wrote to standard output, and what it told.
    fn ran(args: &[&str], input: &[u8]) -> (i32, Vec<u8>, String) {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let code = run(&args, &mut Io { input: &mut Cursor::new(input), output: &mut output, errors: &mut errors });
        (code, output, String::from_utf8(errors).unwrap())
    }

    fn path(path: &Path) -> String {
        path.display().to_string()
    }

    #[test]
    fn standard_input_decompresses_to_standard_output_however_the_options_are_written() {
        let data = noise(CHUNK * 2 + 777);
        let frame = compressed(&data);
        for args in [
            &["-d", "-c"][..],
            &["-dc"],
            &["-cd"],
            &["-k", "-d"],
            &["--decompress", "--stdout"],
            &["--uncompress"],
            &["-d"],
            &["-d", "-c", "-"],
            &["-d", "-T0", "-q", "--no-progress", "-c"],
            &["-dc", "--long=31"],
            &["-d", "-19", "-c"],
            &["-dcT4", "--threads=2", "--sparse"],
        ] {
            let (code, output, errors) = ran(args, &frame);
            assert_eq!((code, errors.as_str()), (DONE, ""), "{args:?}");
            assert!(output == data, "{args:?}");
        }
    }

    #[test]
    fn frames_one_after_another_decompress_whole_and_skippable_ones_add_nothing() {
        let mut frames = compressed(b"first ");
        frames.extend_from_slice(&[0x5e, 0x2a, 0x4d, 0x18, 3, 0, 0, 0, 1, 2, 3]);
        frames.extend(compressed(b"second"));
        assert_eq!(ran(&["-dc"], &frames), (DONE, b"first second".to_vec(), String::new()));
    }

    #[test]
    fn input_that_is_empty_cut_short_or_not_zstd_fails() {
        let data = noise(300_000);
        let frame = compressed(&data);
        let (code, output, errors) = ran(&["-dc"], b"");
        assert_eq!((code, output.len()), (FAILED, 0));
        assert!(errors.contains("stdin: unexpected end of file"), "{errors}");
        let (code, output, _) = ran(&["-dc"], &frame[..frame.len() / 2]);
        assert_eq!(code, FAILED);
        assert!(output.len() < data.len() && data.starts_with(&output), "what came before the cut is written");
        assert_eq!(ran(&["-dc"], b"hello\n").0, FAILED);
        let mut trailed = frame.clone();
        trailed.extend_from_slice(b"junk\n");
        let (code, output, _) = ran(&["-dc"], &trailed);
        assert_eq!(code, FAILED, "what follows the frames is no frame");
        assert!(output == data, "the frame before it is written whole");
    }

    #[test]
    fn a_frame_whose_checksum_does_not_match_fails() {
        let mut checked = Vec::new();
        let mut encoder = zstd::stream::write::Encoder::new(&mut checked, 3).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(b"a checked frame").unwrap();
        encoder.finish().unwrap();
        let last = checked.len() - 1;
        checked[last] ^= 0xff;
        let (code, _, errors) = ran(&["-dc"], &checked);
        assert_eq!(code, FAILED);
        assert!(errors.contains("checksum"), "{errors}");
    }

    #[test]
    fn with_c_and_f_what_is_not_zstd_passes_through_as_it_is() {
        let plain = b"no zstd here\n".repeat(1000);
        assert_eq!(ran(&["-dcf"], &plain), (DONE, plain.clone(), String::new()));
        assert_eq!(ran(&["-d", "-c", "-f", "--pass-through"], b"ab"), (DONE, b"ab".to_vec(), String::new()), "however short");
        assert_eq!(ran(&["-dc"], &plain).0, FAILED);
        assert_eq!(ran(&["-dcf", "--no-pass-through"], &plain).0, FAILED);
        assert_eq!(ran(&["-dcf"], &compressed(b"zstd")).1, b"zstd", "zstd still decompresses");
    }

    #[test]
    fn a_file_decompresses_beside_itself_into_its_name_without_the_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let data = noise(5000);
        let input = dir.path().join("super.img.zst");
        fs::write(&input, compressed(&data)).unwrap();
        let (code, output, errors) = ran(&["-d", &path(&input)], b"");
        assert_eq!((code, output.len()), (DONE, 0));
        assert_eq!(fs::read(dir.path().join("super.img")).unwrap(), data);
        assert!(input.exists(), "the input stays");
        assert!(errors.contains(": 5000 bytes"), "{errors}");
        let tar = dir.path().join("system.tzst");
        fs::write(&tar, compressed(b"tar")).unwrap();
        assert_eq!(ran(&["-d", "-q", &path(&tar)], b""), (DONE, Vec::new(), String::new()));
        assert_eq!(fs::read(dir.path().join("system.tar")).unwrap(), b"tar");
        let odd = dir.path().join("super.img.zstd");
        fs::write(&odd, compressed(b"odd")).unwrap();
        let (code, _, errors) = ran(&["-d", &path(&odd)], b"");
        assert_eq!(code, FAILED);
        assert!(errors.contains("unknown suffix"), "{errors}");
    }

    #[test]
    fn a_file_already_there_is_replaced_only_when_forced() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("boot.img.zst");
        fs::write(&input, compressed(b"new")).unwrap();
        let out = dir.path().join("boot.img");
        fs::write(&out, b"old").unwrap();
        let (code, _, errors) = ran(&["-d", &path(&input)], b"");
        assert_eq!(code, FAILED);
        assert!(errors.contains("already exists"), "{errors}");
        assert_eq!(fs::read(&out).unwrap(), b"old");
        assert_eq!(ran(&["-df", &path(&input)], b"").0, DONE);
        assert_eq!(fs::read(&out).unwrap(), b"new");
        let (code, _, errors) = ran(&["-df", "-o", &path(&input), &path(&input)], b"");
        assert_eq!(code, FAILED);
        assert!(errors.contains("the input itself"), "{errors}");
        assert!(input.exists());
    }

    #[test]
    fn what_is_not_a_regular_file_is_written_over_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("vbmeta.img.zst");
        fs::write(&input, compressed(b"vbmeta")).unwrap();
        assert_eq!(ran(&["-d", "-q", "-o", "/dev/null", &path(&input)], b"").0, DONE, "as a partition is");
    }

    #[test]
    fn rm_removes_an_input_once_it_is_decompressed_into_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("cust.img.zst");
        fs::write(&input, compressed(b"cust")).unwrap();
        assert_eq!(ran(&["-dc", "--rm", &path(&input)], b"").1, b"cust");
        assert!(input.exists(), "standard output is no file");
        assert_eq!(ran(&["-d", "--rm", &path(&input)], b"").0, DONE);
        assert!(!input.exists());
        assert_eq!(fs::read(dir.path().join("cust.img")).unwrap(), b"cust");
    }

    #[test]
    fn o_names_the_file_one_input_decompresses_into() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.zst"), dir.path().join("b.zst"));
        fs::write(&a, compressed(b"a")).unwrap();
        fs::write(&b, compressed(b"b")).unwrap();
        let out = dir.path().join("out.img");
        assert_eq!(ran(&["-q", "-d", "-o", &path(&out), &path(&a)], b"").0, DONE);
        assert_eq!(fs::read(&out).unwrap(), b"a");
        let other = dir.path().join("other.img");
        assert_eq!(ran(&["-d", "-o", &path(&other), &path(&a), &path(&b)], b"").0, FAILED);
        assert!(!other.exists());
        assert_eq!(ran(&["-qdo", &path(&other), "-"], &compressed(b"from stdin")).0, DONE);
        assert_eq!(fs::read(&other).unwrap(), b"from stdin");
    }

    #[test]
    fn a_file_made_for_an_input_that_fails_is_taken_away() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("super.img.zst");
        let frame = compressed(&noise(100_000));
        fs::write(&input, &frame[..frame.len() / 2]).unwrap();
        assert_eq!(ran(&["-d", &path(&input)], b"").0, FAILED);
        assert!(!dir.path().join("super.img").exists());
    }

    #[test]
    fn a_missing_input_is_told_and_the_others_still_decompress() {
        let dir = tempfile::tempdir().unwrap();
        let (good, missing) = (dir.path().join("good.zst"), dir.path().join("missing.zst"));
        fs::write(&good, compressed(b"good")).unwrap();
        let (code, output, errors) = ran(&["-dc", &path(&missing), &path(&good)], b"");
        assert_eq!((code, output.as_slice()), (FAILED, b"good".as_slice()));
        assert!(errors.contains("missing.zst"), "{errors}");
        assert_eq!(ran(&["-dc", &path(dir.path())], b"").0, FAILED, "a folder is no input");
    }

    #[test]
    fn t_checks_each_input_and_writes_nothing() {
        let frame = compressed(b"checked");
        assert_eq!(ran(&["-t", "-q"], &frame), (DONE, Vec::new(), String::new()));
        assert_eq!(ran(&["-dt"], &frame[..frame.len() - 2]).0, FAILED);
    }

    #[test]
    fn a_frame_s_window_is_held_to_what_the_options_allow() {
        // Written a part at a time, the frame cannot say its size, and its
        // window stays the 256 MiB it was made with.
        let mut frame = Vec::new();
        let mut encoder = zstd::stream::write::Encoder::new(&mut frame, 3).unwrap();
        encoder.window_log(28).unwrap();
        encoder.write_all(b"a frame made ").unwrap();
        encoder.write_all(b"with a large window").unwrap();
        encoder.finish().unwrap();
        let data = b"a frame made with a large window".to_vec();
        let (code, _, errors) = ran(&["-dc"], &frame);
        assert_eq!(code, FAILED);
        assert!(errors.contains("memory"), "{errors}");
        assert_eq!(ran(&["-dc", "--long"], &frame).0, FAILED);
        for args in [&["-dc", "--long=28"][..], &["-dc", "-M256MB"], &["-dcM256M"], &["-dc", "--memory=256MiB"], &["-dc", "--memory=200MB"]] {
            assert_eq!(ran(args, &frame), (DONE, data.clone(), String::new()), "{args:?}");
        }
        assert_eq!(ran(&["-dc", "--memory=128MB"], &frame).0, FAILED);
    }

    #[test]
    fn compressing_and_options_it_does_not_have_are_refused() {
        for args in [
            &[][..],
            &["-c"],
            &["-z"],
            &["-19"],
            &["-d", "-l"],
            &["-d", "-D", "dictionary"],
            &["-d", "-r"],
            &["-d", "--ultra"],
            &["-d", "--no-check"],
            &["-d", "-o"],
            &["-dox", "out"],
            &["-d", "--long=40"],
            &["-d", "--long=x"],
            &["-d", "-Mlots"],
            &["-d", "--memory=12TB"],
            &["-d", "--threads=all"],
        ] {
            let (code, output, errors) = ran(args, &compressed(b"x"));
            assert_eq!((code, output.len()), (FAILED, 0), "{args:?}");
            assert!(errors.starts_with("zstd: "), "{args:?}: {errors}");
        }
    }

    #[test]
    fn it_tells_what_it_is_and_how_to_run_it() {
        let (code, output, _) = ran(&["-V"], b"");
        assert_eq!(code, DONE);
        assert!(String::from_utf8(output).unwrap().contains("libzstd 1."));
        let (code, output, _) = ran(&["--help"], b"");
        assert_eq!(code, DONE);
        assert!(String::from_utf8(output).unwrap().starts_with("Usage: zstd -d"));
    }

    #[test]
    fn after_two_dashes_a_dash_starts_a_file_name() {
        let args: Vec<OsString> = ["-d", "--", "-q.zst", "--"].iter().map(OsString::from).collect();
        let Ok(Asked::Decompress(request)) = parse(&args) else { panic!("{args:?} asks to decompress") };
        assert_eq!(request.inputs, [Input::File("-q.zst".into()), Input::File("--".into())]);
        assert_eq!(request.verbosity, 2, "-q names a file there");
    }
}
