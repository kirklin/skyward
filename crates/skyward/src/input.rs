use anyhow::{anyhow, Context, Result};
use evdev::raw_stream::RawDevice;
use evdev::{AbsoluteAxisCode, EventSummary, InputEvent, KeyCode, PropType, SynchronizationCode};
use skyward_config::Write;
use skyward_ui::app::{Key, Touch};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

pub enum Event {
    /// A touch and when the panel reported it, on [`monotonic_ms`]'s clock.
    Touch(Touch, u64),
    /// A button going down, or coming back up.
    Key(Key, bool),
}

/// Milliseconds on the monotonic clock, which the touch panel stamps its
/// events with.
pub fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    assert_eq!(rc, 0, "read the monotonic clock");
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

const KEYS: [KeyCode; 4] = [KeyCode::KEY_POWER, KeyCode::KEY_VOLUMEUP, KeyCode::KEY_VOLUMEDOWN, KeyCode::KEY_BACK];

fn key_of(code: KeyCode) -> Option<Key> {
    match code {
        KeyCode::KEY_POWER => Some(Key::Power),
        KeyCode::KEY_VOLUMEUP => Some(Key::VolumeUp),
        KeyCode::KEY_VOLUMEDOWN => Some(Key::VolumeDown),
        KeyCode::KEY_BACK => Some(Key::Back),
        _ => None,
    }
}

/// A device read failed because it was unplugged.
fn gone(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ENODEV)
}

/// Slot 0's value of a multitouch axis. The kernel keeps multitouch values
/// per slot, apart from the axis state the ABS queries return, and hands
/// them out through EVIOCGMTSLOTS: the axis code, then one value per slot.
fn slot0_value(dev: &RawDevice, axis: AbsoluteAxisCode, slots: usize) -> io::Result<i32> {
    let mut buf = vec![0i32; 1 + slots];
    buf[0] = i32::from(axis.0);
    // _IOC(_IOC_READ, 'E', 0x0a, size)
    let request = (2u32 << 30) | (((buf.len() * 4) as u32) << 16) | (u32::from(b'E') << 8) | 0x0a;
    if unsafe { libc::ioctl(dev.as_raw_fd(), request as _, buf.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf[1])
}

/// The direct-touch panel. Only the first finger is followed (multitouch
/// protocol B, slot 0).
struct Panel {
    dev: RawDevice,
    path: PathBuf,
    x_range: (i32, i32),
    y_range: (i32, i32),
    slots: usize,
    slot: i32,
    down: bool,
    was_down: bool,
    raw: (i32, i32),
    moved: bool,
    /// The kernel dropped events that were not read in time; what arrives
    /// up to the next report is incomplete.
    dropped: bool,
    /// Whether the panel reports its two ranges the other way round from the
    /// screen it lies under, as one mounted across that screen does: what it
    /// calls its first axis then runs along the screen's second.
    swapped: bool,
    /// Whether the numbers the panel reports run the other way from the
    /// screen's, for each of the screen's own axes.
    reversed: (bool, bool),
}

impl Panel {
    fn new(path: PathBuf, dev: RawDevice, w: f32, h: f32, reversed: (bool, bool)) -> Result<Self> {
        let mut x_range = None;
        let mut y_range = None;
        let mut slots = 1;
        for (axis, info) in dev.get_absinfo().context("read touch axis ranges")? {
            match axis {
                AbsoluteAxisCode::ABS_MT_POSITION_X => x_range = Some((info.minimum(), info.maximum())),
                AbsoluteAxisCode::ABS_MT_POSITION_Y => y_range = Some((info.minimum(), info.maximum())),
                AbsoluteAxisCode::ABS_MT_SLOT => slots = info.maximum() as usize + 1,
                _ => {}
            }
        }
        let x_range = x_range.ok_or_else(|| anyhow!("touch panel reports no X range"))?;
        let y_range = y_range.ok_or_else(|| anyhow!("touch panel reports no Y range"))?;
        // Stamp events with the clock the UI keeps time by: how fast a
        // finger moved when it lifted is measured from these stamps.
        // EVIOCSCLOCKID: _IOW('E', 0xa0, int)
        let request = (1u32 << 30) | (4u32 << 16) | (u32::from(b'E') << 8) | 0xa0;
        let clock: libc::c_int = libc::CLOCK_MONOTONIC;
        if unsafe { libc::ioctl(dev.as_raw_fd(), request as _, &clock) } < 0 {
            return Err(io::Error::last_os_error()).context("stamp touch events with the monotonic clock");
        }
        set_nonblocking(dev.as_raw_fd()).context("make touch non-blocking")?;
        // A panel mounted across the screen reports what it calls its first
        // axis along the screen's second, so its two ranges follow the
        // screen's the other way round. The ranges say which way it lies.
        let span = |(low, high): (i32, i32)| (high - low).max(1) as f32;
        let across = (span(x_range) / w - span(y_range) / h).abs();
        let along = (span(x_range) / h - span(y_range) / w).abs();
        let swapped = along < across;
        let mut panel = Panel {
            dev,
            path,
            x_range,
            y_range,
            slots,
            slot: 0,
            down: false,
            was_down: false,
            raw: (0, 0),
            moved: false,
            dropped: false,
            swapped,
            reversed,
        };
        // The kernel drops a position equal to the one a slot already holds,
        // so a touch landing where the last one lifted carries no position;
        // start from what the panel holds now.
        panel.resync().context("read the touch state")?;
        eprintln!(
            "touch {:?}: x {x_range:?} y {y_range:?}, {slots} slots, slot 0 at {:?}, {} across the screen {w} by {h}",
            panel.dev.name().unwrap_or("?"),
            panel.raw,
            if swapped { "swapped" } else { "along" }
        );
        Ok(panel)
    }

    /// Read the current slot and slot 0's finger back from the panel.
    fn resync(&mut self) -> io::Result<()> {
        self.slot = self.dev.get_abs_state()?[usize::from(AbsoluteAxisCode::ABS_MT_SLOT.0)].value;
        self.down = slot0_value(&self.dev, AbsoluteAxisCode::ABS_MT_TRACKING_ID, self.slots)? >= 0;
        self.raw = (
            slot0_value(&self.dev, AbsoluteAxisCode::ABS_MT_POSITION_X, self.slots)?,
            slot0_value(&self.dev, AbsoluteAxisCode::ABS_MT_POSITION_Y, self.slots)?,
        );
        self.moved = true;
        Ok(())
    }

    fn read(&mut self, w: f32, h: f32, out: &mut Vec<Event>) -> io::Result<()> {
        let events: Vec<InputEvent> = match self.dev.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };
        for e in events {
            match e.destructure() {
                // Skip to the next report and read the state back there.
                EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) => self.dropped = true,
                EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
                    if std::mem::take(&mut self.dropped) {
                        self.resync()?;
                    }
                    let time = e.as_ref().time;
                    let at = time.tv_sec as u64 * 1000 + time.tv_usec as u64 / 1000;
                    let span = |r: (i32, i32)| (r.1 - r.0).max(1) as f32;
                    // What the panel calls its first axis runs along the
                    // screen's second where it lies across the screen, and
                    // its second along the screen's first.
                    let (raw_x, range_x, raw_y, range_y) = match self.swapped {
                        false => (self.raw.0, self.x_range, self.raw.1, self.y_range),
                        true => (self.raw.1, self.y_range, self.raw.0, self.x_range),
                    };
                    let mut x = (raw_x - range_x.0) as f32 / span(range_x) * w;
                    let mut y = (raw_y - range_y.0) as f32 / span(range_y) * h;
                    // A panel mirrored against the screen counts one of the
                    // screen's axes from the other end.
                    if self.reversed.0 {
                        x = w - x;
                    }
                    if self.reversed.1 {
                        y = h - y;
                    }
                    match (self.was_down, self.down) {
                        (false, true) => out.push(Event::Touch(Touch::Down(x, y), at)),
                        (true, true) if self.moved => out.push(Event::Touch(Touch::Move(x, y), at)),
                        (true, false) => out.push(Event::Touch(Touch::Up, at)),
                        _ => {}
                    }
                    self.was_down = self.down;
                    self.moved = false;
                }
                _ if self.dropped => {}
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_MT_SLOT, v) => self.slot = v,
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_MT_TRACKING_ID, v) if self.slot == 0 => self.down = v >= 0,
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_MT_POSITION_X, v) if self.slot == 0 => {
                    self.raw.0 = v;
                    self.moved = true;
                }
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_MT_POSITION_Y, v) if self.slot == 0 => {
                    self.raw.1 = v;
                    self.moved = true;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// The touchscreen and the hardware buttons. /dev/input is watched, so a
/// device that shows up later, such as a touch panel whose driver module
/// loads after the UI starts, is picked up when it appears.
pub struct Inputs {
    watch: OwnedFd,
    panel: Option<Panel>,
    buttons: Vec<(PathBuf, RawDevice)>,
    seen: HashSet<PathBuf>,
    /// What the touchscreen is told each time its panel is taken in.
    touch_writes: Vec<Write>,
    /// Whether the panel reports the screen's two axes the other way round,
    /// the first of the pair being the horizontal one.
    reversed: (bool, bool),
    w: f32,
    h: f32,
}

impl Inputs {
    pub fn open(w: u32, h: u32, touch_writes: Vec<Write>, reversed: Vec<String>) -> Result<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("create inotify instance");
        }
        let watch = unsafe { OwnedFd::from_raw_fd(fd) };
        let dir = CString::new("/dev/input")?;
        // IN_ATTRIB too: ueventd sets a node's owner and mode right after
        // creating it, and the node may only open once that is done.
        if unsafe { libc::inotify_add_watch(fd, dir.as_ptr(), libc::IN_CREATE | libc::IN_ATTRIB) } < 0 {
            return Err(io::Error::last_os_error()).context("watch /dev/input");
        }
        // Which of the screen's own axes the panel reports the other way
        // round, as the device's configuration says.
        let reversed = (reversed.iter().any(|a| a == "x"), reversed.iter().any(|a| a == "y"));
        let mut inputs =
            Inputs { watch, panel: None, buttons: Vec::new(), seen: HashSet::new(), touch_writes, reversed, w: w as f32, h: h as f32 };
        inputs.scan()?;
        if inputs.panel.is_none() {
            eprintln!("no touch panel yet; waiting for one to appear under /dev/input");
        }
        Ok(inputs)
    }

    /// Take in the devices under /dev/input not seen before: the first
    /// direct multitouch device as the panel, and every device with a
    /// power, volume or back key as buttons. A panel taken in is told what
    /// the device description writes to the touchscreen.
    fn scan(&mut self) -> Result<()> {
        for (path, dev) in evdev::raw_stream::enumerate() {
            if !self.seen.insert(path.clone()) {
                continue;
            }
            let direct = dev.properties().contains(PropType::DIRECT);
            let mt = dev
                .supported_absolute_axes()
                .is_some_and(|a| a.contains(AbsoluteAxisCode::ABS_MT_POSITION_X) && a.contains(AbsoluteAxisCode::ABS_MT_POSITION_Y));
            let keys = dev.supported_keys().is_some_and(|k| KEYS.iter().any(|c| k.contains(*c)));
            eprintln!("input {}: {:?} direct={direct} multitouch={mt} keys={keys}", path.display(), dev.name().unwrap_or("?"));
            if direct && mt && self.panel.is_none() {
                self.panel = Some(Panel::new(path, dev, self.w, self.h, self.reversed)?);
                for w in &self.touch_writes {
                    fs::write(&w.path, &w.value).with_context(|| format!("write {} to {}", w.value, w.path))?;
                }
            } else if keys {
                set_nonblocking(dev.as_raw_fd()).context("make buttons non-blocking")?;
                self.buttons.push((path, dev));
            }
        }
        Ok(())
    }

    /// Empty the watch's queue; true when /dev/input changed.
    fn changed(&self) -> Result<bool> {
        let mut buf = [0u8; 4096];
        let mut any = false;
        loop {
            let n = unsafe { libc::read(self.watch.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n >= 0 {
                any = true;
                continue;
            }
            let err = io::Error::last_os_error();
            match err.kind() {
                ErrorKind::WouldBlock => return Ok(any),
                ErrorKind::Interrupted => continue,
                _ => return Err(err).context("read /dev/input changes"),
            }
        }
    }

    /// Whether a touch panel has come up.
    pub fn has_panel(&self) -> bool {
        self.panel.is_some()
    }

    pub fn fds(&self) -> Vec<RawFd> {
        let mut fds = vec![self.watch.as_raw_fd()];
        fds.extend(self.panel.iter().map(|p| p.dev.as_raw_fd()));
        fds.extend(self.buttons.iter().map(|(_, b)| b.as_raw_fd()));
        fds
    }

    /// Everything the devices have queued, as touches and key presses.
    pub fn read(&mut self) -> Result<Vec<Event>> {
        if self.changed()? {
            self.scan()?;
        }
        let mut out = Vec::new();
        if let Some(panel) = &mut self.panel {
            match panel.read(self.w, self.h, &mut out) {
                Ok(()) => {}
                Err(e) if gone(&e) => {
                    eprintln!("input {}: removed", panel.path.display());
                    self.seen.remove(&panel.path);
                    self.panel = None;
                }
                Err(e) => return Err(e).context("read touch events"),
            }
        }
        let mut removed = Vec::new();
        for (i, (path, b)) in self.buttons.iter_mut().enumerate() {
            match b.fetch_events() {
                Ok(events) => {
                    // The kernel's own repeats of a key held are left out;
                    // the UI times its own.
                    for e in events {
                        if let EventSummary::Key(_, code, value @ (0 | 1)) = e.destructure() {
                            if let Some(k) = key_of(code) {
                                out.push(Event::Key(k, value == 1));
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) if gone(&e) => {
                    eprintln!("input {}: removed", path.display());
                    removed.push(i);
                }
                Err(e) => return Err(e).context("read button events"),
            }
        }
        for i in removed.into_iter().rev() {
            let (path, _) = self.buttons.remove(i);
            self.seen.remove(&path);
        }
        Ok(out)
    }
}
