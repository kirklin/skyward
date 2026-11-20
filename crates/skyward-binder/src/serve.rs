//! The objects this program serves to others over the driver: each under a
//! number the driver carries a call to it by, answering the calls every
//! object answers whatever its interface, and handing the rest to the
//! object itself.

use crate::parcel::{Parcel, Reader};
use crate::status::{self, Refusal};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The calls libbinder answers for every object before any interface sees
/// them: whether the object is there, which interface it serves, which
/// process serves it, and what it carries besides its interface.
const PING: u32 = u32::from_be_bytes(*b"_PNG");
const INTERFACE: u32 = u32::from_be_bytes(*b"_NTF");
const DEBUG_PID: u32 = u32::from_be_bytes(*b"_PID");
const EXTENSION: u32 = u32::from_be_bytes(*b"_EXT");

/// The calls every service of the newer kind answers about its interface,
/// numbered from the top of the range its own methods are numbered from
/// the bottom of: which version of it is served, and the sum of that
/// version's definition.
const INTERFACE_VERSION: u32 = 0x00ff_ffff;
const INTERFACE_HASH: u32 = 0x00ff_fffe;

/// The first number an object is served under; none is served under zero,
/// which is how an object left out is written.
const FIRST: u64 = 1;

/// An object served: what it is to the programs that call it. `C` is what
/// an object needs of the program while it answers, which on the phone is
/// the driver itself, to call other programs through.
pub trait Object<C>: Send + Sync {
    /// The interface it serves, by its full name.
    fn interface(&self) -> &'static str;

    /// The version of the interface it serves, and the sum of that
    /// version's definition, in hex.
    fn version(&self) -> i32;
    fn hash(&self) -> &'static str;

    /// Answer the method numbered `code`, reading what it takes from
    /// `read`: the reply, opened with [`status::went_through`]; none where
    /// the interface has no such method; or why it is turned down.
    fn call(&self, with: &C, code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal>;

    /// No other program holds the object any more, so none can call it
    /// again: one that holds something open for its callers lets it go.
    fn let_go(&self, _with: &C) {}
}

/// What a call is answered with.
pub enum Answer {
    Reply(Parcel),
    /// There is no such object, or it has no such method, or the call is
    /// for another interface than the object's.
    Unknown,
}

struct Served<C> {
    object: Arc<dyn Object<C>>,
    /// It is served for as long as the program runs, whether or not any
    /// other program holds it.
    kept: bool,
}

/// The objects served, each under its number.
pub struct Objects<C> {
    served: Mutex<(u64, HashMap<u64, Served<C>>)>,
}

impl<C> Default for Objects<C> {
    fn default() -> Self {
        Objects { served: Mutex::new((FIRST, HashMap::new())) }
    }
}

impl<C> Objects<C> {
    /// Serve `object` for as long as the program runs, and give its
    /// number.
    pub fn keep(&self, object: Arc<dyn Object<C>>) -> u64 {
        self.add(object, true)
    }

    /// Serve `object` until no other program holds it any more, and give
    /// its number, which is written into the reply that hands it over.
    pub fn lend(&self, object: Arc<dyn Object<C>>) -> u64 {
        self.add(object, false)
    }

    fn add(&self, object: Arc<dyn Object<C>>, kept: bool) -> u64 {
        let mut served = self.served.lock().expect("nothing panics while the objects are held");
        let number = served.0;
        served.0 += 1;
        served.1.insert(number, Served { object, kept });
        number
    }

    /// The driver says no other program holds the object numbered `number`
    /// any more: one lent is told so and served no longer.
    pub fn let_go(&self, with: &C, number: u64) {
        let lent = {
            let mut served = self.served.lock().expect("nothing panics while the objects are held");
            match served.1.get(&number) {
                Some(s) if !s.kept => served.1.remove(&number),
                _ => None,
            }
        };
        if let Some(lent) = lent {
            lent.object.let_go(with);
        }
    }

    /// Whether an object is served under `number`.
    pub fn serves(&self, number: u64) -> bool {
        self.served.lock().expect("nothing panics while the objects are held").1.contains_key(&number)
    }

    /// Answer the call of the method numbered `code` on the object
    /// numbered `number`, which carries `data`.
    pub fn answer(&self, with: &C, number: u64, code: u32, data: &[u8]) -> Answer {
        // The object answers outside the table's lock: it may serve
        // another object while it does.
        let found = self.served.lock().expect("nothing panics while the objects are held").1.get(&number).map(|s| s.object.clone());
        let Some(object) = found else { return Answer::Unknown };
        let mut read = Reader::new(data);
        match code {
            PING => Answer::Reply(Parcel::default()),
            INTERFACE => {
                let mut reply = Parcel::default();
                reply.string(object.interface());
                Answer::Reply(reply)
            }
            DEBUG_PID => {
                let mut reply = Parcel::default();
                reply.i32(std::process::id() as i32);
                Answer::Reply(reply)
            }
            EXTENSION => {
                let mut reply = Parcel::default();
                reply.no_object();
                Answer::Reply(reply)
            }
            code => {
                if read.call().ok().as_deref() != Some(object.interface()) {
                    return Answer::Unknown;
                }
                let answered = match code {
                    INTERFACE_VERSION => {
                        let mut reply = status::went_through();
                        reply.i32(object.version());
                        Ok(Some(reply))
                    }
                    INTERFACE_HASH => {
                        let mut reply = status::went_through();
                        reply.string(object.hash());
                        Ok(Some(reply))
                    }
                    code => object.call(with, code, &mut read),
                };
                match answered {
                    Ok(Some(reply)) => Answer::Reply(reply),
                    Ok(None) => Answer::Unknown,
                    Err(refusal) => Answer::Reply(refusal.reply()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An object that adds one to the number it is given, and turns down
    /// the number zero. It counts the times it is let go.
    #[derive(Default)]
    struct Adder(std::sync::atomic::AtomicU32);

    impl Object<()> for Adder {
        fn let_go(&self, _with: &()) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        fn interface(&self) -> &'static str {
            "a.b.IAdder"
        }

        fn version(&self) -> i32 {
            3
        }

        fn hash(&self) -> &'static str {
            "0123abcd"
        }

        fn call(&self, _with: &(), code: u32, read: &mut Reader) -> Result<Option<Parcel>, Refusal> {
            if code != 1 {
                return Ok(None);
            }
            let n = read.i32().map_err(|e| Refusal::illegal_argument(e.to_string()))?;
            if n == 0 {
                return Err(Refusal::service(5, "zero"));
            }
            let mut reply = status::went_through();
            reply.i32(n + 1);
            Ok(Some(reply))
        }
    }

    fn call(interface: &str, n: i32) -> Parcel {
        let mut call = Parcel::call(interface);
        call.i32(n);
        call
    }

    fn reply(answer: Answer) -> Parcel {
        match answer {
            Answer::Reply(reply) => reply,
            Answer::Unknown => panic!("the call was not taken"),
        }
    }

    #[test]
    fn a_call_reaches_the_object_it_is_numbered_for() {
        let objects = Objects::<()>::default();
        let number = objects.keep(Arc::new(Adder::default()));
        let answered = reply(objects.answer(&(), number, 1, call("a.b.IAdder", 41).bytes()));
        let mut read = Reader::new(answered.bytes());
        status::ok(&mut read).unwrap();
        assert_eq!(read.i32().unwrap(), 42);
        let refused = reply(objects.answer(&(), number, 1, call("a.b.IAdder", 0).bytes()));
        let e = status::ok(&mut Reader::new(refused.bytes())).unwrap_err();
        assert_eq!(status::refused_with(&e), Some(5), "its own refusal goes back in the reply");
        assert!(matches!(objects.answer(&(), number, 2, call("a.b.IAdder", 1).bytes()), Answer::Unknown), "no such method");
        assert!(matches!(objects.answer(&(), number, 1, call("a.b.IOther", 1).bytes()), Answer::Unknown), "a call for another interface");
        assert!(matches!(objects.answer(&(), number + 1, 1, call("a.b.IAdder", 1).bytes()), Answer::Unknown), "no such object");
    }

    #[test]
    fn every_object_says_what_it_is_whatever_its_interface() {
        let objects = Objects::<()>::default();
        let number = objects.keep(Arc::new(Adder::default()));
        assert!(reply(objects.answer(&(), number, PING, &[])).bytes().is_empty(), "it is there");
        let named = reply(objects.answer(&(), number, INTERFACE, &[]));
        assert_eq!(Reader::new(named.bytes()).string().unwrap(), "a.b.IAdder");
        let pid = reply(objects.answer(&(), number, DEBUG_PID, &[]));
        assert_eq!(Reader::new(pid.bytes()).i32().unwrap(), std::process::id() as i32);
        let version = reply(objects.answer(&(), number, INTERFACE_VERSION, Parcel::call("a.b.IAdder").bytes()));
        let mut read = Reader::new(version.bytes());
        status::ok(&mut read).unwrap();
        assert_eq!(read.i32().unwrap(), 3);
        let hash = reply(objects.answer(&(), number, INTERFACE_HASH, Parcel::call("a.b.IAdder").bytes()));
        let mut read = Reader::new(hash.bytes());
        status::ok(&mut read).unwrap();
        assert_eq!(read.string().unwrap(), "0123abcd");
    }

    #[test]
    fn an_object_lent_is_served_until_it_is_let_go_and_one_kept_for_good() {
        let objects = Objects::<()>::default();
        let (always, once) = (Arc::new(Adder::default()), Arc::new(Adder::default()));
        let (kept, lent) = (objects.keep(always.clone()), objects.lend(once.clone()));
        assert!(kept != 0 && lent != 0 && kept != lent, "each under a number of its own, none under zero");
        objects.let_go(&(), kept);
        objects.let_go(&(), lent);
        objects.let_go(&(), lent);
        assert!(objects.serves(kept));
        assert!(!objects.serves(lent));
        assert!(matches!(objects.answer(&(), lent, PING, &[]), Answer::Unknown));
        let told = |adder: &Adder| adder.0.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!((told(&always), told(&once)), (0, 1), "the one lent is told once that it is let go");
    }
}
