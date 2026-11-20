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
    /// The server could not or would not look the name up, and says so
    /// with this code: it failed at it, or turns the asker away. Another
    /// server may know the name.
    Unanswered(ResponseCode),
}

/// The answer `packet` gives to question `id`, after the names it points
/// to; a packet that answers another question gives none. A name the
/// server says does not exist, or has no IPv4 address, is an error: it is
/// what any server says of it.
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
        other => return Ok(Some(Answer::Unanswered(other))),
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
    let servers: Vec<SocketAddrV4> = servers.iter().map(|server| SocketAddrV4::new(*server, PORT)).collect();
    ask_in_turn(host, &servers)
}

/// The IPv4 addresses of `host`, asked of the servers at `servers` one
/// after another until one gives them; where none does, why the last did
/// not.
fn ask_in_turn(host: &str, servers: &[SocketAddrV4]) -> Result<Vec<Ipv4Addr>> {
    if servers.is_empty() {
        bail!("the network gave no DNS server");
    }
    let mut last = None;
    for server in servers {
        match ask_server(host, *server)? {
            Ok(found) => return Ok(found),
            Err(why) => last = Some(why),
        }
    }
    Err(last.expect("there was a server to ask"))
}

/// Ask the server at `at` over UDP, then over TCP where no whole answer
/// comes over UDP, as some networks let a question out over UDP and never
/// let its answer back in: the addresses of `host`, or why the server gave
/// none, which is for the next server to be asked: it did not answer, or
/// answered that it could not or would not look the name up. A name that
/// is not there is an error, as it is not there at the next server either.
fn ask_server(host: &str, at: SocketAddrV4) -> Result<Result<Vec<Ipv4Addr>, anyhow::Error>> {
    let unanswered = |code: ResponseCode| anyhow::anyhow!("the DNS server at {} answered {code} for {host}", at.ip());
    match ask(host, at)? {
        Some(Answer::Addresses(found)) => return Ok(Ok(found)),
        Some(Answer::Unanswered(code)) => return Ok(Err(unanswered(code))),
        Some(Answer::Truncated) | None => {}
    }
    Ok(match ask_tcp(host, at)? {
        Ok(Answer::Addresses(found)) => Ok(found),
        Ok(Answer::Unanswered(code)) => Err(unanswered(code)),
        Ok(Answer::Truncated) => bail!("the DNS server at {at} cut its answer short over TCP too"),
        Err(why) => Err(anyhow::anyhow!("the DNS server at {} did not answer over UDP, nor over TCP: {why}", at.ip())),
    })
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
/// length in two bytes: its answer for `host`, or why the server could not
/// be asked or its answer read.
fn ask_tcp(host: &str, at: SocketAddrV4) -> Result<Result<Answer, std::io::Error>> {
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
        Some(answer) => Ok(Ok(answer)),
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

    /// What a server answers to `asked` that gives `code` and no addresses.
    fn coded(asked: &[u8], code: ResponseCode) -> Vec<u8> {
        let q = Message::from_vec(asked).unwrap();
        let mut m = Message::new(q.metadata.id, MessageType::Response, OpCode::Query);
        m.metadata.response_code = code;
        m.add_query(q.queries[0].clone());
        m.to_vec().unwrap()
    }

    /// A server on this computer that answers the one question it is
    /// asked over UDP as `reply` makes of it, and where it listens.
    fn serving(reply: impl FnOnce(&[u8]) -> Vec<u8> + Send + 'static) -> (std::thread::JoinHandle<()>, SocketAddrV4) {
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let at = SocketAddrV4::new(Ipv4Addr::LOCALHOST, udp.local_addr().unwrap().port());
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let (n, from) = udp.recv_from(&mut buf).unwrap();
            udp.send_to(&reply(&buf[..n]), from).unwrap();
        });
        (server, at)
    }

    #[test]
    fn a_server_that_will_not_look_the_name_up_says_so_with_its_code() {
        let q = question(4, "update.example.com").unwrap();
        for code in [ResponseCode::ServFail, ResponseCode::Refused] {
            assert_eq!(answer(&coded(&q, code), 4, "update.example.com").unwrap(), Some(Answer::Unanswered(code)));
        }
    }

    /// The first server the network gave fails at the question, or turns
    /// the phone away, and the second knows the name.
    #[test]
    fn the_next_server_is_asked_where_one_will_not_look_the_name_up() {
        for code in [ResponseCode::ServFail, ResponseCode::Refused] {
            let (unwilling, first) = serving(move |asked| coded(asked, code));
            let (knowing, second) = serving(|asked| served(asked, &ADDRESSES, false));
            assert_eq!(ask_in_turn("update.example.com", &[first, second]).unwrap(), ADDRESSES, "after {code}");
            unwilling.join().unwrap();
            knowing.join().unwrap();
        }
    }

    #[test]
    fn where_no_server_will_look_the_name_up_the_last_one_s_code_is_given() {
        let (failing, first) = serving(|asked| coded(asked, ResponseCode::ServFail));
        let (refusing, second) = serving(|asked| coded(asked, ResponseCode::Refused));
        let why = ask_in_turn("update.example.com", &[first, second]).unwrap_err().to_string();
        assert_eq!(why, format!("the DNS server at 127.0.0.1 answered {} for update.example.com", ResponseCode::Refused));
        failing.join().unwrap();
        refusing.join().unwrap();
    }

    /// A name that does not exist does not exist at the next server
    /// either, which is asked nothing.
    #[test]
    fn a_name_the_first_server_says_does_not_exist_is_asked_of_no_other() {
        let (denying, first) = serving(|asked| coded(asked, ResponseCode::NXDomain));
        let unasked = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        unasked.set_nonblocking(true).unwrap();
        let second = SocketAddrV4::new(Ipv4Addr::LOCALHOST, unasked.local_addr().unwrap().port());
        let why = ask_in_turn("nothing.example", &[first, second]).unwrap_err().to_string();
        assert_eq!(why, "nothing.example does not exist");
        denying.join().unwrap();
        let mut buf = [0u8; 1500];
        assert_eq!(unasked.recv_from(&mut buf).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
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
