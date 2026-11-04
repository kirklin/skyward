//! Where a root manager's APK comes from when it is downloaded rather than
//! installed already: the three managers that patch a boot image, and the two
//! tiers each is built in. GitHub lists a manager's releases as JSON, and the
//! APK is one of the files each release carries.

/// A root manager whose APK can be downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// KernelSU, the one most phones run.
    Official,
    /// SukiSU Ultra, a KernelSU fork.
    SukiSu,
    /// BakaSU, the one that was ReSukiSU.
    BakaSu,
}

/// Which build of it: the release marked stable, or the newest it has, which
/// may be a pre-release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Stable,
    Dev,
}

/// Where a downloaded manager's package is kept.
pub const MANAGERS: &str = "/sdcard/downloaded_manager";

/// The packages a download left in [`MANAGERS`], the newest first: one of
/// them can be chosen as it is, rather than fetched again.
pub fn downloaded() -> Vec<std::path::PathBuf> {
    let mut found: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(MANAGERS) else { return Vec::new() };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().to_ascii_lowercase().ends_with(".apk") {
            continue;
        }
        let at = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        found.push((at, path));
    }
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found.into_iter().map(|(_, path)| path).collect()
}

/// The ways to reach GitHub a page offers: itself, and the services that
/// carry it faster where it is slow.
pub const MIRRORS: [(&str, &str); 5] = [
    ("直连", ""),
    ("huime.top", "https://proxy.huime.top/"),
    ("gh-proxy.com", "https://gh-proxy.com/"),
    ("ghfast.top", "https://ghfast.top/"),
    ("ghproxy.net", "https://ghproxy.net/"),
];

/// `url` reached through `mirror`, or as it is where there is none.
pub fn through(mirror: &str, url: &str) -> String {
    if mirror.is_empty() {
        url.to_string()
    } else {
        format!("{mirror}{url}")
    }
}

/// Where the newest release of `variant` is described.
pub fn latest(variant: Variant) -> String {
    format!("https://api.github.com/repos/{}/releases/latest", variant.repo())
}

/// Where the package the newest run of `variant`'s manager workflow built is
/// fetched from: nightly.link hands out what a workflow built, without a
/// token, which is the only way to a manager that publishes no release.
/// Where the newest releases of `variant` are listed, pre-releases among
/// them: a manager whose workflow builds nothing right now still publishes
/// its package there.
pub fn newest(variant: Variant) -> String {
    format!("https://api.github.com/repos/{}/releases?per_page=5", variant.repo())
}

pub fn nightly(variant: Variant) -> String {
    // Each repository names what its workflow uploads, and the name is what
    // nightly.link hands out: KernelSU's is lower case, the two forks' is not.
    // Each repository names its workflow and what it uploads, and both are
    // what nightly.link hands out: one leaves the extension off the workflow
    // and names its build, the others do not.
    let (workflow, package) = match variant {
        Variant::Official => ("build-manager.yml", "manager.zip"),
        Variant::SukiSu => ("build-manager.yml", "Manager.zip"),
        Variant::BakaSu => ("build-manager", "Manager-release.zip"),
    };
    format!("https://nightly.link/{}/workflows/{workflow}/main/{package}", variant.repo())
}

impl Variant {
    /// The repository it is built in.
    pub fn repo(self) -> &'static str {
        match self {
            Variant::Official => "tiann/KernelSU",
            Variant::SukiSu => "SukiSU-Ultra/SukiSU-Ultra",
            Variant::BakaSu => "Baka-SU/BakaSU",
        }
    }

    /// What it is called, as the download page names it.
    pub fn name(self) -> &'static str {
        match self {
            Variant::Official => "KernelSU",
            Variant::SukiSu => "SukiSU Ultra",
            Variant::BakaSu => "BakaSU",
        }
    }

    /// The two branches and the two tiers, in the order a page lists them.
    pub fn all() -> [(Variant, Tier); 6] {
        [
            (Variant::Official, Tier::Stable),
            (Variant::Official, Tier::Dev),
            (Variant::SukiSu, Tier::Stable),
            (Variant::SukiSu, Tier::Dev),
            (Variant::BakaSu, Tier::Stable),
            (Variant::BakaSu, Tier::Dev),
        ]
    }

    /// The URL whose answer lists its releases, newest first.
    pub fn releases(self) -> String {
        format!("https://api.github.com/repos/{}/releases?per_page=20", self.repo())
    }
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Stable => "stable",
            Tier::Dev => "dev",
        }
    }
}

/// Whether a manager publishes its package in its releases at all: the two
/// that do have a stable tier, and one that builds in CI alone has none.
pub fn has_releases(variant: Variant) -> bool {
    matches!(variant, Variant::Official | Variant::SukiSu)
}

/// The APK to download from what `releases` answered: the newest release that
/// carries one, a pre-release only for the dev tier.
pub fn apk_in(json: &str, tier: Tier) -> Option<String> {
    // Each release holds `"prerelease": true|false` and the files it carries;
    // a release is taken whole, so the fields are read in the order GitHub
    // writes them and the search starts again at the next release.
    for release in json.split("\"prerelease\"").skip(1) {
        let Some((flag, rest)) = release.split_once(',') else { continue };
        let pre = flag.trim().trim_start_matches(':').trim() == "true";
        if pre && tier == Tier::Stable {
            continue;
        }
        // The files of this release end where the next one begins.
        let files = rest.split("\"prerelease\"").next().unwrap_or(rest);
        for file in files.split("\"browser_download_url\"").skip(1) {
            let start = file.find("https://")?;
            let url = &file[start..];
            let end = url.find('"')?;
            let url = &url[..end];
            if url.to_ascii_lowercase().ends_with(".apk") {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// The file name to save `url` as: what it ends with.
pub fn file_name(url: &str) -> String {
    url.rsplit('/').next().unwrap_or("manager.apk").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASES: &str = r#"[
      {"tag_name": "v3.0", "prerelease": false, "assets": [
        {"name": "notes.txt", "browser_download_url": "https://example.test/notes.txt"},
        {"name": "KernelSU.apk", "browser_download_url": "https://example.test/v3.0/KernelSU.apk"}
      ]},
      {"tag_name": "v3.1-beta", "prerelease": true, "assets": [
        {"name": "KernelSU.apk", "browser_download_url": "https://example.test/v3.1/KernelSU.apk"}
      ]}
    ]"#;

    #[test]
    fn the_stable_tier_takes_the_newest_release_that_is_not_a_pre_release() {
        assert_eq!(apk_in(RELEASES, Tier::Stable).as_deref(), Some("https://example.test/v3.0/KernelSU.apk"));
    }

    #[test]
    fn the_dev_tier_takes_a_pre_release_too() {
        // The list is newest first, so the pre-release comes second here;
        // the stable one is still the first that carries an APK.
        assert_eq!(apk_in(RELEASES, Tier::Dev).as_deref(), Some("https://example.test/v3.0/KernelSU.apk"));
        let only_pre = r#"[{"prerelease": true, "assets": [{"browser_download_url": "https://example.test/b/KernelSU.apk"}]}]"#;
        assert_eq!(apk_in(only_pre, Tier::Dev).as_deref(), Some("https://example.test/b/KernelSU.apk"));
        assert_eq!(apk_in(only_pre, Tier::Stable), None);
    }

    #[test]
    fn a_release_with_no_apk_is_passed_over() {
        let json = r#"[{"prerelease": false, "assets": [{"browser_download_url": "https://example.test/notes.txt"}]},
                       {"prerelease": false, "assets": [{"browser_download_url": "https://example.test/old/KernelSU.apk"}]}]"#;
        assert_eq!(apk_in(json, Tier::Stable).as_deref(), Some("https://example.test/old/KernelSU.apk"));
    }

    #[test]
    fn a_file_is_named_by_the_end_of_its_url() {
        assert_eq!(file_name("https://example.test/v3.0/KernelSU.apk"), "KernelSU.apk");
        assert_eq!(file_name("https://example.test"), "example.test");
    }

    #[test]
    fn each_manager_names_its_own_repository() {
        assert_eq!(Variant::Official.releases(), "https://api.github.com/repos/tiann/KernelSU/releases?per_page=20");
        assert!(Variant::SukiSu.releases().contains("SukiSU-Ultra"));
    }
}
