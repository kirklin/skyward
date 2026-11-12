//! Skyward's own releases on GitHub: which is the newest, and the zip in
//! it for one phone, as the release workflow names each phone's.

use anyhow::{anyhow, bail, ensure, Context, Error, Result};
use serde::Deserialize;

/// Where GitHub answers which release of Skyward is the newest. This build
/// of Skyward is not the one upstream hands out: it is the one for the MIX
/// Flip 2, and it asks its own releases where the next build of itself is.
pub const LATEST: &str = "https://api.github.com/repos/XQAB/skyward/releases/latest";

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
    /// Where GitHub's API hands it out, which [`locate`] asks for where the
    /// file itself is.
    pub url: String,
    /// The release's own link to it on github.com.
    pub link: String,
    pub size: u64,
    /// Its SHA-256 as GitHub gives it, in hex.
    pub sha256: String,
}

/// A mirror that some networks reach far faster than github.com, which
/// caps what it gives them to a crawl. What a mirror hands over is checked
/// against the SHA-256 GitHub itself took of the file as it was uploaded, so
/// a mirror cannot pass off another file as this build, and one that is
/// down, or that gives something else, costs only the try before the file
/// itself is asked for where GitHub sends it.
pub const MIRROR: &str = "https://gh-proxy.com/";

impl Build {
    /// Where to download the zip from, in the order to try: through the
    /// mirror the pages chose, when one was chosen, then the file's own
    /// short-lived address where GitHub's API gave one, then the release's
    /// link on github.com.
    pub fn sources(&self, located: Option<String>, mirror: Option<&str>) -> Vec<String> {
        let mut urls: Vec<String> = mirror.map(|m| format!("{m}{}", self.link)).into_iter().collect();
        urls.extend(located);
        urls.push(self.link.clone());
        urls
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    url: String,
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

/// A version as semver reads it. Releases of this build are numbered the way
/// its images are named, 0.44.00, which semver refuses: its numbers carry no
/// leading zero. Only the zero is taken off, the shape stays as it is, so a
/// pre-release such as 0.1.3-rc.1 is still read as one.
fn numbered(text: &str) -> String {
    let mut out = String::new();
    for (i, part) in text.split('.').enumerate() {
        if i > 0 {
            out.push('.');
        }
        let run: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if run.is_empty() {
            out.push_str(part);
            continue;
        }
        let digits = &run[run.chars().take_while(|&c| c == '0').count()..];
        // A number of nothing but zeros is one zero: 00 is 0.
        out.push_str(if digits.is_empty() { "0" } else { digits });
        out.push_str(&part[run.len()..]);
    }
    out
}

/// The newest release that `answer`, GitHub's answer at [`LATEST`], tells
/// of, and its zip for the phone `codename` names, as the build of version
/// `running` sees it.
pub fn newest(answer: &str, codename: &str, running: &str) -> Result<Newest> {
    let release: Release = serde_json::from_str(answer).context("read GitHub's answer")?;
    let tag = release.tag_name;
    let version = tag.strip_prefix('v').with_context(|| format!("the newest release is tagged {tag}, which is not v and a version"))?;
    let numbered_version = numbered(version);
    let newest = semver::Version::parse(&numbered_version).with_context(|| format!("the newest release is tagged {tag}"))?;
    let this = semver::Version::parse(&numbered(running)).with_context(|| format!("this build's version is {running}"))?;
    // Each asset is named skyward-, the tag, the phone's codename, and the
    // stock build the image is made on: a zip holding the image, or the bare
    // image itself.
    let start = format!("skyward-{tag}-{codename}-");
    // Both the bare image and the zip of it are published nowadays, so the
    // choice cannot be left to the order GitHub happens to list them in.
    // The bare image is taken when it is there: no unpacking, no zstd.
    let mut assets: Vec<Asset> = release
        .assets
        .into_iter()
        .filter(|a| a.name.starts_with(&start) && (a.name.ends_with(".zip") || a.name.ends_with(".img")))
        .collect();
    let bare = assets.iter().position(|a| a.name.ends_with(".img"));
    let build = match match bare {
        Some(at) => Some(assets.swap_remove(at)),
        None if !assets.is_empty() => Some(assets.swap_remove(0)),
        None => None,
    } {
        None => None,
        Some(a) => {
            let sha256 = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).with_context(|| format!("GitHub gives no SHA-256 of {}", a.name))?;
            ensure!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()), "GitHub gives {} the SHA-256 {sha256}", a.name);
            Some(Build { sha256: sha256.to_lowercase(), name: a.name, url: a.url, link: a.browser_download_url, size: a.size })
        }
    };
    Ok(Newest { version: numbered_version, newer: newest > this, build })
}

/// The mirrors known to carry GitHub's API, and not only the files it hands
/// out, so that asking what the newest release is can go through one as well.
const API_MIRRORS: [&str; 3] = ["gh-proxy.com", "gh-proxy.org", "gh-proxy.cc"];

/// Where the question is sent when the mirror chosen for the files carries
/// only the files: ghfast hands the package out quickly, but answers the
/// question with a refusal.
const QUERY_MIRRORS: [&str; 2] = ["https://gh-proxy.com/", "https://gh-proxy.org/"];

/// Whether `mirror` carries GitHub's API, not just its files.
fn carries_api(mirror: &str) -> bool {
    API_MIRRORS.iter().any(|host| mirror.contains(host))
}

/// Ask GitHub for Skyward's newest release, and its zip for the phone
/// `codename` names, as the build of version `running`, through `mirror`
/// where that one carries the API too, and from GitHub itself otherwise.
pub fn ask(agent: &ureq::Agent, codename: &str, running: &str, mirror: Option<&str>) -> Result<Newest> {
    let mut asked = Vec::new();
    match mirror.filter(|m| !m.is_empty()) {
        // The mirror chosen for the files answers the question too, where it
        // carries the API.
        Some(m) if carries_api(m) => asked.push(format!("{m}{LATEST}")),
        // Where it carries only the files, the question goes through a mirror
        // that carries the API, so that a fast download mirror still gets a
        // check that answers.
        Some(_) => asked.extend(QUERY_MIRRORS.iter().map(|m| format!("{m}{LATEST}"))),
        // No mirror chosen: GitHub itself is asked, as it always was.
        None => {}
    }
    asked.push(LATEST.to_string());
    let mut last: Option<Error> = None;
    for url in asked {
        match asked_at(agent, &url, codename, running) {
            Ok(newest) => return Ok(newest),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("GitHub was not asked")))
}

fn asked_at(agent: &ureq::Agent, url: &str, codename: &str, running: &str) -> Result<Newest> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", format!("Skyward/{running}"))
        .call()
        .with_context(|| format!("ask {url}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {url}"))?;
    if !status.is_success() {
        bail!(refusal(status, &body));
    }
    newest(&body, codename, running)
}

/// Where the file a build's `url` hands out is now, as GitHub's API sends
/// the build of version `running` that asks for it on: a short-lived
/// address of the file itself, away from github.com.
pub fn locate(agent: &ureq::Agent, url: &str, running: &str) -> Result<String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/octet-stream")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", format!("Skyward/{running}"))
        .config()
        .max_redirects(0)
        .build()
        .call()
        .with_context(|| format!("ask {url}"))?;
    let status = response.status();
    if status.is_redirection() {
        let location = response.headers().get(ureq::http::header::LOCATION).with_context(|| format!("GitHub answered {status}, and names no address"))?;
        return Ok(location.to_str().context("GitHub names an address that is not text")?.to_string());
    }
    ensure!(!status.is_success(), "GitHub sends the file itself in its answer, at no address of its own");
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {url}"))?;
    bail!(refusal(status, &body))
}

/// Why GitHub turned a question down, as it says, as when too many questions
/// came from the address that asked.
fn refusal(status: ureq::http::StatusCode, body: &str) -> String {
    match serde_json::from_str::<Message>(body) {
        Ok(m) => format!("GitHub answered {status}: {}", m.message),
        Err(_) => format!("GitHub answered {status}"),
    }
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
                url: "https://api.github.com/repos/kirklin/skyward/releases/assets/719751598".into(),
                link: "https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip".into(),
                size: 42739608,
                sha256: "62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534".into(),
            })
        );
        assert_eq!(newest(ANSWERED_013, "nezha", "0.1.2").unwrap().build.unwrap().size, 43145196);
    }

    #[test]
    fn a_zip_is_tried_through_a_mirror_then_from_where_github_sends_it() {
        let build = newest(ANSWERED_013, "ishtar", "0.1.2").unwrap().build.unwrap();
        let link = "https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip";
        let mirrored = format!("{MIRROR}{link}");
        assert_eq!(
            build.sources(Some("https://release-assets.githubusercontent.com/a?sig=1".into()), Some(MIRROR)),
            [mirrored.clone(), "https://release-assets.githubusercontent.com/a?sig=1".into(), link.into()],
            "the mirror first, then the address GitHub sent it to, then its link"
        );
        assert_eq!(build.sources(None, Some(MIRROR)), [mirrored, link.into()], "where GitHub's API gave no address");
        assert_eq!(build.sources(None, None), vec![link.to_string()], "with no mirror chosen, straight to GitHub");
    }

    /// A server on this computer that answers one request with `answer`,
    /// and gives the request it had.
    fn server(answer: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/repos/kirklin/skyward/releases/assets/719751598", listener.local_addr().unwrap());
        let held = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 512];
            while !request.ends_with(b"\r\n\r\n") {
                let n = s.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
            }
            s.write_all(answer.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, held)
    }

    #[test]
    fn github_s_api_sends_a_zip_on_to_where_the_file_is() {
        let agent = crate::http::agent(Default::default());
        let sent_on = "HTTP/1.1 302 Found\r\nLocation: https://release-assets.githubusercontent.com/a?sig=1\r\nContent-Length: 0\r\n\r\n";
        let (url, held) = server(sent_on.into());
        assert_eq!(locate(&agent, &url, "0.1.2").unwrap(), "https://release-assets.githubusercontent.com/a?sig=1", "the address, not followed");
        let request = held.join().unwrap().to_lowercase();
        assert!(request.contains("accept: application/octet-stream\r\n"), "asked for the file itself: {request}");
        assert!(request.contains("user-agent: skyward/0.1.2\r\n"), "{request}");
        let why = r#"{"message":"API rate limit exceeded for 203.0.113.7"}"#;
        let (url, _) = server(format!("HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{why}", why.len()));
        assert_eq!(locate(&agent, &url, "0.1.2").unwrap_err().to_string(), "GitHub answered 403 Forbidden: API rate limit exceeded for 203.0.113.7");
        let (url, _) = server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nPK".into());
        assert!(locate(&agent, &url, "0.1.2").is_err(), "the file itself, with no address to download it from again");
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
    fn a_release_numbered_as_its_image_is_named_is_read_as_its_number() {
        let answer = ANSWERED_013.replace("v0.1.3", "v0.44.00");
        let n = newest(&answer, "ishtar", "0.43.00").unwrap();
        assert_eq!(n.version, "0.44.0");
        assert!(n.newer, "0.44.00 comes after 0.43.00");
        assert!(!newest(&answer, "ishtar", "0.44.00").unwrap().newer, "the same build");
        assert!(n.build.is_some(), "its zip is named after the tag as it is");
        assert_eq!(numbered("0.1.3-rc.1"), "0.1.3-rc.1", "a pre-release keeps its shape");
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
