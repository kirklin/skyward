//! A nonblocking Qualcomm SSC client, for the direction the phone is held in.
//!
//! No Android framework and no sensor HAL is needed: the kernel's QRTR
//! sockets reach the sensors' protection domain on the ADSP directly, and only
//! the small part of protobuf the sensor service speaks is parsed here.
//! `pd-mapper` has to be running, or the ADSP never loads that domain.
//!
//! Not used on the phone this was written for: on `bixi` (Xiaomi MIX Flip 2,
//! SM8750) the ADSP does publish SSC at node 5 port 18 once `pd-mapper` runs,
//! and it answers these lookups, but every acceleration name comes back with
//! no sensor UID — the accelerometer is served by Qualcomm's QSH/QESDK stack
//! instead, which needs the vendor sensor HAL and its linker namespace. The
//! code is kept because it is the shortest path to gravity on a phone whose
//! sensors are on plain SSC.
#![allow(dead_code)]

use anyhow::{bail, ensure, Context, Result};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

/// The socket family the kernel gives QRTR, not yet in libc.
const AF_QRTR: i32 = 42;
/// The name service every QRTR node answers on.
const CONTROL: u32 = 0xffff_fffe;
/// The sensors' service on the ADSP.
const SSC: u32 = 400;
/// The UID the sensor service answers lookups under.
const LOOKUP: [u64; 2] = [0xabab_abab_abab_abab; 2];
/// How phones name the sensor that reports acceleration.
const ACCEL_NAMES: [&str; 5] = ["accel", "accelerometer", "gravity", "linear_accel", "device_orientation"];

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Address {
    family: u16,
    node: u32,
    port: u32,
}

/// Where the phone is held, from the acceleration it feels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Direction {
    #[default]
    Flat,
    Portrait,
    UpsideDown,
    LandscapeLeft,
    LandscapeRight,
}

impl Direction {
    fn of(sample: [f32; 3]) -> Self {
        // Gravity, not motion: the phone lying flat is the only case where
        // neither axis clearly wins.
        let [x, y, _] = sample;
        const FLAT: f32 = 3.0;
        if x.abs() < FLAT && y.abs() < FLAT {
            Self::Flat
        } else if y <= -FLAT {
            Self::Portrait
        } else if y >= FLAT {
            Self::UpsideDown
        } else if x <= -FLAT {
            Self::LandscapeLeft
        } else {
            Self::LandscapeRight
        }
    }
}

struct Connection {
    socket: OwnedFd,
    server: Option<Address>,
    sensor: Option<[u64; 2]>,
    transaction: u16,
    heard: Instant,
    /// How many messages have been described to the log so far.
    notes: u32,
}

impl Connection {
    fn open() -> Result<Self> {
        let fd = unsafe { libc::socket(AF_QRTR, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
        ensure!(fd >= 0, "open QRTR: {}", io::Error::last_os_error());
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut local = Address::default();
        let mut size = std::mem::size_of::<Address>() as libc::socklen_t;
        let got = unsafe { libc::getsockname(fd, (&mut local as *mut Address).cast(), &mut size) };
        ensure!(got == 0, "read QRTR address: {}", io::Error::last_os_error());
        let connection = Self { socket, server: None, sensor: None, transaction: 0, heard: Instant::now(), notes: 0 };
        let lookup = [10u32, SSC, 0, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        connection.send(Address { family: AF_QRTR as u16, node: local.node, port: CONTROL }, &lookup)?;
        Ok(connection)
    }

    fn send(&self, address: Address, data: &[u8]) -> Result<()> {
        let sent = unsafe {
            libc::sendto(
                self.socket.as_raw_fd(),
                data.as_ptr().cast(),
                data.len(),
                0,
                (&address as *const Address).cast(),
                std::mem::size_of::<Address>() as libc::socklen_t,
            )
        };
        ensure!(sent == data.len() as isize, "send SSC request: {}", io::Error::last_os_error());
        Ok(())
    }

    fn request(&mut self, uid: [u64; 2], message: u32, payload: &[u8]) -> Result<()> {
        self.transaction = self.transaction.wrapping_add(1);
        self.send(self.server.context("SSC has no address")?, &request(self.transaction, uid, message, payload))
    }

    fn read(&mut self) -> Result<Vec<[f32; 3]>> {
        let mut samples = Vec::new();
        // Bound the work of a frame even if the service floods this socket.
        for _ in 0..128 {
            let mut buffer = [0u8; 65536];
            let mut from = Address::default();
            let mut size = std::mem::size_of::<Address>() as libc::socklen_t;
            let n = unsafe {
                libc::recvfrom(
                    self.socket.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    0,
                    (&mut from as *mut Address).cast(),
                    &mut size,
                )
            };
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("receive SSC data");
            }
            let data = &buffer[..n as usize];
            if from.port == CONTROL {
                if data.len() != 20 {
                    continue;
                }
                let words: Vec<_> = data.as_chunks::<4>().0.iter().map(|v| u32::from_le_bytes(*v)).collect();
                if words[0] == 4 && words[1] != SSC {
                    let _ = skyward_sys::kmsg(&format!(
                        "orientation: service {} is at node {} port {}",
                        words[1], words[3], words[4]
                    ));
                    continue;
                }
                if words[1] != SSC {
                    continue;
                }
                if words[0] == 5 {
                    bail!("the sensor service stopped");
                }
                if words[0] == 4 && self.server.is_none() {
                    self.server = Some(Address { family: AF_QRTR as u16, node: words[3], port: words[4] });
                    let _ = skyward_sys::kmsg(&format!("orientation: SSC is at node {} port {}", words[3], words[4]));
                    // Ask under every name a phone gives the sensor that measures
                    // acceleration, and take the first one the service knows.
                    for name in ACCEL_NAMES {
                        let mut query = Vec::new();
                        bytes(&mut query, 1, name.as_bytes());
                        query.extend_from_slice(&[0x10, 1, 0x18, 1]);
                        self.request(LOOKUP, 512, &query)?;
                    }
                }
                continue;
            }
            if !self.server.is_some_and(|s| s.node == from.node && s.port == from.port) {
                continue;
            }
            if self.notes < 10 {
                self.notes += 1;
                let head: Vec<String> =
                    data.iter().take(40).map(|b| format!("{b:02x}")).collect();
                let _ = skyward_sys::kmsg(&format!("orientation: raw {} bytes, {}", data.len(), head.join("")));
            }
            for (uid, message, payload) in events(data)? {
                if self.notes < 24 {
                    self.notes += 1;
                    let _ = skyward_sys::kmsg(&format!(
                        "orientation: uid {:x}/{:x} message {message} payload {} bytes",
                        uid[0],
                        uid[1],
                        payload.len()
                    ));
                }
                if uid == LOOKUP && message == 768 && self.sensor.is_none() {
                    let fields = fields(payload)?;
                    let names: Vec<_> = fields
                        .iter()
                        .filter(|&&(tag, wire, _)| tag == 1 && wire == 2)
                        .map(|&(_, _, value)| String::from_utf8_lossy(value).into_owned())
                        .collect();
                    let uid = fields
                        .iter()
                        .find_map(|&(tag, wire, value)| (tag == 2 && wire == 2).then(|| sensor_uid(value).ok()).flatten());
                    let _ = skyward_sys::kmsg(&format!("orientation: suid reply names {names:?} uid {uid:?}"));
                    if let Some(uid) = uid {
                        self.sensor = Some(uid);
                        let mut config = vec![0x0d]; // sns_std_sensor_config.sample_rate
                        config.extend_from_slice(&10f32.to_le_bytes());
                        self.request(uid, 513, &config)?;
                    }
                } else if Some(uid) == self.sensor && message == 1025 {
                    samples.push(acceleration(payload)?);
                    self.heard = Instant::now();
                }
            }
        }
        ensure!(self.heard.elapsed() < Duration::from_secs(10), "the sensor service stopped reporting");
        Ok(samples)
    }
}

/// Watch the phone's direction and keep the pages turned to it.
pub struct Sensor {
    connection: Option<Connection>,
    direction: Direction,
    next_try: Instant,
    next_log: Instant,
}

impl Default for Sensor {
    fn default() -> Self {
        Self { connection: None, direction: Direction::default(), next_try: Instant::now(), next_log: Instant::now() }
    }
}

impl Sensor {
    /// The raw descriptor to wait on, once the service is found.
    pub fn fd(&self) -> Option<RawFd> {
        self.connection.as_ref().map(|s| s.socket.as_raw_fd())
    }

    /// Take in what the service sent. Returns the direction when it changed.
    pub fn read(&mut self) -> Option<Direction> {
        if self.connection.is_none() {
            if Instant::now() < self.next_try {
                return None;
            }
            match Connection::open() {
                Ok(connection) => self.connection = Some(connection),
                Err(e) => {
                    let _ = skyward_sys::kmsg(&format!("orientation: {e:#}; retry in 5 seconds"));
                    self.next_try = Instant::now() + Duration::from_secs(5);
                    return None;
                }
            }
        }
        let result = self.connection.as_mut().map(|c| c.read());
        match result {
            Some(Ok(samples)) => {
                let mut changed = None;
                for sample in samples {
                    let next = Direction::of(sample);
                    if Instant::now() >= self.next_log {
                        let _ = skyward_sys::kmsg(&format!(
                            "orientation: x {:.1} y {:.1} z {:.1} -> {next:?}",
                            sample[0], sample[1], sample[2]
                        ));
                        self.next_log = Instant::now() + Duration::from_millis(300);
                    }
                    if next != self.direction && next != Direction::Flat {
                        self.direction = next;
                        changed = Some(next);
                    }
                }
                changed
            }
            Some(Err(e)) => {
                let _ = skyward_sys::kmsg(&format!("orientation: {e:#}; retry in 5 seconds"));
                self.connection = None;
                self.next_try = Instant::now() + Duration::from_secs(5);
                None
            }
            None => None,
        }
    }
}

/// Start watching the phone's direction. The protection-domain mapper has to
/// be running before the ADSP loads the sensors' domain, so it is started
/// first when nothing else has.
pub fn start() {
    let _ = std::thread::Builder::new().name("orientation".into()).spawn(|| {
        start_mapper();
        let mut sensor = Sensor::default();
        loop {
            let _ = sensor.read();
            std::thread::sleep(Duration::from_millis(30));
        }
    });
}

fn start_mapper() {
    let running = std::fs::read_dir("/proc").is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            std::fs::read_to_string(entry.path().join("cmdline")).is_ok_and(|line| line.contains("pd-mapper"))
        })
    });
    if running {
        return;
    }
    match std::process::Command::new("/vendor/bin/pd-mapper").spawn() {
        Ok(_) => {
            let _ = skyward_sys::kmsg("orientation: started pd-mapper");
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(e) => {
            let _ = skyward_sys::kmsg(&format!("orientation: pd-mapper is not started: {e}"));
        }
    }
}

// Only the small subset of protobuf the sensor service uses is parsed here.
// Every length and wire type is checked before slicing, and unknown fields
// are skipped, so a malformed message cannot panic the UI.

fn varint(data: &mut &[u8]) -> Result<u64> {
    let mut result = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = data.split_first().context("short protobuf varint")?;
        *data = rest;
        ensure!(shift < 63 || byte <= 1, "protobuf varint overflow");
        result |= (byte as u64 & 127) << shift;
        if byte & 128 == 0 {
            return Ok(result);
        }
    }
    bail!("protobuf varint overflow")
}

fn fields(mut data: &[u8]) -> Result<Vec<(u64, u8, &[u8])>> {
    let mut result = Vec::new();
    while !data.is_empty() {
        let key = varint(&mut data)?;
        ensure!(key >> 3 != 0, "protobuf field zero");
        let wire = (key & 7) as u8;
        let size = match wire {
            0 => {
                let before = data;
                varint(&mut data)?;
                result.push((key >> 3, wire, &before[..before.len() - data.len()]));
                continue;
            }
            1 => 8,
            2 => usize::try_from(varint(&mut data)?)?,
            5 => 4,
            _ => bail!("unsupported protobuf wire type {wire}"),
        };
        ensure!(size <= data.len(), "short protobuf field");
        let (value, rest) = data.split_at(size);
        result.push((key >> 3, wire, value));
        data = rest;
    }
    Ok(result)
}

fn sensor_uid(data: &[u8]) -> Result<[u64; 2]> {
    let fields = fields(data)?;
    let word = |tag| {
        fields
            .iter()
            .find(|&&(n, w, _)| n == tag && w == 1)
            .map(|&(_, _, v)| u64::from_le_bytes(v.try_into().unwrap()))
            .context("missing sensor UID")
    };
    Ok([word(1)?, word(2)?])
}

fn acceleration(data: &[u8]) -> Result<[f32; 3]> {
    let mut values = Vec::new();
    for (tag, wire, value) in fields(data)? {
        if tag != 1 {
            continue;
        }
        ensure!(wire == 2 || wire == 5, "invalid acceleration field");
        ensure!(value.len() % 4 == 0, "short acceleration float");
        values.extend(value.as_chunks::<4>().0.iter().map(|v| f32::from_le_bytes(*v)));
    }
    values.try_into().map_err(|_| anyhow::anyhow!("acceleration needs three axes"))
}

type SensorEvent<'a> = ([u64; 2], u32, &'a [u8]);

fn events(data: &[u8]) -> Result<Vec<SensorEvent<'_>>> {
    ensure!(data.len() >= 7, "short QMI header");
    let size = u16::from_le_bytes([data[5], data[6]]) as usize;
    ensure!(data.len() == size + 7, "short QMI message");
    let mut tlvs = &data[7..];
    let mut result = Vec::new();
    while !tlvs.is_empty() {
        ensure!(tlvs.len() >= 3, "short QMI TLV");
        let tag = tlvs[0];
        let size = u16::from_le_bytes([tlvs[1], tlvs[2]]) as usize;
        ensure!(size <= tlvs.len() - 3, "short QMI TLV value");
        let value = &tlvs[3..size + 3];
        tlvs = &tlvs[size + 3..];
        if data[0] == 2 && tag == 2 {
            ensure!(value == [0, 0, 0, 0], "the sensor service rejected a request: {value:?}");
        }
        let message = u16::from_le_bytes([data[3], data[4]]);
        if data[0] != 4 || ![0x21, 0x22].contains(&message) || tag != 2 {
            continue;
        }
        ensure!(value.len() >= 2, "missing SSC payload length");
        let size = u16::from_le_bytes([value[0], value[1]]) as usize;
        ensure!(size == value.len() - 2, "short SSC payload");
        let envelope = fields(&value[2..])?;
        let uid = envelope.iter().find(|&&(t, w, _)| t == 1 && w == 2).context("missing SSC UID")?;
        let uid = sensor_uid(uid.2)?;
        for &(_, _, event) in envelope.iter().filter(|&&(t, w, _)| t == 2 && w == 2) {
            let fields = fields(event)?;
            let message = fields.iter().find(|&&(t, w, _)| t == 1 && w == 5).context("missing SSC event ID")?;
            let payload = fields.iter().find(|&&(t, w, _)| t == 3 && w == 2).context("missing SSC event payload")?;
            result.push((uid, u32::from_le_bytes(message.2.try_into().unwrap()), payload.2));
        }
    }
    Ok(result)
}

fn bytes(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag * 8 + 2);
    let mut size = value.len();
    while size >= 128 {
        out.push((size as u8 & 127) | 128);
        size >>= 7;
    }
    out.push(size as u8);
    out.extend_from_slice(value);
}

fn request(transaction: u16, uid: [u64; 2], message: u32, payload: &[u8]) -> Vec<u8> {
    let mut id = vec![9];
    id.extend_from_slice(&uid[0].to_le_bytes());
    id.push(17);
    id.extend_from_slice(&uid[1].to_le_bytes());
    let mut envelope = Vec::new();
    bytes(&mut envelope, 1, &id);
    envelope.push(0x15);
    envelope.extend_from_slice(&message.to_le_bytes());
    bytes(&mut envelope, 3, &[8, 1, 16, 1]); // APSS, non-wakeup delivery
    let mut body = Vec::new();
    bytes(&mut body, 2, payload);
    bytes(&mut envelope, 4, &body);
    let mut qmi = vec![0];
    qmi.extend_from_slice(&transaction.to_le_bytes());
    qmi.extend_from_slice(&0x20u16.to_le_bytes());
    qmi.extend_from_slice(&(envelope.len() as u16 + 9).to_le_bytes());
    qmi.push(1);
    qmi.extend_from_slice(&(envelope.len() as u16 + 2).to_le_bytes());
    qmi.extend_from_slice(&(envelope.len() as u16).to_le_bytes());
    qmi.extend_from_slice(&envelope);
    qmi.extend_from_slice(&[0x10, 1, 0, 1]); // large indications
    qmi
}
