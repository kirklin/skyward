//! wpa_supplicant's control interface: the commands the recovery sends it,
//! how its answers and events read, and the connection they go over.

use anyhow::{bail, ensure, Context, Result};
use skyward_files::hex;
use std::collections::BTreeMap;

/// How a network is protected, as a scan finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Security {
    Open,
    /// Enhanced Open: encrypted, and joined with no password.
    Owe,
    /// WEP, which the recovery does not join.
    Wep,
    /// WPA or WPA2 with a password.
    Psk,
    /// WPA3 with a password.
    Sae,
    /// WPA2 and WPA3 both, with the one password.
    PskSae,
    /// A login of each user's own, which the recovery does not join.
    Enterprise,
}

impl Security {
    /// What the flags of a scan result say, such as
    /// `[WPA-PSK-CCMP][WPA2-PSK-CCMP][ESS]`: each bracket a protocol
    /// written as its name, its key managements joined by `+`, then its
    /// ciphers. A network offering a password and a login of its own is
    /// joined with the password.
    pub fn of(flags: &str) -> Security {
        let items: Vec<&str> = flags.split(['[', ']']).filter(|item| !item.is_empty()).collect();
        let protected = items.iter().filter(|item| ["WPA-", "WPA2-", "RSN-"].iter().any(|p| item.starts_with(p)));
        let words: Vec<&str> = protected.flat_map(|item| item.split(['-', '+', '/'])).collect();
        let has = |word: &str| words.contains(&word);
        match (has("PSK"), has("SAE")) {
            (true, true) => Security::PskSae,
            (false, true) => Security::Sae,
            (true, false) => Security::Psk,
            _ if has("OWE") => Security::Owe,
            _ if has("EAP") => Security::Enterprise,
            _ if items.contains(&"WEP") => Security::Wep,
            _ => Security::Open,
        }
    }

    /// Whether joining takes a password.
    pub fn password(self) -> bool {
        matches!(self, Security::Psk | Security::Sae | Security::PskSae)
    }

    /// Whether the recovery joins such a network.
    pub fn joinable(self) -> bool {
        !matches!(self, Security::Wep | Security::Enterprise)
    }
}

/// An access point a scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bss {
    /// Where it is in the supplicant's list, which keeps an access point
    /// at the same place from one scan to the next.
    pub id: u32,
    pub bssid: String,
    /// The channel's frequency, in MHz.
    pub frequency: u32,
    /// The signal's level, in dBm.
    pub signal: i32,
    pub security: Security,
    /// The network's name as the access point sends it, which need not be
    /// UTF-8; empty for one that does not say it.
    pub ssid: Vec<u8>,
}

/// Which fields of an access point [`bss_command`] asks for, and that a
/// line of `====` follows each: its place in the list, BSSID, frequency,
/// signal level, flags and SSID.
const BSS_FIELDS: u32 = 0x1 | 0x2 | 0x4 | 0x80 | 0x800 | 0x1000 | 0x2_0000;

/// What asks for the access points found, from the one at place `first`
/// on. One answer holds as many as fit; the rest are asked for after the
/// last that came.
pub fn bss_command(first: u32) -> String {
    format!("BSS RANGE={first}- MASK={BSS_FIELDS:#x}")
}

/// The access points in an answer to [`bss_command`], in the order of
/// their places.
pub fn bss_list(answer: &str) -> Result<Vec<Bss>> {
    let mut found = Vec::new();
    for block in answer.split("====\n") {
        if block.trim().is_empty() {
            continue;
        }
        let fields = fields(block);
        let field = |name: &str| fields.get(name).copied().with_context(|| format!("an access point is listed with no {name}"));
        found.push(Bss {
            id: field("id")?.parse().context("an access point's place is not a number")?,
            bssid: field("bssid")?.to_string(),
            frequency: field("freq")?.parse().context("an access point's frequency is not a number")?,
            signal: field("level")?.parse().context("an access point's signal level is not a number")?,
            security: Security::of(field("flags")?),
            ssid: unescape(field("ssid")?)?,
        });
    }
    Ok(found)
}

/// The `name=value` lines of an answer, such as `STATUS` or
/// `SIGNAL_POLL` gives.
pub fn fields(answer: &str) -> BTreeMap<&str, &str> {
    answer.lines().filter_map(|line| line.split_once('=')).collect()
}

/// Bytes the supplicant writes as C would, with `\\`, `\"`, `\e`, `\n`,
/// `\r`, `\t` and `\xNN` for what is not printable ASCII, read back.
pub fn unescape(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        out.push(match bytes.next() {
            Some(b'\\') => b'\\',
            Some(b'"') => b'"',
            Some(b'e') => 0x1b,
            Some(b'n') => b'\n',
            Some(b'r') => b'\r',
            Some(b't') => b'\t',
            Some(b'x') => {
                let hex = [bytes.next(), bytes.next()];
                let [Some(hi), Some(lo)] = hex else { bail!("{text:?} ends inside an escape") };
                let digits = [hi, lo];
                let digits = std::str::from_utf8(&digits).context("an escape holds no hex digits")?;
                u8::from_str_radix(digits, 16).with_context(|| format!("\\x{digits} in {text:?} is not a byte"))?
            }
            other => bail!("{text:?} holds an escape the supplicant does not write: {other:?}"),
        });
    }
    Ok(out)
}

/// What sets up a network with id `id` to join it: its name, how it is
/// protected and its password. The name and a WPA3 password go as hex,
/// which takes any byte; a WPA2 password goes in quotes, which the
/// supplicant takes up to the last one, so a quote inside it stays. A
/// network `hidden`, which answers only a scan that names it, is scanned
/// for by name.
pub fn network_commands(id: u32, ssid: &[u8], security: Security, password: &str, hidden: bool) -> Result<Vec<String>> {
    let set = |name: &str, value: String| format!("SET_NETWORK {id} {name} {value}");
    let mut commands = vec![set("ssid", hex(ssid))];
    if hidden {
        commands.push(set("scan_ssid", "1".into()));
    }
    let psk = || -> Result<String> {
        ensure!((8..=63).contains(&password.len()), "a WPA2 password is 8 to 63 characters long");
        ensure!(password.bytes().all(|b| (32..=126).contains(&b)), "a WPA2 password is printable ASCII");
        Ok(format!("\"{password}\""))
    };
    match security {
        Security::Open => commands.push(set("key_mgmt", "NONE".into())),
        Security::Owe => {
            commands.push(set("key_mgmt", "OWE".into()));
            commands.push(set("ieee80211w", "2".into()));
        }
        Security::Psk => {
            commands.push(set("key_mgmt", "WPA-PSK WPA-PSK-SHA256".into()));
            commands.push(set("psk", psk()?));
        }
        Security::Sae => {
            ensure!(!password.is_empty(), "a WPA3 password is not empty");
            commands.push(set("key_mgmt", "SAE".into()));
            commands.push(set("sae_password", hex(password.as_bytes())));
            commands.push(set("ieee80211w", "2".into()));
        }
        Security::PskSae => {
            commands.push(set("key_mgmt", "WPA-PSK WPA-PSK-SHA256 SAE".into()));
            commands.push(set("psk", psk()?));
            commands.push(set("sae_password", hex(password.as_bytes())));
            commands.push(set("ieee80211w", "1".into()));
        }
        Security::Wep | Security::Enterprise => bail!("the recovery does not join a network protected so"),
    }
    Ok(commands)
}

/// What the supplicant tells a control connection attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A scan has ended and its results can be read.
    ScanResults,
    /// A scan did not start or did not end.
    ScanFailed,
    /// The network chosen is joined: the keys are agreed, and addresses
    /// can be asked for.
    Connected,
    /// The network was left, by the phone or by the access point.
    Disconnected,
    /// The network chosen was set aside for a while after failing to join;
    /// `WRONG_KEY` is a wrong password.
    TemporarilyDisabled { reason: String },
    /// The network chosen is not in range.
    NotFound,
    /// The access point turned the phone away.
    Rejected,
    /// The supplicant is ending.
    Terminating,
}

/// An event from its message, such as `<3>CTRL-EVENT-CONNECTED - ...`; a
/// message that is none of the events above gives none.
pub fn event(message: &str) -> Option<Event> {
    let text = match message.strip_prefix('<') {
        Some(rest) => rest.split_once('>')?.1,
        None => message,
    };
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    Some(match name {
        "CTRL-EVENT-SCAN-RESULTS" => Event::ScanResults,
        "CTRL-EVENT-SCAN-FAILED" => Event::ScanFailed,
        "CTRL-EVENT-CONNECTED" => Event::Connected,
        "CTRL-EVENT-DISCONNECTED" => Event::Disconnected,
        "CTRL-EVENT-SSID-TEMP-DISABLED" => {
            let reason = rest.split(' ').find_map(|word| word.strip_prefix("reason=")).unwrap_or_default();
            Event::TemporarilyDisabled { reason: reason.to_string() }
        }
        "CTRL-EVENT-NETWORK-NOT-FOUND" => Event::NotFound,
        "CTRL-EVENT-ASSOC-REJECT" | "CTRL-EVENT-AUTH-REJECT" => Event::Rejected,
        "CTRL-EVENT-TERMINATING" => Event::Terminating,
        _ => return None,
    })
}

#[cfg(target_os = "android")]
pub use control::Control;

#[cfg(target_os = "android")]
mod control {
    use anyhow::{bail, Context, Result};
    use std::os::android::net::SocketAddrExt;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::net::{SocketAddr, UnixDatagram};
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    /// How long an answer may take.
    const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

    /// The longest message the supplicant sends.
    const MESSAGE_MAX: usize = 8192;

    /// A connection to the supplicant's control socket. It is sent from an
    /// address of its own in the abstract namespace: the supplicant drops
    /// root as it starts, and could not answer a socket file root made.
    pub struct Control {
        socket: UnixDatagram,
    }

    impl Control {
        pub fn open(socket: &Path) -> Result<Control> {
            static OPENED: AtomicU32 = AtomicU32::new(0);
            let name = format!("skyward-wpa-{}-{}", std::process::id(), OPENED.fetch_add(1, Ordering::Relaxed));
            let local = SocketAddr::from_abstract_name(name.as_bytes()).context("name a control connection")?;
            let s = UnixDatagram::bind_addr(&local).context("make a control connection")?;
            s.connect(socket).with_context(|| format!("connect to {}", socket.display()))?;
            s.set_read_timeout(Some(ANSWER_TIMEOUT)).context("time the control connection's answers")?;
            Ok(Control { socket: s })
        }

        /// Send `command` and give the answer. An event that comes ahead of
        /// it on an attached connection is not it.
        pub fn ask(&self, command: &str) -> Result<String> {
            self.socket.send(command.as_bytes()).with_context(|| format!("send {}", first_word(command)))?;
            let mut buf = vec![0u8; MESSAGE_MAX];
            loop {
                let n = self.socket.recv(&mut buf).with_context(|| format!("wait for the answer to {}", first_word(command)))?;
                let answer = String::from_utf8_lossy(&buf[..n]).into_owned();
                if !answer.starts_with('<') {
                    return Ok(answer);
                }
            }
        }

        /// Send a command that answers `OK`.
        pub fn order(&self, command: &str) -> Result<()> {
            match self.ask(command)?.trim_end() {
                "OK" => Ok(()),
                other => bail!("{} answered {other}", first_word(command)),
            }
        }

        /// Have events sent to this connection from now on.
        pub fn attach(&self) -> Result<()> {
            self.order("ATTACH")
        }

        /// The messages that have come since the last call, without waiting.
        pub fn messages(&self) -> Result<Vec<String>> {
            let mut buf = vec![0u8; MESSAGE_MAX];
            let mut out = Vec::new();
            loop {
                let n = unsafe { libc::recv(self.socket.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
                if n >= 0 {
                    out.push(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
                    continue;
                }
                let e = std::io::Error::last_os_error();
                match e.kind() {
                    std::io::ErrorKind::WouldBlock => return Ok(out),
                    std::io::ErrorKind::Interrupted => continue,
                    _ => return Err(e).context("read the supplicant's events"),
                }
            }
        }

        /// Readable while messages wait.
        pub fn fd(&self) -> RawFd {
            self.socket.as_raw_fd()
        }
    }

    impl Drop for Control {
        fn drop(&mut self) {
            // The supplicant keeps sending to an attached connection until
            // told, and drops it only after a few sends fail.
            let _ = self.socket.send(b"DETACH");
        }
    }

    /// A command's name, for what went wrong with it; a password set with
    /// one stays out of the log.
    fn first_word(command: &str) -> &str {
        command.split(' ').next().unwrap_or(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_is_read_from_the_flags_a_scan_gives() {
        let cases = [
            ("[WPA-PSK-CCMP][WPA2-PSK-CCMP][ESS]", Security::Psk),
            ("[WPA2-PSK-CCMP+TKIP][WPS][ESS]", Security::Psk),
            ("[WPA2-PSK-SHA256-CCMP][ESS]", Security::Psk),
            ("[WPA2-PSK+SAE-CCMP][ESS]", Security::PskSae),
            ("[RSN-SAE-CCMP][ESS]", Security::Sae),
            ("[WPA2-SAE-EXT-KEY-GCMP-256][ESS]", Security::Sae),
            ("[WPA2-FT/SAE+SAE-CCMP][ESS]", Security::Sae),
            ("[WPA2-OWE-CCMP][ESS]", Security::Owe),
            ("[WPA2-EAP-CCMP][ESS]", Security::Enterprise),
            ("[WPA2-EAP+PSK-CCMP][ESS]", Security::Psk),
            ("[WEP][ESS]", Security::Wep),
            ("[ESS]", Security::Open),
            ("[WPA2-PSK-CCMP][ESS][FILS]", Security::Psk),
        ];
        for (flags, security) in cases {
            assert_eq!(Security::of(flags), security, "{flags}");
        }
        assert!(Security::PskSae.password() && !Security::Owe.password() && !Security::Open.password());
        assert!(!Security::Wep.joinable() && !Security::Enterprise.joinable() && Security::Sae.joinable());
    }

    /// Two access points as the 13 Ultra's supplicant listed them, the
    /// second named in Chinese.
    const LISTED: &str = "id=33\nbssid=02:00:00:00:00:02\nfreq=5200\nlevel=-66\nflags=[WPA-PSK-CCMP][WPA2-PSK-CCMP][ESS]\nssid=Skyward-Studio\n====\nid=34\nbssid=02:00:00:00:00:03\nfreq=5180\nlevel=-50\nflags=[WPA-PSK-CCMP][WPA2-PSK-CCMP][ESS]\nssid=Skyward\\xe5\\xae\\xa2\\xe5\\x8e\\x85\n====\n";

    #[test]
    fn access_points_are_read_one_block_each() {
        assert_eq!(bss_command(0), "BSS RANGE=0- MASK=0x21887");
        let found = bss_list(LISTED).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0],
            Bss { id: 33, bssid: "02:00:00:00:00:02".into(), frequency: 5200, signal: -66, security: Security::Psk, ssid: b"Skyward-Studio".to_vec() }
        );
        assert_eq!(String::from_utf8(found[1].ssid.clone()).unwrap(), "Skyward客厅");
        assert!(bss_list("").unwrap().is_empty(), "no access point found answers nothing");
        assert!(bss_list("id=1\nbssid=x\n====\n").is_err(), "a block missing a field is not an access point");
    }

    #[test]
    fn names_are_read_back_as_the_supplicant_escapes_them() {
        assert_eq!(unescape("plain").unwrap(), b"plain");
        assert_eq!(unescape(r#"a\"b\\c\td\e"#).unwrap(), b"a\"b\\c\td\x1b");
        assert_eq!(unescape(r"\x00\xff").unwrap(), [0, 255]);
        assert!(unescape(r"\x4").is_err());
        assert!(unescape(r"\q").is_err());
        assert_eq!(hex(b"ab\x00"), "616200");
    }

    #[test]
    fn a_network_is_set_up_with_its_name_and_password_written_as_the_supplicant_takes_them() {
        let wpa2 = network_commands(0, "Skyward客厅".as_bytes(), Security::Psk, "pass\"word", false).unwrap();
        assert_eq!(
            wpa2,
            ["SET_NETWORK 0 ssid 536b7977617264e5aea2e58e85", "SET_NETWORK 0 key_mgmt WPA-PSK WPA-PSK-SHA256", "SET_NETWORK 0 psk \"pass\"word\""]
        );
        let wpa3 = network_commands(2, b"Skyward",Security::Sae, "short", false).unwrap();
        assert_eq!(wpa3[1..], ["SET_NETWORK 2 key_mgmt SAE", "SET_NETWORK 2 sae_password 73686f7274", "SET_NETWORK 2 ieee80211w 2"]);
        let both = network_commands(1, b"Skyward",Security::PskSae, "12345678", false).unwrap();
        assert!(both.contains(&"SET_NETWORK 1 psk \"12345678\"".to_string()));
        assert!(both.contains(&"SET_NETWORK 1 sae_password 3132333435363738".to_string()));
        assert_eq!(network_commands(0, b"Skyward",Security::Open, "", false).unwrap()[1], "SET_NETWORK 0 key_mgmt NONE");
        assert!(network_commands(0, b"Skyward",Security::Psk, "1234567", false).is_err(), "a WPA2 password is 8 characters at least");
        assert!(network_commands(0, b"Skyward",Security::Psk, "密码密码密码密码", false).is_err(), "a WPA2 password is ASCII");
        assert!(network_commands(0, b"Skyward",Security::Wep, "12345", false).is_err());
        let hidden = network_commands(3, b"Skyward", Security::Open, "", true).unwrap();
        assert_eq!(hidden, ["SET_NETWORK 3 ssid 536b7977617264", "SET_NETWORK 3 scan_ssid 1", "SET_NETWORK 3 key_mgmt NONE"]);
    }

    #[test]
    fn events_are_known_by_their_names() {
        assert_eq!(event("<3>CTRL-EVENT-SCAN-RESULTS "), Some(Event::ScanResults));
        assert_eq!(event("<3>CTRL-EVENT-CONNECTED - Connection to 02:00:00:00:00:01 completed [id=0 id_str=]"), Some(Event::Connected));
        assert_eq!(event("<3>CTRL-EVENT-DISCONNECTED bssid=02:00:00:00:00:01 reason=3 locally_generated=1"), Some(Event::Disconnected));
        assert_eq!(
            event("<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"Skyward\" auth_failures=1 duration=10 reason=WRONG_KEY"),
            Some(Event::TemporarilyDisabled { reason: "WRONG_KEY".into() })
        );
        assert_eq!(event("<3>CTRL-EVENT-NETWORK-NOT-FOUND"), Some(Event::NotFound));
        assert_eq!(event("<3>CTRL-EVENT-ASSOC-REJECT bssid=00:00:00:00:00:00 status_code=16"), Some(Event::Rejected));
        assert_eq!(event("<2>Trying to associate with 02:00:00:00:00:01"), None);
        assert_eq!(event("CTRL-EVENT-TERMINATING"), Some(Event::Terminating));
    }

    #[test]
    fn answers_read_as_their_fields() {
        let status = fields("bssid=02:00:00:00:00:01\nfreq=5805\nssid=Skyward-5G\nwpa_state=COMPLETED\nip_address=192.168.1.5\n");
        assert_eq!(status["wpa_state"], "COMPLETED");
        assert_eq!(status["freq"], "5805");
    }
}
