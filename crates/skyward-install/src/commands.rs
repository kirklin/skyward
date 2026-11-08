//! The commands install scripts written for other recoveries run that a
//! stock recovery does not carry: 7za, which takes a partition image out of
//! the zip the script came in, and zstd, which decompresses it on its way to
//! the partition; avbctl, which turns off verified boot's checks before a
//! script writes a system that would not pass them, and resetprop, which
//! reads the phone's properties and sets them; and unzip, as the stock
//! recovery's own but pouring out what `-p` takes as it reads it. The
//! ramdisk links each to the recovery service binary, which runs the one it
//! is started as. Unlike a copy of a command a script brings along, built
//! against libraries each phone's recovery carries in a version of its
//! own, these need none of them.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};

mod avbctl;
mod resetprop;
mod sevenzip;
mod unzip;
mod zstandard;

/// How much is read and written at a time: a whole partition image may go
/// through a command.
const CHUNK: usize = 1 << 20;

/// What a command reads and writes besides the files it names.
struct Io<'a> {
    input: &'a mut dyn Read,
    output: &'a mut dyn Write,
    errors: &'a mut dyn Write,
}

/// A command: its arguments, without its name, to the status it ends with.
type Command = fn(&[OsString], &mut Io) -> i32;

/// The command a program started as `name` is, if it is one of them.
fn command(name: &OsStr) -> Option<Command> {
    match name.to_str()? {
        "7za" => Some(sevenzip::run),
        "avbctl" => Some(avbctl::run),
        "resetprop" => Some(resetprop::run),
        "unzip" => Some(unzip::run),
        "zstd" => Some(zstandard::run),
        _ => None,
    }
}

/// Run the command this program was started as, `name`, with `args`, and
/// give the status it ends with; none when `name` is not one of them.
pub fn run(name: &OsStr, args: &[OsString]) -> Option<i32> {
    let command = command(name)?;
    // As with the programs these stand in for, writing to a pipe whose
    // reader has gone ends the command.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let (stdin, stdout, stderr) = (io::stdin(), io::stdout(), io::stderr());
    let (mut input, mut output, mut errors) = (stdin.lock(), stdout.lock(), stderr.lock());
    Some(command(args, &mut Io { input: &mut input, output: &mut output, errors: &mut errors }))
}

/// Where copying stopped.
#[derive(Debug)]
enum Failed {
    Reading(io::Error),
    Writing(io::Error),
}

/// Copy what is left of `from` to `to`, giving how much there was.
fn copy(from: &mut dyn Read, to: &mut dyn Write) -> Result<u64, Failed> {
    let mut chunk = vec![0; CHUNK];
    let mut copied = 0;
    loop {
        let read = match from.read(&mut chunk) {
            Ok(0) => return Ok(copied),
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Failed::Reading(e)),
        };
        to.write_all(&chunk[..read]).map_err(Failed::Writing)?;
        copied += read as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn every_command_the_ramdisk_links_is_one_this_binary_runs() {
        for link in skyward_config::ramdisk::COMMANDS {
            let name = Path::new(link).file_name().expect("a link has a name");
            assert!(command(name).is_some(), "{link}");
        }
        assert!(command(OsStr::new("recovery")).is_none());
        assert!(command(OsStr::new("zipinfo")).is_none(), "the stock recovery's own answers to that name");
    }
}
