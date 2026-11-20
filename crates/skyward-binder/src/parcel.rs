use anyhow::{bail, Context, Result};

/// The mark libbinder writes into every call header. Both ends have to agree
/// on it: it tells apart the copies of libbinder a phone carries, and the one
/// the recovery's own programs are built from is the recovery copy.
const HEADER: i32 = i32::from_be_bytes(*b"RECO");

/// Strict mode gathers violations rather than acting on them; a caller with
/// no policy of its own still sets that bit.
const STRICT_MODE_PENALTY_GATHER: i32 = i32::MIN;

/// The work source a caller passes on when it has none to pass on.
const UNSET_WORK_SOURCE: i32 = -1;

/// An object in the sending process, as a transaction carries it to the
/// process it is sent to, and one of the sending process's own, as it
/// writes it.
const BINDER_TYPE_HANDLE: u32 = u32::from_be_bytes([b's', b'h', b'*', 0x85]);
const BINDER_TYPE_BINDER: u32 = u32::from_be_bytes([b's', b'b', b'*', 0x85]);

/// What libbinder says of every object of its own it hands over: that
/// calls on it may carry file descriptors, and that they run at the
/// ordinary scheduling policy's lowest priority unless the caller's is
/// higher.
const OBJECT_FLAGS: u32 = 0x100 | 19;

/// How far an object's interface is meant to travel, written after the
/// object: one the phone's own partitions and the system both keep to,
/// which is the only kind a service the phone describes may be, and what
/// is written after an object left out.
const STABILITY_VINTF: i32 = 0b11_1111;
const STABILITY_UNDECLARED: i32 = 0;

/// The room an object takes among the values, before how far it travels.
const OBJECT_BYTES: usize = 24;

/// What a call carries. The values themselves are most of it; a call that
/// hands over an address says where among the values that address sits, and
/// how much room the far side needs for everything the addresses point at.
pub trait Payload {
    fn values(&self) -> &[u8];

    fn addresses(&self) -> &[u64] {
        &[]
    }

    fn pointed(&self) -> u64 {
        0
    }
}

/// The bytes of a call or its reply, in the layout libbinder reads: values
/// little-endian, each one padded out to four bytes.
#[derive(Default)]
pub struct Parcel {
    data: Vec<u8>,
    /// Where among the values each object sits, which the driver is told
    /// so that it can turn an object into what the other side holds it by.
    objects: Vec<u64>,
}

impl Payload for Parcel {
    fn values(&self) -> &[u8] {
        self.bytes()
    }

    fn addresses(&self) -> &[u64] {
        &self.objects
    }
}

impl Parcel {
    /// A call to `interface`, opened with the header libbinder reads before
    /// it looks at anything the method itself takes.
    pub fn call(interface: &str) -> Parcel {
        let mut p = Parcel::default();
        p.i32(STRICT_MODE_PENALTY_GATHER);
        p.i32(UNSET_WORK_SOURCE);
        p.i32(HEADER);
        p.string(interface);
        p
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.i32(v as i32);
    }

    /// A byte array: its length, its bytes, then padding to the next four.
    pub fn blob(&mut self, v: &[u8]) {
        self.i32(v.len() as i32);
        self.data.extend_from_slice(v);
        self.pad();
    }

    /// Where a byte array may be left out, `None` is written as a length of
    /// -1 rather than as an empty one.
    pub fn opt_blob(&mut self, v: Option<&[u8]>) {
        match v {
            Some(b) => self.blob(b),
            None => self.i32(-1),
        }
    }

    /// Where a value may be left out, what precedes it says whether it is
    /// there; nothing follows when it is not.
    pub fn none(&mut self) {
        self.i32(0);
    }

    /// One of this program's own objects, by the number it serves it
    /// under: the driver gives the other side something to call it by.
    pub fn object(&mut self, number: u64) {
        self.objects.push(self.data.len() as u64);
        self.data.extend_from_slice(&BINDER_TYPE_BINDER.to_le_bytes());
        self.data.extend_from_slice(&OBJECT_FLAGS.to_le_bytes());
        self.data.extend_from_slice(&number.to_le_bytes());
        self.data.extend_from_slice(&number.to_le_bytes());
        self.i32(STABILITY_VINTF);
    }

    /// An object left out where one may be: libbinder writes it as one of
    /// its own that is nowhere, which the driver passes on as it is.
    pub fn no_object(&mut self) {
        self.data.extend_from_slice(&BINDER_TYPE_BINDER.to_le_bytes());
        self.data.extend_from_slice(&[0; OBJECT_BYTES - 4]);
        self.i32(STABILITY_UNDECLARED);
    }

    /// Several texts: how many, then each.
    pub fn strings(&mut self, all: &[&str]) {
        self.i32(all.len() as i32);
        for s in all {
            self.string(s);
        }
    }

    /// Text, as the count of UTF-16 units, those units, a terminator, and
    /// padding to the next four.
    pub fn string(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.i32(units.len() as i32);
        for u in &units {
            self.data.extend_from_slice(&u.to_le_bytes());
        }
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.pad();
    }

    /// A value of its own takes no room past its length; what follows starts
    /// at the next multiple of four.
    fn pad(&mut self) {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
    }

    /// Where the next value will go, for a length written once the value it
    /// counts has been written.
    pub fn here(&self) -> usize {
        self.data.len()
    }

    /// Fill in a length left for later at `at`.
    pub fn fill(&mut self, at: usize, v: i32) {
        self.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// Reads back what the other side wrote.
pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    pub fn here(&self) -> usize {
        self.at
    }

    pub fn go(&mut self, at: usize) -> Result<()> {
        if at > self.data.len() {
            bail!("the reply ends at {} and holds no position {at}", self.data.len());
        }
        self.at = at;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at + n;
        if end > self.data.len() {
            bail!("the reply holds {} bytes, {n} more were read at {}", self.data.len(), self.at);
        }
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.i32()? != 0)
    }

    pub fn blob(&mut self) -> Result<Vec<u8>> {
        match self.opt_blob()? {
            Some(out) => Ok(out),
            None => bail!("a byte array was left out where one was expected"),
        }
    }

    /// A byte array that may be left out, which is written as a length of
    /// -1.
    pub fn opt_blob(&mut self) -> Result<Option<Vec<u8>>> {
        let len = self.i32()?;
        if len == -1 {
            return Ok(None);
        }
        if len < 0 {
            bail!("a byte array of length {len} was read");
        }
        let out = self.take(len as usize)?.to_vec();
        self.skip_padding();
        Ok(Some(out))
    }

    /// The header a call opens with, which libbinder writes before what the
    /// method takes, and the interface it names. The mark in it tells apart
    /// the copies of libbinder a phone carries, and a caller may be built
    /// from any of them, so it is read past.
    pub fn call(&mut self) -> Result<String> {
        self.i32().context("read the caller's strict mode")?;
        self.i32().context("read the caller's work source")?;
        self.i32().context("read the mark of the caller's libbinder")?;
        self.string().context("read the interface called")
    }

    pub fn string(&mut self) -> Result<String> {
        let units = self.i32()?;
        if units < 0 {
            bail!("text of length {units} was read where some was expected");
        }
        let raw = self.take((units as usize + 1) * 2)?;
        self.skip_padding();
        let utf16: Vec<u16> = raw[..units as usize * 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        Ok(String::from_utf16_lossy(&utf16))
    }

    /// The handle of an object in the sending process. The other kinds of
    /// object a transaction can carry take the same room, so a reply holding
    /// one of those can still be read past. What follows the object says how
    /// far its interface is meant to travel, which this side does not act on.
    pub fn object(&mut self) -> Result<Option<u32>> {
        let raw = self.take(24)?;
        let kind = u32::from_le_bytes(raw[0..4].try_into().expect("four bytes"));
        let handle = u32::from_le_bytes(raw[8..12].try_into().expect("four bytes"));
        self.i32().context("read how far the object's interface travels")?;
        Ok((kind == BINDER_TYPE_HANDLE).then_some(handle))
    }

    /// Step into a structure the other side wrote whole: it says first
    /// whether the structure is there at all, then how long it is. The
    /// answer is where it ends, so fields this side does not know about can
    /// be stepped over.
    pub fn structure(&mut self) -> Result<usize> {
        if self.i32().context("read whether the structure is there")? == 0 {
            bail!("the structure was left out where one was expected");
        }
        let at = self.here();
        let size = self.i32().context("read how long the structure is")?;
        if size < 4 {
            bail!("a structure of {size} bytes cannot hold even its own length");
        }
        Ok(at + size as usize)
    }

    fn skip_padding(&mut self) {
        while !self.at.is_multiple_of(4) && self.at < self.data.len() {
            self.at += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_opens_with_the_header_and_the_interface_name() {
        let p = Parcel::call("a.b.IC");
        let mut r = Reader::new(p.bytes());
        assert_eq!(r.i32().unwrap(), i32::MIN, "strict mode gathers");
        assert_eq!(r.i32().unwrap(), -1, "no work source");
        assert_eq!(r.i32().unwrap(), 0x5245_434f, "the recovery's mark, RECO");
        assert_eq!(r.string().unwrap(), "a.b.IC");
        assert_eq!(r.here(), p.bytes().len(), "the header is all of it");
    }

    #[test]
    fn the_header_of_a_call_gives_its_interface_whichever_libbinder_wrote_it() {
        let mut ours = Parcel::call("a.b.IC");
        ours.i32(7);
        let mut read = Reader::new(ours.bytes());
        assert_eq!(read.call().unwrap(), "a.b.IC");
        assert_eq!(read.i32().unwrap(), 7, "what the method takes comes next");
        let mut vendor = Parcel::default();
        vendor.i32(i32::MIN);
        vendor.i32(-1);
        vendor.i32(i32::from_be_bytes(*b"VNDR"));
        vendor.string("a.b.IC");
        assert_eq!(Reader::new(vendor.bytes()).call().unwrap(), "a.b.IC", "the vendor partition's copy");
        assert!(Reader::new(&[0; 8]).call().is_err(), "a call cut short");
    }

    #[test]
    fn an_object_of_ones_own_is_written_where_the_driver_is_told_it_is() {
        let mut p = Parcel::default();
        p.i32(0);
        p.object(0x1234);
        p.no_object();
        p.strings(&["eSE1", "SIM1"]);
        assert_eq!(p.addresses(), [4], "one object, after the first value; one left out is none");
        let mut r = Reader::new(p.bytes());
        r.i32().unwrap();
        let raw = &p.bytes()[4..32];
        assert_eq!(&raw[0..4], b"\x85*bs", "one of the sender's own");
        assert_eq!(u32::from_le_bytes(raw[4..8].try_into().unwrap()), 0x113, "takes file descriptors, at the lowest ordinary priority");
        assert_eq!(u64::from_le_bytes(raw[8..16].try_into().unwrap()), 0x1234);
        assert_eq!(u64::from_le_bytes(raw[16..24].try_into().unwrap()), 0x1234);
        assert_eq!(i32::from_le_bytes(raw[24..28].try_into().unwrap()), 63, "an interface the phone and the system both keep to");
        assert_eq!(r.object().unwrap(), None, "the sender's own is no handle to the reader");
        assert_eq!(r.object().unwrap(), None, "and neither is one left out");
        assert_eq!(r.i32().unwrap(), 2);
        assert_eq!((r.string().unwrap(), r.string().unwrap()), ("eSE1".to_string(), "SIM1".to_string()));
        assert_eq!(r.here(), p.bytes().len());
    }

    #[test]
    fn a_byte_array_left_out_is_told_from_an_empty_one() {
        let mut p = Parcel::default();
        p.opt_blob(None);
        p.blob(&[]);
        p.blob(&[9]);
        let mut r = Reader::new(p.bytes());
        assert_eq!(r.opt_blob().unwrap(), None);
        assert_eq!(r.opt_blob().unwrap(), Some(Vec::new()));
        assert_eq!(r.opt_blob().unwrap(), Some(vec![9]));
        let mut left_out = Parcel::default();
        left_out.opt_blob(None);
        assert!(Reader::new(left_out.bytes()).blob().is_err(), "where one has to be there");
    }

    #[test]
    fn values_round_trip_through_their_padding() {
        let mut p = Parcel::default();
        p.i32(-7);
        p.i64(1 << 40);
        p.bool(true);
        p.blob(&[1, 2, 3]);
        p.string("路");
        p.opt_blob(None);
        p.blob(&[]);
        assert_eq!(p.bytes().len() % 4, 0, "every value ends on a four-byte boundary");
        let mut r = Reader::new(p.bytes());
        assert_eq!(r.i32().unwrap(), -7);
        assert_eq!(r.i64().unwrap(), 1 << 40);
        assert!(r.bool().unwrap());
        assert_eq!(r.blob().unwrap(), vec![1, 2, 3]);
        assert_eq!(r.string().unwrap(), "路");
        assert_eq!(r.i32().unwrap(), -1, "a byte array left out");
        assert_eq!(r.blob().unwrap(), Vec::<u8>::new());
        assert_eq!(r.here(), p.bytes().len());
    }

    #[test]
    fn a_length_can_be_filled_in_once_what_it_counts_is_written() {
        let mut p = Parcel::default();
        let at = p.here();
        p.i32(0);
        p.i64(9);
        let size = (p.here() - at) as i32;
        p.fill(at, size);
        let mut r = Reader::new(p.bytes());
        assert_eq!(r.i32().unwrap(), 12);
        assert_eq!(r.i64().unwrap(), 9);
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let p = Parcel::default();
        assert!(Reader::new(p.bytes()).i32().is_err());
        let mut short = Parcel::default();
        short.i32(1000);
        assert!(Reader::new(short.bytes()).blob().is_err());
    }

    /// An object as the driver lays it out: its kind, its flags, the handle
    /// or address, its cookie, and how far its interface travels.
    fn object(kind: &[u8; 4], handle: u64) -> Parcel {
        let mut p = Parcel::default();
        p.i32(i32::from_be_bytes(*kind));
        p.i32(0);
        p.i64(handle as i64);
        p.i64(0);
        p.i32(63);
        p
    }

    #[test]
    fn an_object_gives_up_its_handle_only_when_it_is_a_handle() {
        let held = object(b"sh*\x85", 42);
        let mut r = Reader::new(held.bytes());
        assert_eq!(r.object().unwrap(), Some(42));
        assert_eq!(r.here(), held.bytes().len(), "an object is read whole");
        let own = object(b"sb*\x85", 42);
        assert_eq!(Reader::new(own.bytes()).object().unwrap(), None);
    }

    #[test]
    fn a_structure_says_where_it_ends() {
        let mut p = Parcel::default();
        p.i32(1);
        let at = p.here();
        p.i32(0);
        p.i32(7);
        p.i32(8);
        let size = (p.here() - at) as i32;
        p.fill(at, size);
        p.i32(99);
        let mut r = Reader::new(p.bytes());
        let end = r.structure().unwrap();
        assert_eq!(r.i32().unwrap(), 7);
        r.go(end).unwrap();
        assert_eq!(r.i32().unwrap(), 99, "what follows the structure comes next");
    }

    #[test]
    fn a_structure_left_out_is_an_error() {
        let mut p = Parcel::default();
        p.i32(0);
        assert!(Reader::new(p.bytes()).structure().is_err());
    }
}
