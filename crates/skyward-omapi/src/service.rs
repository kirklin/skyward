//! The secure element service itself: the object programs find in the
//! service directory, and the readers, sessions and channels it hands
//! them, each behaving as the one in Android's own service does towards a
//! program of the phone's.

use crate::apdu::{self, Next, Refused, Selected};
use crate::element::{self, Element, NotOpened};
use anyhow::{Context, Result};
use skyward_binder::driver::Binder;
use skyward_binder::parcel::{Parcel, Reader};
use skyward_binder::serve::{Object, Objects};
use skyward_binder::status::{self, Refusal};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The interfaces served, the version of them and the sum of that
/// version's definition.
const SERVICE: &str = "android.se.omapi.ISecureElementService";
const READER: &str = "android.se.omapi.ISecureElementReader";
const SESSION: &str = "android.se.omapi.ISecureElementSession";
const CHANNEL: &str = "android.se.omapi.ISecureElementChannel";
const VERSION: i32 = 1;
const HASH: &str = "894069bcfe4f35ceb2088278ddf87c83adee8014";

/// The service's own errors: reading from or writing to the secure element
/// failed, and the secure element has no such applet.
const IO_ERROR: i32 = 1;
const NO_SUCH_ELEMENT_ERROR: i32 = 2;

/// The basic channel's number.
const BASIC: u8 = 0;

/// How long a secure element's own service gets to say the chip is
/// connected once it is asked to start.
const CONNECTS_IN: Duration = Duration::from_secs(5);

/// The reader names Android gives embedded secure elements, which come
/// before the SIM cards' in the list of readers.
const EMBEDDED: &str = "eSE";

fn io(e: impl std::fmt::Display) -> Refusal {
    Refusal::service(IO_ERROR, e.to_string())
}

fn refused(r: Refused) -> Refusal {
    match r {
        Refused::Malformed(why) => Refusal::illegal_argument(why),
        Refused::NotAllowed(why) => Refusal::security(why),
    }
}

/// A reply that says yes or no.
fn says(yes: bool) -> Parcel {
    let mut reply = status::went_through();
    reply.bool(yes);
    reply
}

/// A reply that hands over one of the objects served, or none.
fn hands(number: Option<u64>) -> Parcel {
    let mut reply = status::went_through();
    match number {
        Some(number) => reply.object(number),
        None => reply.no_object(),
    }
    reply
}

/// A reply of bytes, or of none where there are none to give.
fn gives(bytes: Option<&[u8]>) -> Parcel {
    let mut reply = status::went_through();
    reply.opt_blob(bytes);
    reply
}

/// The object a secure element's own service tells its state to.
struct Told {
    name: String,
    connected: Arc<AtomicBool>,
}

impl Object<Binder> for Told {
    fn interface(&self) -> &'static str {
        element::CALLBACK
    }

    fn version(&self) -> i32 {
        element::VERSION
    }

    fn hash(&self) -> &'static str {
        element::HASH
    }

    fn call(&self, _with: &Binder, code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
        if code != element::STATE_CHANGED {
            return Ok(None);
        }
        let (connected, why) = element::state_changed(read).map_err(|e| Refusal::illegal_argument(format!("{e:#}")))?;
        self.connected.store(connected, Ordering::SeqCst);
        let _ = skyward_sys::kmsg(&format!("omapi: {} is {}{}", self.name, if connected { "connected" } else { "not connected" }, if why.is_empty() { String::new() } else { format!(" ({why})") }));
        Ok(Some(status::went_through()))
    }
}

/// One secure element, as the service keeps it.
struct Terminal {
    /// Its reader's name, which is what its own service registers under.
    name: String,
    objects: Arc<Objects<Binder>>,
    connected: Arc<AtomicBool>,
    /// The number the object its own service tells its state to is served
    /// under.
    told: u64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Its own service has been asked to start.
    started: bool,
    /// The applet the secure element selects by itself is the one on the
    /// basic channel.
    own_applet_on_basic: bool,
    channels: HashMap<u8, Arc<Channel>>,
    sessions: Vec<Arc<Session>>,
}

impl Terminal {
    fn new(name: &str, objects: &Arc<Objects<Binder>>) -> Arc<Terminal> {
        let connected = Arc::new(AtomicBool::new(false));
        let told = objects.keep(Arc::new(Told { name: name.to_string(), connected: connected.clone() }));
        let state = Mutex::new(State { own_applet_on_basic: true, ..State::default() });
        Arc::new(Terminal { name: name.to_string(), objects: objects.clone(), connected, told, state })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("nothing panics while a secure element's state is held")
    }

    /// The secure element's own service.
    fn element<'a>(&self, binder: &'a Binder) -> Result<Element<'a>> {
        Element::find(binder, &self.name)?.with_context(|| format!("nothing has registered the secure element {}", self.name))
    }

    /// Have the secure element's own service start, where it has not been
    /// asked to yet, and wait for it to say the chip is connected. It says
    /// so as it is asked or soon after, on whichever thread takes calls.
    fn start(&self, binder: &Binder) -> Result<()> {
        {
            let mut state = self.state();
            if !state.started {
                self.element(binder)?.init(self.told)?;
                state.started = true;
            }
        }
        let until = Instant::now() + CONNECTS_IN;
        while !self.connected.load(Ordering::SeqCst) {
            anyhow::ensure!(Instant::now() < until, "the secure element {} is not connected", self.name);
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn present(&self, binder: &Binder) -> bool {
        self.element(binder).and_then(|e| e.present()).unwrap_or(false)
    }

    /// What the chip says of itself as it is powered up, where it is
    /// connected and says anything.
    fn atr(&self, binder: &Binder) -> Option<Vec<u8>> {
        if !self.connected.load(Ordering::SeqCst) {
            return None;
        }
        self.element(binder).and_then(|e| e.atr()).ok().filter(|atr| !atr.is_empty())
    }

    /// Send `command` and give the answer whole: what the secure element
    /// says is left of it is fetched and joined on, and a command it says
    /// expected the wrong length is sent again expecting the right one.
    fn transmit(&self, binder: &Binder, command: &[u8]) -> Result<Vec<u8>, Refusal> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(io("Secure Element is not connected"));
        }
        let element = self.element(binder).map_err(|e| io(format!("{e:#}")))?;
        let send = |command: &[u8]| element.transmit(command).map_err(|e| io(format!("{e:#}")));
        let mut command = command.to_vec();
        let mut answer = send(&command)?;
        loop {
            match apdu::next(&command, &answer).ok_or_else(|| io("the secure element answered less than how the command went"))? {
                Next::Done => return Ok(answer),
                Next::Again(again) => {
                    command = again;
                    answer = send(&command)?;
                }
                Next::More(fetch) => answer = apdu::joined(&answer, &send(&fetch)?),
            }
        }
    }

    /// Open a channel for `session` on the applet named `name`, or on the
    /// one the secure element selects by itself: the basic channel, or a
    /// logical one. None where the secure element has none to give.
    fn open(self: &Arc<Self>, binder: &Binder, session: &Arc<Session>, name: Option<&[u8]>, answers: u8, logical: bool) -> Result<Option<Arc<Channel>>, Refusal> {
        self.start(binder).map_err(|e| io(format!("{e:#}")))?;
        let mut state = self.state();
        if !logical && (state.channels.contains_key(&BASIC) || (name.is_none() && !state.own_applet_on_basic)) {
            return Ok(None);
        }
        let element = self.element(binder).map_err(|e| io(format!("{e:#}")))?;
        let asked = name.unwrap_or_default();
        let opened = match logical {
            true => element.open_logical(asked, answers),
            false => element.open_basic(asked, answers).map(|answer| (BASIC, answer)),
        };
        let (number, answer) = match opened {
            Ok(opened) => opened,
            Err(NotOpened::NoChannel) => return Ok(None),
            Err(NotOpened::NoSuchApplet) => return Err(Refusal::service(NO_SUCH_ELEMENT_ERROR, "the secure element has no such applet")),
            Err(NotOpened::Unsupported) => return Err(Refusal::unsupported("the secure element opens no channel this way")),
            Err(NotOpened::Failed(why)) => return Err(io(why)),
        };
        let channel = Arc::new(Channel {
            terminal: self.clone(),
            session: Arc::downgrade(session),
            number,
            name: name.map(<[u8]>::to_vec),
            answer: Mutex::new(answer),
            closed: AtomicBool::new(false),
        });
        if !logical && name.is_some() {
            state.own_applet_on_basic = false;
        }
        state.channels.insert(number, channel.clone());
        Ok(Some(channel))
    }

    /// Close the channel numbered `number` on the secure element. One that
    /// does not close there is closed here all the same, as nothing can be
    /// sent down it again.
    fn close(&self, binder: &Binder, number: u8) {
        let mut state = self.state();
        if self.connected.load(Ordering::SeqCst)
            && let Err(e) = self.element(binder).and_then(|e| e.close(number))
        {
            let _ = skyward_sys::kmsg(&format!("omapi: channel {number} of {} did not close: {e:#}", self.name));
        }
        state.channels.remove(&number);
    }

    /// A session of this secure element has closed: with the last of them,
    /// the basic channel is back on the applet the secure element selects
    /// by itself.
    fn closed(&self, session: &Session) {
        let mut state = self.state();
        state.sessions.retain(|s| !std::ptr::eq(s.as_ref(), session));
        if state.sessions.is_empty() {
            state.own_applet_on_basic = true;
        }
    }
}

/// The service, as the service directory hands it out.
struct Service {
    terminals: Vec<Arc<Terminal>>,
}

impl Object<Binder> for Service {
    fn interface(&self) -> &'static str {
        SERVICE
    }

    fn version(&self) -> i32 {
        VERSION
    }

    fn hash(&self) -> &'static str {
        HASH
    }

    fn call(&self, _with: &Binder, code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
        const GET_READERS: u32 = 1;
        const GET_READER: u32 = 2;
        const IS_NFC_EVENT_ALLOWED: u32 = 3;
        Ok(Some(match code {
            GET_READERS => {
                let names: Vec<&str> = self.terminals.iter().map(|t| t.name.as_str()).collect();
                let mut reply = status::went_through();
                reply.strings(&names);
                reply
            }
            GET_READER => {
                let name = read.string().map_err(|e| Refusal::illegal_argument(format!("{e:#}")))?;
                let terminal = self.terminals.iter().find(|t| t.name == name).ok_or_else(|| Refusal::illegal_argument(format!("Reader: {name} not supported")))?;
                hands(Some(terminal.objects.lend(Arc::new(Read { terminal: terminal.clone() }))))
            }
            // Which apps may hear of what the secure element does over NFC
            // is told from rules a recovery, which runs no app, does not
            // read: no answer, as where the secure element holds none.
            IS_NFC_EVENT_ALLOWED => {
                let mut reply = status::went_through();
                reply.i32(-1);
                reply
            }
            _ => return Ok(None),
        }))
    }
}

/// A reader: one secure element, as a program is handed it.
struct Read {
    terminal: Arc<Terminal>,
}

impl Object<Binder> for Read {
    fn interface(&self) -> &'static str {
        READER
    }

    fn version(&self) -> i32 {
        VERSION
    }

    fn hash(&self) -> &'static str {
        HASH
    }

    fn call(&self, with: &Binder, code: u32, _read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
        const IS_SECURE_ELEMENT_PRESENT: u32 = 1;
        const OPEN_SESSION: u32 = 2;
        const CLOSE_SESSIONS: u32 = 3;
        const RESET: u32 = 4;
        let terminal = &self.terminal;
        Ok(Some(match code {
            IS_SECURE_ELEMENT_PRESENT => says(terminal.present(with)),
            OPEN_SESSION => {
                terminal.start(with).map_err(|e| io(format!("{e:#}")))?;
                if !terminal.present(with) {
                    return Err(io("Secure Element is not present."));
                }
                let session = Arc::new(Session { terminal: terminal.clone(), atr: terminal.atr(with), closed: AtomicBool::new(false), channels: Mutex::new(Vec::new()) });
                terminal.state().sessions.push(session.clone());
                hands(Some(terminal.objects.lend(session)))
            }
            CLOSE_SESSIONS => {
                let sessions = terminal.state().sessions.clone();
                for session in sessions {
                    session.close(with);
                }
                status::went_through()
            }
            RESET => says(terminal.element(with).and_then(|e| e.reset()).is_ok()),
            _ => return Ok(None),
        }))
    }
}

/// A session: what a program opens channels through.
struct Session {
    terminal: Arc<Terminal>,
    /// What the chip said of itself when the session opened.
    atr: Option<Vec<u8>>,
    closed: AtomicBool,
    channels: Mutex<Vec<Arc<Channel>>>,
}

impl Session {
    fn channels(&self) -> std::sync::MutexGuard<'_, Vec<Arc<Channel>>> {
        self.channels.lock().expect("nothing panics while a session's channels are held")
    }

    fn close_channels(&self, binder: &Binder) {
        let open = self.channels().clone();
        for channel in open {
            channel.close(binder);
        }
    }

    fn close(&self, binder: &Binder) {
        self.close_channels(binder);
        self.terminal.closed(self);
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Open a channel as a program asks for one: on the applet it names,
    /// saying what it wants back of it, and handing over the object it
    /// would be told through of the channel closing.
    fn open(self: &Arc<Self>, binder: &Binder, read: &mut Reader, logical: bool) -> Result<Parcel, Refusal> {
        let taken = |e: anyhow::Error| Refusal::illegal_argument(format!("{e:#}"));
        let name = read.opt_blob().map_err(taken)?.unwrap_or_default();
        let answers = read.i32().map_err(taken)? as u8;
        let listener = read.object().map_err(taken)?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(Refusal::illegal_state("Session is closed"));
        }
        if listener.is_none() {
            return Err(Refusal::null_pointer("listener must not be null"));
        }
        if !apdu::answers_taken(answers) {
            return Err(Refusal::unsupported(format!("p2 not supported: {answers:02x}")));
        }
        let name = apdu::name(&name).map_err(refused)?;
        let Some(channel) = self.terminal.open(binder, self, name, answers, logical)? else { return Ok(hands(None)) };
        self.channels().push(channel.clone());
        Ok(hands(Some(self.terminal.objects.lend(channel))))
    }
}

impl Object<Binder> for Session {
    fn interface(&self) -> &'static str {
        SESSION
    }

    fn version(&self) -> i32 {
        VERSION
    }

    fn hash(&self) -> &'static str {
        HASH
    }

    fn call(&self, with: &Binder, code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
        const GET_ATR: u32 = 1;
        const CLOSE: u32 = 2;
        const CLOSE_CHANNELS: u32 = 3;
        const IS_CLOSED: u32 = 4;
        const OPEN_BASIC_CHANNEL: u32 = 5;
        const OPEN_LOGICAL_CHANNEL: u32 = 6;
        // A session is found again among its secure element's, to hand a
        // channel the session it belongs to.
        let whole = || self.terminal.state().sessions.iter().find(|s| std::ptr::eq(s.as_ref(), self)).cloned().ok_or_else(|| Refusal::illegal_state("Session is closed"));
        Ok(Some(match code {
            GET_ATR => gives(self.atr.as_deref()),
            CLOSE => {
                self.close(with);
                status::went_through()
            }
            CLOSE_CHANNELS => {
                self.close_channels(with);
                status::went_through()
            }
            IS_CLOSED => says(self.closed.load(Ordering::SeqCst)),
            OPEN_BASIC_CHANNEL => whole()?.open(with, read, false)?,
            OPEN_LOGICAL_CHANNEL => whole()?.open(with, read, true)?,
            _ => return Ok(None),
        }))
    }

    fn let_go(&self, with: &Binder) {
        if !self.closed.load(Ordering::SeqCst) {
            self.close(with);
        }
    }
}

/// A channel: what a program sends an applet commands down.
struct Channel {
    terminal: Arc<Terminal>,
    session: Weak<Session>,
    number: u8,
    /// The name the applet was selected by; none where the channel is on
    /// the applet the secure element selects by itself.
    name: Option<Vec<u8>>,
    /// What the applet answered to being selected.
    answer: Mutex<Vec<u8>>,
    closed: AtomicBool,
}

impl Channel {
    fn answer(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        self.answer.lock().expect("nothing panics while an applet's answer is held")
    }

    fn close(&self, binder: &Binder) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // The basic channel is never closed on the secure element, so the
        // applet it selects by itself is put back on it; one that will not
        // be selected leaves the channel as it is.
        if self.number == BASIC
            && let Some(select) = apdu::select(&[], BASIC, false)
        {
            let _ = self.terminal.transmit(binder, &select);
        }
        self.terminal.close(binder, self.number);
        if let Some(session) = self.session.upgrade() {
            session.channels().retain(|c| !std::ptr::eq(c.as_ref(), self));
        }
    }

    fn open(&self) -> Result<(), Refusal> {
        match self.closed.load(Ordering::SeqCst) {
            true => Err(Refusal::illegal_state("Channel is closed")),
            false => Ok(()),
        }
    }

    fn transmit(&self, binder: &Binder, mut command: Vec<u8>) -> Result<Vec<u8>, Refusal> {
        self.open()?;
        apdu::check(&command).map_err(refused)?;
        command[0] = apdu::on_channel(command[0], self.number).ok_or_else(|| Refusal::illegal_argument("Channel number must be within [0..19]"))?;
        self.terminal.transmit(binder, &command)
    }

    /// Select the applet after the one selected now that goes by the same
    /// name; says whether there was one.
    fn select_next(&self, binder: &Binder) -> Result<bool, Refusal> {
        self.open()?;
        let name = self.name.as_deref().filter(|n| !n.is_empty()).ok_or_else(|| Refusal::unsupported("No aid given"))?;
        let select = apdu::select(name, self.number, true).ok_or_else(|| Refusal::illegal_argument("Channel number must be within [0..19]"))?;
        let answer = self.terminal.transmit(binder, &select)?;
        match apdu::selected(&answer) {
            Selected::Another => {
                *self.answer() = answer;
                Ok(true)
            }
            Selected::NoOther => Ok(false),
            Selected::Neither => Err(Refusal::unsupported("Unsupported operation")),
        }
    }
}

impl Object<Binder> for Channel {
    fn interface(&self) -> &'static str {
        CHANNEL
    }

    fn version(&self) -> i32 {
        VERSION
    }

    fn hash(&self) -> &'static str {
        HASH
    }

    fn call(&self, with: &Binder, code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
        const CLOSE: u32 = 1;
        const IS_CLOSED: u32 = 2;
        const IS_BASIC_CHANNEL: u32 = 3;
        const GET_SELECT_RESPONSE: u32 = 4;
        const TRANSMIT: u32 = 5;
        const SELECT_NEXT: u32 = 6;
        Ok(Some(match code {
            CLOSE => {
                self.close(with);
                status::went_through()
            }
            IS_CLOSED => says(self.closed.load(Ordering::SeqCst)),
            IS_BASIC_CHANNEL => says(self.number == BASIC),
            GET_SELECT_RESPONSE => match &self.name {
                Some(_) => gives(Some(&self.answer())),
                None => gives(None),
            },
            TRANSMIT => {
                let command = read.opt_blob().map_err(|e| Refusal::illegal_argument(format!("{e:#}")))?.ok_or_else(|| Refusal::null_pointer("Command must not be null"))?;
                gives(Some(&self.transmit(with, command)?))
            }
            SELECT_NEXT => says(self.select_next(with)?),
            _ => return Ok(None),
        }))
    }

    fn let_go(&self, with: &Binder) {
        self.close(with);
    }
}

/// How many threads take the calls. A secure element's own service may
/// tell its state from a thread of its own while a call to it is still
/// awaited on the thread that made it, so one is never enough.
const THREADS: usize = 2;

/// Serve the secure element service for as long as the program runs, and
/// register it with the service directory: one reader for each secure
/// element whose own service is registered now, the embedded ones first.
/// Gives the readers' names.
pub fn start() -> Result<Vec<String>> {
    let objects: Arc<Objects<Binder>> = Arc::new(Objects::default());
    let binder = Arc::new(Binder::serving(objects.clone()).context("open the driver to serve the secure element service through")?);
    for thread in 0..THREADS {
        let taking = binder.clone();
        std::thread::Builder::new()
            .name(format!("omapi-{thread}"))
            .spawn(move || {
                let Err(e) = taking.serve();
                let _ = skyward_sys::kmsg(&format!("omapi: a thread stopped taking calls: {e:#}"));
            })
            .context("start a thread to take the secure element service's calls")?;
    }
    let prefix = format!("{}/", element::INTERFACE);
    let mut names: Vec<String> = skyward_binder::service::names(&binder)?.into_iter().filter_map(|name| name.strip_prefix(&prefix).map(str::to_string)).collect();
    names.sort_by_key(|name| (!name.starts_with(EMBEDDED), name.clone()));
    let terminals: Vec<Arc<Terminal>> = names.iter().map(|name| Terminal::new(name, &objects)).collect();
    for terminal in &terminals {
        // A secure element that does not connect now is asked again when
        // a program first uses it.
        if let Err(e) = terminal.start(&binder) {
            let _ = skyward_sys::kmsg(&format!("omapi: {e:#}"));
        }
    }
    let service = objects.keep(Arc::new(Service { terminals }));
    skyward_binder::service::add(&binder, crate::SERVICE, service)?;
    Ok(names)
}
