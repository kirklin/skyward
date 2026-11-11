//! Installing a package that a computer sends with `adb sideload`, or that
//! is a file on the phone: A/B update packages, which the phone's own
//! update_engine applies, zips that install themselves by running the
//! script inside them, and partition images, which are written here.

pub mod adb;
pub mod asked;
pub mod blockmap;
pub mod commands;
pub mod fuse;
pub mod image;
pub mod keep;
pub mod ota;
pub mod package;
pub mod script;
pub mod signature;
pub mod upgrade;
pub mod wipe;

#[cfg(target_os = "android")]
mod ffs;
#[cfg(target_os = "android")]
mod run;

pub use image::{ImageKind, Ramdisk};
#[cfg(target_os = "android")]
pub use run::{Detach, Installer};
use skyward_info::{Battery, Charge};
use skyward_vab::{Slot, SnapshotState};

/// The least charge to install on, without and with a charger, as
/// Android's recovery asks.
const BATTERY_ALONE: u8 = 20;
const BATTERY_CHARGING: u8 = 15;

/// The least charge to install on, with a charger in or not.
pub fn least_charge(charging: bool) -> u8 {
    if charging {
        BATTERY_CHARGING
    } else {
        BATTERY_ALONE
    }
}

/// Why `battery` holds too little charge to install on, if it does. A
/// charger is in while the battery charges, is full, or is held from
/// charging.
pub fn battery_refusal(battery: &Battery) -> Option<Refusal> {
    let charging = matches!(battery.charge, Charge::Charging | Charge::Full | Charge::NotCharging);
    (battery.level < least_charge(charging)).then_some(Refusal::Battery { level: battery.level, charging })
}

/// Where the installer keeps what update_engine wrote while it last
/// installed an update, in the recovery service's own folder.
pub const ENGINE_LOG: &str = "/skyward/run/update_engine.log";

/// Where the installer keeps what the install script it last ran showed
/// and asked for, with what it and the programs it ran wrote, in the
/// recovery service's own folder.
pub const SCRIPT_LOG: &str = "/skyward/run/install-script.log";

/// What the screen asks of an install besides the package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// This recovery goes back into the recovery partitions an install
    /// writes over.
    pub keep: bool,
    /// A zip with an install script runs only if it was signed with one of
    /// the keys the phone's updates are signed with.
    pub signed: bool,
}

/// What the installer tells the screen, in the order it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// USB is ready for `adb sideload`.
    Waiting,
    /// A package of this many bytes is being looked at: one the computer
    /// started sending, or a file on the phone.
    Checking { size: u64 },
    /// An update package passed the checks and waits for
    /// [`Command::Install`] before anything is written: the version it
    /// brings, and whether all user data is to be erased before that
    /// system starts. One that erases always waits, and so does one from a
    /// file on the phone, which a single tap picks.
    Ready { version: Option<String>, wipe: bool },
    /// An update package passed the checks and is being installed; the
    /// version it brings.
    Installing { version: Option<String> },
    /// Super has no room to keep the running system beside the new one, so
    /// the update writes over it: until it finishes, the phone may not
    /// start.
    Overwriting,
    /// A partition image, held until [`Command::Flash`] says where it goes:
    /// the partitions of this phone it fits, the likeliest first.
    Image { kind: ImageKind, size: u64, partitions: Vec<&'static str> },
    /// A zip that installs itself by running the script inside it, held
    /// until [`Command::Install`]: a script may write over any partition,
    /// so it always waits.
    Script,
    /// A system to install beside the installed one, held until
    /// [`Command::InstallSystem`] says how much room its root takes: its
    /// name, its version, and the sizes there is room for, the least first.
    System { name: String, version: String, sizes: Vec<u64> },
    /// The install script has started.
    Running,
    /// A line the install script shows.
    Print(String),
    Progress(Stage),
    /// The install wrote over this recovery in the recovery partitions,
    /// which the screen asked to keep: how writing it back went.
    Reflashed(Reflash),
    /// In rescue, the computer asked for all user data to be erased: the
    /// screen erases it and says how that went with [`Command::Wiped`].
    Wipe,
    /// In rescue, the computer asked for the phone to leave for `target`:
    /// empty for the system, or `bootloader`, `fastboot`, `recovery` or
    /// `rescue`. The installer finishes after it.
    Reboot(String),
    /// In rescue, what the computer asked for last ended this way; the
    /// computer may ask for more.
    Done(Outcome),
    Finished(Outcome),
}

/// How this recovery went back into the recovery partitions an install
/// wrote over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reflash {
    /// Into those of these slots.
    Written(Vec<Slot>),
    /// It could not be, for this reason.
    Failed(String),
}

/// How far the work is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// update_engine writes the new system: the bytes of its payload read
    /// so far.
    Writing { done: u64, total: u64 },
    /// It reads back what it wrote, one partition after another.
    Verifying { index: usize, count: usize, partition: String },
    /// It runs the package's own last steps and switches slots.
    Finishing,
    /// An image is written and read back: bytes so far, of all.
    Flashing { done: u64, total: u64 },
    /// The install script runs, as far as it says it is, in thousandths.
    Script { thousandths: u16 },
    /// The zip's signature is checked: bytes read so far of what it is a
    /// signature of, and of all of that.
    Signature { done: u64, total: u64 },
    /// This recovery is written back into the recovery partitions the
    /// install wrote over, and read back: bytes so far, of all.
    Reflashing { done: u64, total: u64 },
    /// Room for a system is set aside in the data partition: bytes so far,
    /// of all.
    Allocating { done: u64, total: u64 },
    /// The system's root is written and read back: bytes so far, of all.
    Root { done: u64, total: u64 },
    /// The system's boot set is put in the store.
    Storing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The update is in `slot`, which starts next; `wipe` when all user data
    /// is to be erased before it does.
    Installed { slot: Slot, wipe: bool },
    /// The image is in the partition of these slots.
    Flashed { partition: &'static str, slots: Vec<Slot> },
    /// The install script ran to its end; `wipe_cache` when it asked for
    /// the cache partition to be erased after.
    Ran { wipe_cache: bool },
    /// The system of this name is installed beside the installed one.
    SystemInstalled { name: String },
    /// Stopped before anything was written.
    Cancelled,
    /// Not installed, for this reason; nothing was written.
    Refused(Refusal),
    Failed(Failure),
}

/// Why a package is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A file this recovery cannot install.
    Unknown,
    /// A partition image, where the system asked for a package that
    /// installs itself.
    Image,
    /// An update package that is not for A/B phones; the type it names.
    NotAb(String),
    /// A package in misc that does not say it erases the phone for good;
    /// the type it names.
    NotBrick(String),
    /// A package that erases the phone for good and names no phone, where
    /// the system is signed with the keys its maker keeps.
    NoSerial,
    /// The phone came to the recovery after its kernel stopped: an install
    /// the system asked for before that is not started, as Android's own
    /// recovery does not start one.
    Panicked,
    /// Made for other devices: the ones it names.
    OtherDevice(String),
    /// Made for particular phones, not this one.
    OtherSerial,
    /// An incremental package, which needs snapshots that recovery cannot
    /// make on a Virtual A/B phone.
    Incremental,
    /// Older than the installed system: both build times.
    Older { package: i64, installed: i64 },
    /// Its security patch is older than the installed one, which leaves
    /// the data partition unreadable unless it is erased.
    OlderPatch { package: String, installed: String },
    /// An image for partitions this phone does not have.
    NoPartition,
    /// Too little charge to install safely: the level, and whether a
    /// charger is in.
    Battery { level: u8, charging: bool },
    /// An earlier update is still in flight; writing over it would break
    /// it.
    Pending(SnapshotState),
    /// The computer's adb is too old to send a package a block at a time.
    OldHost,
    /// A zip with an install script that has no signature of the whole of
    /// it, where the screen asked for one.
    Unsigned,
    /// A zip with an install script whose signature was made with none of
    /// the phone's update keys, or not of the zip as it is, where the
    /// screen asked for one made with them.
    Unverified,
    /// A system to install beside the installed one, which only someone
    /// at the screen installs, not the system.
    SystemAsked,
    /// A system to install while the data partition, where its root goes,
    /// is not open.
    DataClosed,
    /// A system to install while the one of this name is installed beside
    /// the installed system already.
    SystemInstalled(String),
    /// A system whose root takes `least` bytes, where the data partition
    /// has room for `room`.
    NoRoom { least: u64, room: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// update_engine stopped with this error code.
    Engine(i32),
    /// The install script ended with this status, or 128 and the signal
    /// that ended it, as a shell tells it.
    Script(i32),
    /// The computer stopped sending: the cable came out or `adb sideload`
    /// was stopped.
    Disconnected,
    /// Reading the file on the phone failed, for this reason: the USB
    /// drive came out, or the storage failed.
    Unreadable(String),
    /// The package changed where it is while it was being read: on the
    /// computer as it was sent, or the file on the phone.
    Changed,
    /// Anything else: the error and its causes.
    Error(String),
}

/// What the screen tells the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Write the image held in [`Event::Image`] to `partition` in these
    /// slots.
    Flash { partition: &'static str, slots: Vec<Slot> },
    /// Install the update held in [`Event::Ready`], or run the script held
    /// in [`Event::Script`].
    Install,
    /// Install the system held in [`Event::System`] with a root of `size`
    /// bytes, one of those it offered.
    InstallSystem { size: u64 },
    /// The user data the computer asked to be erased in [`Event::Wipe`]
    /// was erased, or was not.
    Wiped(bool),
    /// Stop without installing anything.
    Cancel,
}
