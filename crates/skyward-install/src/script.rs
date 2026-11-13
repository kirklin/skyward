//! What an install script tells the recovery while it runs, a line at a
//! time over the pipe it is handed, as Android's recovery hears it; and the
//! progress bar those lines move.

use std::time::Duration;

/// The kernel's shebang interpreter, without its optional argument.
/// Spaces and tabs delimit it; a carriage return is part of its name,
/// so a CRLF script must not appear to name a different interpreter.
pub fn interpreter(prefix: &[u8]) -> anyhow::Result<Option<&[u8]>> {
    let Some(rest) = prefix.strip_prefix(b"#!") else { return Ok(None) };
    let rest = &rest[rest.iter().take_while(|b| matches!(b, b' ' | b'\t')).count()..];
    let end = rest.iter().position(|b| matches!(b, b' ' | b'\t' | b'\n' | 0));
    let name = &rest[..end.unwrap_or(rest.len())];
    anyhow::ensure!(!name.is_empty(), "install script names no interpreter");
    anyhow::ensure!(end.is_some() || prefix.len() < 256, "install script interpreter name is too long");
    Ok(Some(name))
}

#[cfg(test)]
mod interpreter_tests {
    use super::interpreter;

    #[test]
    fn bash_and_optional_arguments_are_not_confused_with_sh() {
        assert_eq!(interpreter(b"#!/sbin/bash\nprintf ok\n").unwrap(), Some(&b"/sbin/bash"[..]));
        assert_eq!(interpreter(b"#! \t/system/bin/sh -e\nprintf ok").unwrap(), Some(&b"/system/bin/sh"[..]));
        assert_eq!(interpreter(b"#!/sbin/bash\r\n").unwrap(), Some(&b"/sbin/bash\r"[..]));
        assert_eq!(interpreter(b"\x7fELF").unwrap(), None);
    }

    #[test]
    fn missing_and_truncated_interpreters_are_refused() {
        assert!(interpreter(b"#!\n").is_err());
        assert!(interpreter(b"#!\nthis is not an interpreter").is_err());
        assert!(interpreter(&[b"#!".as_slice(), &[b'x'; 254]].concat()).is_err());
    }
}

/// What a line the script wrote asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The bar's next `share` is this step's, filled by itself over
    /// `seconds` when they are more than none, and by `set_progress`
    /// otherwise.
    Progress { share: f32, seconds: f32 },
    /// How far through its step the script is, 0 to 1.
    SetProgress(f32),
    /// A line to show the user. An empty one only ends the line before
    /// it, in the recoveries these scripts are written for.
    Print(String),
    /// Erase the cache partition once the script has succeeded.
    WipeCache,
    /// A line for the log alone.
    Log(String),
    /// Something a recovery with another kind of screen does, which this
    /// one takes note of: clear_display, enable_reboot or retry_update.
    Other(String),
}

/// The request in `line`, without its line ending; `None` for one this
/// recovery does not know, or whose numbers do not read.
pub fn parse(line: &str) -> Option<Request> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (command, args) = line.split_once(' ').unwrap_or((line, ""));
    let numbers = || args.split_whitespace().map(|n| n.parse::<f32>().ok()).collect::<Option<Vec<f32>>>();
    match command {
        "progress" => match numbers()?.as_slice() {
            [share, seconds] => Some(Request::Progress { share: *share, seconds: *seconds }),
            _ => None,
        },
        "set_progress" => match numbers()?.as_slice() {
            [fraction] => Some(Request::SetProgress(*fraction)),
            _ => None,
        },
        // What follows the command shows as it was written, its leading
        // spaces too, as recoveries that run these scripts show it.
        "ui_print" => Some(Request::Print(args.to_string())),
        "wipe_cache" => Some(Request::WipeCache),
        "log" => Some(Request::Log(args.trim().to_string())),
        "clear_display" | "enable_reboot" | "retry_update" => Some(Request::Other(command.to_string())),
        _ => None,
    }
}

/// The bar as the script moves it: each `progress` begins a step after the
/// last one's, and a step goes forward only, by `set_progress` or by the
/// time it said it takes, whichever is further. A `set_progress` before any
/// step moves the whole bar.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bar {
    start: f32,
    size: f32,
    fraction: f32,
    seconds: f32,
    /// When the step under way began, from when the script started.
    began: Duration,
    stepped: bool,
    /// How far a `set_progress` before any step moved the whole bar.
    loose: Option<f32>,
}

impl Bar {
    /// Hear a request at `at`, from when the script started; only those
    /// that move the bar change it.
    pub fn hear(&mut self, request: &Request, at: Duration) {
        match *request {
            Request::Progress { share, seconds } => {
                self.start += self.size;
                self.size = share.max(0.0);
                self.fraction = 0.0;
                self.seconds = seconds;
                self.began = at;
                self.stepped = true;
            }
            Request::SetProgress(fraction) if self.stepped => self.fraction = self.fraction.max(fraction.clamp(0.0, 1.0)),
            Request::SetProgress(fraction) => self.loose = Some(fraction.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    /// How far the bar is at `at`, 0 to 1; none before the script first
    /// moves it.
    pub fn share(&self, at: Duration) -> Option<f32> {
        if !self.stepped {
            return self.loose;
        }
        let timed = match self.seconds > 0.0 {
            true => (at.saturating_sub(self.began).as_secs_f32() / self.seconds).min(1.0),
            false => 0.0,
        };
        Some((self.start + self.fraction.max(timed) * self.size).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_read_as_android_s_recovery_reads_them() {
        assert_eq!(parse("progress 0.5 10\n"), Some(Request::Progress { share: 0.5, seconds: 10.0 }));
        assert_eq!(parse("set_progress 0.25"), Some(Request::SetProgress(0.25)));
        assert_eq!(parse("ui_print - Installing"), Some(Request::Print("- Installing".into())));
        assert_eq!(parse("ui_print   *  Magisk  *\r\n"), Some(Request::Print("  *  Magisk  *".into())), "leading spaces stay");
        assert_eq!(parse("ui_print"), Some(Request::Print(String::new())), "an empty one");
        assert_eq!(parse("ui_print "), Some(Request::Print(String::new())));
        assert_eq!(parse("wipe_cache"), Some(Request::WipeCache));
        assert_eq!(parse("log E1001: Failed to update system image."), Some(Request::Log("E1001: Failed to update system image.".into())));
        assert_eq!(parse("enable_reboot"), Some(Request::Other("enable_reboot".into())));
    }

    #[test]
    fn what_does_not_read_asks_for_nothing() {
        assert_eq!(parse("progress 0.5"), None);
        assert_eq!(parse("progress half 10"), None);
        assert_eq!(parse("set_progress"), None);
        assert_eq!(parse("set_progress 0.1 0.2"), None);
        assert_eq!(parse("reboot_now"), None);
        assert_eq!(parse(""), None);
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn steps_follow_one_another_and_go_forward_only() {
        let mut bar = Bar::default();
        assert_eq!(bar.share(at(0)), None, "nothing until the script moves it");
        bar.hear(&Request::Progress { share: 0.5, seconds: 0.0 }, at(0));
        assert_eq!(bar.share(at(0)), Some(0.0));
        bar.hear(&Request::SetProgress(0.5), at(10));
        assert_eq!(bar.share(at(10)), Some(0.25));
        bar.hear(&Request::SetProgress(0.2), at(20));
        assert_eq!(bar.share(at(20)), Some(0.25), "it does not go back");
        bar.hear(&Request::Progress { share: 0.25, seconds: 0.0 }, at(30));
        bar.hear(&Request::SetProgress(1.0), at(40));
        assert_eq!(bar.share(at(40)), Some(0.75), "the second step begins where the first one's share ends");
        bar.hear(&Request::Progress { share: 0.25, seconds: 0.0 }, at(50));
        bar.hear(&Request::SetProgress(2.0), at(60));
        assert_eq!(bar.share(at(60)), Some(1.0), "a fraction past the end stops at it");
    }

    #[test]
    fn a_step_with_a_time_fills_by_itself() {
        let mut bar = Bar::default();
        bar.hear(&Request::Progress { share: 0.4, seconds: 10.0 }, at(1_000));
        assert_eq!(bar.share(at(6_000)), Some(0.2));
        bar.hear(&Request::SetProgress(0.75), at(6_000));
        assert_eq!(bar.share(at(6_000)), Some(0.3), "set further than the time has filled it");
        assert_eq!(bar.share(at(30_000)), Some(0.4), "filled, it waits at the step's end");
    }

    #[test]
    fn a_fraction_before_any_step_moves_the_whole_bar() {
        let mut bar = Bar::default();
        bar.hear(&Request::SetProgress(0.3), at(0));
        assert_eq!(bar.share(at(0)), Some(0.3));
        bar.hear(&Request::Print("- Done".into()), at(5));
        assert_eq!(bar.share(at(5)), Some(0.3), "printing moves nothing");
    }
}
