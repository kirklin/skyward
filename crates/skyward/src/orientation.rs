//! Nonblocking Qualcomm SSC client. No Android framework or sensor HAL is needed.
//! Wire format: sns_client_api_v01 QMI, sns_client/sns_suid/sns_std_sensor protobufs.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use anyhow::{bail, ensure, Context, Result};
use skyward_config::Orientation;
use skyward_ui::rotation::{Gravity, Rotation};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const AF_QRTR: i32 = 42;
const CONTROL: u32 = 0xffff_fffe;
const SSC: u32 = 400;
const LOOKUP: [u64; 2] = [0xabab_abab_abab_abab; 2];

/// Give SSC only its protection-domain metadata and a RAM copy of its
/// calibration. The listener can update that copy without changing persist.
#[cfg(target_os = "android")]
pub fn prepare(config: &Orientation) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    if let Some(file) = &config.firmware {
        let metadata = Path::new("/skyward/run/sensor-firmware/image");
        fs::create_dir_all(metadata)?;
        fs::set_permissions(metadata.parent().unwrap(), fs::Permissions::from_mode(0o755))?;
        fs::set_permissions(metadata, fs::Permissions::from_mode(0o755))?;
        // pd-mapper scans image/*.jsn; the configured domain keeps unrelated
        // audio and charging protection domains out of this service.
        let target = metadata.join(Path::new(file).file_name().context("SSC metadata has no name")?);
        fs::copy(Path::new("/firmware/image").join(file), &target)?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o644))?;
    }
    if let Some(calibration) = &config.calibration {
        let state = Path::new("/skyward/run/sensor-persist");
        let readonly = Path::new("/skyward/run/sensor-persist-ro");
        fs::create_dir_all(state)?;
        fs::create_dir_all("/mnt/vendor/persist")?;
        if !state.join(".ready").exists() {
            fs::create_dir_all(readonly)?;
            skyward_block::mount::mount(
                &Path::new(skyward_block::BY_NAME).join(&calibration.partition),
                readonly, "ext4", &["ro".into(), "noload".into()],
            )?;
            let copied = copy_calibration(&readonly.join(&calibration.path), &state.join(&calibration.path));
            let unmounted = skyward_block::mount::umount(readonly);
            copied?;
            unmounted?;
            fs::write(state.join(".ready"), [])?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "android", test))]
fn copy_calibration(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(source).with_context(|| format!("read {}", source.display()))?;
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_calibration(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        ensure!(metadata.is_file(), "{} is not a calibration file or directory", source.display());
        if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
        fs::copy(source, target)?;
    }
    fs::set_permissions(target, metadata.permissions())?;
    std::os::unix::fs::chown(target, Some(metadata.uid()), Some(metadata.gid()))?;
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Address { family: u16, node: u32, port: u32 }

struct Connection {
    socket: OwnedFd,
    server: Option<Address>,
    sensor: Option<[u64; 2]>,
    transaction: u16,
    heard: u64,
}

/// Closing the socket also removes its subscriptions. Disabled/sleeping tablets
/// and fastbootd neither sample acceleration nor wake to poll for a service.
pub struct Sensor {
    config: Orientation,
    connection: Option<Connection>,
    gravity: Gravity,
    retry_at: u64,
}

impl Sensor {
    pub fn new(config: &Orientation) -> Self {
        Self { config: config.clone(), connection: None, gravity: Gravity::default(), retry_at: 0 }
    }

    pub fn fd(&self) -> Option<RawFd> { self.connection.as_ref().map(|s| s.socket.as_raw_fd()) }

    pub fn read(&mut self, enabled: bool, now: u64) -> Option<Rotation> {
        if !enabled {
            self.connection = None;
            self.gravity = Gravity::default();
            self.retry_at = 0;
            return None;
        }
        let result = (|| -> Result<Option<Rotation>> {
            if self.connection.is_none() && now >= self.retry_at {
                self.connection = Some(Connection::open(now)?);
            }
            let Some(connection) = &mut self.connection else { return Ok(None) };
            let mut direction = None;
            for mut sample in connection.read(now)? {
                if self.config.swap_xy { sample.swap(0, 1); }
                if self.config.flip_x { sample[0] = -sample[0]; }
                if self.config.flip_y { sample[1] = -sample[1]; }
                if let Some(next) = self.gravity.sample(sample, now) { direction = Some(next); }
            }
            Ok(direction)
        })();
        match result {
            Ok(direction) => direction,
            Err(error) => {
                let _ = skyward_sys::kmsg(&format!("orientation sensor: {error:#}; retry in 5 seconds"));
                eprintln!("orientation sensor: {error:#}; retry in 5 seconds");
                self.connection = None;
                self.gravity = Gravity::default();
                self.retry_at = now.saturating_add(5000);
                None
            }
        }
    }
}

impl Connection {
    fn open(now: u64) -> Result<Self> {
        let fd = unsafe { libc::socket(AF_QRTR, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
        ensure!(fd >= 0, "open QRTR: {}", io::Error::last_os_error());
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut local = Address::default();
        let mut size = std::mem::size_of::<Address>() as libc::socklen_t;
        let got = unsafe { libc::getsockname(fd, (&mut local as *mut Address).cast(), &mut size) };
        ensure!(got == 0, "read QRTR address: {}", io::Error::last_os_error());
        let connection = Self { socket, server: None, sensor: None, transaction: 0, heard: now };
        let lookup = [10u32, SSC, 0, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        connection.send(Address { family: AF_QRTR as u16, node: local.node, port: CONTROL }, &lookup)?;
        Ok(connection)
    }

    fn send(&self, address: Address, data: &[u8]) -> Result<()> {
        let sent = unsafe { libc::sendto(self.socket.as_raw_fd(), data.as_ptr().cast(), data.len(), 0,
            (&address as *const Address).cast(), std::mem::size_of::<Address>() as libc::socklen_t) };
        ensure!(sent == data.len() as isize, "send SSC request: {}", io::Error::last_os_error());
        Ok(())
    }

    fn request(&mut self, uid: [u64; 2], message: u32, payload: &[u8]) -> Result<()> {
        self.transaction = self.transaction.wrapping_add(1);
        self.send(self.server.context("SSC has no address")?, &request(self.transaction, uid, message, payload))
    }

    fn read(&mut self, now: u64) -> Result<Vec<[f32; 3]>> {
        let mut samples = Vec::new();
        // Bound work per frame even if a driver floods this socket.
        for _ in 0..128 {
            let mut buffer = [0u8; 65536];
            let mut from = Address::default();
            let mut size = std::mem::size_of::<Address>() as libc::socklen_t;
            let n = unsafe { libc::recvfrom(self.socket.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0,
                (&mut from as *mut Address).cast(), &mut size) };
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock { break; }
                if error.kind() == io::ErrorKind::Interrupted { continue; }
                return Err(error).context("receive SSC data");
            }
            let data = &buffer[..n as usize];
            if from.port == CONTROL {
                if data.len() != 20 { continue; }
                let words: Vec<_> = data.as_chunks::<4>().0.iter().map(|v| u32::from_le_bytes(*v)).collect();
                if words[1] != SSC { continue; }
                if words[0] == 5 { bail!("SSC service stopped"); }
                if words[0] == 4 && self.server.is_none() {
                    self.server = Some(Address { family: AF_QRTR as u16, node: words[3], port: words[4] });
                    let mut query = Vec::new();
                    bytes(&mut query, 1, b"accel");
                    query.extend_from_slice(&[0x10, 1, 0x18, 1]); // updates, default sensor only
                    self.request(LOOKUP, 512, &query)?;
                }
                continue;
            }
            if !self.server.is_some_and(|s| s.node == from.node && s.port == from.port) { continue; }
            for (uid, message, payload) in events(data)? {
                if uid == LOOKUP && message == 768 && self.sensor.is_none() {
                    let fields = fields(payload)?;
                    if !fields.iter().any(|&(tag, wire, value)| tag == 1 && wire == 2 && value == b"accel") { continue; }
                    if let Some(uid) = fields.iter().find_map(|&(tag, wire, value)|
                        (tag == 2 && wire == 2).then(|| sensor_uid(value).ok()).flatten()) {
                        self.sensor = Some(uid);
                        let mut config = vec![0x0d]; // sns_std_sensor_config.sample_rate
                        config.extend_from_slice(&10f32.to_le_bytes());
                        self.request(uid, 513, &config)?;
                    }
                } else if Some(uid) == self.sensor && message == 1025 {
                    samples.push(acceleration(payload)?);
                    self.heard = now;
                }
            }
        }
        ensure!(now.saturating_sub(self.heard) < 10000, "SSC stopped reporting acceleration");
        Ok(samples)
    }
}

// Only the small subset of protobuf used by SSC is needed here. All incoming
// lengths and wire types are checked before slicing; unknown fields are skipped.
fn varint(data: &mut &[u8]) -> Result<u64> {
    let mut result = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = data.split_first().context("short protobuf varint")?;
        *data = rest;
        ensure!(shift < 63 || byte <= 1, "protobuf varint overflow");
        result |= (byte as u64 & 127) << shift;
        if byte & 128 == 0 { return Ok(result); }
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
            0 => { let before = data; varint(&mut data)?; result.push((key>>3, wire, &before[..before.len()-data.len()])); continue; }
            1 => 8,
            2 => usize::try_from(varint(&mut data)?)?,
            5 => 4,
            _ => bail!("unsupported protobuf wire type {wire}"),
        };
        ensure!(size <= data.len(), "short protobuf field");
        let (value, rest) = data.split_at(size);
        result.push((key>>3, wire, value));
        data = rest;
    }
    Ok(result)
}

fn sensor_uid(data: &[u8]) -> Result<[u64; 2]> {
    let fields = fields(data)?;
    let word = |tag| fields.iter().find(|&&(n,w,_)| n == tag && w == 1)
        .map(|&(_,_,v)| u64::from_le_bytes(v.try_into().unwrap())).context("missing sensor UID");
    Ok([word(1)?, word(2)?])
}

fn acceleration(data: &[u8]) -> Result<[f32; 3]> {
    let mut values = Vec::new();
    for (tag, wire, value) in fields(data)? {
        if tag != 1 { continue; }
        ensure!(wire == 2 || wire == 5, "invalid acceleration field");
        ensure!(value.len().is_multiple_of(4), "short acceleration float");
        values.extend(value.as_chunks::<4>().0.iter().map(|v| f32::from_le_bytes(*v)));
    }
    values.try_into().map_err(|_| anyhow::anyhow!("acceleration needs three axes"))
}

type SensorEvent<'a> = ([u64; 2], u32, &'a [u8]);

fn events(data: &[u8]) -> Result<Vec<SensorEvent<'_>>> {
    ensure!(data.len() >= 7, "short QMI header");
    let size = u16::from_le_bytes([data[5],data[6]]) as usize;
    ensure!(data.len() == size+7, "short QMI message");
    let mut tlvs = &data[7..];
    let mut result = Vec::new();
    while !tlvs.is_empty() {
        ensure!(tlvs.len() >= 3, "short QMI TLV");
        let tag = tlvs[0];
        let size = u16::from_le_bytes([tlvs[1],tlvs[2]]) as usize;
        ensure!(size <= tlvs.len()-3, "short QMI TLV value");
        let value = &tlvs[3..size+3];
        tlvs = &tlvs[size+3..];
        if data[0] == 2 && tag == 2 {
            ensure!(value == [0,0,0,0], "SSC rejected a request: {value:?}");
        }
        let message = u16::from_le_bytes([data[3],data[4]]);
        if data[0] != 4 || ![0x21,0x22].contains(&message) || tag != 2 { continue; }
        ensure!(value.len() >= 2, "missing SSC payload length");
        let size = u16::from_le_bytes([value[0],value[1]]) as usize;
        ensure!(size == value.len()-2, "short SSC payload");
        let envelope = fields(&value[2..])?;
        let uid = envelope.iter().find(|&&(t,w,_)| t == 1 && w == 2).context("missing SSC UID")?;
        let uid = sensor_uid(uid.2)?;
        for &(_,_,event) in envelope.iter().filter(|&&(t,w,_)| t == 2 && w == 2) {
            let fields = fields(event)?;
            let message = fields.iter().find(|&&(t,w,_)| t == 1 && w == 5).context("missing SSC event ID")?;
            let payload = fields.iter().find(|&&(t,w,_)| t == 3 && w == 2).context("missing SSC event payload")?;
            result.push((uid, u32::from_le_bytes(message.2.try_into().unwrap()), payload.2));
        }
    }
    Ok(result)
}

fn bytes(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag*8+2);
    let mut size = value.len();
    while size >= 128 { out.push((size as u8 & 127)|128); size >>= 7; }
    out.push(size as u8);
    out.extend_from_slice(value);
}

fn request(transaction: u16, uid: [u64; 2], message: u32, payload: &[u8]) -> Vec<u8> {
    let mut id = vec![9]; id.extend_from_slice(&uid[0].to_le_bytes());
    id.push(17); id.extend_from_slice(&uid[1].to_le_bytes());
    let mut envelope = Vec::new();
    bytes(&mut envelope, 1, &id);
    envelope.push(0x15); envelope.extend_from_slice(&message.to_le_bytes());
    bytes(&mut envelope, 3, &[8,1,16,1]); // APSS, non-wakeup delivery
    let mut body = Vec::new(); bytes(&mut body, 2, payload); bytes(&mut envelope, 4, &body);
    let mut qmi = vec![0]; qmi.extend_from_slice(&transaction.to_le_bytes());
    qmi.extend_from_slice(&0x20u16.to_le_bytes());
    qmi.extend_from_slice(&(envelope.len() as u16+9).to_le_bytes());
    qmi.push(1); qmi.extend_from_slice(&(envelope.len() as u16+2).to_le_bytes());
    qmi.extend_from_slice(&(envelope.len() as u16).to_le_bytes());
    qmi.extend_from_slice(&envelope);
    qmi.extend_from_slice(&[0x10,1,0,1]); // large indications
    qmi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_copy_is_independent_and_does_not_follow_links() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("ram");
        std::fs::create_dir_all(source.join("registry")).unwrap();
        std::fs::write(source.join("registry/settings"), b"calibration").unwrap();
        copy_calibration(&source, &target).unwrap();
        std::fs::write(target.join("registry/settings"), b"updated").unwrap();
        assert_eq!(std::fs::read(source.join("registry/settings")).unwrap(), b"calibration");
        std::os::unix::fs::symlink("/etc/passwd", source.join("outside")).unwrap();
        assert!(copy_calibration(&source, &target).is_err());
    }

    #[test]
    fn decodes_packed_and_unpacked_acceleration_and_rejects_short_fields() {
        let mut packed = vec![10,12];
        for value in [8.2f32,0.0,5.4] { packed.extend_from_slice(&value.to_le_bytes()); }
        packed.extend_from_slice(&[16,3]);
        assert_eq!(acceleration(&packed).unwrap(), [8.2,0.0,5.4]);
        let mut unpacked = Vec::new();
        for value in [8.2f32,0.0,5.4] { unpacked.push(13); unpacked.extend_from_slice(&value.to_le_bytes()); }
        assert_eq!(acceleration(&unpacked).unwrap(), [8.2,0.0,5.4]);
        for end in 0..14 { assert!(acceleration(&packed[..end]).is_err()); }
        assert!(fields(&[0xff;10]).is_err());
        assert!(fields(&[10,255,255,255,255,15]).is_err());
    }

    #[test]
    fn request_has_the_qmi_lengths_and_single_byte_report_type_accepted_by_ssc() {
        let query = [10,5,b'a',b'c',b'c',b'e',b'l',16,1,24,1];
        let data = request(1,LOOKUP,512,&query);
        assert_eq!(u16::from_le_bytes([data[5],data[6]]) as usize, data.len()-7);
        assert_eq!(&data[data.len()-4..], &[16,1,0,1]);
        assert_eq!(u16::from_le_bytes([data[10],data[11]]) as usize, data.len()-16);
        assert!(events(&[2,1,0,32,0,7,0,2,4,0,1,0,1,0]).is_err());
        for end in 0..7 { assert!(events(&data[..end]).is_err()); }
    }
}
