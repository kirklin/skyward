use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;

/// A folder of its own for a test, empty, under `TMPDIR`.
fn scratch(test: &str) -> PathBuf {
    let at = std::env::temp_dir().join(format!("skyward-mtp-responder-{test}-{}", std::process::id()));
    if at.exists() {
        fs::remove_dir_all(&at).unwrap();
    }
    fs::create_dir_all(&at).unwrap();
    at
}

fn device() -> Device {
    Device {
        manufacturer: "Maker".into(),
        model: "Model 1".into(),
        version: "1.0".into(),
        serial: "0123456789".into(),
        name: "Phone".into(),
        battery: Box::new(|| Some(76)),
    }
}

fn storage(top: &Path) -> Storage {
    Storage { top: top.to_path_buf(), description: "内部共享存储空间".into() }
}

fn cmd(code: u16, params: &[u32]) -> Command {
    Command { code, transaction: 1, params: params.to_vec() }
}

/// A responder offering `top`, with a session open.
fn open(top: &Path) -> Responder {
    let mut r = Responder::new(device(), TimeZone::UTC, Arc::default());
    assert!(r.offer(Some(storage(top))).unwrap().is_empty(), "no session to tell");
    assert_eq!(r.answer(&cmd(op::OPEN_SESSION, &[1]), None).code, response::OK);
    r
}

fn dataset(reply: Reply) -> Vec<u8> {
    assert_eq!(reply.code, response::OK, "{:?}", reply.why);
    match reply.data {
        Some(Outgoing::Dataset(bytes)) => bytes,
        other => panic!("not a dataset: {other:?}"),
    }
}

fn handles(r: &mut Responder, parent: u32, format: u32) -> Vec<u32> {
    let bytes = dataset(r.answer(&cmd(op::GET_OBJECT_HANDLES, &[0xFFFF_FFFF, format, parent]), None));
    let mut rd = Reader::new(&bytes);
    let n = rd.u32().unwrap();
    (0..n).map(|_| rd.u32().unwrap()).collect()
}

/// What GetObjectInfo says: format, size, parent, association type, name
/// and time last changed.
#[derive(Debug, PartialEq)]
struct Info {
    format: u16,
    size: u32,
    parent: u32,
    association: u16,
    name: String,
    modified: String,
}

fn info(r: &mut Responder, h: u32) -> Info {
    let bytes = dataset(r.answer(&cmd(op::GET_OBJECT_INFO, &[h]), None));
    let mut rd = Reader::new(&bytes);
    assert_eq!(rd.u32(), Some(STORAGE_ID));
    let format = rd.u16().unwrap();
    assert_eq!(rd.u16(), Some(0), "not protected");
    let size = rd.u32().unwrap();
    rd.u16().unwrap();
    for _ in 0..6 {
        rd.u32().unwrap();
    }
    let parent = rd.u32().unwrap();
    let association = rd.u16().unwrap();
    rd.u32().unwrap();
    rd.u32().unwrap();
    let name = rd.string().unwrap();
    let created = rd.string().unwrap();
    let modified = rd.string().unwrap();
    assert_eq!(created, modified);
    assert_eq!(rd.string().as_deref(), Some(""), "no keywords");
    assert!(rd.rest().is_empty());
    Info { format, size, parent, association, name, modified }
}

/// The handle of the entry named `name` in `parent`.
fn find(r: &mut Responder, parent: u32, name: &str) -> u32 {
    let all = handles(r, parent, 0);
    all.into_iter().find(|&h| info(r, h).name == name).unwrap_or_else(|| panic!("{name} is not listed"))
}

/// An object info as the computer sends it with SendObjectInfo.
fn object_info(format: u16, size: u32, name: &str, modified: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(0).u16(format).u16(0).u32(size).u16(0);
    for _ in 0..7 {
        w.u32(0);
    }
    w.u16(0).u32(0).u32(0).string(name).string("").string(modified).string("");
    w.into_bytes()
}

/// Send a file the way the computer does: its info, then its contents
/// into the file the phone made. The handle and the response.
fn send(r: &mut Responder, parent: u32, name: &str, contents: &[u8], announced: u32) -> (u32, Reply) {
    let info_cmd = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, parent]);
    assert!(matches!(r.incoming(&info_cmd), Incoming::Dataset));
    let made = r.answer(&info_cmd, Some(&object_info(format::TEXT, announced, name, "20261021T121500")));
    assert_eq!(made.code, response::OK, "{:?}", made.why);
    let handle = made.params[2];
    let send_cmd = cmd(op::SEND_OBJECT, &[]);
    let Incoming::File { mut file, offset, unbounded } = r.incoming(&send_cmd) else { panic!("the contents go into a file") };
    assert_eq!((offset, unbounded), (0, announced == u32::MAX));
    file.write_all(contents).unwrap();
    drop(file);
    (handle, r.received(&send_cmd, Received::Done(contents.len() as u64)).expect("an answer"))
}

#[test]
fn the_device_says_what_it_is_and_does_without_a_session() {
    let mut r = Responder::new(device(), TimeZone::UTC, Arc::default());
    let bytes = dataset(r.answer(&cmd(op::GET_DEVICE_INFO, &[]), None));
    let mut rd = Reader::new(&bytes);
    assert_eq!((rd.u16(), rd.u32(), rd.u16()), (Some(100), Some(6), Some(100)));
    assert_eq!(rd.string().as_deref(), Some("microsoft.com: 1.0; android.com: 1.0;"));
    assert_eq!(rd.u16(), Some(0));
    let mut array = || -> Vec<u16> { (0..rd.u32().unwrap()).map(|_| rd.u16().unwrap()).collect() };
    let (operations, events, properties, captured, accepted) = (array(), array(), array(), array(), array());
    assert!(operations.contains(&op::GET_PARTIAL_OBJECT_64) && operations.contains(&op::GET_OBJECT_PROP_LIST));
    assert!(!operations.contains(&op::SET_OBJECT_REFERENCES), "not offered");
    assert!(events.contains(&event::OBJECT_ADDED));
    assert!(properties.contains(&device::DEVICE_FRIENDLY_NAME));
    assert!(captured.is_empty(), "the phone takes no pictures over MTP");
    assert!(accepted.contains(&format::ASSOCIATION) && accepted.contains(&format::UNDEFINED));
    let strings: Vec<String> = (0..4).map(|_| rd.string().unwrap()).collect();
    assert_eq!(strings, ["Maker", "Model 1", "1.0", "0123456789"]);
    assert!(rd.rest().is_empty());
}

#[test]
fn a_session_opens_once_and_the_storage_needs_one() {
    let at = scratch("session");
    let mut r = Responder::new(device(), TimeZone::UTC, Arc::default());
    r.offer(Some(storage(&at))).unwrap();
    assert_eq!(r.answer(&cmd(op::GET_STORAGE_IDS, &[]), None).code, response::SESSION_NOT_OPEN);
    assert_eq!(r.answer(&cmd(op::OPEN_SESSION, &[]), None).code, response::INVALID_PARAMETER);
    assert_eq!(r.answer(&cmd(op::OPEN_SESSION, &[7]), None).code, response::OK);
    let again = r.answer(&cmd(op::OPEN_SESSION, &[8]), None);
    assert_eq!((again.code, again.params), (response::SESSION_ALREADY_OPEN, vec![7]));
    let ids = dataset(r.answer(&cmd(op::GET_STORAGE_IDS, &[]), None));
    assert_eq!(ids, [1, 0, 0, 0, 1, 0, 1, 0]);
    let bytes = dataset(r.answer(&cmd(op::GET_STORAGE_INFO, &[STORAGE_ID]), None));
    let mut rd = Reader::new(&bytes);
    assert_eq!((rd.u16(), rd.u16(), rd.u16()), (Some(storage::FIXED_RAM), Some(storage::HIERARCHICAL), Some(storage::READ_WRITE)));
    let (total, free) = (rd.u64().unwrap(), rd.u64().unwrap());
    assert!(total > 0 && free <= total);
    rd.u32().unwrap();
    assert_eq!(rd.string().as_deref(), Some("内部共享存储空间"));
    assert_eq!(rd.string().as_deref(), Some(""));
    assert_eq!(r.answer(&cmd(op::GET_STORAGE_INFO, &[0x0002_0001]), None).code, response::INVALID_STORAGE_ID);
    assert_eq!(r.answer(&cmd(op::CLOSE_SESSION, &[]), None).code, response::OK);
    assert_eq!(r.answer(&cmd(op::CLOSE_SESSION, &[]), None).code, response::SESSION_NOT_OPEN);
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn read_only_io_keeps_its_reason_through_wrappers_and_transfer_responses() {
    let direct = io::Error::from_raw_os_error(libc::EROFS);
    assert_eq!(access_failed(&direct), response::STORE_READ_ONLY);
    let wrapped = anyhow::Error::from(direct).context("write an MTP object");
    assert_eq!(write_failed(&wrapped, response::PARTIAL_DELETION), response::STORE_READ_ONLY);
    assert_eq!(access_failed(&io::Error::from_raw_os_error(libc::EACCES)), response::ACCESS_DENIED);
    assert_eq!(write_failed(&anyhow::anyhow!("not an I/O failure"), response::PARTIAL_DELETION), response::PARTIAL_DELETION);

    let at = scratch("read-only-response");
    let mut r = open(&at);
    let info_cmd = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, PARENT_ROOT]);
    assert_eq!(r.answer(&info_cmd, Some(&object_info(format::TEXT, 1, "partial.txt", ""))).code, response::OK);
    let reply = r.received(&cmd(op::SEND_OBJECT, &[]), Received::Failed(io::Error::from_raw_os_error(libc::EROFS))).unwrap();
    assert_eq!(reply.code, response::STORE_READ_ONLY);
    assert!(!at.join("partial.txt").exists(), "the incomplete upload is cleaned up");
    let partial = cmd(op::SEND_PARTIAL_OBJECT, &[1, 0, 0, 1]);
    let reply = r.received(&partial, Received::Failed(io::Error::from_raw_os_error(libc::EROFS))).unwrap();
    assert_eq!((reply.code, reply.params), (response::STORE_READ_ONLY, vec![0]));
    fs::remove_dir_all(&at).unwrap();
}

/// Run explicitly inside an isolated real read-only mount, seeded with
/// plain.txt, 中文文件.txt and an empty destination/ directory. Neither
/// chmod nor a fake stat result is accepted as a read-only filesystem.
#[test]
#[ignore = "requires SKYWARD_MTP_RO_ROOT on an isolated read-only mount"]
fn a_read_only_mount_is_reported_and_writes_are_refused_without_changing_files() {
    let top = PathBuf::from(std::env::var_os("SKYWARD_MTP_RO_ROOT").expect("isolated fixture mount path"));
    assert!(space(&top).unwrap().2, "the fixture must really be mounted read-only");
    assert_eq!(fs::write(top.join("must-not-be-created"), b"x").unwrap_err().raw_os_error(), Some(libc::EROFS));
    let plain = b"immutable MTP fixture\n";
    let chinese = "只读传输测试\n".as_bytes();
    assert_eq!(fs::read(top.join("plain.txt")).unwrap(), plain);
    assert_eq!(fs::read(top.join("中文文件.txt")).unwrap(), chinese);
    let mut r = open(&top);
    let bytes = dataset(r.answer(&cmd(op::GET_STORAGE_INFO, &[STORAGE_ID]), None));
    let mut rd = Reader::new(&bytes);
    assert_eq!((rd.u16(), rd.u16(), rd.u16()), (Some(storage::FIXED_RAM), Some(storage::HIERARCHICAL), Some(storage::READ_ONLY_WITHOUT_DELETE)));
    for (name, expected) in [("plain.txt", plain.as_slice()), ("中文文件.txt", chinese)] {
        let handle = find(&mut r, PARENT_ROOT, name);
        let reply = r.answer(&cmd(op::GET_OBJECT, &[handle]), None);
        assert_eq!(reply.code, response::OK);
        let Some(Outgoing::File { mut file, offset, length }) = reply.data else { panic!("download is a file") };
        assert_eq!((offset, length), (0, expected.len() as u64));
        let mut got = Vec::new();
        file.read_to_end(&mut got).unwrap();
        assert_eq!(got, expected);
    }
    for (format, size, name) in [(format::TEXT, 3, "new.txt"), (format::TEXT, u32::MAX, "large.txt"), (format::ASSOCIATION, 0, "new-folder")] {
        let reply = r.answer(&cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, PARENT_ROOT]), Some(&object_info(format, size, name, "")));
        assert_eq!(reply.code, response::STORE_READ_ONLY, "upload {name}: {:?}", reply.why);
        assert!(!top.join(name).exists());
    }
    let file = find(&mut r, PARENT_ROOT, "plain.txt");
    let folder = find(&mut r, PARENT_ROOT, "destination");
    let mut rename = Writer::new();
    rename.string("renamed.txt");
    let reply = r.answer(&cmd(op::SET_OBJECT_PROP_VALUE, &[file, u32::from(prop::OBJECT_FILE_NAME)]), Some(&rename.into_bytes()));
    assert_eq!(reply.code, response::STORE_READ_ONLY, "rename: {:?}", reply.why);
    for command in [cmd(op::DELETE_OBJECT, &[file]), cmd(op::MOVE_OBJECT, &[file, STORAGE_ID, folder]),
                    cmd(op::COPY_OBJECT, &[file, STORAGE_ID, folder]), cmd(op::BEGIN_EDIT_OBJECT, &[file])] {
        let reply = r.answer(&command, None);
        assert_eq!(reply.code, response::STORE_READ_ONLY, "{:#06x}: {:?}", command.code, reply.why);
    }
    assert_eq!(fs::read(top.join("plain.txt")).unwrap(), plain);
    assert_eq!(fs::read(top.join("中文文件.txt")).unwrap(), chinese);
    assert!(!top.join("renamed.txt").exists());
    assert_eq!(fs::read_dir(top.join("destination")).unwrap().count(), 0);
}

#[test]
fn entries_keep_their_handles_from_one_session_and_connection_to_the_next() {
    let at = scratch("sessions");
    fs::write(at.join("a.txt"), b"a").unwrap();
    let mut r = open(&at);
    let a = find(&mut r, 0xFFFF_FFFF, "a.txt");
    assert_eq!(r.answer(&cmd(op::CLOSE_SESSION, &[]), None).code, response::OK);
    r.disconnected();
    assert_eq!(r.answer(&cmd(op::OPEN_SESSION, &[2]), None).code, response::OK);
    assert_eq!(info(&mut r, a).name, "a.txt", "the handle the last session had still names the file");
    r.offer(None).unwrap();
    r.offer(Some(storage(&at))).unwrap();
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_INFO, &[a]), None).code, response::INVALID_OBJECT_HANDLE, "a storage offered again starts afresh");
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn an_open_session_is_told_as_the_storage_goes_and_comes() {
    let at = scratch("offer");
    let mut r = open(&at);
    assert_eq!(r.offer(None).unwrap(), [Event { code: event::STORE_REMOVED, param: STORAGE_ID }]);
    assert_eq!(dataset(r.answer(&cmd(op::GET_STORAGE_IDS, &[]), None)), [0, 0, 0, 0]);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_HANDLES, &[0xFFFF_FFFF, 0, 0xFFFF_FFFF]), None).code, response::INVALID_STORAGE_ID);
    assert_eq!(r.offer(Some(storage(&at))).unwrap(), [Event { code: event::STORE_ADDED, param: STORAGE_ID }]);
    assert!(handles(&mut r, 0xFFFF_FFFF, 0).is_empty());
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn folders_list_their_entries_and_objects_describe_themselves() {
    let at = scratch("list");
    fs::create_dir_all(at.join("DCIM/Camera")).unwrap();
    fs::write(at.join("DCIM/Camera/IMG_1.jpg"), b"jpeg").unwrap();
    fs::write(at.join("notes.txt"), b"twelve bytes").unwrap();
    let mut r = open(&at);
    let top = handles(&mut r, 0xFFFF_FFFF, 0);
    assert_eq!(top.len(), 2);
    let dcim = find(&mut r, 0xFFFF_FFFF, "DCIM");
    let notes = find(&mut r, 0xFFFF_FFFF, "notes.txt");
    assert_eq!(
        info(&mut r, dcim),
        Info { format: format::ASSOCIATION, size: 0, parent: 0, association: 1, name: "DCIM".into(), modified: info(&mut r, dcim).modified }
    );
    let notes_info = info(&mut r, notes);
    assert_eq!((notes_info.format, notes_info.size, notes_info.parent, notes_info.association), (format::TEXT, 12, 0, 0));
    assert_eq!(notes_info.modified.len(), 15);
    let camera = find(&mut r, dcim, "Camera");
    let photo = find(&mut r, camera, "IMG_1.jpg");
    assert_eq!(info(&mut r, photo).parent, camera);
    assert_eq!(handles(&mut r, 0, 0).len(), 4, "a parent of 0 is every object");
    assert_eq!(handles(&mut r, 0, u32::from(format::EXIF_JPEG)), [photo]);
    let count = r.answer(&cmd(op::GET_NUM_OBJECTS, &[STORAGE_ID, 0, 0xFFFF_FFFF]), None);
    assert_eq!((count.code, count.params), (response::OK, vec![2]));
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_HANDLES, &[STORAGE_ID, 0, notes]), None).code, response::INVALID_PARENT_OBJECT);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_HANDLES, &[STORAGE_ID, 0, 999]), None).code, response::INVALID_OBJECT_HANDLE);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_INFO, &[999]), None).code, response::INVALID_OBJECT_HANDLE);
    fs::remove_dir_all(at.join("DCIM")).unwrap();
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_INFO, &[photo]), None).code, response::INVALID_OBJECT_HANDLE, "gone from the disk");
    fs::remove_dir_all(&at).unwrap();
}

/// The entries of an object property list: handle, property, type and the
/// value's bytes.
fn prop_list(bytes: &[u8]) -> Vec<(u32, u16, u16, Vec<u8>)> {
    let mut rd = Reader::new(bytes);
    let n = rd.u32().unwrap();
    let mut out = Vec::new();
    for _ in 0..n {
        let (h, code, kind) = (rd.u32().unwrap(), rd.u16().unwrap(), rd.u16().unwrap());
        let before = rd.rest();
        match kind {
            kind::UINT16 => drop(rd.u16()),
            kind::UINT32 => drop(rd.u32()),
            kind::UINT64 => drop(rd.u64()),
            kind::UINT128 => drop((rd.u64(), rd.u64())),
            kind::STR => drop(rd.string()),
            other => panic!("type {other:#x}"),
        }
        out.push((h, code, kind, before[..before.len() - rd.rest().len()].to_vec()));
    }
    assert!(rd.rest().is_empty());
    out
}

#[test]
fn property_lists_go_as_deep_as_the_stock_system_s() {
    let at = scratch("props");
    fs::create_dir_all(at.join("Music")).unwrap();
    fs::write(at.join("Music/song.mp3"), b"mp3").unwrap();
    fs::write(at.join("a.txt"), b"a").unwrap();
    let mut r = open(&at);
    let music = find(&mut r, 0xFFFF_FFFF, "Music");
    let a = find(&mut r, 0xFFFF_FFFF, "a.txt");
    let one = prop_list(&dataset(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[a, 0, 0xFFFF_FFFF, 0, 0]), None)));
    assert_eq!(one.len(), OBJECT_PROPERTIES.len());
    assert!(one.iter().all(|(h, ..)| *h == a));
    let size = one.iter().find(|(_, c, ..)| *c == prop::OBJECT_SIZE).unwrap();
    assert_eq!((size.2, size.3.as_slice()), (kind::UINT64, &1u64.to_le_bytes()[..]));
    let deep = prop_list(&dataset(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[music, 0, u32::from(prop::OBJECT_FILE_NAME), 0, 1]), None)));
    let mut names: Vec<String> = deep.iter().map(|(.., v)| Reader::new(v).string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["Music", "song.mp3"], "a folder's entries and the folder");
    let top = prop_list(&dataset(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[0, 0, u32::from(prop::PARENT_OBJECT), 0, 0]), None)));
    assert_eq!(top.len(), 2, "the top folder's entries");
    assert!(top.iter().all(|(.., v)| v.as_slice() == [0, 0, 0, 0]), "at the top, with no parent");
    let all = prop_list(&dataset(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[0xFFFF_FFFF, 0, u32::from(prop::OBJECT_FORMAT), 0, 0xFFFF_FFFF]), None)));
    assert_eq!(all.len(), 3, "every object");
    let only_mp3 = prop_list(&dataset(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[0xFFFF_FFFF, u32::from(format::MP3), u32::from(prop::OBJECT_FORMAT), 0, 0]), None)));
    assert_eq!(only_mp3.len(), 1);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[a, 0, 0, 0, 0]), None).code, response::PARAMETER_NOT_SUPPORTED);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[a, 0, 0, 1, 0]), None).code, response::SPECIFICATION_BY_GROUP_UNSUPPORTED);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[music, 0, 0xFFFF_FFFF, 0, 2]), None).code, response::SPECIFICATION_BY_DEPTH_UNSUPPORTED);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_LIST, &[a, 0, 0xDC99, 0, 0]), None).code, response::OBJECT_PROP_NOT_SUPPORTED);
    let value = dataset(r.answer(&cmd(op::GET_OBJECT_PROP_VALUE, &[a, u32::from(prop::OBJECT_FILE_NAME)]), None));
    assert_eq!(Reader::new(&value).string().as_deref(), Some("a.txt"));
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_VALUE, &[a, 0xDC99]), None).code, response::OBJECT_PROP_NOT_SUPPORTED);
    let supported = dataset(r.answer(&cmd(op::GET_OBJECT_PROPS_SUPPORTED, &[u32::from(format::UNDEFINED)]), None));
    assert_eq!(u32::from_le_bytes(supported[..4].try_into().unwrap()) as usize, OBJECT_PROPERTIES.len());
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn property_descriptions_say_what_can_be_set() {
    let mut r = Responder::new(device(), TimeZone::UTC, Arc::default());
    let name = dataset(r.answer(&cmd(op::GET_OBJECT_PROP_DESC, &[u32::from(prop::OBJECT_FILE_NAME), 0]), None));
    // Code, string, settable, empty default, no group, no form.
    assert_eq!(name, [0x07, 0xDC, 0xFF, 0xFF, 1, 0, 0, 0, 0, 0, form::NONE]);
    let modified = dataset(r.answer(&cmd(op::GET_OBJECT_PROP_DESC, &[u32::from(prop::DATE_MODIFIED), 0]), None));
    assert_eq!((modified[4], *modified.last().unwrap()), (0, form::DATE_TIME));
    let format_desc = dataset(r.answer(&cmd(op::GET_OBJECT_PROP_DESC, &[u32::from(prop::OBJECT_FORMAT), u32::from(format::MP3)]), None));
    assert_eq!(format_desc[5..7], format::MP3.to_le_bytes(), "the format asked about is the default");
    let id = dataset(r.answer(&cmd(op::GET_OBJECT_PROP_DESC, &[u32::from(prop::PERSISTENT_UID), 0]), None));
    assert_eq!(id.len(), 2 + 2 + 1 + 16 + 4 + 1);
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_PROP_DESC, &[0xDC99, 0]), None).code, response::OBJECT_PROP_NOT_SUPPORTED);
}

#[test]
fn files_are_read_whole_or_in_pieces() {
    let at = scratch("read");
    fs::write(at.join("data.bin"), b"0123456789").unwrap();
    fs::create_dir(at.join("folder")).unwrap();
    let mut r = open(&at);
    let file = find(&mut r, 0xFFFF_FFFF, "data.bin");
    let folder = find(&mut r, 0xFFFF_FFFF, "folder");
    let piece = |reply: Reply| -> (Vec<u8>, Vec<u32>) {
        assert_eq!(reply.code, response::OK, "{:?}", reply.why);
        let Some(Outgoing::File { file, offset, length }) = reply.data else { panic!("not a file") };
        let mut buf = vec![0u8; length as usize];
        file.read_exact_at(&mut buf, offset).unwrap();
        (buf, reply.params)
    };
    assert_eq!(piece(r.answer(&cmd(op::GET_OBJECT, &[file]), None)), (b"0123456789".to_vec(), vec![]));
    assert_eq!(piece(r.answer(&cmd(op::GET_PARTIAL_OBJECT, &[file, 2, 3]), None)), (b"234".to_vec(), vec![3]));
    assert_eq!(piece(r.answer(&cmd(op::GET_PARTIAL_OBJECT, &[file, 8, 100]), None)), (b"89".to_vec(), vec![2]), "up to the end");
    assert_eq!(piece(r.answer(&cmd(op::GET_PARTIAL_OBJECT, &[file, 20, 5]), None)), (vec![], vec![0]), "past the end");
    assert_eq!(piece(r.answer(&cmd(op::GET_PARTIAL_OBJECT_64, &[file, 5, 0, 2]), None)), (b"56".to_vec(), vec![2]));
    assert_eq!(piece(r.answer(&cmd(op::GET_PARTIAL_OBJECT_64, &[file, 0, 1, 2]), None)), (vec![], vec![0]), "4 GiB in");
    assert_eq!(r.answer(&cmd(op::GET_OBJECT, &[folder]), None).code, response::INVALID_OBJECT_HANDLE);
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn a_file_sent_is_made_where_asked_with_its_time() {
    let at = scratch("send");
    fs::create_dir(at.join("Download")).unwrap();
    let mut r = open(&at);
    let download = find(&mut r, 0xFFFF_FFFF, "Download");
    let (h, done) = send(&mut r, download, "报告.txt", b"hello", 5);
    assert_eq!(done.code, response::OK, "{:?}", done.why);
    assert_eq!(fs::read(at.join("Download/报告.txt")).unwrap(), b"hello");
    assert_eq!(fs::metadata(at.join("Download/报告.txt")).unwrap().mtime(), 1_792_584_900, "the time the computer gave");
    assert_eq!(handles(&mut r, download, 0), [h], "listed under the handle it was given");
    assert_eq!(info(&mut r, h).name, "报告.txt");
    let (top, done) = send(&mut r, PARENT_ROOT, "top.txt", b"", 0);
    assert_eq!(done.code, response::OK);
    assert_eq!(info(&mut r, top).parent, 0);
    assert!(at.join("top.txt").exists());
    // 4 GiB or more: however much comes is the file.
    let (_, big) = send(&mut r, PARENT_ROOT, "big.bin", b"as much as came", u32::MAX);
    assert_eq!(big.code, response::OK);
    assert_eq!(fs::read(at.join("big.bin")).unwrap(), b"as much as came");
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn a_file_sent_short_or_stopped_is_not_left_behind() {
    let at = scratch("short");
    let mut r = open(&at);
    let (h, short) = send(&mut r, PARENT_ROOT, "short.txt", b"12345", 10);
    assert_eq!(short.code, response::INCOMPLETE_TRANSFER);
    assert!(!at.join("short.txt").exists());
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_INFO, &[h]), None).code, response::INVALID_OBJECT_HANDLE);
    let info_cmd = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, PARENT_ROOT]);
    r.incoming(&info_cmd);
    assert_eq!(r.answer(&info_cmd, Some(&object_info(format::UNDEFINED, 100, "stopped.bin", ""))).code, response::OK);
    let send_cmd = cmd(op::SEND_OBJECT, &[]);
    assert!(matches!(r.incoming(&send_cmd), Incoming::File { .. }));
    assert!(r.received(&send_cmd, Received::Stopped).is_none(), "a cancelled transfer has no answer");
    assert!(!at.join("stopped.bin").exists());
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn send_object_needs_the_info_right_before_it() {
    let at = scratch("order");
    let mut r = open(&at);
    let refused = |r: &mut Responder| match r.incoming(&cmd(op::SEND_OBJECT, &[])) {
        Incoming::Refused(reply) => reply.code,
        other => panic!("{other:?}"),
    };
    assert_eq!(refused(&mut r), response::NO_VALID_OBJECT_INFO);
    let info_cmd = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, PARENT_ROOT]);
    r.incoming(&info_cmd);
    assert_eq!(r.answer(&info_cmd, Some(&object_info(format::TEXT, 1, "x.txt", ""))).code, response::OK);
    let between = cmd(op::GET_STORAGE_IDS, &[]);
    r.incoming(&between);
    r.answer(&between, None);
    assert_eq!(refused(&mut r), response::NO_VALID_OBJECT_INFO, "something else came between");
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn folders_are_made_and_names_are_checked() {
    let at = scratch("folders");
    fs::write(at.join("file.txt"), b"f").unwrap();
    let mut r = open(&at);
    let file = find(&mut r, 0xFFFF_FFFF, "file.txt");
    let make = |r: &mut Responder, parent: u32, format: u16, name: &str| {
        let c = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, parent]);
        r.incoming(&c);
        r.answer(&c, Some(&object_info(format, 0, name, "")))
    };
    let made = make(&mut r, PARENT_ROOT, format::ASSOCIATION, "新建文件夹");
    assert_eq!(made.code, response::OK);
    assert!(at.join("新建文件夹").is_dir());
    let send_cmd = cmd(op::SEND_OBJECT, &[]);
    assert!(matches!(r.incoming(&send_cmd), Incoming::Dataset), "a folder sends nothing");
    assert_eq!(r.answer(&send_cmd, Some(&[])).code, response::OK);
    assert_eq!(info(&mut r, made.params[2]).format, format::ASSOCIATION);
    for bad in ["", ".", "..", "a/b"] {
        assert_eq!(make(&mut r, PARENT_ROOT, format::TEXT, bad).code, response::INVALID_PARAMETER, "{bad:?}");
    }
    assert_eq!(make(&mut r, PARENT_ROOT, format::TEXT, "file.txt").code, response::GENERAL_ERROR, "the name is taken");
    assert_eq!(make(&mut r, file, format::TEXT, "inside.txt").code, response::INVALID_PARENT_OBJECT);
    assert_eq!(make(&mut r, 999, format::TEXT, "lost.txt").code, response::INVALID_OBJECT_HANDLE);
    let info_cmd = cmd(op::SEND_OBJECT_INFO, &[STORAGE_ID, PARENT_ROOT]);
    r.incoming(&info_cmd);
    assert_eq!(r.answer(&info_cmd, Some(&[1, 2, 3])).code, response::INVALID_PARAMETER, "cut short");
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn objects_are_renamed_moved_copied_and_deleted() {
    let at = scratch("work");
    fs::create_dir_all(at.join("into/deep")).unwrap();
    fs::write(at.join("a.txt"), b"a").unwrap();
    fs::write(at.join("b.txt"), b"b").unwrap();
    let mut r = open(&at);
    let (a, into) = (find(&mut r, 0xFFFF_FFFF, "a.txt"), find(&mut r, 0xFFFF_FFFF, "into"));
    let rename = |r: &mut Responder, h: u32, name: &str| {
        let c = cmd(op::SET_OBJECT_PROP_VALUE, &[h, u32::from(prop::OBJECT_FILE_NAME)]);
        assert!(matches!(r.incoming(&c), Incoming::Dataset));
        let mut w = Writer::new();
        w.string(name);
        r.answer(&c, Some(&w.into_bytes())).code
    };
    assert_eq!(rename(&mut r, a, "c.txt"), response::OK);
    assert!(at.join("c.txt").exists() && !at.join("a.txt").exists());
    assert_eq!(info(&mut r, a).name, "c.txt", "the same handle");
    assert_eq!(rename(&mut r, a, "b.txt"), response::GENERAL_ERROR, "the name is taken");
    assert_eq!(r.answer(&cmd(op::MOVE_OBJECT, &[a, STORAGE_ID, into]), None).code, response::OK);
    assert!(at.join("into/c.txt").exists());
    assert_eq!(info(&mut r, a).parent, into);
    assert_eq!(handles(&mut r, into, 0).len(), 2, "the folder read after the move has both");
    let copied = r.answer(&cmd(op::COPY_OBJECT, &[a, STORAGE_ID, 0]), None);
    assert_eq!(copied.code, response::OK, "{:?}", copied.why);
    let copy = copied.params[0];
    assert_ne!(copy, a);
    assert_eq!((fs::read(at.join("c.txt")).unwrap(), info(&mut r, copy).parent), (b"a".to_vec(), 0));
    assert_eq!(r.answer(&cmd(op::COPY_OBJECT, &[a, STORAGE_ID, 0]), None).code, response::GENERAL_ERROR, "the copy is there already");
    assert_eq!(r.answer(&cmd(op::MOVE_OBJECT, &[into, STORAGE_ID, into]), None).code, response::GENERAL_ERROR, "not into itself");
    assert_eq!(r.answer(&cmd(op::MOVE_OBJECT, &[a, STORAGE_ID, copy]), None).code, response::INVALID_PARENT_OBJECT);
    assert_eq!(r.answer(&cmd(op::DELETE_OBJECT, &[into]), None).code, response::OK);
    assert!(!at.join("into").exists(), "a folder goes with what is in it");
    assert_eq!(r.answer(&cmd(op::GET_OBJECT_INFO, &[a]), None).code, response::INVALID_OBJECT_HANDLE);
    assert_eq!(r.answer(&cmd(op::DELETE_OBJECT, &[into]), None).code, response::INVALID_OBJECT_HANDLE);
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn files_are_edited_in_place() {
    let at = scratch("edit");
    fs::write(at.join("doc.txt"), b"abcdef").unwrap();
    let mut r = open(&at);
    let h = find(&mut r, 0xFFFF_FFFF, "doc.txt");
    let partial = cmd(op::SEND_PARTIAL_OBJECT, &[h, 2, 0, 3]);
    assert!(matches!(r.incoming(&partial), Incoming::Refused(_)), "not open to be edited");
    assert_eq!(r.answer(&cmd(op::BEGIN_EDIT_OBJECT, &[h]), None).code, response::OK);
    assert_eq!(r.answer(&cmd(op::BEGIN_EDIT_OBJECT, &[h]), None).code, response::GENERAL_ERROR, "open already");
    let Incoming::File { file, offset, unbounded: false } = r.incoming(&partial) else { panic!("the piece goes into the file") };
    file.write_all_at(b"XYZ", offset).unwrap();
    drop(file);
    let done = r.received(&partial, Received::Done(3)).unwrap();
    assert_eq!((done.code, done.params), (response::OK, vec![3]));
    let past = cmd(op::SEND_PARTIAL_OBJECT, &[h, 9, 0, 1]);
    assert!(matches!(r.incoming(&past), Incoming::Refused(_)), "past the end");
    assert_eq!(r.answer(&cmd(op::TRUNCATE_OBJECT, &[h, 4, 0]), None).code, response::OK);
    assert_eq!(r.answer(&cmd(op::END_EDIT_OBJECT, &[h]), None).code, response::OK);
    assert_eq!(fs::read(at.join("doc.txt")).unwrap(), b"abXY");
    assert_eq!(r.answer(&cmd(op::END_EDIT_OBJECT, &[h]), None).code, response::GENERAL_ERROR);
    assert_eq!(r.answer(&cmd(op::TRUNCATE_OBJECT, &[h, 0, 0]), None).code, response::GENERAL_ERROR);
    fs::remove_dir_all(&at).unwrap();
}

#[test]
fn device_properties_are_described_read_and_set() {
    let mut r = Responder::new(device(), TimeZone::UTC, Arc::default());
    let name = |r: &mut Responder| Reader::new(&dataset(r.answer(&cmd(op::GET_DEVICE_PROP_VALUE, &[u32::from(device::DEVICE_FRIENDLY_NAME)]), None))).string().unwrap();
    assert_eq!(name(&mut r), "Phone");
    let desc = dataset(r.answer(&cmd(op::GET_DEVICE_PROP_DESC, &[u32::from(device::DEVICE_FRIENDLY_NAME)]), None));
    let mut rd = Reader::new(&desc);
    assert_eq!((rd.u16(), rd.u16(), rd.u8()), (Some(device::DEVICE_FRIENDLY_NAME), Some(kind::STR), Some(1)));
    assert_eq!((rd.string().as_deref(), rd.string().as_deref(), rd.u8()), (Some(""), Some("Phone"), Some(form::NONE)));
    let set = cmd(op::SET_DEVICE_PROP_VALUE, &[u32::from(device::DEVICE_FRIENDLY_NAME)]);
    assert!(matches!(r.incoming(&set), Incoming::Dataset));
    let mut w = Writer::new();
    w.string("Kirk 的手机");
    assert_eq!(r.answer(&set, Some(&w.into_bytes())).code, response::OK);
    assert_eq!(name(&mut r), "Kirk 的手机");
    assert_eq!(r.answer(&cmd(op::RESET_DEVICE_PROP_VALUE, &[u32::from(device::DEVICE_FRIENDLY_NAME)]), None).code, response::OK);
    assert_eq!(name(&mut r), "Phone");
    let battery = dataset(r.answer(&cmd(op::GET_DEVICE_PROP_DESC, &[u32::from(device::BATTERY_LEVEL)]), None));
    assert_eq!(battery, [0x01, 0x50, 0x02, 0x00, 0, 0, 76, form::RANGE, 0, 100, 1]);
    let kind_of = dataset(r.answer(&cmd(op::GET_DEVICE_PROP_VALUE, &[u32::from(device::PERCEIVED_DEVICE_TYPE)]), None));
    assert_eq!(kind_of, 3u32.to_le_bytes());
    let set_battery = cmd(op::SET_DEVICE_PROP_VALUE, &[u32::from(device::BATTERY_LEVEL)]);
    assert_eq!(r.answer(&set_battery, Some(&[50])).code, response::DEVICE_PROP_NOT_SUPPORTED);
    assert_eq!(r.answer(&cmd(op::GET_DEVICE_PROP_DESC, &[0x5011]), None).code, response::DEVICE_PROP_NOT_SUPPORTED);
}
