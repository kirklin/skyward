use std::collections::HashMap;

/// Where the phone keeps the rules for its own device nodes, and the names
/// its own partitions give to the accounts those rules name.
pub const RULE_FILES: [&str; 2] = ["/vendor/etc/ueventd.rc", "/odm/etc/ueventd.rc"];
pub const PASSWD_FILES: [&str; 2] = ["/vendor/etc/passwd", "/odm/etc/passwd"];
pub const GROUP_FILES: [&str; 2] = ["/vendor/etc/group", "/odm/etc/group"];

/// The accounts Android fixes once and for all. The rules, and the init
/// scripts of the phone's own programs, name them rather than their
/// numbers, and a recovery has no list of its own to look them up in.
const ACCOUNTS: [(&str, u32); 34] = [
    ("root", 0),
    ("system", 1000),
    ("radio", 1001),
    ("bluetooth", 1002),
    ("graphics", 1003),
    ("input", 1004),
    ("audio", 1005),
    ("camera", 1006),
    ("log", 1007),
    ("compass", 1008),
    ("mount", 1009),
    ("wifi", 1010),
    ("adb", 1011),
    ("install", 1012),
    ("media", 1013),
    ("dhcp", 1014),
    ("sdcard_rw", 1015),
    ("vpn", 1016),
    ("keystore", 1017),
    ("usb", 1018),
    ("drm", 1019),
    ("gps", 1021),
    ("media_rw", 1023),
    ("mtp", 1024),
    ("drmrpc", 1026),
    ("nfc", 1027),
    ("audioserver", 1041),
    ("cameraserver", 1047),
    ("shell", 2000),
    ("inet", 3003),
    ("readproc", 3009),
    ("wakelock", 3010),
    ("uhid", 3011),
    ("nobody", 9999),
];

/// What a rule says one or more device nodes should look like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The node's path, which may end in `*` to stand for a family of them.
    pub path: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

impl Rule {
    pub fn matches(&self, node: &str) -> bool {
        match self.path.strip_suffix('*') {
            Some(prefix) => node.starts_with(prefix),
            None => node == self.path,
        }
    }
}

/// The accounts a phone adds to the fixed ones, read from the passwd and
/// group files on its own partitions.
pub fn accounts(files: &[&str]) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = ACCOUNTS.iter().map(|&(n, id)| (n.to_string(), id)).collect();
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines() {
            let mut fields = line.split(':');
            let (Some(name), Some(_), Some(id)) = (fields.next(), fields.next(), fields.next()) else { continue };
            if let Ok(id) = id.parse() {
                out.insert(name.to_string(), id);
            }
        }
    }
    out
}

/// What a rule calls an account, as a number. The phone's own programs are
/// given accounts of their own, numbered in their names.
pub fn account(known: &HashMap<String, u32>, name: &str) -> Option<u32> {
    if let Some(id) = known.get(name) {
        return Some(*id);
    }
    name.strip_prefix("oem_").and_then(|n| n.parse().ok())
}

/// The device-node rules in a ueventd file, in the order they are written:
/// where two of them name the same node, the later one has the say. Lines
/// about files under /sys say which attribute they are about and are left
/// to the phone's own programs.
pub fn rules(text: &str, known: &HashMap<String, u32>) -> Vec<Rule> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("/dev/") {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [path, mode, user, group, ..] = fields[..] else { continue };
        let (Ok(mode), Some(uid), Some(gid)) = (u32::from_str_radix(mode.trim_start_matches('0'), 8), account(known, user), account(known, group)) else {
            continue;
        };
        out.push(Rule { path: path.to_string(), mode, uid, gid });
    }
    out
}

/// Give every device node the owner and permissions the phone's own rules
/// give it. A recovery's own rules were all it had when it started, so the
/// nodes its vendor partition describes are left as the kernel made them,
/// and programs that give up being root cannot open them.
#[cfg(target_os = "android")]
pub fn apply() -> anyhow::Result<usize> {
    use anyhow::Context;
    use std::path::Path;
    let known = accounts(&PASSWD_FILES.iter().chain(GROUP_FILES.iter()).copied().collect::<Vec<_>>());
    let mut rules = Vec::new();
    for file in RULE_FILES {
        if let Ok(text) = std::fs::read_to_string(file) {
            rules.extend(super::ueventd::rules(&text, &known));
        }
    }
    let mut changed = 0;
    for node in nodes(Path::new("/dev"))? {
        if let Some(rule) = rules.iter().rev().find(|r| r.matches(&node)) {
            set(&node, rule).with_context(|| format!("give {node} the owner and permissions its phone describes"))?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Every device node under `dir`, which a recovery's /dev holds few enough
/// of to walk whole.
#[cfg(target_os = "android")]
fn nodes(dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    use anyhow::Context;
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
        };
        for entry in listing {
            let entry = entry.with_context(|| format!("read {}", dir.display()))?;
            let kind = entry.file_type().with_context(|| format!("look at {}", entry.path().display()))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else {
                out.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    Ok(out)
}

#[cfg(target_os = "android")]
fn set(node: &str, rule: &Rule) -> anyhow::Result<()> {
    use anyhow::Context;
    use std::ffi::CString;
    let path = CString::new(node)?;
    if unsafe { libc::chown(path.as_ptr(), rule.uid, rule.gid) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set the owner");
    }
    if unsafe { libc::chmod(path.as_ptr(), rule.mode as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set the permissions");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashMap<String, u32> {
        accounts(&[])
    }

    #[test]
    fn a_rule_gives_a_node_its_owner_and_permissions() {
        let parsed = rules("/dev/smcinvoke            0660   system     drmrpc\n", &known());
        assert_eq!(parsed, vec![Rule { path: "/dev/smcinvoke".into(), mode: 0o660, uid: 1000, gid: 1026 }]);
    }

    #[test]
    fn lines_that_are_not_about_a_device_node_are_passed_over() {
        let text = "# a comment\n\
                    subsystem block\n\
                    /sys/class/qdss_bridge/mhi_qdss  mode      0660     system   oem_2902\n\
                    /dev/kgsl                 0666   system     system\n";
        assert_eq!(rules(text, &known()).len(), 1);
    }

    #[test]
    fn a_phones_own_accounts_are_read_from_its_name_for_them() {
        let parsed = rules("/dev/diag 0660 system oem_2901\n", &known());
        assert_eq!(parsed[0].gid, 2901);
    }

    #[test]
    fn a_name_that_means_nothing_here_leaves_the_rule_out() {
        assert!(rules("/dev/odd 0660 nosuchuser system\n", &known()).is_empty());
        assert!(rules("/dev/odd 0660 system\n", &known()).is_empty());
    }

    #[test]
    fn a_rule_can_stand_for_a_family_of_nodes() {
        let parsed = rules("/dev/dma_heap/qcom,*      0444   system     system\n", &known());
        assert!(parsed[0].matches("/dev/dma_heap/qcom,qseecom"));
        assert!(!parsed[0].matches("/dev/dma_heap/system"));
        let exact = rules("/dev/kgsl 0666 system system\n", &known());
        assert!(exact[0].matches("/dev/kgsl"));
        assert!(!exact[0].matches("/dev/kgsl-3d0"));
    }

    #[test]
    fn the_later_of_two_rules_for_a_node_has_the_say() {
        let parsed = rules("/dev/x 0600 root root\n/dev/x 0660 system drmrpc\n", &known());
        let last = parsed.iter().rev().find(|r| r.matches("/dev/x")).expect("a rule for it");
        assert_eq!((last.mode, last.uid, last.gid), (0o660, 1000, 1026));
    }

    #[test]
    fn accounts_a_phone_adds_are_taken_from_its_own_files() {
        let dir = std::env::temp_dir().join(format!("skyward-ueventd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let passwd = dir.join("passwd");
        std::fs::write(&passwd, "vendor_qti_diag::2901:2901::/:/bin/sh\n").unwrap();
        let known = accounts(&[passwd.to_str().unwrap()]);
        assert_eq!(account(&known, "vendor_qti_diag"), Some(2901));
        assert_eq!(account(&known, "system"), Some(1000));
        assert_eq!(account(&known, "nothing"), None);
    }
}
