//! Looking a name up in DNS, of the servers the network gave: a recovery
//! has no resolver of its own to ask.

use anyhow::{bail, Context, Result};
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::time::Duration;

const PORT: u16 = 53;

/// How long a server gets to answer.
const WAIT: Duration = Duration::from_secs(3);

/// A question for the IPv4 addresses of `host`, numbered `id`.
pub fn question(id: u16, host: &str) -> Result<Vec<u8>> {
    let name = Name::from_ascii(host).with_context(|| format!("{host} is not a name DNS knows"))?;
    let mut m = Message::new(id, MessageType::Query, OpCode::Query);
    m.metadata.recursion_desired = true;
    m.add_query(Query::query(name, RecordType::A));
    m.to_vec().context("put a DNS question together")
}

/// What a server answered to question `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Addresses(Vec<Ipv4Addr>),
    /// The answer did not fit in one datagram, and is asked for again over
    /// TCP.
    Truncated,
}

/// The answer `packet` gives to question `id`, after the names it points
/// to; a packet that answers another question gives none.
pub fn answer(packet: &[u8], id: u16, host: &str) -> Result<Option<Answer>> {
    let Ok(m) = Message::from_vec(packet) else { return Ok(None) };
    if m.metadata.id != id || m.metadata.message_type != MessageType::Response {
        return Ok(None);
    }
    if m.metadata.truncation {
        return Ok(Some(Answer::Truncated));
    }
    match m.metadata.response_code {
        ResponseCode::NoError => {}
        ResponseCode::NXDomain => bail!("{host} does not exist"),
        other => bail!("the DNS server answered {other} for {host}"),
    }
    let addresses: Vec<Ipv4Addr> = m
        .answers
        .iter()
        .filter_map(|record| match &record.data {
            RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect();
    if addresses.is_empty() {
        bail!("{host} has no IPv4 address");
    }
    Ok(Some(Answer::Addresses(addresses)))
}

/// A question's number, different from one question to the next.
fn id() -> u16 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (now.subsec_nanos() ^ std::process::id()) as u16
}

/// The IPv4 addresses of `host`, asked of `servers` in turn.
pub fn lookup(host: &str, servers: &[Ipv4Addr]) -> Result<Vec<Ipv4Addr>> {
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        return Ok(vec![address]);
    }
    if servers.is_empty() {
        bail!("the network gave no DNS server");
    }
    let mut last = None;
    for server in servers {
        match ask_server(host, SocketAddrV4::new(*server, PORT))? {
            Ok(found) => return Ok(found),
            Err(why) => last = Some(why),
        }
    }
    Err(last.expect("there was a server to ask"))
}

/// Ask the server at `at` over UDP, then over TCP where no whole answer
/// comes over UDP, as some networks let a question out over UDP and never
/// let its answer back in: the addresses of `host`, or why the server gave
/// none. A name that is not there is an error, as it is not there at the
/// next server either.
fn ask_server(host: &str, at: SocketAddrV4) -> Result<Result<Vec<Ipv4Addr>, anyhow::Error>> {
    if let Some(Answer::Addresses(found)) = ask(host, at)? {
        return Ok(Ok(found));
    }
    Ok(ask_tcp(host, at)?.map_err(|why| anyhow::anyhow!("the DNS server at {} did not answer over UDP, nor over TCP: {why}", at.ip())))
}

/// Ask the server at `at` over UDP; nothing heard in time gives none.
fn ask(host: &str, at: SocketAddrV4) -> Result<Option<Answer>> {
    let id = id();
    let q = question(id, host)?;
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).context("make a DNS socket")?;
    s.set_read_timeout(Some(WAIT)).context("time the DNS socket")?;
    s.send_to(&q, at).with_context(|| format!("ask the DNS server at {at}"))?;
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = match s.recv_from(&mut buf) {
            Ok(got) => got,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).with_context(|| format!("read the answer of the DNS server at {at}")),
        };
        if from != SocketAddr::V4(at) {
            continue;
        }
        if let Some(answer) = answer(&buf[..n], id, host)? {
            return Ok(Some(answer));
        }
    }
}

/// Ask the server at `at` over TCP, where each message goes after its
/// length in two bytes: its addresses for `host`, or why the server could
/// not be asked or its answer read.
fn ask_tcp(host: &str, at: SocketAddrV4) -> Result<Result<Vec<Ipv4Addr>, std::io::Error>> {
    let id = id();
    let q = question(id, host)?;
    let exchange = || -> std::io::Result<Vec<u8>> {
        let mut s = TcpStream::connect_timeout(&SocketAddr::V4(at), WAIT)?;
        s.set_read_timeout(Some(WAIT))?;
        s.set_write_timeout(Some(WAIT))?;
        let len = u16::try_from(q.len()).map_err(std::io::Error::other)?;
        s.write_all(&len.to_be_bytes())?;
        s.write_all(&q)?;
        let mut len = [0u8; 2];
        s.read_exact(&mut len)?;
        let mut packet = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut packet)?;
        Ok(packet)
    };
    let packet = match exchange() {
        Ok(packet) => packet,
        Err(e) => return Ok(Err(e)),
    };
    match answer(&packet, id, host)? {
        Some(Answer::Addresses(found)) => Ok(Ok(found)),
        Some(Answer::Truncated) => bail!("the DNS server at {at} cut its answer short over TCP too"),
        None => bail!("the DNS server at {at} answered another question"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::{A, CNAME};
    use hickory_proto::rr::Record;

    /// What a server answers to `asked`: `host` named on to another name,
    /// and that name's addresses.
    fn served(asked: &[u8], addresses: &[Ipv4Addr], truncated: bool) -> Vec<u8> {
        let q = Message::from_vec(asked).unwrap();
        let mut m = Message::new(q.metadata.id, MessageType::Response, OpCode::Query);
        let name = q.queries[0].name.clone();
        let other = Name::from_ascii("ota.example.cdn.").unwrap();
        m.metadata.truncation = truncated;
        m.add_query(q.queries[0].clone());
        m.answers.push(Record::from_rdata(name, 60, RData::CNAME(CNAME(other.clone()))));
        for a in addresses {
            m.answers.push(Record::from_rdata(other.clone(), 60, RData::A(A(*a))));
        }
        m.to_vec().unwrap()
    }

    #[test]
    fn an_answer_gives_the_addresses_after_the_names_it_points_to() {
        let q = question(9, "update.example.com").unwrap();
        let reply = served(&q, &[Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8)], false);
        assert_eq!(
            answer(&reply, 9, "update.example.com").unwrap(),
            Some(Answer::Addresses(vec![Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8)]))
        );
        assert_eq!(answer(&reply, 10, "update.example.com").unwrap(), None, "another question's answer");
        assert_eq!(answer(&q, 9, "update.example.com").unwrap(), None, "the question itself");
        assert!(answer(&served(&q, &[], false), 9, "update.example.com").is_err(), "a name with no address");
        assert_eq!(answer(&served(&q, &[], true), 9, "update.example.com").unwrap(), Some(Answer::Truncated));
    }

    #[test]
    fn a_name_that_does_not_exist_is_said_to_be_so() {
        let q = Message::from_vec(&question(3, "nothing.example").unwrap()).unwrap();
        let mut m = Message::new(3, MessageType::Response, OpCode::Query);
        m.metadata.response_code = ResponseCode::NXDomain;
        m.add_query(q.queries[0].clone());
        let e = answer(&m.to_vec().unwrap(), 3, "nothing.example").unwrap_err();
        assert_eq!(e.to_string(), "nothing.example does not exist");
    }

    #[test]
    fn an_address_needs_no_server() {
        assert_eq!(lookup("192.168.1.1", &[]).unwrap(), [Ipv4Addr::new(192, 168, 1, 1)]);
        assert!(lookup("example.com", &[]).is_err());
    }

    const ADDRESSES: [Ipv4Addr; 2] = [Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)];

    /// Answer the one question that comes over TCP to `tcp`, whole.
    fn answer_over_tcp(tcp: std::net::TcpListener) {
        let (mut s, _) = tcp.accept().unwrap();
        let mut len = [0u8; 2];
        s.read_exact(&mut len).unwrap();
        let mut q = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut q).unwrap();
        let reply = served(&q, &ADDRESSES, false);
        s.write_all(&(reply.len() as u16).to_be_bytes()).unwrap();
        s.write_all(&reply).unwrap();
    }

    /// A server on this computer that answers the way one on the network
    /// does: over UDP with the answer cut short, then whole over TCP.
    #[test]
    fn an_answer_too_long_for_a_datagram_is_asked_for_again_over_tcp() {
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = udp.local_addr().unwrap().port();
        let tcp = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let (n, from) = udp.recv_from(&mut buf).unwrap();
            udp.send_to(&served(&buf[..n], &[], true), from).unwrap();
            answer_over_tcp(tcp);
        });
        let found = ask_server("update.example.com", SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
        server.join().unwrap();
        assert_eq!(found.unwrap(), ADDRESSES);
    }

    /// A server behind a network that lets questions out over UDP and
    /// never lets the answers back in.
    #[test]
    fn a_server_whose_answers_never_come_over_udp_is_asked_over_tcp() {
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = udp.local_addr().unwrap().port();
        let tcp = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        let server = std::thread::spawn(move || answer_over_tcp(tcp));
        let found = ask_server("update.example.com", SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
        server.join().unwrap();
        assert_eq!(found.unwrap(), ADDRESSES);
        drop(udp);
    }

    #[test]
    fn a_server_that_answers_neither_way_says_why() {
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = udp.local_addr().unwrap().port();
        let why = ask_server("update.example.com", SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap().unwrap_err().to_string();
        assert!(why.starts_with("the DNS server at 127.0.0.1 did not answer over UDP, nor over TCP: "), "{why}");
        drop(udp);
    }
}
