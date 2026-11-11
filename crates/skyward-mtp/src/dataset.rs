//! MTP's containers and the datasets in them: numbers little-endian, arrays
//! led by their length in 32 bits, and strings as UTF-16 led by one byte
//! that counts their code units with the terminating zero.

use crate::codes::container;

/// The header every container starts with.
pub const HEADER: usize = 12;

/// The most code units a string holds, its terminating zero among them.
const STRING_MAX: usize = 255;

/// What a container's header says: how long the container is with the
/// header, what it holds, its operation, response or event, and the
/// transaction it is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub length: u32,
    pub kind: u16,
    pub code: u16,
    pub transaction: u32,
}

impl Header {
    pub fn read(bytes: &[u8]) -> Option<Header> {
        let mut r = Reader::new(bytes.get(..HEADER)?);
        Some(Header { length: r.u32()?, kind: r.u16()?, code: r.u16()?, transaction: r.u32()? })
    }

    pub fn bytes(&self) -> [u8; HEADER] {
        let mut w = Writer::new();
        w.u32(self.length).u16(self.kind).u16(self.code).u32(self.transaction);
        w.into_bytes().try_into().expect("a header is twelve bytes")
    }
}

/// An operation the computer asks for, with the parameters it sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub code: u16,
    pub transaction: u32,
    pub params: Vec<u32>,
}

impl Command {
    /// A command container as it came: its header counts what came, and
    /// whole parameters follow it, five at most.
    pub fn read(bytes: &[u8]) -> Option<Command> {
        let header = Header::read(bytes)?;
        let rest = bytes.len() - HEADER;
        if header.kind != container::COMMAND || header.length as usize != bytes.len() || !rest.is_multiple_of(4) || rest > 5 * 4 {
            return None;
        }
        let mut r = Reader::new(&bytes[HEADER..]);
        let params = (0..rest / 4).map(|_| r.u32()).collect::<Option<_>>()?;
        Some(Command { code: header.code, transaction: header.transaction, params })
    }

    /// The first `N` parameters, which the operation needs; `None` where
    /// the computer sent fewer.
    pub fn take<const N: usize>(&self) -> Option<[u32; N]> {
        self.params.get(..N)?.try_into().ok()
    }
}

/// A response or an event: its code and parameters, with no data.
pub fn short(kind: u16, code: u16, transaction: u32, params: &[u32]) -> Vec<u8> {
    let length = (HEADER + params.len() * 4) as u32;
    let mut w = Writer::new();
    w.u32(length).u16(kind).u16(code).u32(transaction);
    for &p in params {
        w.u32(p);
    }
    w.into_bytes()
}

/// The header of a data container `length` bytes long after the header.
/// One past 4 GiB says so with the largest length, and the computer reads
/// it to its end on the bus instead.
pub fn data_header(code: u16, transaction: u32, length: u64) -> [u8; HEADER] {
    let length = u32::try_from(length + HEADER as u64).unwrap_or(u32::MAX);
    Header { length, kind: container::DATA, code, transaction }.bytes()
}

/// A dataset being written.
#[derive(Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    pub fn u8(&mut self, v: u8) -> &mut Writer {
        self.bytes.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Writer {
        self.bytes.extend(v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Writer {
        self.bytes.extend(v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Writer {
        self.bytes.extend(v.to_le_bytes());
        self
    }

    pub fn u128(&mut self, v: u128) -> &mut Writer {
        self.bytes.extend(v.to_le_bytes());
        self
    }

    /// A string, cut short at the most a string holds, where a character
    /// that takes two code units goes whole or not at all. The empty string
    /// is the count alone.
    pub fn string(&mut self, s: &str) -> &mut Writer {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut n = units.len().min(STRING_MAX - 1);
        if n < units.len() && (0xD800..0xDC00).contains(&units[n - 1]) {
            n -= 1;
        }
        if n == 0 {
            return self.u8(0);
        }
        self.u8(n as u8 + 1);
        for &u in &units[..n] {
            self.u16(u);
        }
        self.u16(0)
    }

    pub fn u16s(&mut self, values: &[u16]) -> &mut Writer {
        self.u32(values.len() as u32);
        for &v in values {
            self.u16(v);
        }
        self
    }

    pub fn u32s(&mut self, values: &[u32]) -> &mut Writer {
        self.u32(values.len() as u32);
        for &v in values {
            self.u32(v);
        }
        self
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// A dataset being read; each value is `None` once the dataset runs out.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes }
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.bytes.split_first_chunk::<N>()?;
        self.bytes = rest;
        Some(*head)
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    pub fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }

    /// A string, which ends at its first zero; one whose code units are not
    /// UTF-16, or that lacks its terminating zero, is `None`.
    pub fn string(&mut self) -> Option<String> {
        let count = self.u8()? as usize;
        if count == 0 {
            return Some(String::new());
        }
        let units = (0..count).map(|_| self.u16()).collect::<Option<Vec<u16>>>()?;
        if units[count - 1] != 0 {
            return None;
        }
        let end = units.iter().position(|&u| u == 0).expect("the last unit is zero");
        String::from_utf16(&units[..end]).ok()
    }

    /// The bytes not read yet.
    pub fn rest(&self) -> &'a [u8] {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings_read_back_as_written() {
        let mut w = Writer::new();
        w.u8(7).u16(0x1234).u32(0xDEAD_BEEF).u64(1 << 40).string("相册 📷 photos").string("");
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!((r.u8(), r.u16(), r.u32(), r.u64()), (Some(7), Some(0x1234), Some(0xDEAD_BEEF), Some(1 << 40)));
        assert_eq!(r.string().as_deref(), Some("相册 📷 photos"));
        assert_eq!(r.string().as_deref(), Some(""));
        assert!(r.rest().is_empty());
        assert_eq!(r.u8(), None, "nothing is left");
    }

    #[test]
    fn a_string_counts_its_terminating_zero_and_the_empty_one_is_its_count() {
        let mut w = Writer::new();
        w.string("ab").string("");
        assert_eq!(w.into_bytes(), [3, b'a', 0, b'b', 0, 0, 0, 0]);
    }

    #[test]
    fn a_long_string_is_cut_short_without_splitting_a_character() {
        let long = "x".repeat(300);
        let mut w = Writer::new();
        w.string(&long);
        let bytes = w.into_bytes();
        assert_eq!(bytes[0], 255);
        assert_eq!(Reader::new(&bytes).string().unwrap(), "x".repeat(254));
        // 253 single units, then a character of two: it does not fit whole.
        let edge = format!("{}😀", "y".repeat(253));
        let mut w = Writer::new();
        w.string(&edge);
        let bytes = w.into_bytes();
        assert_eq!(Reader::new(&bytes).string().unwrap(), "y".repeat(253));
    }

    #[test]
    fn strings_that_are_not_utf16_or_not_ended_are_refused() {
        assert_eq!(Reader::new(&[2, b'a', 0, b'b', 0]).string(), None, "no terminating zero");
        assert_eq!(Reader::new(&[2, 0x00, 0xD8, 0, 0]).string(), None, "a lone surrogate");
        assert_eq!(Reader::new(&[3, b'a', 0]).string(), None, "cut off");
        assert_eq!(Reader::new(&[4, b'a', 0, 0, 0, b'b', 0, 0, 0]).string().as_deref(), Some("a"), "ends at the first zero");
    }

    #[test]
    fn commands_are_read_whole_with_up_to_five_parameters() {
        let bytes = short(container::COMMAND, 0x1007, 9, &[0x0001_0001, 0, 0xFFFF_FFFF]);
        let cmd = Command::read(&bytes).unwrap();
        assert_eq!(cmd, Command { code: 0x1007, transaction: 9, params: vec![0x0001_0001, 0, 0xFFFF_FFFF] });
        assert_eq!(cmd.take::<3>(), Some([0x0001_0001, 0, 0xFFFF_FFFF]));
        assert_eq!(cmd.take::<4>(), None, "the operation needs more than came");
        assert!(Command::read(&short(container::COMMAND, 0x1001, 1, &[])).is_some());
        assert!(Command::read(&short(container::DATA, 0x1001, 1, &[])).is_none(), "not a command");
        assert!(Command::read(&short(container::COMMAND, 0x1001, 1, &[1, 2, 3, 4, 5, 6])).is_none(), "six parameters");
        let mut short_by_one = short(container::COMMAND, 0x1001, 1, &[1]);
        short_by_one.pop();
        assert!(Command::read(&short_by_one).is_none(), "the header counts more than came");
        assert!(Command::read(&[0; 4]).is_none());
    }

    #[test]
    fn a_data_header_past_four_gibibytes_says_the_largest_length() {
        let small = Header::read(&data_header(0x1009, 3, 100)).unwrap();
        assert_eq!(small, Header { length: 112, kind: container::DATA, code: 0x1009, transaction: 3 });
        let big = Header::read(&data_header(0x1009, 3, 5 << 30)).unwrap();
        assert_eq!(big.length, u32::MAX);
        let edge = Header::read(&data_header(0x1009, 3, u64::from(u32::MAX) - 12)).unwrap();
        assert_eq!(edge.length, u32::MAX);
    }
}
