//! The kernel's uevents, each telling of a device added, removed or
//! changed. Only that one came matters here: whatever follows a device
//! reads it anew.

use crate::sleep::Wakeful;
use anyhow::{Context, Result};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// The group the kernel sends its uevents to.
const KERNEL_GROUP: u32 = 1;

/// The kernel's uevent socket. A uevent keeps the phone awake until it is
/// read.
pub struct Listener {
    socket: OwnedFd,
    wakeful: Wakeful,
}

impl Listener {
    pub fn open() -> Result<Listener> {
        let fd = unsafe {
            libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, libc::NETLINK_KOBJECT_UEVENT)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("open the uevent socket");
        }
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        addr.nl_groups = KERNEL_GROUP;
        let len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        if unsafe { libc::bind(fd, (&addr as *const libc::sockaddr_nl).cast(), len) } != 0 {
            return Err(io::Error::last_os_error()).context("listen for the kernel's uevents");
        }
        let wakeful = Wakeful::new()?;
        wakeful.add(fd)?;
        Ok(Listener { socket, wakeful })
    }

    /// Readable while a uevent waits.
    pub fn fd(&self) -> RawFd {
        self.wakeful.fd()
    }

    /// Empty the socket: whether a uevent came, or so many came that the
    /// socket could not hold them all.
    pub fn heard(&self) -> Result<bool> {
        let mut buf = [0u8; 8192];
        let mut any = false;
        loop {
            let n = unsafe { libc::recv(self.socket.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n >= 0 {
                any = true;
                continue;
            }
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EAGAIN) => return Ok(any),
                Some(libc::EINTR) => {}
                Some(libc::ENOBUFS) => any = true,
                _ => return Err(err).context("read the kernel's uevents"),
            }
        }
    }
}
