//! resetprop as install scripts run it: to read the phone's properties, as
//! scripts do where they find no getprop, and to set one, through init. A
//! read-only property init has set already stays as it is: nothing here
//! writes the properties' memory behind init's back.

use super::Io;
use std::ffi::OsString;

/// How resetprop ends.
const DONE: i32 = 0;
const FAILED: i32 = 1;

const HELP: &str = "\
Usage: resetprop [-n] [-v] [NAME [VALUE]]

With no name, prints every property as [name]: [value]. With a name,
prints its value, and ends in failure when it is unset. With a value too,
sets the property through init, which sets a read-only one only once.

  -n, -v       taken as other builds of resetprop take them: the value is
               set through init all the same, and nothing more is told
  -h, --help   show this
";

/// What the command line asks of resetprop.
#[derive(Debug, PartialEq)]
enum Asked {
    All,
    Get(String),
    Set(String, String),
    Help,
}

pub(super) fn run(args: &[OsString], io: &mut Io) -> i32 {
    match parse(args) {
        Ok(Asked::Help) => say(io, HELP),
        Ok(Asked::All) => {
            let mut all = skyward_sys::props("");
            all.sort();
            say(io, &all.iter().map(|(name, value)| format!("[{name}]: [{value}]\n")).collect::<String>())
        }
        // An unset property and an empty one are alike to init.
        Ok(Asked::Get(name)) => match skyward_sys::prop(&name) {
            Some(value) => say(io, &format!("{value}\n")),
            None => FAILED,
        },
        Ok(Asked::Set(name, value)) => match set(&name, &value) {
            Ok(()) => DONE,
            Err(e) => {
                let _ = writeln!(io.errors, "resetprop: {e}");
                FAILED
            }
        },
        Err(e) => {
            let _ = writeln!(io.errors, "resetprop: {e}");
            FAILED
        }
    }
}

fn say(io: &mut Io, text: &str) -> i32 {
    match io.output.write_all(text.as_bytes()).and_then(|()| io.output.flush()) {
        Ok(()) => DONE,
        Err(_) => FAILED,
    }
}

/// What `args` ask: switches first, then a name and a value, which may
/// start with a dash.
fn parse(args: &[OsString]) -> Result<Asked, String> {
    let mut words = Vec::new();
    for arg in args {
        let arg = arg.to_str().ok_or_else(|| format!("{arg:?} is not UTF-8"))?;
        if !words.is_empty() || !arg.starts_with('-') || arg == "-" {
            words.push(arg.to_string());
            continue;
        }
        match arg {
            "--help" => return Ok(Asked::Help),
            "--verbose" => {}
            _ if arg.starts_with("--") => return Err(format!("{arg} is not supported: properties are read here, and set through init")),
            _ => {
                for switch in arg[1..].chars() {
                    match switch {
                        'h' => return Ok(Asked::Help),
                        'n' | 'v' => {}
                        _ => return Err(format!("-{switch} is not supported: properties are read here, and set through init")),
                    }
                }
            }
        }
    }
    let mut words = words.into_iter();
    match (words.next(), words.next(), words.next()) {
        (None, ..) => Ok(Asked::All),
        (Some(name), None, _) => Ok(Asked::Get(name)),
        (Some(name), Some(value), None) => Ok(Asked::Set(name, value)),
        _ => Err("give a name and a value at most".into()),
    }
}

/// Set `name` to `value` through init, which sets a read-only property once
/// only: setting one to the value it has already is no change.
fn set(name: &str, value: &str) -> Result<(), String> {
    if name.starts_with("ro.") {
        if let Some(now) = skyward_sys::prop(name) {
            if now == value {
                return Ok(());
            }
            return Err(format!("{name} is read-only, and init has set it to {now:?} already"));
        }
    }
    skyward_sys::set_prop(name, value).map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Asked, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn no_name_reads_them_all_a_name_reads_one_and_a_value_sets_it() {
        assert_eq!(parsed(&[]), Ok(Asked::All));
        assert_eq!(parsed(&["ro.product.device"]), Ok(Asked::Get("ro.product.device".into())));
        assert_eq!(parsed(&["persist.sys.usb.config", "mtp,adb"]), Ok(Asked::Set("persist.sys.usb.config".into(), "mtp,adb".into())));
        assert!(parsed(&["a", "b", "c"]).is_err());
    }

    #[test]
    fn switches_come_first_and_a_value_may_start_with_a_dash() {
        assert_eq!(parsed(&["-n", "debug.offset", "-1"]), Ok(Asked::Set("debug.offset".into(), "-1".into())));
        assert_eq!(parsed(&["-nv", "--verbose", "ro.boot.slot_suffix"]), Ok(Asked::Get("ro.boot.slot_suffix".into())));
        assert_eq!(parsed(&["-v", "-"]), Ok(Asked::Get("-".into())));
        assert_eq!(parsed(&["-h"]), Ok(Asked::Help));
        assert_eq!(parsed(&["--help", "ro.x"]), Ok(Asked::Help));
    }

    #[test]
    fn what_needs_writing_behind_init_is_refused() {
        for args in [&["-d", "ro.x"][..], &["--delete", "ro.x"], &["-f", "props.txt"], &["--file", "props.txt"], &["-w", "sys.boot_completed"], &["-Z", "ro.x"], &["-p"], &["-np", "persist.x", "1"]] {
            let e = parsed(args).unwrap_err();
            assert!(e.contains("is not supported"), "{args:?}: {e}");
        }
    }
}
