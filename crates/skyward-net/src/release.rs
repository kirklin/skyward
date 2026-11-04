//! The phone maker's update server: which system is the newest for the
//! installed one, asked the way the stock updater asks it, and where the
//! full package of that system is.

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use anyhow::{bail, ensure, Context, Result};
use base64::Engine;
use serde::Deserialize;

/// The key and first block the stock updater seals its questions with,
/// and the server its answers, when no account is signed in.
const KEY: &[u8; 16] = b"miuiotavalided11";
const IV: &[u8; 16] = b"0102030405060708";

/// The stock updater's version the server is told of, which it asks of an
/// Android 15 or later.
const UPDATER: &str = "9.3.7";

/// What the server says when it has answered.
const ANSWERED: u32 = 2000;

/// Where the phone's own updater keeps what it last learned of the newest
/// system, among the settings of its screen.
pub const CACHE: &str = "/data/data/com.android.updater/shared_prefs/version_json.xml";

type Sealer = cbc::Encryptor<aes::Aes128>;
type Opener = cbc::Decryptor<aes::Aes128>;

/// The installed system, as the server is told of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// ro.product.mod_device: the phone, and after it the region its system
    /// is for, such as `_global`; the Chinese system has none.
    pub device: String,
    /// The system's version, such as OS3.0.308.0.WMACNXM.
    pub version: String,
    /// ro.build.version.release.
    pub android: String,
    pub unlocked: bool,
}

impl Installed {
    /// The region the server knows the system's by: what follows the
    /// phone's name in its device name, the European one under its own.
    fn region(&self) -> String {
        match self.device.split_once('_').map(|(_, rest)| rest) {
            None => "CN".into(),
            Some("global") => "MI".into(),
            Some("eea_global") => "EU".into(),
            Some(rest) => rest.trim_end_matches("_global").to_uppercase(),
        }
    }

    /// The question, as the stock updater puts it.
    pub fn question(&self) -> String {
        let region = self.region();
        let language = if region == "CN" { "zh_CN" } else { "en_US" };
        // A system whose version says it is for developers is on their
        // branch; any other on the stable one.
        let branch = if self.version.to_uppercase().ends_with(".DEV") { "X" } else { "F" };
        let mut q = serde_json::json!({
            "b": branch,
            "c": self.android,
            "d": self.device,
            "f": "1",
            "id": "",
            "l": language,
            "ov": self.version,
            "p": self.device,
            "pn": self.device,
            "r": region,
            "security": "",
            "token": "",
            "unlock": if self.unlocked { "1" } else { "0" },
            "v": format!("MIUI-{}", self.version),
        });
        if self.android.split('.').next().and_then(|major| major.parse::<u32>().ok()).is_some_and(|major| major >= 15) {
            q["options"] = serde_json::json!({ "av": UPDATER });
        }
        q.to_string()
    }
}

/// A question sealed, as the form carries it.
pub fn seal(question: &str) -> String {
    let sealed = Sealer::new(KEY.into(), IV.into()).encrypt_padded_vec_mut::<Pkcs7>(question.as_bytes());
    base64::engine::general_purpose::URL_SAFE.encode(sealed)
}

/// An answer opened.
pub fn open(answer: &str) -> Result<String> {
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(answer.trim())
        .context("the update server's answer is not sealed the way it seals them")?;
    let opened = Opener::new(KEY.into(), IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&sealed)
        .map_err(|_| anyhow::anyhow!("the update server's answer does not open with its key"))?;
    String::from_utf8(opened).context("the update server's answer is not text")
}

/// The newest system, and where its full package is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The package's own name.
    pub name: String,
    /// Where it can be downloaded, best first. The server signs each for a
    /// day only.
    pub urls: Vec<String>,
    /// The package's MD5, in hex.
    pub md5: String,
    /// How big the server says it is, such as 7.1G, in units of 1024.
    pub size: String,
    /// How many bytes it is, as where it is downloaded from says; none
    /// until that is asked.
    pub length: Option<u64>,
}

/// Why the server gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unanswered {
    /// The network joined wants a login in a browser first.
    Login,
    /// The network joined does not reach the internet: why.
    Offline(String),
    /// The server could not be asked, or its answer not read: why.
    Failed(String),
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "Code")]
    code: Option<Code>,
    #[serde(rename = "LatestRom")]
    latest: Option<Rom>,
    #[serde(rename = "MirrorList", default)]
    mirrors: Vec<String>,
}

#[derive(Deserialize)]
struct Code {
    code: u32,
    message: Option<String>,
}

#[derive(Deserialize)]
struct Rom {
    version: String,
    filename: String,
    md5: String,
    filesize: String,
}

/// The newest system the opened `answer` tells of; none when the server
/// knows of none for the installed system.
pub fn release(answer: &str) -> Result<Option<Release>> {
    let a: Answer = serde_json::from_str(answer).context("read the update server's answer")?;
    if let Some(code) = &a.code {
        ensure!(
            code.code == ANSWERED,
            "the update server answered {}{}",
            code.code,
            code.message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default()
        );
    }
    let Some(rom) = a.latest else { return Ok(None) };
    let name = rom.filename.split('?').next().unwrap_or_default().to_string();
    ensure!(!name.is_empty() && !name.contains('/'), "the update server names the package {}", rom.filename);
    // A mirror that keeps to HTTPS comes before one that does not.
    let mut mirrors = a.mirrors;
    mirrors.sort_by_key(|m| !m.starts_with("https://"));
    let urls = mirrors.iter().map(|m| format!("{}/{}/{}", m.trim_end_matches('/'), rom.version, rom.filename)).collect::<Vec<_>>();
    if urls.is_empty() {
        bail!("the update server says of no place to download {name}");
    }
    Ok(Some(Release { version: rom.version, name, urls, md5: rom.md5.to_lowercase(), size: rom.filesize, length: None }))
}

/// The newest system the phone's own updater last wrote of, where it wrote
/// of one newer than `installed`: the map it keeps holds the answer the
/// server gave it, which a sealed question on its own does not get.
pub fn cached(xml: &str, installed: &str) -> Option<Release> {
    let settings = xml.split("name=\"new_version\"").nth(1)?;
    let (_, rest) = settings.split_once('>')?;
    let json = rest.split("</string>").next()?;
    let cached: Cached = serde_json::from_str(&unescape(json)).ok()?;
    let rom = cached.latest.as_ref()?;
    let release = rom.release(&cached.mirrors)?;
    (release.version != installed).then_some(release)
}

/// A text the updater's map keeps, with what XML writes in place of some of
/// its characters.
fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[derive(Deserialize)]
struct Cached {
    #[serde(rename = "LatestRom")]
    latest: Option<CachedRom>,
    #[serde(rename = "MirrorList", default)]
    mirrors: Vec<String>,
}

#[derive(Deserialize)]
struct CachedRom {
    version: String,
    filename: String,
    md5: String,
    filesize: String,
}

impl CachedRom {
    fn release(&self, mirrors: &[String]) -> Option<Release> {
        let name = self.filename.split('?').next().unwrap_or_default().to_string();
        if name.is_empty() || name.contains('/') {
            return None;
        }
        let urls: Vec<String> = mirrors.iter().map(|m| format!("{}/{}/{}", m.trim_end_matches('/'), self.version, self.filename)).collect();
        if urls.is_empty() {
            return None;
        }
        Some(Release {
            version: self.version.clone(),
            name,
            urls,
            md5: self.md5.to_lowercase(),
            size: self.filesize.clone(),
            length: None,
        })
    }
}

/// How many bytes the package at `url` is, as its server says.
pub fn length(agent: &ureq::Agent, url: &str) -> Result<u64> {
    let head = agent.head(url).call().context("ask the server about the package")?;
    ensure!(head.status().is_success(), "the server answered {}", head.status());
    crate::download::content_length(head.headers()).context("the server does not say how long the package is")
}

/// Ask `server` which system is the newest for `installed`.
pub fn ask(agent: &ureq::Agent, server: &str, installed: &Installed) -> Result<Option<Release>> {
    let q = seal(&installed.question());
    let mut response = agent
        .post(server)
        .send_form([("q", q.as_str()), ("t", ""), ("s", "1")])
        .with_context(|| format!("ask {server}"))?;
    let status = response.status();
    ensure!(status.is_success(), "{server} answered {status}");
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {server}"))?;
    release(&open(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thirteen_ultra() -> Installed {
        Installed { device: "ishtar".into(), version: "OS3.0.306.0.WMACNXM".into(), android: "16".into(), unlocked: true }
    }

    #[test]
    fn the_question_says_what_the_stock_updater_says() {
        let q: serde_json::Value = serde_json::from_str(&thirteen_ultra().question()).unwrap();
        assert_eq!(q["d"], "ishtar");
        assert_eq!(q["pn"], "ishtar");
        assert_eq!(q["r"], "CN");
        assert_eq!(q["l"], "zh_CN");
        assert_eq!(q["b"], "F");
        assert_eq!(q["ov"], "OS3.0.306.0.WMACNXM");
        assert_eq!(q["v"], "MIUI-OS3.0.306.0.WMACNXM");
        assert_eq!(q["unlock"], "1");
        assert_eq!(q["options"]["av"], UPDATER);
        let older = Installed { android: "14".into(), ..thirteen_ultra() };
        let q: serde_json::Value = serde_json::from_str(&older.question()).unwrap();
        assert!(q.get("options").is_none(), "only Android 15 and later is asked the updater's version");
    }

    #[test]
    fn a_region_is_read_from_the_device_name() {
        let region = |device: &str| Installed { device: device.into(), ..thirteen_ultra() }.region();
        assert_eq!(region("ishtar"), "CN");
        assert_eq!(region("nuwa_global"), "MI");
        assert_eq!(region("nuwa_eea_global"), "EU");
        assert_eq!(region("nuwa_in_global"), "IN");
        assert_eq!(region("nuwa_tw_global"), "TW");
    }

    #[test]
    fn a_question_sealed_opens_again_as_the_server_opens_it() {
        let q = thirteen_ultra().question();
        let sealed = seal(&q);
        assert!(!sealed.contains('+') && !sealed.contains('/'), "sealed for a form, as the stock updater does");
        let standard = base64::engine::general_purpose::STANDARD.encode(base64::engine::general_purpose::URL_SAFE.decode(&sealed).unwrap());
        assert_eq!(open(&standard).unwrap(), q);
        assert!(open("not sealed").is_err());
    }

    /// What the server answered of the 13 Ultra's OS3.0.306.0.WMACNXM on
    /// 2026-10-22, with the parts that do not matter here left out.
    const ANSWERED_306: &str = r#"{"UserLevel":9,"LatestRom":{"type":"rom","device":"ishtar","name":"3.0.308.0.WMACNXM","md5":"E3D01BE14A3CF95D8E1171A1A2A28EB9","codebase":"16.0","branch":"F","filename":"ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0-e3d01be14a.zip?t=1792686801&s=eb90f665fb7c903a898efb9c8c5e297d","filesize":"7.1G","version":"OS3.0.308.0.WMACNXM","changelog":{"系统":[{"txt":"修复 系统稳定性偶现问题，提升用户体验"}],"相机":[{"txt":"优化 拍照速度"},{"txt":"修复 偶现闪退"}]},"Erase":0},"CurrentRom":{"version":"OS3.0.306.0.WMACNXM","filename":"ishtar-ota_full-OS3.0.306.0.WMACNXM-user-16.0-42184b59d7.zip"},"IncrementRom":{"type":"ota","filename":"ishtar-ota_incremental-OS3.0.306.0.WMACNXM-OS3.0.308.0.WMACNXM-user-16.0-33f55688b4.zip?t=1792686801&s=cc6f4b0ca2124927b494954ab13cf29c"},"MirrorList":["http://superota.d.miui.com","https://ultimateota.d.miui.com","https://superota.d.miui.com"],"Code":{"code":2000,"message":"success"}}"#;

    #[test]
    fn the_answer_gives_the_newest_full_package_and_where_it_is() {
        let r = release(ANSWERED_306).unwrap().expect("the server knows a newer system");
        assert_eq!(r.version, "OS3.0.308.0.WMACNXM");
        assert_eq!(r.name, "ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0-e3d01be14a.zip");
        assert_eq!(r.md5, "e3d01be14a3cf95d8e1171a1a2a28eb9");
        assert_eq!(r.size, "7.1G");
        assert_eq!(
            r.urls,
            [
                "https://ultimateota.d.miui.com/OS3.0.308.0.WMACNXM/ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0-e3d01be14a.zip?t=1792686801&s=eb90f665fb7c903a898efb9c8c5e297d",
                "https://superota.d.miui.com/OS3.0.308.0.WMACNXM/ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0-e3d01be14a.zip?t=1792686801&s=eb90f665fb7c903a898efb9c8c5e297d",
                "http://superota.d.miui.com/OS3.0.308.0.WMACNXM/ishtar-ota_full-OS3.0.308.0.WMACNXM-user-16.0-e3d01be14a.zip?t=1792686801&s=eb90f665fb7c903a898efb9c8c5e297d",
            ]
        );
        assert_eq!(r.length, None, "the length is asked of where the package is");
    }

    #[test]
    fn an_answer_with_nothing_or_a_failure_says_so() {
        assert_eq!(release(r#"{"Code":{"code":2000,"message":"success"},"MirrorList":[]}"#).unwrap(), None);
        let e = release(r#"{"Code":{"code":2001,"message":"device not found"}}"#).unwrap_err();
        assert_eq!(e.to_string(), "the update server answered 2001 (device not found)");
        let nowhere = r#"{"LatestRom":{"version":"V","filename":"a.zip","md5":"m","filesize":"1G"},"MirrorList":[]}"#;
        assert!(release(nowhere).is_err(), "a package with no place to download it from");
    }
}
