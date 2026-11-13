//! The system's security patch level as a date, and the floor a recovery
//! may tell the key service the installed system has at least. Recovery
//! only: nothing here writes a file or a disk key, and it enables no key
//! upgrade; it never lowers a system's own later date.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::str::FromStr;

/// A patch level as Android writes it: `YYYY-MM-DD`, strict ASCII, a real
/// Gregorian date of a year 2000 through 9999. Ordered, and so compared,
/// as the date it is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PatchLevel {
    year: u16,
    month: u8,
    day: u8,
}

/// The days a month of a year holds: February counts its leap day when the
/// year is divisible by 4, except a century, which counts it only when
/// divisible by 400.
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        2 => 28,
        _ => 31,
    }
}

impl FromStr for PatchLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let b = s.as_bytes();
        let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits(0..4) || !digits(5..7) || !digits(8..10) {
            bail!("expected security patch date YYYY-MM-DD, got {s:?}");
        }
        let (year, month, day) = (s[0..4].parse::<u16>().unwrap(), s[5..7].parse::<u8>().unwrap(), s[8..10].parse::<u8>().unwrap());
        if !(2000..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
            bail!("invalid security patch date {s:?}");
        }
        Ok(PatchLevel { year, month, day })
    }
}

impl fmt::Display for PatchLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Read a patch level from a configuration's string; anything else the
/// configuration holds for it is a wrong configuration, not a date guessed.
impl<'de> Deserialize<'de> for PatchLevel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The patch level the recovery tells the key service the system has. With
/// no floor configured, `current` stands exactly as the installed system
/// wrote it, however nonstandard, so an old profile changes nothing. With
/// one, `current` must be a real date, and the later of the two is given:
/// a floor lifts, never lowers.
pub fn select(current: &str, floor: Option<PatchLevel>) -> Result<String> {
    let Some(floor) = floor else { return Ok(current.to_string()) };
    let current: PatchLevel = current.parse().with_context(|| format!("the installed system's patch level {current:?}"))?;
    Ok(std::cmp::max(current, floor).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{Error as TestError, StrDeserializer};

    fn date(s: &str) -> PatchLevel {
        s.parse().unwrap()
    }

    #[test]
    fn the_centuries_leap_as_they_do() {
        assert_eq!(date("2000-02-29").to_string(), "2000-02-29"); // divisible by 400
        assert_eq!(date("2400-02-29").to_string(), "2400-02-29");
        assert!(PatchLevel::from_str("2100-02-29").is_err()); // a century without the day
        assert!(PatchLevel::from_str("2024-02-29").is_ok());
        assert!(PatchLevel::from_str("2023-02-29").is_err());
        assert_eq!(date("9999-12-31").to_string(), "9999-12-31");
    }

    #[test]
    fn dates_that_are_not_patch_levels_are_refused() {
        for bad in [
            "1999-12-31",
            "2024-13-01",
            "2024-00-10",
            "2024-01-00",
            "2024-01-32",
            "2024-04-31",
            "2024-1-05",
            "2024-01-5",
            "24-01-05",
            "2024/01/05",
            " 2024-01-05",
            "2024-01-05 ",
            "",
            "2024-01-05\n",
            "２０２４-01-05",
            "2024-A1-05",
        ] {
            assert!(PatchLevel::from_str(bad).is_err(), "{bad} was taken");
        }
    }

    #[test]
    fn without_a_floor_the_installed_value_stands_as_written() {
        for as_is in ["2024-03-05", "vendor's own", "P", ""] {
            assert_eq!(select(as_is, None).unwrap(), as_is);
        }
    }

    #[test]
    fn the_floor_lifts_and_never_lowers() {
        assert_eq!(select("2023-05-01", Some(date("2024-01-01"))).unwrap(), "2024-01-01");
        assert_eq!(select("2025-07-02", Some(date("2024-06-01"))).unwrap(), "2025-07-02");
        assert_eq!(select("2024-01-01", Some(date("2024-01-01"))).unwrap(), "2024-01-01");
        assert!(select("nonsense", Some(date("2024-01-01"))).is_err());
    }

    #[test]
    fn a_configuration_string_reads_as_the_date_it_holds() {
        let read = PatchLevel::deserialize(StrDeserializer::<TestError>::new("2024-03-05")).unwrap();
        assert_eq!(read, date("2024-03-05"));
        assert_eq!(read.to_string().parse::<PatchLevel>().unwrap(), read);
        assert!(PatchLevel::deserialize(StrDeserializer::<TestError>::new("2024-3-5")).is_err());
    }
}
