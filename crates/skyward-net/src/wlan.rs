//! The WLAN as the pages show it, and how it moves on with what the
//! supplicant tells and the address the network gives.

use crate::dhcp::Lease;
use crate::reach::Reach;
use crate::supplicant::{Bss, Event, Security};
use std::net::Ipv4Addr;

/// The WLAN, as the pages show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wlan {
    pub power: Power,
    /// The networks in range, strongest first, each once however many
    /// access points it has; one that does not say its name is left out.
    pub networks: Vec<Network>,
    /// The network chosen, and how joining it goes.
    pub joined: Option<Joined>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Power {
    #[default]
    Off,
    /// The driver and the supplicant are being brought up.
    Starting,
    On,
    /// It did not come up, or the supplicant went away: why.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: Vec<u8>,
    /// Its name to show: as UTF-8, with what is not put right.
    pub name: String,
    /// The strongest of its access points, in dBm, and that one's channel.
    pub signal: i32,
    pub frequency: u32,
    /// The bands its access points are on.
    pub bands: Bands,
    pub security: Security,
}

/// The bands a network's access points are on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bands {
    pub ghz2: bool,
    pub ghz5: bool,
    pub ghz6: bool,
}

impl Bands {
    /// The band of a channel of `mhz`.
    pub fn of(mhz: u32) -> Bands {
        Bands { ghz2: (2400..2500).contains(&mhz), ghz5: (4900..5925).contains(&mhz), ghz6: (5925..7125).contains(&mhz) }
    }

    fn join(self, other: Bands) -> Bands {
        Bands { ghz2: self.ghz2 || other.ghz2, ghz5: self.ghz5 || other.ghz5, ghz6: self.ghz6 || other.ghz6 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub ssid: Vec<u8>,
    pub name: String,
    pub security: Security,
    pub stage: Stage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Reaching the access point and agreeing keys with it.
    Joining,
    /// Joined, and asking for an address.
    Addressing,
    Connected(Link),
    /// The access point refused the password.
    WrongPassword,
    /// The network is not in range; the supplicant keeps looking for it.
    NotFound,
    /// Joining failed; the supplicant tries again after a while.
    Failed,
    /// Joined, and no address was given: why.
    NoAddress(String),
}

/// A network joined, and what it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub address: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    /// In dBm, and the channel's frequency in MHz.
    pub signal: i32,
    pub frequency: u32,
    /// How fast the link is, in Mbit/s, where the driver says.
    pub speed: Option<u32>,
    /// Which generation of Wi-Fi the link is, where the supplicant says.
    pub generation: Option<u8>,
    /// The phone's own hardware address.
    pub mac: String,
    /// How far the network reaches, once looked at.
    pub reach: Option<Reach>,
}

impl Link {
    pub fn new(lease: &Lease, mac: [u8; 6]) -> Link {
        Link {
            address: lease.address,
            mask: lease.mask,
            router: lease.router,
            dns: lease.dns.clone(),
            signal: 0,
            frequency: 0,
            speed: None,
            generation: None,
            mac: mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"),
            reach: None,
        }
    }
}

/// How fast data went over the network joined in the last second, in and
/// out, in bytes a second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traffic {
    pub rx: u64,
    pub tx: u64,
}

/// The networks the access points found make, strongest first.
pub fn networks(found: &[Bss]) -> Vec<Network> {
    let mut out: Vec<Network> = Vec::new();
    for bss in found.iter().filter(|b| !b.ssid.is_empty() && b.ssid.iter().any(|&c| c != 0)) {
        match out.iter_mut().find(|n| n.ssid == bss.ssid) {
            Some(n) => {
                n.bands = n.bands.join(Bands::of(bss.frequency));
                if bss.signal > n.signal {
                    n.signal = bss.signal;
                    n.frequency = bss.frequency;
                }
            }
            None => out.push(Network {
                ssid: bss.ssid.clone(),
                name: String::from_utf8_lossy(&bss.ssid).into_owned(),
                signal: bss.signal,
                frequency: bss.frequency,
                bands: Bands::of(bss.frequency),
                security: bss.security,
            }),
        }
    }
    out.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.name.cmp(&b.name)));
    out
}

/// What joining has to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing beyond showing the stage.
    Show,
    /// Ask for an address.
    Address,
    /// Give back the address and stop keeping it: the network was left.
    Unaddress,
    /// Stop trying the network: its password is wrong.
    Forget,
}

impl Joined {
    /// Move on with an event from the supplicant.
    pub fn on(&mut self, event: &Event) -> Next {
        match event {
            // Said again of a network joined, as the supplicant does on
            // moving to another of its access points, it changes nothing:
            // the address asked for or had still holds.
            Event::Connected if matches!(self.stage, Stage::Addressing | Stage::Connected(_) | Stage::NoAddress(_)) => Next::Show,
            Event::Connected => {
                self.stage = Stage::Addressing;
                Next::Address
            }
            Event::Disconnected => {
                let had = matches!(self.stage, Stage::Addressing | Stage::Connected(_) | Stage::NoAddress(_));
                // A refused password goes on showing after the leaving it
                // brings.
                if self.stage != Stage::WrongPassword {
                    self.stage = Stage::Joining;
                }
                if had {
                    Next::Unaddress
                } else {
                    Next::Show
                }
            }
            Event::TemporarilyDisabled { reason } if reason == "WRONG_KEY" => {
                self.stage = Stage::WrongPassword;
                Next::Forget
            }
            Event::TemporarilyDisabled { .. } | Event::Rejected if !matches!(self.stage, Stage::Connected(_)) => {
                self.stage = Stage::Failed;
                Next::Show
            }
            Event::NotFound if self.stage == Stage::Joining || self.stage == Stage::Failed => {
                self.stage = Stage::NotFound;
                Next::Show
            }
            _ => Next::Show,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bss(id: u32, ssid: &[u8], signal: i32, frequency: u32) -> Bss {
        Bss { id, bssid: format!("00:00:00:00:00:{id:02x}"), frequency, signal, security: Security::Psk, ssid: ssid.to_vec() }
    }

    #[test]
    fn a_network_shows_once_with_its_strongest_access_point() {
        let found = [
            bss(1, b"Skyward", -62, 2462),
            bss(2, b"Skyward", -34, 5805),
            bss(3, b"", -30, 2462),
            bss(4, b"\0\0\0", -30, 2462),
            bss(5, "Skyward客厅".as_bytes(), -50, 5180),
            bss(6, b"\xff\xfe", -90, 2412),
        ];
        let n = networks(&found);
        let names: Vec<&str> = n.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Skyward", "Skyward客厅", "\u{fffd}\u{fffd}"], "the strongest first, nameless ones left out");
        assert_eq!((n[0].signal, n[0].frequency), (-34, 5805));
        assert_eq!(n[0].bands, Bands { ghz2: true, ghz5: true, ghz6: false }, "both its access points' bands");
        assert_eq!(n[1].bands, Bands { ghz2: false, ghz5: true, ghz6: false });
        assert_eq!(Bands::of(5955), Bands { ghz2: false, ghz5: false, ghz6: true });
        assert_eq!(n[2].ssid, b"\xff\xfe", "a name that is not UTF-8 is joined as it is");
    }

    fn joining() -> Joined {
        Joined { ssid: b"Skyward".to_vec(), name: "Skyward".into(), security: Security::Psk, stage: Stage::Joining }
    }

    #[test]
    fn joining_moves_on_with_what_the_supplicant_tells() {
        let mut j = joining();
        assert_eq!(j.on(&Event::Connected), Next::Address);
        assert_eq!(j.stage, Stage::Addressing);
        assert_eq!(j.on(&Event::Connected), Next::Show, "said again, the address is asked for once");
        j.stage = Stage::NoAddress("no DHCP server offered an address".into());
        assert_eq!(j.on(&Event::Connected), Next::Show, "an address not given is asked for again in time");
        j.stage = Stage::Addressing;
        assert_eq!(j.on(&Event::ScanResults), Next::Show);
        assert_eq!(j.on(&Event::Disconnected), Next::Unaddress, "an address asked for or had goes with the network");
        assert_eq!(j.stage, Stage::Joining, "the supplicant joins it again by itself");
        assert_eq!(j.on(&Event::Disconnected), Next::Show, "no address to give back");
        assert_eq!(j.on(&Event::NotFound), Next::Show);
        assert_eq!(j.stage, Stage::NotFound);
        assert_eq!(j.on(&Event::Connected), Next::Address);
        let lease = Lease {
            address: Ipv4Addr::new(192, 168, 0, 102),
            mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(Ipv4Addr::new(192, 168, 0, 1)),
            dns: Vec::new(),
            server: Ipv4Addr::new(192, 168, 0, 1),
            seconds: 86400,
            renew: 43200,
            rebind: 75600,
        };
        j.stage = Stage::Connected(Link::new(&lease, [0; 6]));
        assert_eq!(j.on(&Event::Connected), Next::Show, "moved to another access point, the address is kept");
        assert!(matches!(j.stage, Stage::Connected(_)));
    }

    #[test]
    fn a_refused_password_stops_the_trying() {
        let mut j = joining();
        assert_eq!(j.on(&Event::TemporarilyDisabled { reason: "WRONG_KEY".into() }), Next::Forget);
        assert_eq!(j.stage, Stage::WrongPassword);
        assert_eq!(j.on(&Event::Disconnected), Next::Show);
        assert_eq!(j.stage, Stage::WrongPassword, "what was wrong stays said");
        let mut j = joining();
        assert_eq!(j.on(&Event::TemporarilyDisabled { reason: "CONN_FAILED".into() }), Next::Show);
        assert_eq!(j.stage, Stage::Failed);
    }

    #[test]
    fn a_link_names_the_phones_own_address() {
        let lease = Lease {
            address: Ipv4Addr::new(192, 168, 1, 23),
            mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: vec![Ipv4Addr::new(192, 168, 1, 1)],
            server: Ipv4Addr::new(192, 168, 1, 1),
            seconds: 86400,
            renew: 43200,
            rebind: 75600,
        };
        let link = Link::new(&lease, [0x02, 0x00, 0x00, 0x00, 0x00, 0x04]);
        assert_eq!(link.mac, "02:00:00:00:00:04");
        assert_eq!(link.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }
}

/// Where the phone keeps what it knows of the networks it joined, their
/// keys among it.
const SAVED: &str = "/data/misc/apexdata/com.android.wifi/WifiConfigStore.xml";

/// A network the phone joined before: what it is called, and the key it
/// keeps for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub ssid: Vec<u8>,
    pub key: String,
}

/// The networks the phone's own Wi-Fi settings keep a key for, as they wrote
/// them: none before the data partition opens.
pub fn known() -> Vec<Known> {
    let Ok(settings) = std::fs::read_to_string(SAVED) else { return Vec::new() };
    let mut out: Vec<Known> = Vec::new();
    let mut ssid: Option<Vec<u8>> = None;
    // Each network writes its name and its key on lines of their own, the
    // name first: a key belongs to the name before it.
    for line in settings.lines() {
        if let Some(value) = setting(line, "SSID") {
            ssid = Some(value.into_bytes());
            continue;
        }
        if let Some(value) = setting(line, "PreSharedKey") {
            if let Some(name) = ssid.take() {
                if !value.is_empty() {
                    out.push(Known { ssid: name, key: value });
                }
            }
        }
    }
    out
}

/// The text a line of the settings holds under `name`, unquoted: `None`
/// where the line holds another setting.
fn setting(line: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\"");
    if !line.contains(&marker) {
        return None;
    }
    let (_, after) = line.split_once('>')?;
    let (value, _) = after.split_once("</string>")?;
    let value = unescape(value.trim());
    Some(value.trim_matches('"').to_string())
}

/// The text the settings keep, with what XML writes in place of some of its
/// characters.
fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Remember `ssid` and `key` in the phone's own Wi-Fi settings, so its system
/// knows the network the recovery joined: a network already there gives the
/// shape, and only its name and key change. `Ok(false)` where nothing was
/// written, as when no network there keeps a key to copy.
pub fn remember(ssid: &str, key: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(SAVED)?;
    let Some((block_start, block)) = template(&text) else { return Ok(false) };
    let mut block = block.to_string();
    block = rewrite(&block, "ConfigKey", &format!("\"{ssid}\"WPA_PSK"));
    block = rewrite(&block, "SSID", &format!("\"{ssid}\""));
    block = rewrite(&block, "PreSharedKey", &format!("\"{key}\""));
    let Some(end) = text.rfind("</NetworkList>") else { return Ok(false) };
    // The settings are backed up beside themselves before they change.
    std::fs::copy(SAVED, format!("{SAVED}.bak"))?;
    let mut written = String::with_capacity(text.len() + block.len());
    written.push_str(&text[..end]);
    written.push_str(&block);
    written.push_str(&text[end..]);
    std::fs::write(SAVED, written)?;
    let _ = block_start;
    Ok(true)
}

/// The first network in `text` that keeps a key, and where its block starts.
fn template(text: &str) -> Option<(usize, &str)> {
    let mut from = 0;
    while let Some(at) = text[from..].find("<WifiConfiguration>") {
        let start = from + at;
        let end = text[start..].find("</WifiConfiguration>")? + start + "</WifiConfiguration>".len();
        let block = &text[start..end];
        if block.contains("name=\"PreSharedKey\"") {
            return Some((start, block));
        }
        from = end;
    }
    None
}

/// `block` with the string named `name` holding `value`, escaped as the
/// settings write it.
fn rewrite(block: &str, name: &str, value: &str) -> String {
    let marker = format!("<string name=\"{name}\">");
    let Some(start) = block.find(&marker) else { return block.to_string() };
    let Some(end) = block[start..].find("</string>") else { return block.to_string() };
    let end = start + end + "</string>".len();
    let escaped = value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!("{}{marker}{escaped}</string>{}", &block[..start], &block[end..])
}
