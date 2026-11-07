//! Skyward's own releases on GitHub: which is the newest, and the zip in
//! it for one phone, as the release workflow names each phone's.

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

/// Where GitHub answers which release of Skyward is the newest.
pub const LATEST: &str = "https://api.github.com/repos/kirklin/skyward/releases/latest";

/// The version of GitHub's API the answer is read as.
const API_VERSION: &str = "2022-11-28";

/// The newest release, as the build of Skyward that asked sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newest {
    /// Its version, such as 0.1.4.
    pub version: String,
    /// It is newer than the build that asked.
    pub newer: bool,
    /// Its zip for the phone; none when it has none.
    pub build: Option<Build>,
}

/// A phone's zip in a release, which holds Skyward's image for the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// The zip's own name, such as
    /// skyward-v0.1.4-ishtar-OS3.0.308.0.WMACNXM.zip.
    pub name: String,
    /// Where it is downloaded from.
    pub url: String,
    pub size: u64,
    /// Its SHA-256 as GitHub gives it, in hex.
    pub sha256: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    /// The digest GitHub took of the file as it was uploaded, such as
    /// `sha256:` and the SHA-256 in hex.
    digest: Option<String>,
    browser_download_url: String,
}

/// Why GitHub turned a question down, as it says.
#[derive(Deserialize)]
struct Message {
    message: String,
}

/// The newest release that `answer`, GitHub's answer at [`LATEST`], tells
/// of, and its zip for the phone `codename` names, as the build of version
/// `running` sees it.
pub fn newest(answer: &str, codename: &str, running: &str) -> Result<Newest> {
    let release: Release = serde_json::from_str(answer).context("read GitHub's answer")?;
    let tag = release.tag_name;
    let version = tag.strip_prefix('v').with_context(|| format!("the newest release is tagged {tag}, which is not v and a version"))?;
    let newest = semver::Version::parse(version).with_context(|| format!("the newest release is tagged {tag}"))?;
    let this = semver::Version::parse(running).with_context(|| format!("this build's version is {running}"))?;
    // Each zip is named skyward-, the tag, the phone's codename, and the
    // stock build the image is made on.
    let start = format!("skyward-{tag}-{codename}-");
    let build = match release.assets.into_iter().find(|a| a.name.starts_with(&start) && a.name.ends_with(".zip")) {
        None => None,
        Some(a) => {
            let sha256 = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).with_context(|| format!("GitHub gives no SHA-256 of {}", a.name))?;
            ensure!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()), "GitHub gives {} the SHA-256 {sha256}", a.name);
            Some(Build { sha256: sha256.to_lowercase(), name: a.name, url: a.browser_download_url, size: a.size })
        }
    };
    Ok(Newest { version: version.to_string(), newer: newest > this, build })
}

/// Ask GitHub for Skyward's newest release, and its zip for the phone
/// `codename` names, as the build of version `running`.
pub fn ask(agent: &ureq::Agent, codename: &str, running: &str) -> Result<Newest> {
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", format!("Skyward/{running}"))
        .call()
        .with_context(|| format!("ask {LATEST}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {LATEST}"))?;
    if !status.is_success() {
        // GitHub says why, as when too many questions came from the address
        // that asked.
        match serde_json::from_str::<Message>(&body) {
            Ok(m) => bail!("GitHub answered {status}: {}", m.message),
            Err(_) => bail!("GitHub answered {status}"),
        }
    }
    newest(&body, codename, running)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What GitHub answered of the newest release on 2026-11-07, with the
    /// parts that do not matter here left out.
    const ANSWERED_013: &str = r#"{"url":"https://api.github.com/repos/kirklin/skyward/releases/437365758","tag_name":"v0.1.3","name":"v0.1.3","draft":false,"prerelease":false,"assets":[{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751468","id":719751468,"name":"SHA256SUMS","label":"","content_type":"application/octet-stream","state":"uploaded","size":2337,"digest":"sha256:052f9ca757baeb2eac78c8e546bcc54178f6bb9bf9e5bde53fa81b3f48448957","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/SHA256SUMS"},{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751598","id":719751598,"name":"skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip","label":"Xiaomi 13 Ultra (ishtar)","content_type":"application/zip","state":"uploaded","size":42739608,"digest":"sha256:62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip"},{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751747","id":719751747,"name":"skyward-v0.1.3-nezha-OS4.0.4.0.XPACNXM.zip","label":"Xiaomi 17 Ultra (nezha)","content_type":"application/zip","state":"uploaded","size":43145196,"digest":"sha256:9f6165079529418d8f6635beddfdd9abf9a70c89d69eb467a9922b0db69bfaf1","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-nezha-OS4.0.4.0.XPACNXM.zip"}]}"#;

    #[test]
    fn the_newest_release_gives_the_zip_for_the_phone() {
        let n = newest(ANSWERED_013, "ishtar", "0.1.2").unwrap();
        assert_eq!(n.version, "0.1.3");
        assert!(n.newer);
        assert_eq!(
            n.build,
            Some(Build {
                name: "skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip".into(),
                url: "https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip".into(),
                size: 42739608,
                sha256: "62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534".into(),
            })
        );
        assert_eq!(newest(ANSWERED_013, "nezha", "0.1.2").unwrap().build.unwrap().size, 43145196);
    }

    #[test]
    fn a_release_is_newer_only_than_an_older_build() {
        assert!(!newest(ANSWERED_013, "ishtar", "0.1.3").unwrap().newer, "the same version");
        assert!(!newest(ANSWERED_013, "ishtar", "0.2.0").unwrap().newer, "a build ahead of every release");
        assert!(newest(ANSWERED_013, "ishtar", "0.1.3-rc.1").unwrap().newer, "a build before the release it led to");
        assert!(newest(ANSWERED_013, "ishtar", "0.1").is_err(), "a version this build cannot have");
    }

    #[test]
    fn a_phone_is_found_by_its_whole_codename() {
        assert_eq!(newest(ANSWERED_013, "fuxi", "0.1.2").unwrap().build, None, "a phone the release has no zip for");
        assert_eq!(newest(ANSWERED_013, "ish", "0.1.2").unwrap().build, None, "the start of another phone's codename");
        assert_eq!(newest(ANSWERED_013, "SHA256SUMS", "0.1.2").unwrap().build, None);
    }

    #[test]
    fn a_zip_needs_its_sha256_and_a_release_its_version() {
        let unsummed = ANSWERED_013.replace("\"digest\":\"sha256:62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534\"", "\"digest\":null");
        let e = newest(&unsummed, "ishtar", "0.1.2").unwrap_err();
        assert_eq!(e.to_string(), "GitHub gives no SHA-256 of skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip");
        assert!(newest(&unsummed, "nezha", "0.1.2").is_ok(), "another phone's zip has its own");
        let untagged = ANSWERED_013.replace("\"tag_name\":\"v0.1.3\"", "\"tag_name\":\"nightly\"");
        assert!(newest(&untagged, "ishtar", "0.1.2").is_err());
        assert!(newest("{}", "ishtar", "0.1.2").is_err());
    }
}
