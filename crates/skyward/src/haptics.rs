use anyhow::{anyhow, ensure, Context, Result};
use evdev::raw_stream::RawDevice;
use skyward_config::Strength;
use std::f32::consts::{PI, TAU};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// linux/input-event-codes.h
const EV_FF: u16 = 0x15;
const FF_PERIODIC: u16 = 0x51;
const FF_CUSTOM: u16 = 0x5d;

/// The rate a made click's samples are played at, the lowest the driver
/// takes: some fifty samples to each swing of the motor.
const RATE: u32 = 8000;

/// A made click: swings of the motor at its resonant frequency that drive
/// it, then swings turned over that stop it, so that it does not ring on.
const DRIVE_SWINGS: f32 = 2.0;
const BRAKE_SWINGS: f32 = 1.5;
/// The ticks of the screen going off by the power key, as the phone's own
/// system plays them on the 13 Ultra, whose motor swings at [`TICKS_HZ`]: a
/// crisp tick near twice the motor's frequency, then a softer tap below it,
/// half-swing by half-swing. Each half-swing is its start, peak and end in
/// milliseconds, its signed share of the full, and how steeply it rises and
/// falls: 1 on half a cosine, 2 on its square. For another motor the times
/// go in proportion. They are made at [`TICKS_RATE`].
const TICKS_HZ: f32 = 172.5;
const TICKS: [(f32, f32, f32, f32, i32, i32); 11] = [
    (0.88, 2.29, 3.04, 0.504, 1, 1),
    (3.04, 3.79, 4.38, -0.717, 1, 1),
    (4.38, 5.08, 5.92, 1.0, 1, 1),
    (5.92, 6.54, 7.38, -0.819, 1, 1),
    (7.38, 7.83, 8.83, 0.559, 1, 1),
    (8.83, 9.33, 10.33, -0.220, 1, 1),
    (33.54, 39.67, 42.42, 0.906, 2, 1),
    (42.42, 44.46, 46.71, -0.906, 1, 1),
    (46.71, 48.83, 51.17, 0.717, 1, 1),
    (51.17, 53.46, 56.63, -1.0, 1, 1),
    (56.63, 58.63, 67.67, 0.260, 1, 2),
];
const TICKS_RATE: u32 = 24000;

/// The vibration motor, played through the kernel's force-feedback
/// events. The driver fills its queue with a made click's samples as the
/// click is loaded, refilling it for no second play, and keeps the click
/// playing after its samples have run out, even when told to stop, until
/// the click is taken away. So each click is loaded as it is played, at the
/// strength the slider gives, and taken away once it has played out.
pub struct Haptics {
    dev: File,
    click: Click,
    ticks: Click,
    strength: Strength,
    /// The click playing, by its slot and its count, until it is taken
    /// away; a click taken away early by the next is not taken away again.
    playing: Arc<Mutex<Option<(i16, u64)>>>,
    clicks: u64,
}

/// What a click is.
enum Click {
    /// A waveform the driver keeps, by its number. The driver says how long
    /// it plays as it loads it.
    Kept(i16),
    /// Samples made at the motor's resonant frequency, which the driver
    /// plays as they are, at `rate` a second.
    Samples { data: Vec<i8>, rate: u32 },
}

impl Haptics {
    /// The force-feedback device and the configured click. Loading a click
    /// fills the driver's queue with what it plays, so the click is loaded
    /// only as it is played.
    pub fn open(cfg: &skyward_config::Haptics) -> Result<Haptics> {
        let (path, dev) = motor()?;
        let hz = resonance(&cfg.resonance)?;
        let click = match cfg.click {
            Some(id) => {
                eprintln!("haptics {}: clicks with waveform {id}, ticks made for a motor swinging at {hz} Hz", path.display());
                Click::Kept(id)
            }
            None => {
                eprintln!("haptics {}: clicks and ticks made for a motor swinging at {hz} Hz", path.display());
                Click::Samples { data: made(hz), rate: RATE }
            }
        };
        let ticks = Click::Samples { data: ticks(hz), rate: TICKS_RATE };
        Ok(Haptics { dev, click, ticks, strength: cfg.strength, playing: Arc::new(Mutex::new(None)), clicks: 0 })
    }

    /// One short click at the strength the slider's `position` gives, after
    /// taking away one still playing. Gives how long it plays.
    pub fn click(&mut self, position: f32) -> Result<Duration> {
        self.start(false, share(&self.strength, position))
    }

    /// The ticks of the screen going off, as strong as their samples are
    /// whatever the slider says. Gives how long they play.
    pub fn ticks(&mut self) -> Result<Duration> {
        self.start(true, 1.0)
    }

    /// Play the ticks, or else the click, at `share` of its full strength,
    /// after taking away what still plays.
    fn start(&mut self, ticks: bool, share: f32) -> Result<Duration> {
        let mut playing = self.playing.lock().expect("no thread panics while it holds the click playing");
        if let Some((slot, _)) = playing.take() {
            erase(&self.dev, slot).context("take away the click still playing")?;
        }
        let effect = if ticks { &self.ticks } else { &self.click };
        let loaded = upload(&self.dev, effect, share).context("load the click")?;
        play(&mut self.dev, loaded.slot).context("play the click")?;
        self.clicks += 1;
        let this = (loaded.slot, self.clicks);
        *playing = Some(this);
        drop(playing);
        let dev = self.dev.try_clone().context("share the motor")?;
        let length = loaded.length;
        // Timed from now: a thread may wait a while before it first runs.
        let until = Instant::now() + length;
        let playing = self.playing.clone();
        std::thread::spawn(move || {
            std::thread::sleep(until.saturating_duration_since(Instant::now()));
            let mut playing = playing.lock().expect("no thread panics while it holds the click playing");
            if *playing == Some(this) {
                *playing = None;
                if let Err(e) = erase(&dev, this.0) {
                    eprintln!("take away the click: {e}");
                }
            }
        });
        Ok(length)
    }
}

/// Play the configured click once at the slider's `position`, saying how
/// strongly and for how long, for trying out a motor.
pub fn probe(cfg: &skyward_config::Haptics, position: f32) -> Result<()> {
    let mut haptics = Haptics::open(cfg)?;
    let length = haptics.click(position)?;
    // Once it has played out, the click goes with the device as the
    // command ends.
    std::thread::sleep(length);
    println!("played the click at {:.3} of its full strength, {} ms long", share(&cfg.strength, position), length.as_millis());
    Ok(())
}

/// The first input device under /dev/input that takes periodic
/// force-feedback effects, opened for writing.
fn motor() -> Result<(PathBuf, File)> {
    for (path, dev) in evdev::raw_stream::enumerate() {
        if takes_periodic(&dev) {
            let file = OpenOptions::new().read(true).write(true).open(&path).with_context(|| format!("open {}", path.display()))?;
            return Ok((path, file));
        }
    }
    Err(anyhow!("no input device takes force-feedback effects"))
}

fn takes_periodic(dev: &RawDevice) -> bool {
    dev.supported_ff().is_some_and(|ff| ff.contains(evdev::FFEffectCode::FF_PERIODIC))
}

/// The share of the click's full strength at the slider's `position`.
fn share(strength: &Strength, position: f32) -> f32 {
    strength.lightest * (strength.strongest / strength.lightest).powf(position.clamp(0.0, 1.0))
}

/// The motor's resonant frequency in hertz, from the file the driver says
/// it in, in tenths of a hertz.
fn resonance(file: &str) -> Result<f32> {
    let text = fs::read_to_string(file).with_context(|| format!("read {file}"))?;
    let tenths: u32 = text.split_whitespace().next().and_then(|n| n.parse().ok()).with_context(|| format!("{file} holds {text:?}"))?;
    ensure!(tenths > 0, "{file} says the motor swings at 0 Hz");
    Ok(tenths as f32 / 10.0)
}

/// A click's samples at [`RATE`] for a motor that swings at `hz`: whole
/// swings that drive it, then swings turned over that stop it.
fn made(hz: f32) -> Vec<i8> {
    let at = |swings: f32| (swings * RATE as f32 / hz).round() as usize;
    let (drive, all) = (at(DRIVE_SWINGS), at(DRIVE_SWINGS + BRAKE_SWINGS));
    (0..all)
        .map(|i| {
            let wave = (TAU * hz * i as f32 / RATE as f32).sin() * f32::from(i8::MAX);
            (if i < drive { wave } else { -wave }).round() as i8
        })
        .collect()
}

/// The ticks at [`TICKS_RATE`] for a motor that swings at `hz`, still
/// between the half-swings.
fn ticks(hz: f32) -> Vec<i8> {
    let scale = TICKS_HZ / hz;
    let at = |ms: f32| (ms * scale * TICKS_RATE as f32 / 1000.0).round() as usize;
    let mut samples = vec![0i8; at(TICKS[TICKS.len() - 1].2)];
    for (start, peak, end, share, rise, fall) in TICKS {
        for (i, sample) in samples.iter_mut().enumerate().take(at(end)).skip(at(start)) {
            let t = i as f32 * 1000.0 / TICKS_RATE as f32 / scale;
            let k = if t < peak {
                ((1.0 - (PI * (t - start) / (peak - start)).cos()) / 2.0).powi(rise)
            } else {
                ((1.0 + (PI * (t - peak) / (end - peak)).cos()) / 2.0).powi(fall)
            };
            *sample = (share * k * f32::from(i8::MAX)).round() as i8;
        }
    }
    samples
}

/// A click as the driver loaded it: the slot the kernel keeps it in, and
/// how long it plays.
struct Loaded {
    slot: i16,
    length: Duration,
}

/// Samples handed to the driver to play as they are, as its
/// `custom_fifo_data` lays them out.
#[repr(C)]
struct Samples {
    /// Unused by the driver.
    index: u32,
    length: u32,
    rate: u32,
    data: *const i8,
}

/// Load `click` at `share` of its full strength as a custom periodic
/// effect in a slot of its own. The driver drives the motor at the click's
/// highest voltage times the magnitude.
fn upload(dev: &File, click: &Click, share: f32) -> Result<Loaded> {
    // The driver tells the two kinds apart by the length in bytes of what
    // the effect points to, and loads nothing for any other length, though
    // the upload still succeeds. For a waveform it keeps it reads the
    // waveform's number from the first of three values and writes how long
    // it plays into the other two, in whole seconds and then milliseconds.
    let mut kept: [i16; 3] = [0; 3];
    let mut samples = Samples { index: 0, length: 0, rate: RATE, data: std::ptr::null() };
    let (len, data) = match click {
        Click::Kept(id) => {
            kept[0] = *id;
            (std::mem::size_of_val(&kept), kept.as_mut_ptr())
        }
        Click::Samples { data, rate } => {
            samples.length = u32::try_from(data.len())?;
            samples.rate = *rate;
            samples.data = data.as_ptr();
            (std::mem::size_of::<Samples>(), (&mut samples as *mut Samples).cast::<i16>())
        }
    };
    let periodic = libc::ff_periodic_effect {
        waveform: FF_CUSTOM,
        period: 0,
        magnitude: (share.clamp(0.0, 1.0) * f32::from(i16::MAX)).round() as i16,
        offset: 0,
        phase: 0,
        envelope: libc::ff_envelope { attack_length: 0, attack_level: 0, fade_length: 0, fade_level: 0 },
        custom_len: len as u32,
        custom_data: data,
    };
    let mut effect = libc::ff_effect {
        type_: FF_PERIODIC,
        id: -1,
        direction: 0,
        trigger: libc::ff_trigger { button: 0, interval: 0 },
        replay: libc::ff_replay { length: 0, delay: 0 },
        u: [0; 4],
    };
    // The union holds the periodic effect; it is exactly as large.
    const _: () = assert!(std::mem::size_of::<libc::ff_periodic_effect>() == std::mem::size_of::<[u64; 4]>());
    effect.u = unsafe { std::mem::transmute::<libc::ff_periodic_effect, [u64; 4]>(periodic) };
    // EVIOCSFF: _IOW('E', 0x80, struct ff_effect)
    let size = std::mem::size_of::<libc::ff_effect>() as u32;
    let request = (1u32 << 30) | (size << 16) | (u32::from(b'E') << 8) | 0x80;
    if unsafe { libc::ioctl(dev.as_raw_fd(), request as _, &mut effect) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let length = match click {
        Click::Kept(id) => {
            let [_, seconds, millis] = kept;
            let length = Duration::from_secs(u64::try_from(seconds)?) + Duration::from_millis(u64::try_from(millis)?);
            ensure!(!length.is_zero(), "the driver loaded waveform {id} without saying how long it plays");
            length
        }
        // The queue is full before the samples start playing.
        Click::Samples { data, rate } => Duration::from_secs_f64(data.len() as f64 / f64::from(*rate)),
    };
    Ok(Loaded { slot: effect.id, length })
}

/// Start the loaded effect in slot `id`.
fn play(dev: &mut File, id: i16) -> io::Result<()> {
    let event = libc::input_event { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, type_: EV_FF, code: id as u16, value: 1 };
    let bytes = unsafe { std::slice::from_raw_parts((&event as *const libc::input_event).cast::<u8>(), std::mem::size_of::<libc::input_event>()) };
    dev.write_all(bytes)
}

/// Take the effect in slot `id` away, which stops it for good.
fn erase(dev: &File, id: i16) -> io::Result<()> {
    // EVIOCRMFF: _IOW('E', 0x81, int)
    let request = (1u32 << 30) | ((std::mem::size_of::<libc::c_int>() as u32) << 16) | (u32::from(b'E') << 8) | 0x81;
    if unsafe { libc::ioctl(dev.as_raw_fd(), request as _, libc::c_int::from(id)) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_made_click_drives_whole_swings_then_turns_over() {
        let samples = made(172.4);
        let drive = (DRIVE_SWINGS * RATE as f32 / 172.4).round() as usize;
        assert_eq!(samples.len(), (3.5 * RATE as f32 / 172.4).round() as usize);
        assert_eq!(samples[0], 0);
        assert_eq!(*samples.iter().max().unwrap(), i8::MAX);
        let quarter = (RATE as f32 / 172.4 / 4.0).round() as usize;
        assert!(samples[quarter] > 120, "it rises first");
        assert!(samples[drive + quarter] < -120, "it falls first once turned over");
    }

    #[test]
    fn the_screen_going_off_ticks_crisply_then_taps_softly() {
        let at = |ms: f32| (ms * TICKS_RATE as f32 / 1000.0).round() as usize;
        let samples = ticks(TICKS_HZ);
        assert_eq!(samples.len(), at(67.67));
        assert_eq!(samples[at(5.08)], i8::MAX, "the tick swings up to the full");
        assert_eq!(samples[at(53.46)], -i8::MAX, "and the tap's last full swing stops the motor");
        assert!(samples[at(10.33)..at(33.54)].iter().all(|&s| s == 0), "still between");
        let crossings = |s: &[i8]| s.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!(crossings(&samples[..at(10.33)]) > crossings(&samples[at(33.54)..]), "the tick swings faster than the tap");
        assert_eq!(ticks(TICKS_HZ * 2.0).len(), samples.len().div_ceil(2), "a motor twice as quick ticks in half the time");
    }

    #[test]
    fn the_slider_spans_the_strengths_each_step_the_same_factor() {
        let s = Strength { lightest: 0.25, strongest: 1.0 };
        assert_eq!((share(&s, 0.0), share(&s, 1.0)), (0.25, 1.0));
        assert!((share(&s, 0.5) - 0.5).abs() < 1e-6);
        assert!((share(&s, 0.25) / share(&s, 0.0) - share(&s, 1.0) / share(&s, 0.75)).abs() < 1e-6, "each step the same factor");
        assert_eq!((share(&s, -1.0), share(&s, 2.0)), (0.25, 1.0), "no further than its ends");
    }
}
