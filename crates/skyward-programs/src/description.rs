//! The descriptions of the services a phone's own programs register, as
//! its partitions carry them for the service directories to read: which
//! interfaces the phone has, of which kind, and under which names.

use anyhow::{bail, ensure, Context, Result};

/// The two kinds of hardware service, each with a directory of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The newer kind, registered with the service directory.
    Aidl,
    /// The older kind, registered with the directory for hardware services.
    Hidl,
}

/// One service a description tells of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    pub kind: Kind,
    /// The package the interface is in, such as
    /// android.hardware.security.keymint.
    pub package: String,
    /// The version of the package: a number for the newer kind, where the
    /// description gives one, and two for the older, such as 1.0.
    pub version: Option<String>,
    /// The interface, such as IKeyMintDevice.
    pub interface: String,
    /// Which of them it is, such as default.
    pub instance: String,
}

impl Described {
    /// The name the service is registered under: for the newer kind the
    /// package, the interface and the instance; for the older the package
    /// at its version, the interface and the instance.
    pub fn name(&self) -> String {
        match (self.kind, &self.version) {
            (Kind::Hidl, Some(version)) => format!("{}@{version}::{}/{}", self.package, self.interface, self.instance),
            _ => format!("{}.{}/{}", self.package, self.interface, self.instance),
        }
    }

    /// The service of the newer kind registered under `name`, which reads
    /// as the package, the interface and then which of them it is; its
    /// version is no part of its name.
    pub fn named(name: &str) -> Result<Described> {
        let parts = name.rsplit_once('/').and_then(|(interface, instance)| interface.rsplit_once('.').map(|(package, interface)| (package, interface, instance)));
        let (package, interface, instance) = parts.with_context(|| format!("{name} does not read as a package, an interface and which of them it is"))?;
        ensure!(![package, interface, instance].iter().any(|part| part.is_empty() || part.contains(['<', '>', '&', '@', ':'])), "{name} is not how a service of the newer kind is named");
        Ok(Described { kind: Kind::Aidl, package: package.into(), version: None, interface: interface.into(), instance: instance.into() })
    }
}

/// A description that tells of the services of the newer kind registered
/// under `names`, each once, as what the running system offers. A recovery
/// has no framework of its own, and gives its service directory what the
/// phone's own programs register in the place the framework's descriptions
/// are read from: the directory takes a description that says it is the
/// phone's own there for none at all.
pub fn offering(names: &[String]) -> Result<String> {
    let mut out = String::from("<manifest version=\"1.0\" type=\"framework\">\n");
    let mut told: Vec<&String> = Vec::new();
    for name in names {
        if told.contains(&name) {
            continue;
        }
        told.push(name);
        let service = Described::named(name)?;
        out.push_str("    <hal format=\"aidl\">\n");
        out.push_str(&format!("        <name>{}</name>\n", service.package));
        out.push_str(&format!("        <fqname>{}/{}</fqname>\n", service.interface, service.instance));
        out.push_str("    </hal>\n");
    }
    out.push_str("</manifest>\n");
    Ok(out)
}

/// The services the description `text` tells of. A service is given either
/// as its interface with its instances, under the versions the description
/// lists, or as a name of its own that holds them all; one given both ways
/// is told once. What the description holds of other kinds, as libraries
/// are, is passed over.
pub fn read(text: &str) -> Result<Vec<Described>> {
    let doc = roxmltree::Document::parse(text).context("read the description as XML")?;
    let root = doc.root_element();
    ensure!(root.has_tag_name("manifest"), "the description is a {}, not a manifest", root.tag_name().name());
    let mut out: Vec<Described> = Vec::new();
    for hal in root.children().filter(|n| n.has_tag_name("hal")) {
        let kind = match hal.attribute("format").unwrap_or("hidl") {
            "aidl" => Kind::Aidl,
            "hidl" => Kind::Hidl,
            _ => continue,
        };
        let text_of = |node: roxmltree::Node, tag: &str| -> Result<String> {
            let child = node.children().find(|n| n.has_tag_name(tag)).with_context(|| format!("a {} without its {tag}", node.tag_name().name()))?;
            Ok(child.text().unwrap_or_default().trim().to_string())
        };
        let package = text_of(hal, "name")?;
        let versions: Vec<String> = hal.children().filter(|n| n.has_tag_name("version")).map(|n| n.text().unwrap_or_default().trim().to_string()).collect();
        ensure!(kind == Kind::Aidl || !versions.is_empty() || hal.children().all(|n| !n.has_tag_name("interface")), "{package} is of the older kind and gives no version");
        let mut told = |described: Described| {
            if !out.contains(&described) {
                out.push(described);
            }
        };
        for interface in hal.children().filter(|n| n.has_tag_name("interface")) {
            let name = text_of(interface, "name")?;
            for instance in interface.children().filter(|n| n.has_tag_name("instance")) {
                let instance = instance.text().unwrap_or_default().trim().to_string();
                match kind {
                    // A service of the newer kind has one version, which
                    // is no part of its name.
                    Kind::Aidl => told(Described { kind, package: package.clone(), version: versions.first().cloned(), interface: name.clone(), instance }),
                    Kind::Hidl => {
                        for version in &versions {
                            told(Described { kind, package: package.clone(), version: Some(version.clone()), interface: name.clone(), instance: instance.clone() });
                        }
                    }
                }
            }
        }
        for fqname in hal.children().filter(|n| n.has_tag_name("fqname")) {
            let fqname = fqname.text().unwrap_or_default().trim();
            let (interface, instance) = fqname.split_once('/').with_context(|| format!("{package} names {fqname}, which is no interface and instance"))?;
            let described = match (kind, interface.strip_prefix('@')) {
                (Kind::Aidl, None) => Described { kind, package: package.clone(), version: versions.first().cloned(), interface: interface.into(), instance: instance.into() },
                (Kind::Hidl, Some(versioned)) => {
                    let (version, interface) = versioned.split_once("::").with_context(|| format!("{package} names {fqname}, which gives no version and interface"))?;
                    Described { kind, package: package.clone(), version: Some(version.into()), interface: interface.into(), instance: instance.into() }
                }
                _ => bail!("{package} names {fqname}, which is not how a service of its kind is named"),
            };
            told(described);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_of_the_newer_kind_is_named_by_its_package_interface_and_instance() {
        let text = r#"<manifest version="1.0" type="device">
            <hal format="aidl">
                <name>android.hardware.security.keymint</name>
                <version>3</version>
                <fqname>IKeyMintDevice/default</fqname>
                <fqname>IRemotelyProvisionedComponent/default</fqname>
            </hal>
            <hal format="aidl">
                <name>android.hardware.weaver</name>
                <interface>
                    <name>IWeaver</name>
                    <instance>default</instance>
                </interface>
            </hal>
        </manifest>"#;
        let names: Vec<String> = read(text).unwrap().iter().map(Described::name).collect();
        assert_eq!(
            names,
            [
                "android.hardware.security.keymint.IKeyMintDevice/default",
                "android.hardware.security.keymint.IRemotelyProvisionedComponent/default",
                "android.hardware.weaver.IWeaver/default",
            ]
        );
        let all = read(text).unwrap();
        assert_eq!((all[0].kind, all[0].version.as_deref(), all[2].version.as_deref()), (Kind::Aidl, Some("3"), None));
    }

    #[test]
    fn a_service_of_the_older_kind_is_named_at_each_version_and_is_that_kind_unless_said_otherwise() {
        let text = r#"<manifest version="1.0" type="device">
            <hal>
                <name>android.hardware.gatekeeper</name>
                <transport>hwbinder</transport>
                <version>1.0</version>
                <interface>
                    <name>IGatekeeper</name>
                    <instance>default</instance>
                </interface>
                <fqname>@1.0::IGatekeeper/default</fqname>
            </hal>
            <hal format="hidl">
                <name>android.hardware.boot</name>
                <transport>hwbinder</transport>
                <fqname>@1.2::IBootControl/default</fqname>
            </hal>
        </manifest>"#;
        let all = read(text).unwrap();
        let names: Vec<String> = all.iter().map(Described::name).collect();
        assert_eq!(names, ["android.hardware.gatekeeper@1.0::IGatekeeper/default", "android.hardware.boot@1.2::IBootControl/default"], "one given both ways is told once");
        assert!(all.iter().all(|d| d.kind == Kind::Hidl));
    }

    #[test]
    fn a_description_written_for_names_reads_back_as_those_services_each_once() {
        let names: Vec<String> = [
            "android.hardware.security.keymint.IKeyMintDevice/default",
            "android.hardware.security.keymint.IRemotelyProvisionedComponent/default",
            "android.hardware.secure_element.ISecureElement/eSE1",
            "android.hardware.security.keymint.IKeyMintDevice/default",
        ]
        .map(str::to_string)
        .to_vec();
        let text = offering(&names).unwrap();
        assert!(text.starts_with("<manifest version=\"1.0\" type=\"framework\">\n"), "as what the running system offers");
        let read: Vec<Described> = read(&text).unwrap();
        assert_eq!(read.iter().map(Described::name).collect::<Vec<_>>(), names[..3]);
        assert!(read.iter().all(|d| d.kind == Kind::Aidl && d.version.is_none()));
        assert_eq!(offering(&[]).unwrap(), "<manifest version=\"1.0\" type=\"framework\">\n</manifest>\n");
        for name in ["android.hardware.gatekeeper@1.0::IGatekeeper/default", "IWeaver/default", "android.hardware.weaver.IWeaver", "a.<b>.IA/default", "a.IA/"] {
            assert!(offering(&[name.to_string()]).is_err(), "{name} names no service of the newer kind");
        }
    }

    #[test]
    fn what_is_no_service_is_passed_over_and_what_is_no_description_refused() {
        let libraries = r#"<manifest version="1.0" type="device"><hal format="native"><name>netutils-wrapper</name><version>1.0</version></hal><sepolicy><version>202404</version></sepolicy></manifest>"#;
        assert!(read(libraries).unwrap().is_empty());
        assert!(read("<compatibility-matrix version=\"1.0\" type=\"framework\"/>").is_err(), "another kind of file");
        assert!(read("not XML").is_err());
        let unnamed = r#"<manifest version="1.0" type="device"><hal format="aidl"><fqname>IWeaver/default</fqname></hal></manifest>"#;
        assert!(read(unnamed).is_err(), "a service in no package");
        let mixed = r#"<manifest version="1.0" type="device"><hal format="aidl"><name>a.b</name><fqname>@1.0::IA/default</fqname></hal></manifest>"#;
        assert!(read(mixed).is_err(), "a name of the older kind in a service of the newer");
    }
}
