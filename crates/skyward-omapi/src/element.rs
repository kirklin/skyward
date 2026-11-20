//! The phone's own service of one secure element, which hands commands to
//! the chip: the calls it takes, how each is written and how its answer
//! reads.

// The calls and answers serve the phone's own driver and the host tests.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use anyhow::{Context, Result};
use skyward_binder::parcel::{Parcel, Reader};
use skyward_binder::status;

/// The interface, and what each secure element's service registers as:
/// this and then its name, such as eSE1.
pub const INTERFACE: &str = "android.hardware.secure_element.ISecureElement";

/// The interface the service tells its state through, which whoever uses
/// the service serves, with its version and the sum of that version's
/// definition.
pub const CALLBACK: &str = "android.hardware.secure_element.ISecureElementCallback";
pub const VERSION: i32 = 1;
pub const HASH: &str = "3430a4053a6e5f73f2d0c250cab7a8ba654efdae";

/// The one method of [`CALLBACK`]: the secure element came to be
/// connected, or stopped being.
pub const STATE_CHANGED: u32 = 1;

const CLOSE_CHANNEL: u32 = 1;
const GET_ATR: u32 = 2;
const INIT: u32 = 3;
const IS_CARD_PRESENT: u32 = 4;
const OPEN_BASIC_CHANNEL: u32 = 5;
const OPEN_LOGICAL_CHANNEL: u32 = 6;
const RESET: u32 = 7;
const TRANSMIT: u32 = 8;

/// The service's own errors.
const CHANNEL_NOT_AVAILABLE: i32 = 2;
const NO_SUCH_ELEMENT: i32 = 3;
const UNSUPPORTED_OPERATION: i32 = 4;

/// Why a channel did not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotOpened {
    /// The secure element has no channel to spare.
    NoChannel,
    /// It has no applet of the name asked for.
    NoSuchApplet,
    /// It does not open a channel this way.
    Unsupported,
    /// Anything else: why.
    Failed(String),
}

/// A call of the method numbered `code`, as [`Element::call`] sends it.
type Call = (u32, Parcel);

fn call(code: u32) -> Call {
    (code, Parcel::call(INTERFACE))
}

/// Have the service tell its state to the object served under `callback`.
fn ask_init(callback: u64) -> Call {
    let (code, mut parcel) = call(INIT);
    parcel.object(callback);
    (code, parcel)
}

fn ask_present() -> Call {
    call(IS_CARD_PRESENT)
}

fn ask_atr() -> Call {
    call(GET_ATR)
}

fn ask_reset() -> Call {
    call(RESET)
}

/// Open the basic or a logical channel on the applet named `name`, an
/// empty name asking for the one the secure element selects by itself;
/// `answers` says what the caller wants back of the applet.
fn ask_open(logical: bool, name: &[u8], answers: u8) -> Call {
    let (code, mut parcel) = call(if logical { OPEN_LOGICAL_CHANNEL } else { OPEN_BASIC_CHANNEL });
    parcel.blob(name);
    parcel.i32(i32::from(answers as i8));
    (code, parcel)
}

fn ask_close(channel: u8) -> Call {
    let (code, mut parcel) = call(CLOSE_CHANNEL);
    parcel.i32(i32::from(channel as i8));
    (code, parcel)
}

fn ask_transmit(command: &[u8]) -> Call {
    let (code, mut parcel) = call(TRANSMIT);
    parcel.blob(command);
    (code, parcel)
}

/// Why opening a channel was refused, from how the call ended.
fn not_opened(e: anyhow::Error) -> NotOpened {
    match status::refused_with(&e) {
        Some(CHANNEL_NOT_AVAILABLE) => NotOpened::NoChannel,
        Some(NO_SUCH_ELEMENT) => NotOpened::NoSuchApplet,
        Some(UNSUPPORTED_OPERATION) => NotOpened::Unsupported,
        _ => NotOpened::Failed(format!("{e:#}")),
    }
}

/// What the applet answered to being selected on the basic channel.
fn opened_basic(read: &mut Reader) -> Result<Vec<u8>, NotOpened> {
    status::ok(read).map_err(not_opened)?;
    read.blob().map_err(not_opened)
}

/// The number of the logical channel opened, and what the applet answered
/// to being selected on it.
fn opened_logical(read: &mut Reader) -> Result<(u8, Vec<u8>), NotOpened> {
    status::ok(read).map_err(not_opened)?;
    let taken = (|| -> Result<(i32, Vec<u8>)> {
        let end = read.structure().context("read what the secure element's service answered")?;
        let channel = read.i32().context("read the channel's number")?;
        let answer = read.blob().context("read what the applet answered")?;
        read.go(end)?;
        Ok((channel, answer))
    })();
    let (channel, answer) = taken.map_err(not_opened)?;
    // A logical channel has a number from one upwards; the service gives
    // none, or the basic channel's, where it has none to spare.
    match u8::try_from(channel) {
        Ok(channel) if channel > 0 => Ok((channel, answer)),
        _ => Err(NotOpened::NoChannel),
    }
}

/// What the secure element answered to a command.
fn transmitted(read: &mut Reader) -> Result<Vec<u8>> {
    status::ok(read).context("send the secure element a command")?;
    let answer = read.blob().context("read what the secure element answered")?;
    anyhow::ensure!(!answer.is_empty(), "the secure element answered nothing");
    Ok(answer)
}

/// What the service tells of its state: whether the secure element is
/// connected, and why it says so.
pub fn state_changed(read: &mut Reader) -> Result<(bool, String)> {
    Ok((read.bool().context("read whether the secure element is connected")?, read.string().context("read why")?))
}

#[cfg(target_os = "android")]
pub use phone::Element;

#[cfg(target_os = "android")]
mod phone {
    use super::*;
    use skyward_binder::driver::{Binder, Handle};
    use skyward_binder::service;

    /// The service of one secure element, found through the driver the
    /// recovery's own objects are served through.
    pub struct Element<'a> {
        service: Handle<'a>,
    }

    impl<'a> Element<'a> {
        /// The service of the secure element named `name`, such as eSE1;
        /// none while nothing has registered it.
        pub fn find(binder: &'a Binder, name: &str) -> Result<Option<Element<'a>>> {
            Ok(service::find(binder, &format!("{INTERFACE}/{name}"))?.map(|service| Element { service }))
        }

        fn call(&self, (code, parcel): Call, what: &str) -> Result<skyward_binder::driver::Reply<'a>> {
            self.service.transact(code, &parcel).with_context(|| what.to_string())
        }

        /// Have the service tell its state to the object served under
        /// `callback`. It connects to the chip as it is asked this.
        pub fn init(&self, callback: u64) -> Result<()> {
            let reply = self.call(ask_init(callback), "have the secure element's service start")?;
            status::ok(&mut reply.read()).context("have the secure element's service start")
        }

        pub fn present(&self) -> Result<bool> {
            let reply = self.call(ask_present(), "ask whether the secure element is there")?;
            let mut read = reply.read();
            status::ok(&mut read).context("ask whether the secure element is there")?;
            read.bool().context("read whether the secure element is there")
        }

        /// What the chip says of itself as it is powered up; empty where
        /// it says nothing.
        pub fn atr(&self) -> Result<Vec<u8>> {
            let reply = self.call(ask_atr(), "ask what the secure element says of itself")?;
            let mut read = reply.read();
            status::ok(&mut read).context("ask what the secure element says of itself")?;
            read.blob().context("read what the secure element says of itself")
        }

        pub fn reset(&self) -> Result<()> {
            let reply = self.call(ask_reset(), "reset the secure element")?;
            status::ok(&mut reply.read()).context("reset the secure element")
        }

        pub fn open_basic(&self, name: &[u8], answers: u8) -> Result<Vec<u8>, NotOpened> {
            let reply = self.call(ask_open(false, name, answers), "open the basic channel").map_err(not_opened)?;
            opened_basic(&mut reply.read())
        }

        pub fn open_logical(&self, name: &[u8], answers: u8) -> Result<(u8, Vec<u8>), NotOpened> {
            let reply = self.call(ask_open(true, name, answers), "open a logical channel").map_err(not_opened)?;
            opened_logical(&mut reply.read())
        }

        pub fn close(&self, channel: u8) -> Result<()> {
            let reply = self.call(ask_close(channel), "close a channel")?;
            status::ok(&mut reply.read()).context("close a channel")
        }

        pub fn transmit(&self, command: &[u8]) -> Result<Vec<u8>> {
            let reply = self.call(ask_transmit(command), "send the secure element a command")?;
            transmitted(&mut reply.read())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skyward_binder::status::Refusal;

    /// What a call carries after the header every call opens with.
    fn carried(parcel: &Parcel) -> Reader<'_> {
        let mut read = Reader::new(parcel.bytes());
        assert_eq!(read.call().unwrap(), INTERFACE);
        read
    }

    #[test]
    fn the_methods_are_numbered_in_the_order_the_interface_declares_them() {
        let codes = [ask_close(1).0, ask_atr().0, ask_init(1).0, ask_present().0, ask_open(false, &[], 0).0, ask_open(true, &[], 0).0, ask_reset().0, ask_transmit(&[]).0];
        assert_eq!(codes, [1, 2, 3, 4, 5, 6, 7, 8], "closeChannel, getAtr, init, isCardPresent, openBasicChannel, openLogicalChannel, reset, transmit");
    }

    #[test]
    fn a_channel_is_asked_for_by_the_applets_name_and_what_is_wanted_back() {
        let (_, parcel) = ask_open(true, &[0xa0, 0, 0, 0, 3], 0x0c);
        let mut read = carried(&parcel);
        assert_eq!(read.blob().unwrap(), [0xa0, 0, 0, 0, 3]);
        assert_eq!(read.i32().unwrap(), 0x0c, "a byte takes the room of a number");
        assert_eq!(read.here(), parcel.bytes().len());
        let (_, close) = ask_close(3);
        assert_eq!(carried(&close).i32().unwrap(), 3);
        let (_, sent) = ask_transmit(&[0x80, 0xca, 0, 0xfe]);
        assert_eq!(carried(&sent).blob().unwrap(), [0x80, 0xca, 0, 0xfe]);
    }

    #[test]
    fn the_service_is_handed_the_object_it_tells_its_state_to() {
        let (_, parcel) = ask_init(7);
        use skyward_binder::parcel::Payload;
        assert_eq!(parcel.addresses().len(), 1, "one object of the recovery's own");
        let at = parcel.addresses()[0] as usize;
        assert_eq!(u64::from_le_bytes(parcel.bytes()[at + 8..at + 16].try_into().unwrap()), 7);
    }

    /// The reply of a logical channel opened: the structure, there, with
    /// its length, the channel's number and what the applet answered.
    fn logical(channel: i32, answer: &[u8]) -> Parcel {
        let mut reply = status::went_through();
        reply.i32(1);
        let at = reply.here();
        reply.i32(0);
        reply.i32(channel);
        reply.blob(answer);
        let size = (reply.here() - at) as i32;
        reply.fill(at, size);
        reply
    }

    #[test]
    fn a_logical_channel_opened_gives_its_number_and_the_applets_answer() {
        let reply = logical(2, &[0x6f, 0x00, 0x90, 0x00]);
        assert_eq!(opened_logical(&mut Reader::new(reply.bytes())), Ok((2, vec![0x6f, 0x00, 0x90, 0x00])));
        for none in [0, -1, 256] {
            let reply = logical(none, &[]);
            assert_eq!(opened_logical(&mut Reader::new(reply.bytes())), Err(NotOpened::NoChannel), "channel {none}");
        }
    }

    #[test]
    fn why_a_channel_did_not_open_is_read_from_the_services_own_error() {
        let refused = |code: i32| Refusal::service(code, "").reply();
        let opened = |reply: &Parcel| opened_logical(&mut Reader::new(reply.bytes())).unwrap_err();
        assert_eq!(opened(&refused(CHANNEL_NOT_AVAILABLE)), NotOpened::NoChannel);
        assert_eq!(opened(&refused(NO_SUCH_ELEMENT)), NotOpened::NoSuchApplet);
        assert_eq!(opened(&refused(UNSUPPORTED_OPERATION)), NotOpened::Unsupported);
        assert!(matches!(opened(&refused(5)), NotOpened::Failed(_)), "an error of reading or writing");
        assert!(matches!(opened_basic(&mut Reader::new(refused(1).bytes())).unwrap_err(), NotOpened::Failed(_)));
        let mut basic = status::went_through();
        basic.blob(&[0x90, 0x00]);
        assert_eq!(opened_basic(&mut Reader::new(basic.bytes())), Ok(vec![0x90, 0x00]));
    }

    #[test]
    fn an_empty_answer_to_a_command_is_a_failure() {
        let mut answered = status::went_through();
        answered.blob(&[0x90, 0x00]);
        assert_eq!(transmitted(&mut Reader::new(answered.bytes())).unwrap(), [0x90, 0x00]);
        let mut nothing = status::went_through();
        nothing.blob(&[]);
        assert!(transmitted(&mut Reader::new(nothing.bytes())).is_err());
        assert!(transmitted(&mut Reader::new(Refusal::service(5, "").reply().bytes())).is_err());
    }

    #[test]
    fn the_state_told_is_whether_the_chip_is_connected_and_why() {
        let mut told = Parcel::default();
        told.bool(true);
        told.string("SE initialized");
        assert_eq!(state_changed(&mut Reader::new(told.bytes())).unwrap(), (true, "SE initialized".to_string()));
    }
}
