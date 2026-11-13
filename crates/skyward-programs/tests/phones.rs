//! What a recovery does to use each phone's keys, found in what the
//! phone's own partitions say, read from the files `skyward-pack described`
//! took out of the phone's stock build into tests/described. For the two
//! phones the recovery has used the keys of, what is found is what was
//! started on the phone then.

use skyward_config::Service;
use skyward_programs::naming::{self, Does};
use skyward_programs::phone::{Happens, Partition, Phone, PARTITIONS};
use skyward_programs::script::{Socket, SocketKind};
use skyward_programs::secure::{self, Started, Step};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn described(codename: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/described").join(codename)
}

/// Where a recovery has each partition mounted: the phone's own where the
/// phone has them, the installed system's under /mnt.
fn mounted(partition: &str) -> String {
    match partition {
        "vendor" | "odm" => format!("/{partition}"),
        other => format!("/mnt/{other}"),
    }
}

/// The phone `codename` names, as its stock build describes it, with the
/// properties its bootloader and its build give.
fn phone(codename: &str, properties: &[(&str, &str)]) -> Phone {
    let at = described(codename);
    let partitions = PARTITIONS.iter().map(|name| Partition { name: name.to_string(), mounted: mounted(name).into(), at: at.join(name) }).collect();
    let properties: HashMap<String, String> = properties.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
    Phone::read(partitions, properties).unwrap()
}

/// The steps, each as the name of the program's file, or as the secure
/// element service where the recovery serves it.
fn names(steps: &[Step]) -> Vec<&str> {
    steps
        .iter()
        .map(|step| match step {
            Step::Start(started) => started.service.path.rsplit('/').next().unwrap(),
            Step::ServeElements => "(the secure element service)",
        })
        .collect()
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// The folders a program finds its libraries in, by the partition it is
/// on.
const ON_VENDOR: [&str; 2] = ["/system/lib64", "/vendor/lib64"];
const ON_ODM: [&str; 5] = ["/system/lib64", "/vendor/lib64", "/odm/lib64", "/mnt/system/system/lib64", "/mnt/system/system/lib64/bootstrap"];
const ON_SYSTEM: [&str; 3] = ["/mnt/system/system/lib64/bootstrap", "/mnt/system/system/lib64", "/mnt/system_ext/lib64"];

/// A program kept running as `user` in `groups`, with nothing else said
/// of it.
fn program(path: &str, libraries: &[&str], user: &str, groups: &[&str]) -> Service {
    Service {
        path: path.into(),
        args: Vec::new(),
        libraries: strings(libraries),
        oneshot: false,
        ready_property: None,
        ready_service: None,
        registers: Vec::new(),
        ready_hardware_service: None,
        when: None,
        user: Some(user.into()),
        groups: strings(groups),
        capabilities: Vec::new(),
    }
}

fn socket(name: &str, group: &str) -> Socket {
    Socket { name: name.into(), kind: SocketKind::Stream, passcred: false, listen: false, mode: 0o660, user: Some("system".into()), group: Some(group.into()) }
}

const KEYS: &str = "android.hardware.security.keymint.IKeyMintDevice/default";
const PROVISIONING: &str = "android.hardware.security.keymint.IRemotelyProvisionedComponent/default";
const CLOCK: &str = "android.hardware.security.secureclock.ISecureClock/default";
const SHARED_SECRET: &str = "android.hardware.security.sharedsecret.ISharedSecret/default";
const CREDENTIALS: &str = "android.hardware.gatekeeper.IGatekeeper/default";
const WEAVER: &str = "android.hardware.weaver.IWeaver/default";
const BOOT: &str = "android.hardware.boot.IBootControl/default";

/// The listeners of the secure world, which every phone here starts the
/// same.
fn listeners() -> Service {
    Service { ready_property: Some("vendor.sys.listeners.registered".into()), ..program("/vendor/bin/qseecomd", &ON_VENDOR, "root", &["root", "drmrpc"]) }
}

/// The builds of the hardware a partition's main descriptions are named
/// after, which a phone is one of: `manifest_<build>.xml` in the
/// partition's description folder.
fn builds(codename: &str, partition: &str) -> Vec<String> {
    let folder = described(codename).join(partition).join("etc/vintf");
    let mut builds: Vec<String> = std::fs::read_dir(&folder)
        .unwrap_or_else(|e| panic!("list {}: {e}", folder.display()))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|name| Some(name.strip_prefix("manifest_")?.strip_suffix(".xml")?.to_string()))
        .collect();
    builds.sort();
    builds
}

/// Every phone's stock build names one program for each service the
/// recovery asks, whichever build of the hardware the phone is: the odm
/// partition's description is the one named after the phone where it has
/// one, and the vendor partition's any of those it carries. A phone with a
/// weaver starts what the weaver goes through before it. No service is
/// told of as registered by two programs, all the phone describes reads
/// back from the description written of it for the service directory, what
/// the programs register among it, and every account a program runs as or
/// is handed a socket of is one the phone's own lists of accounts give a
/// number.
#[test]
fn every_phone_names_one_program_for_each_service() {
    let mut codenames: Vec<String> = std::fs::read_dir(described("")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    codenames.sort();
    assert!(codenames.len() >= 21, "the phones described are {codenames:?}");
    for codename in &codenames {
        let mut vendor = builds(codename, "vendor");
        if vendor.is_empty() {
            vendor.push(String::new());
        }
        let hardware = builds(codename, "odm").into_iter().find(|build| build == codename).unwrap_or_default();
        let mut found: Vec<(String, Vec<Step>)> = Vec::new();
        for build in &vendor {
            let properties = [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", build.as_str()), ("ro.boot.product.hardware.sku", hardware.as_str())];
            let read = phone(codename, &properties);
            let steps = secure::steps(&read).unwrap_or_else(|e| panic!("{codename}, as the build {build:?} of its hardware: {e:#}"));
            let told_of = read.told_of();
            let written = skyward_programs::description::offering(&told_of).unwrap_or_else(|e| panic!("{codename}, as the build {build:?} of its hardware: {e:#}"));
            let read_back: Vec<String> = skyward_programs::description::read(&written).unwrap().iter().map(|described| described.name()).collect();
            assert_eq!(read_back, told_of, "{codename}, as the build {build:?} of its hardware: what the directory is told of is all the phone describes");
            for started in secure::programs(&steps) {
                for name in started.service.ready_service.iter().chain(&started.service.registers) {
                    assert!(told_of.contains(name), "{codename}, as the build {build:?} of its hardware, describes {name}");
                }
            }
            found.push((build.clone(), steps));
        }
        let (_, first) = &found[0];
        for (build, steps) in &found {
            assert_eq!(steps, first, "{codename} does the same as the build {build:?} of its hardware as any other");
        }
        let steps = names(first);
        if let Some(weaver) = steps.iter().position(|name| name.contains("weaver-service")) {
            assert!(weaver > 0 && (steps[weaver - 1].contains("secure_element") || steps[weaver - 1] == "(the secure element service)"), "{codename}: {steps:?}");
        }
        let lists: Vec<String> = ["vendor/etc/passwd", "odm/etc/passwd", "vendor/etc/group", "odm/etc/group"].iter().map(|list| described(codename).join(list).to_string_lossy().into_owned()).collect();
        let known = skyward_sys::ueventd::accounts(&lists.iter().map(String::as_str).collect::<Vec<_>>());
        let mut registered: Vec<&String> = Vec::new();
        for started in secure::programs(first) {
            let service = &started.service;
            let handed = started.sockets.iter().flat_map(|socket| socket.user.iter().chain(&socket.group));
            for account in service.user.iter().chain(&service.groups).chain(handed) {
                assert!(skyward_sys::ueventd::account(&known, account).is_some(), "{codename}: {} runs with the account {account}, which the phone gives no number", service.path);
            }
            assert!(Path::new(&service.path).starts_with("/vendor") || Path::new(&service.path).starts_with("/odm") || Path::new(&service.path).starts_with("/mnt/system_ext"), "{codename}: {}", service.path);
            assert!(service.registers.len() < 8, "{codename}: {} registers what one program does, {:?}", service.path, service.registers);
            for name in service.ready_service.iter().chain(&service.registers) {
                assert!(!registered.contains(&name), "{codename}: {name} is registered once");
                registered.push(name);
            }
            let said: Vec<String> = service.ready_property.iter().chain(&service.ready_service).chain(&service.ready_hardware_service).chain(&service.registers).cloned().collect();
            println!("{codename}: {} as {} in {:?}: {}", service.path, service.user.as_deref().unwrap_or("root"), service.groups, said.join(", "));
        }
        println!("{codename}: {}", steps.join(", "));
    }
}

/// The 17 Ultra in the build whose weaver is Goodix's, which goes through
/// Goodix's own service: the seven programs the recovery was given by hand
/// for it and used its keys with.
#[test]
fn nezha_s_programs_are_found_as_they_were_started_on_the_phone() {
    let steps = secure::steps(&phone("nezha", &[("ro.boot.hardware", "qcom"), ("ro.special_edition", "true")])).unwrap();
    let expected = [
        Started { service: listeners(), sockets: vec![socket("notify-topology", "drmrpc")], environment: Vec::new(), anew: false },
        Started {
            service: Service { capabilities: strings(&["WAKE_ALARM", "BLOCK_SUSPEND"]), ..program("/vendor/bin/hlosminkdaemon", &ON_VENDOR, "system", &["system", "oem_2912", "wakelock"]) },
            sockets: vec![socket("ssgtzd", "oem_2912"), socket("hlos_mink_opener", "system")],
            environment: Vec::new(),
            anew: false,
        },
        Started::from(Service {
            ready_service: Some(KEYS.into()),
            registers: strings(&[PROVISIONING, CLOCK, SHARED_SECRET]),
            ..program("/vendor/bin/hw/android.hardware.security.onekeymint-service-qti", &ON_VENDOR, "system", &["system", "drmrpc"])
        }),
        Started::from(Service { ready_service: Some(CREDENTIALS.into()), ..program("/vendor/bin/hw/android.hardware.gatekeeper-rust-service-qti", &ON_VENDOR, "system", &["system"]) }),
        Started::from(Service {
            ready_service: Some("android.hardware.secure_element.ISecureElement/eSE2".into()),
            registers: strings(&["vendor.goodix.hardware.secure_element_aidl.IGdxEse/default"]),
            ..program("/odm/bin/hw/android.hardware.secure_element-service-goodix", &ON_ODM, "system", &["system"])
        }),
        Started::from(Service {
            ready_service: Some(WEAVER.into()),
            capabilities: strings(&["WAKE_ALARM"]),
            ..program("/odm/bin/hw/android.hardware.weaver-service", &ON_ODM, "system", &["system", "drmrpc"])
        }),
        Started::from(Service { ready_service: Some(BOOT.into()), ..program("/vendor/bin/hw/android.hardware.boot-service.qti", &ON_VENDOR, "root", &["root"]) }),
    ];
    let found: Vec<Started> = secure::programs(&steps).into_iter().cloned().collect();
    assert_eq!(found, expected);
    assert_eq!(steps.len(), found.len(), "the recovery serves nothing itself on this build");
}

#[test]
fn nezha_s_other_builds_reach_their_weaver_s_secure_element_through_the_secure_element_service() {
    for properties in [&[("ro.boot.hardware", "qcom")][..], &[("ro.boot.hardware", "qcom"), ("ro.special_edition", "false")][..]] {
        let steps = secure::steps(&phone("nezha", properties)).unwrap();
        assert_eq!(
            names(&steps),
            [
                "qseecomd",
                "hlosminkdaemon",
                "android.hardware.security.onekeymint-service-qti",
                "android.hardware.gatekeeper-rust-service-qti",
                "android.hardware.secure_element-service.qti",
                "(the secure element service)",
                "android.hardware.weaver-service.thales",
                "android.hardware.boot-service.qti",
            ]
        );
        let found = secure::programs(&steps);
        let [.., element, weaver, _] = &found[..] else { panic!("seven programs") };
        // The phone tells of the secure element built into it in the
        // description of its NFC chip's service, which its init starts
        // only when the NFC service itself is asked for: the secure
        // element's own service is the one that is always there, and the
        // NFC service stays that program's to register.
        let own = Service {
            ready_service: Some("android.hardware.secure_element.ISecureElement/eSE1".into()),
            ..program("/vendor/bin/hw/android.hardware.secure_element-service.qti", &ON_VENDOR, "system", &["nfc", "system"])
        };
        assert_eq!(**element, Started::from(own));
        let thales = Service { ready_service: Some(WEAVER.into()), ..program("/odm/bin/hw/android.hardware.weaver-service.thales", &ON_ODM, "vendor_thales_weaver", &["vendor_thales_weaver", "system", "drmrpc"]) };
        assert_eq!(**weaver, Started { anew: true, ..Started::from(thales) }, "it holds on to the service the recovery serves");
        assert!(found.iter().filter(|started| started.anew).count() == 1);
    }
}

/// The 13 Ultra, a phone without a weaver whose credential service and
/// boot control are of the older kind: the programs the recovery was given
/// by hand for it and used its keys with, and two things its own init does
/// that it was not given then. Its init starts the opener of the secure
/// world's apps as the phone starts, and its key service serves the shared
/// secret, the secure clock and the provisioning as well.
#[test]
fn ishtar_s_programs_are_found_as_they_were_started_on_the_phone_and_its_opener_with_them() {
    let properties = [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", "kalama"), ("ro.boot.product.hardware.sku", "ishtar")];
    let steps = secure::steps(&phone("ishtar", &properties)).unwrap();
    let expected = [
        Started { service: listeners(), sockets: vec![socket("notify-topology", "drmrpc")], environment: Vec::new(), anew: false },
        Started {
            service: program("/vendor/bin/ssgtzd", &ON_VENDOR, "system", &["system", "oem_2912"]),
            sockets: vec![socket("ssgtzd", "oem_2912"), socket("ssgtzd_opener", "system")],
            environment: Vec::new(),
            anew: false,
        },
        Started::from(Service {
            ready_service: Some(KEYS.into()),
            registers: strings(&[SHARED_SECRET, CLOCK, PROVISIONING]),
            ..program("/vendor/bin/hw/android.hardware.security.keymint-service-qti", &ON_VENDOR, "system", &["system", "drmrpc"])
        }),
        Started::from(Service { ready_property: Some("hwservicemanager.ready".into()), ..program("/mnt/system_ext/bin/hwservicemanager", &ON_SYSTEM, "system", &["system", "readproc"]) }),
        Started::from(Service {
            ready_hardware_service: Some("android.hardware.gatekeeper@1.0::IGatekeeper/default".into()),
            ..program("/vendor/bin/hw/android.hardware.gatekeeper@1.0-service-qti", &ON_VENDOR, "system", &["system"])
        }),
        Started::from(Service { ready_hardware_service: Some("android.hardware.boot@1.2::IBootControl/default".into()), ..program("/vendor/bin/hw/android.hardware.boot@1.2-service", &ON_VENDOR, "root", &["root"]) }),
    ];
    let found: Vec<Started> = secure::programs(&steps).into_iter().cloned().collect();
    assert_eq!(found, expected);
    assert_eq!(steps.len(), found.len(), "a phone without a weaver");
}

/// A phone whose weaver's secure element is behind a service of its own,
/// and whose credential service is told of in the description its whole
/// vendor partition shares, which tells of nothing else it registers.
#[test]
fn dada_s_weaver_goes_through_the_secure_element_service_to_the_element_s_own_service() {
    let properties = [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", "sun")];
    let steps = secure::steps(&phone("dada", &properties)).unwrap();
    assert_eq!(
        names(&steps),
        [
            "qseecomd",
            "ssgtzd",
            "hlosminkdaemon",
            "android.hardware.security.keymint-service-qti",
            "android.hardware.gatekeeper-service-qti",
            "android.hardware.secure_element-service.qti",
            "(the secure element service)",
            "android.hardware.weaver-service.thales",
            "android.hardware.boot-service.qti",
        ]
    );
    let found = secure::programs(&steps);
    let at = |path: &str| found.iter().find(|s| s.service.path == path).unwrap_or_else(|| panic!("{path} is started"));
    let opener = at("/vendor/bin/hlosminkdaemon");
    assert_eq!((opener.service.user.as_deref(), &opener.service.groups[..]), (Some("system"), &strings(&["system"])[..]), "as its script on the phone has it");
    let credentials = Service { ready_service: Some(CREDENTIALS.into()), ..program("/vendor/bin/hw/android.hardware.gatekeeper-service-qti", &ON_VENDOR, "system", &["system"]) };
    assert_eq!(*at("/vendor/bin/hw/android.hardware.gatekeeper-service-qti"), &Started::from(credentials));
    let element = Service {
        ready_service: Some("android.hardware.secure_element.ISecureElement/eSE1".into()),
        ..program("/vendor/bin/hw/android.hardware.secure_element-service.qti", &ON_VENDOR, "system", &["nfc", "system"])
    };
    assert_eq!(*at("/vendor/bin/hw/android.hardware.secure_element-service.qti"), &Started::from(element));
    assert!(at("/odm/bin/hw/android.hardware.weaver-service.thales").anew);
}

/// A phone that tells of its secure element in the description its whole
/// vendor partition shares: the element's service registers that alone.
#[test]
fn byron_s_secure_element_is_told_of_among_all_its_vendor_partition_registers() {
    for build in builds("byron", "vendor") {
        let properties = [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", build.as_str())];
        let steps = secure::steps(&phone("byron", &properties)).unwrap();
        let found = secure::programs(&steps);
        let element = found.iter().find(|s| s.service.path.ends_with("secure_element-service.qti")).expect("its secure element has a service of its own");
        assert_eq!(element.service.ready_service.as_deref(), Some("android.hardware.secure_element.ISecureElement/eSE1"));
        assert!(element.service.registers.is_empty(), "{:?}", element.service.registers);
    }
}

/// A script may name a program its partition does not carry, which init
/// leaves unstarted: the phone runs only those that are there, where the
/// script names them or behind the link one partition keeps to another.
#[test]
fn a_program_its_partition_does_not_carry_is_not_one_the_phone_runs() {
    let tree = tempfile::tempdir().unwrap();
    let (vendor, odm) = (tree.path().join("vendor"), tree.path().join("odm"));
    for folder in [vendor.join("etc/init"), vendor.join("bin"), odm.join("bin")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let script = "service there /vendor/bin/there\n    class hal\n\
                  service gone /vendor/bin/gone\n    class hal\n\
                  service linked /vendor/odm/bin/linked\n    class hal\n\
                  service elsewhere /system/bin/elsewhere\n    class hal\n";
    std::fs::write(vendor.join("etc/init/programs.rc"), script).unwrap();
    std::fs::write(vendor.join("bin/there"), b"").unwrap();
    std::fs::write(odm.join("bin/linked"), b"").unwrap();
    let partitions = vec![Partition { name: "vendor".into(), mounted: "/vendor".into(), at: vendor }, Partition { name: "odm".into(), mounted: "/odm".into(), at: odm }];
    let phone = Phone::read(partitions, HashMap::new()).unwrap();
    let runs = |name: &str| phone.runs(phone.programs.iter().find(|p| p.name == name).unwrap());
    assert_eq!(runs("there"), Happens::Yes);
    assert_eq!(runs("gone"), Happens::No);
    assert_eq!(runs("linked"), Happens::Yes, "on the odm partition, which the vendor partition links to");
    assert_eq!(runs("elsewhere"), Happens::No, "on a partition that is not read");
    let linked = phone.programs.iter().find(|p| p.name == "linked").unwrap();
    assert_eq!(phone.started_from(&linked.path), "/odm/bin/linked");
}

/// What init makes of the bootloader's properties names the script of the
/// phone's hardware: without the bootloader's word for the hardware no
/// such script is read, and the phone's programs with it.
#[test]
fn the_hardware_s_script_is_named_by_what_the_bootloader_calls_the_hardware() {
    let named = phone("nezha", &[("ro.boot.hardware", "qcom")]);
    assert_eq!(named.properties.get("ro.hardware").map(String::as_str), Some("qcom"));
    assert_eq!(named.properties.get("ro.bootmode").map(String::as_str), Some("unknown"));
    assert_eq!(named.properties.get("ro.serialno"), None);
    let unnamed = phone("nezha", &[("ro.hardware", "qcom")]);
    assert_eq!(unnamed.properties.get("ro.hardware").map(String::as_str), Some("unknown"), "init sets it from the bootloader's alone");
    assert!(unnamed.actions.len() < named.actions.len(), "the hardware's script and those it has init read are left unread");
}

/// No bootloader says which build of the hardware a phone's vendor
/// partition describes: on every phone here the phone's init names it as
/// the phone starts, after the chip a program of the vendor partition's
/// makes out, and the vendor partition keeps a main description for each
/// chip it serves and none for a phone left unnamed. A phone so named has
/// nothing more to do.
#[test]
fn every_phone_s_init_names_the_build_its_vendor_partition_describes_after_its_chip() {
    let codenames: Vec<String> = std::fs::read_dir(described("")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(codenames.len() >= 21, "the phones described are {codenames:?}");
    let makes_the_chip_out = Service { oneshot: true, user: None, groups: Vec::new(), ..program("/vendor/bin/init.qti.qcv.sh", &ON_VENDOR, "root", &[]) };
    let named_as_init_names_it = [
        Does::Run(Box::new(Started::from(makes_the_chip_out))),
        Does::Set { name: "ro.boot.product.vendor.sku".into(), value: "${ro.vendor.qti.soc_name}".into() },
    ];
    for codename in &codenames {
        let hardware = [("ro.boot.hardware", "qcom"), ("ro.boot.product.hardware.sku", codename.as_str())];
        assert_eq!(naming::steps(&phone(codename, &hardware)).unwrap(), named_as_init_names_it, "{codename}");
        let chips = builds(codename, "vendor");
        assert!(!chips.is_empty(), "{codename} keeps a main description for each chip its vendor partition serves");
        assert!(!described(codename).join("vendor/etc/vintf/manifest.xml").exists(), "{codename} keeps none for a phone left unnamed");
        for chip in &chips {
            let named = [("ro.boot.hardware", "qcom"), ("ro.boot.product.hardware.sku", codename.as_str()), ("ro.boot.product.vendor.sku", chip.as_str())];
            assert_eq!(naming::steps(&phone(codename, &named)).unwrap(), [], "{codename}, named {chip}");
        }
    }
}
