//! A shell on a pseudo-terminal, for the terminal page.

use anyhow::{bail, Context, Result};
use skyward_ui::app::TermEnd;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

/// The shell a recovery ramdisk carries.
const SHELL: &str = "/system/bin/sh";

/// How long a write waits for the shell to make room for what was typed.
const WRITE_TIMEOUT_MS: i32 = 1000;

pub struct Shell {
    /// The screen's side of the terminal.
    master: File,
    child: Child,
    /// Readable once the shell has ended.
    ended: OwnedFd,
}

impl Shell {
    /// Start the shell in `folder`, on a new pseudo-terminal of `rows` by
    /// `cols`. It leads a session of its own with the terminal as its
    /// controlling one, so Ctrl-C reaches whatever runs in the foreground.
    /// Its prompt names the phone, `codename`, and the folder it is in, as
    /// the system's own shell does over adb.
    pub fn start(rows: u16, cols: u16, codename: &str, folder: &Path) -> Result<Shell> {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
            return Err(io::Error::last_os_error()).context("open a pseudo-terminal");
        }
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        // Nothing the shell starts is to hold the screen's side open, and
        // the screen reads it only when there is something to read.
        let fd = master.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
            || flags < 0
            || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
        {
            return Err(io::Error::last_os_error()).context("set the pseudo-terminal up");
        }
        let mut command = Command::new(SHELL);
        command
            .current_dir(folder)
            .env("TERM", "xterm-256color")
            .env("HOME", "/")
            .env("HOSTNAME", codename)
            .env("PS1", "${HOSTNAME}:${PWD:-?} # ")
            // The socket init made for the recovery service means nothing to
            // what the shell runs.
            .env_remove("ANDROID_SOCKET_recovery")
            .stdin(Stdio::from(slave.try_clone().context("share the terminal")?))
            .stdout(Stdio::from(slave.try_clone().context("share the terminal")?))
            .stderr(Stdio::from(slave));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().with_context(|| format!("start {SHELL}"))?;
        let ended = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as libc::pid_t, 0) };
        if ended < 0 {
            return Err(io::Error::last_os_error()).context("watch the shell");
        }
        let ended = unsafe { OwnedFd::from_raw_fd(ended as RawFd) };
        Ok(Shell { master: File::from(master), child, ended })
    }

    /// What to wait on: the shell writing, and the shell ending.
    pub fn fds(&self) -> [RawFd; 2] {
        [self.master.as_raw_fd(), self.ended.as_raw_fd()]
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) } < 0 {
            return Err(io::Error::last_os_error()).context("resize the pseudo-terminal");
        }
        Ok(())
    }

    /// What the shell wrote since the last call. Once nothing holds the
    /// shell's side open, there is nothing more to read.
    pub fn read(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => return Ok(out),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(libc::EIO) => return Ok(out),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e).context("read what the shell wrote"),
            }
        }
    }

    /// Send the shell what was typed.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let mut at = 0;
        while at < bytes.len() {
            match self.master.write(&bytes[at..]) {
                Ok(n) => at += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let mut room = libc::pollfd { fd: self.master.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                    match unsafe { libc::poll(&mut room, 1, WRITE_TIMEOUT_MS) } {
                        n if n < 0 => return Err(io::Error::last_os_error()).context("wait for the shell to take what was typed"),
                        0 => bail!("the shell has taken nothing typed for {WRITE_TIMEOUT_MS} ms"),
                        _ => {}
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e).context("send the shell what was typed"),
            }
        }
        Ok(())
    }

    /// How the shell ended, once it has.
    pub fn ended(&mut self) -> Result<Option<ExitStatus>> {
        self.child.try_wait().context("see whether the shell has ended")
    }

    /// Whether the shell waits for a command: it, not a command it started,
    /// holds the terminal.
    pub fn waiting(&self) -> Result<bool> {
        let holder = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        if holder < 0 {
            return Err(io::Error::last_os_error()).context("ask who holds the terminal");
        }
        Ok(holder as u32 == self.child.id())
    }

    /// Have the shell, waiting for a command, go to `folder`: whatever was
    /// typed on its line is cleared, and the command shows as if typed.
    pub fn change_to(&mut self, folder: &str) -> Result<()> {
        self.write(&cd(folder))
    }
}

/// What clears a shell's line and takes it to `folder`, the folder's name
/// quoted for any character it holds.
fn cd(folder: &str) -> Vec<u8> {
    format!("\x15cd -- '{}'\r", folder.replace('\'', r"'\''")).into_bytes()
}

/// How a shell ended, as the terminal page tells it.
pub fn how(status: ExitStatus) -> TermEnd {
    match (status.code(), status.signal()) {
        (Some(code), _) => TermEnd::Exited(code),
        (None, Some(signal)) => TermEnd::Signalled(signal),
        (None, None) => TermEnd::Other(status.to_string()),
    }
}
