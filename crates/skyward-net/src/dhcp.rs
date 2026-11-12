//! DHCPv4: asking the network for an address, and keeping it.

use anyhow::{Context, Result};
use dhcproto::v4::{DhcpOption, Flags, Message, MessageType, Opcode, OptionCode};
use dhcproto::{Decodable, Encodable};
use std::net::Ipv4Addr;

/// An address the network gave, and what came with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub address: Ipv4Addr,
    pub mask: Ipv4Addr,
    /// Where what is for other networks goes.
    pub router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    /// The server that gave it, which is asked first to keep it.
    pub server: Ipv4Addr,
    /// Seconds it lasts; after how many of them its server is asked to keep
    /// it, and after how many any server is.
    pub seconds: u32,
    pub renew: u32,
    pub rebind: u32,
}

/// What a server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// `server` offers `address`.
    Offer { address: Ipv4Addr, server: Ipv4Addr },
    Ack(Lease),
    /// The address asked for is not to be had.
    Nak,
}

// A duplicate OFFER may arrive after REQUEST. It is not a refusal: keep
// waiting for ACK/NAK within the same transaction.
#[cfg(any(target_os = "android", test))]
fn answers_phase(reply: &Reply, expect_offer: bool) -> bool {
    matches!((reply, expect_offer), (Reply::Offer { .. }, true) | (Reply::Ack(_) | Reply::Nak, false))
}

/// What every question asks to be told besides the address.
const ASKED: [OptionCode; 6] = [
    OptionCode::SubnetMask,
    OptionCode::Router,
    OptionCode::DomainNameServer,
    OptionCode::AddressLeaseTime,
    OptionCode::Renewal,
    OptionCode::Rebinding,
];

/// How long a lease lasts that does not say, which no server is to send:
/// an hour.
const LEASE_UNSAID: u32 = 3600;

fn question(xid: u32, mac: [u8; 6], kind: MessageType, broadcast: bool, ciaddr: Ipv4Addr) -> Message {
    let unspecified = Ipv4Addr::UNSPECIFIED;
    let mut m = Message::new_with_id(xid, ciaddr, unspecified, unspecified, unspecified, &mac);
    m.set_opcode(Opcode::BootRequest);
    if broadcast {
        m.set_flags(Flags::default().set_broadcast());
    }
    m.opts_mut().insert(DhcpOption::MessageType(kind));
    m.opts_mut().insert(DhcpOption::ParameterRequestList(ASKED.to_vec()));
    m
}

fn bytes(m: &Message) -> Result<Vec<u8>> {
    m.to_vec().context("put a DHCP message together")
}

/// Ask any server for an address. Answers are asked to be broadcast: until
/// it has an address, the interface takes nothing sent to one.
pub fn discover(xid: u32, mac: [u8; 6]) -> Result<Vec<u8>> {
    bytes(&question(xid, mac, MessageType::Discover, true, Ipv4Addr::UNSPECIFIED))
}

/// Take the address `server` offered.
pub fn request(xid: u32, mac: [u8; 6], address: Ipv4Addr, server: Ipv4Addr) -> Result<Vec<u8>> {
    let mut m = question(xid, mac, MessageType::Request, true, Ipv4Addr::UNSPECIFIED);
    m.opts_mut().insert(DhcpOption::RequestedIpAddress(address));
    m.opts_mut().insert(DhcpOption::ServerIdentifier(server));
    bytes(&m)
}

/// Ask to keep `address`, which the interface has: of the server that gave
/// it, sent to it, or of any server, broadcast once that one has not
/// answered in time.
pub fn extend(xid: u32, mac: [u8; 6], address: Ipv4Addr) -> Result<Vec<u8>> {
    bytes(&question(xid, mac, MessageType::Request, false, address))
}

/// What `packet` answers to the question `xid` asked for `mac`; a packet
/// that is no answer to it gives none.
pub fn reply(packet: &[u8], xid: u32, mac: [u8; 6]) -> Option<Reply> {
    let m = Message::from_bytes(packet).ok()?;
    if m.opcode() != Opcode::BootReply || m.xid() != xid || m.chaddr().get(..6) != Some(&mac[..]) {
        return None;
    }
    let opts = m.opts();
    let server = match opts.get(OptionCode::ServerIdentifier) {
        Some(DhcpOption::ServerIdentifier(at)) => *at,
        _ => m.siaddr(),
    };
    match opts.msg_type()? {
        MessageType::Offer => Some(Reply::Offer { address: m.yiaddr(), server }),
        MessageType::Nak => Some(Reply::Nak),
        MessageType::Ack => {
            let seconds = match opts.get(OptionCode::AddressLeaseTime) {
                Some(DhcpOption::AddressLeaseTime(s)) if *s > 0 => *s,
                _ => LEASE_UNSAID,
            };
            // Times a server gives out of order, or of none at all, are
            // taken as the standard shares of the lease instead.
            let renew = match opts.get(OptionCode::Renewal) {
                Some(DhcpOption::Renewal(s)) if *s > 0 && *s < seconds => *s,
                _ => seconds / 2,
            };
            let rebind = match opts.get(OptionCode::Rebinding) {
                Some(DhcpOption::Rebinding(s)) if *s > renew && *s < seconds => *s,
                _ => seconds / 8 * 7,
            };
            let mask = match opts.get(OptionCode::SubnetMask) {
                Some(DhcpOption::SubnetMask(mask)) => *mask,
                _ => natural_mask(m.yiaddr()),
            };
            let router = match opts.get(OptionCode::Router) {
                Some(DhcpOption::Router(routers)) => routers.first().copied(),
                _ => None,
            };
            let dns = match opts.get(OptionCode::DomainNameServer) {
                Some(DhcpOption::DomainNameServer(servers)) => servers.clone(),
                _ => Vec::new(),
            };
            Some(Reply::Ack(Lease { address: m.yiaddr(), mask, router, dns, server, seconds, renew, rebind }))
        }
        _ => None,
    }
}

/// The mask of the class an address is in, for a server that gives none.
fn natural_mask(address: Ipv4Addr) -> Ipv4Addr {
    match address.octets()[0] {
        0..=127 => Ipv4Addr::new(255, 0, 0, 0),
        128..=191 => Ipv4Addr::new(255, 255, 0, 0),
        _ => Ipv4Addr::new(255, 255, 255, 0),
    }
}

#[cfg(target_os = "android")]
pub use client::{obtain, keep, Kept};

#[cfg(target_os = "android")]
mod client {
    use super::{answers_phase, discover, extend, reply, request, Lease, Reply};
    use anyhow::{bail, Context, Result};
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    const CLIENT_PORT: u16 = 68;
    const SERVER_PORT: u16 = 67;

    /// How long each question waits for its answer, a try after another.
    const WAITS: [u64; 5] = [2, 4, 6, 8, 10];

    /// How often a wait looks whether it is to stop.
    const LOOK: Duration = Duration::from_millis(250);

    /// A socket on the DHCP client port of `interface` alone.
    fn socket(interface: &str) -> Result<UdpSocket> {
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("make a DHCP socket");
        }
        let s: UdpSocket = unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) };
        let on: libc::c_int = 1;
        let set = |level, name, value: *const libc::c_void, len| {
            if unsafe { libc::setsockopt(s.as_raw_fd(), level, name, value, len) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        };
        let len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        set(libc::SOL_SOCKET, libc::SO_REUSEADDR, (&on as *const libc::c_int).cast(), len).context("share the DHCP port")?;
        set(libc::SOL_SOCKET, libc::SO_BROADCAST, (&on as *const libc::c_int).cast(), len).context("let the DHCP socket broadcast")?;
        set(libc::SOL_SOCKET, libc::SO_BINDTODEVICE, interface.as_ptr().cast(), interface.len() as libc::socklen_t)
            .with_context(|| format!("keep the DHCP socket to {interface}"))?;
        let at = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: CLIENT_PORT.to_be(),
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        };
        let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        if unsafe { libc::bind(s.as_raw_fd(), (&at as *const libc::sockaddr_in).cast(), len) } != 0 {
            return Err(std::io::Error::last_os_error()).context("take the DHCP client port");
        }
        s.set_read_timeout(Some(LOOK)).context("time the DHCP socket")?;
        Ok(s)
    }

    /// A transaction's number, different from one question to the next.
    fn xid() -> u32 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        now.subsec_nanos() ^ (now.as_secs() as u32).rotate_left(13) ^ std::process::id()
    }

    /// Send `question` to `to` and wait for an answer to transaction `xid`,
    /// sending it again as long as none comes; give up after the last wait.
    fn exchange(s: &UdpSocket, to: SocketAddrV4, question: &[u8], xid: u32, mac: [u8; 6], stop: &AtomicBool, expect_offer: bool) -> Result<Option<Reply>> {
        let mut buf = [0u8; 1500];
        for wait in WAITS {
            s.send_to(question, to).context("send a DHCP question")?;
            let until = Instant::now() + Duration::from_secs(wait);
            while Instant::now() < until {
                if stop.load(Ordering::Relaxed) {
                    bail!(crate::Stopped);
                }
                match s.recv(&mut buf) {
                    Ok(n) => {
                        if let Some(r) = reply(&buf[..n], xid, mac).filter(|r| answers_phase(r, expect_offer)) {
                            return Ok(Some(r));
                        }
                    }
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {}
                    Err(e) => return Err(e).context("read a DHCP answer"),
                }
            }
        }
        Ok(None)
    }

    fn broadcast() -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::BROADCAST, SERVER_PORT)
    }

    /// Ask the network on `interface` for an address: an offer, then the
    /// offer taken. A server that takes back what it offered is asked
    /// again from the start.
    pub fn obtain(interface: &str, mac: [u8; 6], stop: &AtomicBool) -> Result<Lease> {
        let s = socket(interface)?;
        for _ in 0..3 {
            let id = xid();
            let (address, server) = match exchange(&s, broadcast(), &discover(id, mac)?, id, mac, stop, true)? {
                Some(Reply::Offer { address, server }) => (address, server),
                Some(_) => continue,
                None => bail!("no DHCP server offered an address"),
            };
            match exchange(&s, broadcast(), &request(id, mac, address, server)?, id, mac, stop, false)? {
                Some(Reply::Ack(lease)) => return Ok(lease),
                Some(_) => continue,
                None => bail!("the DHCP server at {server} did not give the address it offered"),
            }
        }
        bail!("the DHCP server kept taking back the address it offered")
    }

    /// How asking to keep a lease went.
    pub enum Kept {
        Kept(Lease),
        /// The server says the address is no longer to be had.
        Refused,
        /// No server answered.
        Unanswered,
    }

    /// Ask to keep `lease`: of the server that gave it, or, `anyone`, of
    /// any server.
    pub fn keep(interface: &str, mac: [u8; 6], lease: &Lease, anyone: bool, stop: &AtomicBool) -> Result<Kept> {
        let s = socket(interface)?;
        let id = xid();
        let to = if anyone { broadcast() } else { SocketAddrV4::new(lease.server, SERVER_PORT) };
        Ok(match exchange(&s, to, &extend(id, mac, lease.address)?, id, mac, stop, false)? {
            Some(Reply::Ack(kept)) => Kept::Kept(kept),
            Some(_) => Kept::Refused,
            None => Kept::Unanswered,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x04];

    #[test]
    fn a_duplicate_offer_is_ignored_while_waiting_for_ack() {
        let offer = Reply::Offer { address: Ipv4Addr::new(192, 168, 1, 23), server: Ipv4Addr::new(192, 168, 1, 1) };
        assert!(answers_phase(&offer, true));
        assert!(!answers_phase(&offer, false));
        assert!(!answers_phase(&Reply::Nak, true));
        assert!(answers_phase(&Reply::Nak, false));
    }

    /// What a home router answers, put together as one would.
    fn answer(kind: MessageType, xid: u32, mac: [u8; 6], with: &[DhcpOption]) -> Vec<u8> {
        let server = Ipv4Addr::new(192, 168, 1, 1);
        let mut m = Message::new_with_id(xid, Ipv4Addr::UNSPECIFIED, Ipv4Addr::new(192, 168, 1, 23), server, Ipv4Addr::UNSPECIFIED, &mac);
        m.set_opcode(Opcode::BootReply);
        m.opts_mut().insert(DhcpOption::MessageType(kind));
        m.opts_mut().insert(DhcpOption::ServerIdentifier(server));
        for option in with {
            m.opts_mut().insert(option.clone());
        }
        m.to_vec().unwrap()
    }

    #[test]
    fn questions_say_who_asks_and_what_they_want() {
        let asked = Message::from_bytes(&discover(7, MAC).unwrap()).unwrap();
        assert_eq!((asked.opcode(), asked.xid(), &asked.chaddr()[..6]), (Opcode::BootRequest, 7, &MAC[..]));
        assert!(asked.flags().broadcast(), "answers come broadcast before the interface has an address");
        assert_eq!(asked.opts().msg_type(), Some(MessageType::Discover));
        let taken = Message::from_bytes(&request(7, MAC, Ipv4Addr::new(192, 168, 1, 23), Ipv4Addr::new(192, 168, 1, 1)).unwrap()).unwrap();
        assert_eq!(taken.opts().get(OptionCode::RequestedIpAddress), Some(&DhcpOption::RequestedIpAddress(Ipv4Addr::new(192, 168, 1, 23))));
        assert_eq!(taken.opts().get(OptionCode::ServerIdentifier), Some(&DhcpOption::ServerIdentifier(Ipv4Addr::new(192, 168, 1, 1))));
        let kept = Message::from_bytes(&extend(8, MAC, Ipv4Addr::new(192, 168, 1, 23)).unwrap()).unwrap();
        assert_eq!(kept.ciaddr(), Ipv4Addr::new(192, 168, 1, 23));
        assert!(!kept.flags().broadcast(), "an interface with an address takes answers sent to it");
        assert_eq!(kept.opts().get(OptionCode::ServerIdentifier), None);
    }

    #[test]
    fn an_answer_gives_the_lease_and_what_came_with_it() {
        let offer = answer(MessageType::Offer, 7, MAC, &[]);
        assert_eq!(reply(&offer, 7, MAC), Some(Reply::Offer { address: Ipv4Addr::new(192, 168, 1, 23), server: Ipv4Addr::new(192, 168, 1, 1) }));
        let ack = answer(MessageType::Ack, 7, MAC, &[
            DhcpOption::SubnetMask(Ipv4Addr::new(255, 255, 255, 0)),
            DhcpOption::Router(vec![Ipv4Addr::new(192, 168, 1, 1)]),
            DhcpOption::DomainNameServer(vec![Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(114, 114, 114, 114)]),
            DhcpOption::AddressLeaseTime(86400),
        ]);
        let Some(Reply::Ack(lease)) = reply(&ack, 7, MAC) else { panic!("an ACK gives a lease") };
        assert_eq!(lease.address, Ipv4Addr::new(192, 168, 1, 23));
        assert_eq!(lease.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(lease.dns.len(), 2);
        assert_eq!((lease.seconds, lease.renew, lease.rebind), (86400, 43200, 75600), "times not said are the standard shares");
        let bare = answer(MessageType::Ack, 7, MAC, &[]);
        let Some(Reply::Ack(lease)) = reply(&bare, 7, MAC) else { panic!() };
        assert_eq!((lease.mask, lease.router, lease.seconds), (Ipv4Addr::new(255, 255, 255, 0), None, LEASE_UNSAID));
        let zero = answer(MessageType::Ack, 7, MAC, &[DhcpOption::AddressLeaseTime(7200), DhcpOption::Renewal(0), DhcpOption::Rebinding(0)]);
        let Some(Reply::Ack(lease)) = reply(&zero, 7, MAC) else { panic!() };
        assert_eq!((lease.renew, lease.rebind), (3600, 6300), "times of zero are the standard shares");
        let late = answer(MessageType::Ack, 7, MAC, &[DhcpOption::AddressLeaseTime(600), DhcpOption::Renewal(900), DhcpOption::Rebinding(200)]);
        let Some(Reply::Ack(lease)) = reply(&late, 7, MAC) else { panic!() };
        assert_eq!((lease.renew, lease.rebind), (300, 525), "and so are times past the lease's end or out of order");
        let none = answer(MessageType::Ack, 7, MAC, &[DhcpOption::AddressLeaseTime(0)]);
        let Some(Reply::Ack(lease)) = reply(&none, 7, MAC) else { panic!() };
        assert_eq!(lease.seconds, LEASE_UNSAID, "a lease of no time is taken as one that does not say");
        assert_eq!(reply(&answer(MessageType::Nak, 7, MAC, &[]), 7, MAC), Some(Reply::Nak));
    }

    #[test]
    fn answers_to_others_are_not_taken() {
        assert_eq!(reply(&answer(MessageType::Offer, 8, MAC, &[]), 7, MAC), None, "another transaction");
        assert_eq!(reply(&answer(MessageType::Offer, 7, [0; 6], &[]), 7, MAC), None, "another client");
        assert_eq!(reply(&discover(7, MAC).unwrap(), 7, MAC), None, "a question");
        assert_eq!(reply(b"not dhcp", 7, MAC), None);
    }
}
