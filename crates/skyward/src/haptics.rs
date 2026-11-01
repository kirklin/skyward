use anyhow::{anyhow, ensure, Context, Result};
use evdev::raw_stream::RawDevice;
use skyward_config::Strength;
use std::f32::consts::TAU;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// linux/input-event-codes.h
const EV_FF: u16 = 0x15;
const FF_PERIODIC: u16 = 0x51;
const FF_CUSTOM: u16 = 0x5d;

/// The rate a made click's samples are played at, the lowest the driver
/// takes: some fifty samples to each swing of the motor.
const RATE: u32 = 8000;

/// A made click: swings of the motor at its resonant frequency that drive
/// it, then swings turned over that stop it, so that it does not ring on.
const BRAKE_SWINGS: f32 = 1.5;

/// The vibration motor, played through the kernel's force-feedback
/// events. The driver fills its queue with a made click's samples as the
/// click is loaded, refilling it for no second play, and keeps the click
/// playing after its samples have run out, even when told to stop, until
/// the click is taken away. So each click is loaded as it is played, at the
/// strength the slider gives, and taken away once it has played out.
pub struct Haptics {
    dev: File,
    click: Click,
    strength: Strength,
    /// The greatest share of a click's full strength that ever plays,
    /// where the motor bangs on its stops above it.
    max_amplitude: f32,
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
    /// Samples made at the motor's resonant frequency, played at [`RATE`].
    Made(Vec<i8>),
}

impl Haptics {
    /// The force-feedback device and the configured click. Loading a click
    /// fills the driver's queue with what it plays, so the click is loaded
    /// only as it is played.
    pub fn open(cfg: &skyward_config::Haptics) -> Result<Haptics> {
        let (path, dev) = motor()?;
        let click = match (cfg.click, &cfg.resonance) {
            (Some(id), _) => {
                eprintln!("haptics {}: clicks with waveform {id}", path.display());
                Click::Kept(id)
            }
            (None, Some(file)) => {
                let hz = resonance(file)?;
                eprintln!("haptics {}: clicks made for a motor swinging at {hz} Hz", path.display());
                Click::Made(made(hz, cfg.drive_swings))
            }
            (None, None) => unreachable!("the configuration names a click of one kind"),
        };
        let max_amplitude = cfg.max_amplitude.unwrap_or(cfg.strength.strongest);
        Ok(Haptics {
            dev,
            click,
            strength: cfg.strength,
            max_amplitude,
            playing: Arc::new(Mutex::new(None)),
            clicks: 0,
        })
    }

    /// One short click at the strength the slider's `position` gives, after
    /// taking away one still playing. Gives how long it plays.
    pub fn click(&mut self, position: f32) -> Result<Duration> {
        let mut playing = self.playing.lock().expect("no thread panics while it holds the click playing");
        if let Some((slot, _)) = playing.take() {
            erase(&self.dev, slot).context("take away the click still playing")?;
        }
        let amplitude = share(&self.strength, position).min(self.max_amplitude);
        let loaded = upload(&self.dev, &self.click, amplitude).context("load the click")?;
        play(&mut self.dev, loaded.slot).context("play the click")?;
        self.clicks += 1;
        let this = (loaded.slot, self.clicks);
        *playing = Some(this);
        drop(playing);
        let dev = self.dev.try_clone().context("share the motor")?;
        let (playing, length) = (self.playing.clone(), loaded.length);
        std::thread::spawn(move || {
            std::thread::sleep(length);
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
    let amplitude = share(&cfg.strength, position).min(cfg.max_amplitude.unwrap_or(cfg.strength.strongest));
    let length = haptics.click(position)?;
    // Once it has played out, the click goes with the device as the
    // command ends.
    std::thread::sleep(length);
    println!("played the click at {:.3} of its full strength, {} ms long", amplitude, length.as_millis());
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

/// A click's samples at [`RATE`] for a motor that swings at `hz`: the
/// `drive` whole swings that drive it, then swings turned over that stop
/// it. A motor that bottoms out and bangs on its stops at full swing is
/// driven fewer swings, so it does not reach its stops.
fn made(hz: f32, drive: f32) -> Vec<i8> {
    let at = |swings: f32| (swings * RATE as f32 / hz).round() as usize;
    let (drive_at, all) = (at(drive), at(drive + BRAKE_SWINGS));
    (0..all)
        .map(|i| {
            let wave = (TAU * hz * i as f32 / RATE as f32).sin() * f32::from(i8::MAX);
            (if i < drive_at { wave } else { -wave }).round() as i8
        })
        .collect()
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
        Click::Made(made) => {
            samples.length = u32::try_from(made.len())?;
            samples.data = made.as_ptr();
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
        Click::Made(made) => Duration::from_secs_f64(made.len() as f64 / f64::from(RATE)),
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

    /// The drive the configuration defaults to, matched to the default the
    /// phone without a motor that bangs keeps.
    const DRIVE_SWINGS: f32 = 2.0;

    #[test]
    fn a_made_click_drives_whole_swings_then_turns_over() {
        let samples = made(172.4, DRIVE_SWINGS);
        let drive = (DRIVE_SWINGS * RATE as f32 / 172.4).round() as usize;
        assert_eq!(samples.len(), (3.5 * RATE as f32 / 172.4).round() as usize);
        assert_eq!(samples[0], 0);
        assert_eq!(*samples.iter().max().unwrap(), i8::MAX);
        let quarter = (RATE as f32 / 172.4 / 4.0).round() as usize;
        assert!(samples[quarter] > 120, "it rises first");
        assert!(samples[drive + quarter] < -120, "it falls first once turned over");
    }

    #[test]
    fn a_shorter_drive_swings_the_motor_less_far() {
        let full = made(172.4, DRIVE_SWINGS);
        let short = made(172.4, 1.0);
        let short_drive = (1.0 * RATE as f32 / 172.4).round() as usize;
        assert!(short.len() < full.len(), "fewer swings in all");
        assert_eq!(*short.iter().max().unwrap(), i8::MAX);
        let quarter = (RATE as f32 / 172.4 / 4.0).round() as usize;
        assert!(short[short_drive + quarter] < -120, "it turns over after its own drive");
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
