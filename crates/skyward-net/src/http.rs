//! HTTP and HTTPS, with names looked up in the DNS servers the network
//! gave.

use crate::dns;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The DNS servers to ask, as the network last gave them.
pub type Servers = Arc<Mutex<Vec<Ipv4Addr>>>;

/// How long reaching a server may take.
const CONNECT: Duration = Duration::from_secs(15);

/// How long a connection may go without a byte either way before it is
/// taken as lost: a download read as it comes has no end to wait for.
const STALL: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct Lookup(Servers);

impl Resolver for Lookup {
    fn resolve(&self, uri: &Uri, _config: &Config, _timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().ok_or_else(|| ureq::Error::BadUri(format!("{uri} names no host")))?;
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("http") { 80 } else { 443 });
        let servers = self.0.lock().expect("the DNS servers are not held by a thread that panicked").clone();
        let found = dns::lookup(host, &servers).map_err(|e| ureq::Error::Io(std::io::Error::other(format!("{e:#}"))))?;
        let mut out = self.empty();
        // As many as it holds; the rest are not tried.
        for address in found {
            if out.try_push(SocketAddr::new(IpAddr::V4(address), port)).is_err() {
                break;
            }
        }
        Ok(out)
    }
}

/// An agent that looks names up in `servers`, and gives a server's answer
/// whatever its status: the caller says what a status means.
pub fn agent(servers: Servers) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_connect(Some(CONNECT))
        .timeout_per_read(Some(STALL))
        .timeout_per_write(Some(STALL))
        .http_status_as_error(false)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::new(), Lookup(servers))
}

/// The time of day a server keeps, in Unix seconds: every answer carries
/// it in its `Date` header, which is the phone's for the asking wherever
/// the network reaches.
pub fn server_time(a: &ureq::Agent, url: &str) -> anyhow::Result<i64> {
    use anyhow::{anyhow, Context};
    let answer = match a.head(url).call() {
        Ok(answer) => answer,
        Err(_) => a.get(url).call().with_context(|| format!("ask {url} for the time"))?,
    };
    let date = answer.headers().get("date").and_then(|v| v.to_str().ok()).ok_or_else(|| anyhow!("{url} gave no Date header"))?;
    date_at(date).ok_or_else(|| anyhow!("{url}'s Date header does not read: {date}"))
}

/// The Unix second a `Date` header names, in the form every server sends:
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
pub fn date_at(date: &str) -> Option<i64> {
    let mut parts = date.split_whitespace();
    // The weekday goes first, then the day of the month.
    let _weekday = parts.next()?;
    let day: i64 = parts.next()?.trim_end_matches(',').parse().ok()?;
    let month = month_of(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    let clock = parts.next()?;
    // Every server of this age answers in GMT; one that says otherwise is
    // not read rather than read wrong.
    if parts.next().unwrap_or("GMT") != "GMT" {
        return None;
    }
    let mut hms = clock.split(':');
    let (h, m, s): (i64, i64, i64) = (hms.next()?.parse().ok()?, hms.next()?.parse().ok()?, hms.next()?.parse().ok()?);
    Some(days_from_1970(year, month, day) * 86_400 + h * 3600 + m * 60 + s)
}

fn month_of(name: &str) -> Option<i64> {
    Some(match name {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

/// Days from 1970-01-01 to `year`-`month`-`day` in the Gregorian calendar.
/// March starts the year here, which is what makes the leap day land at its
/// end and the arithmetic count of days come out even.
fn days_from_1970(mut year: i64, month: i64, day: i64) -> i64 {
    if month <= 2 {
        year -= 1;
    }
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_header_names_the_second_it_says() {
        assert_eq!(date_at("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(date_at("Wed, 11 Nov 2026 08:39:02 GMT"), Some(1_794_386_342));
    }

    #[test]
    fn a_date_header_of_another_shape_is_not_read_wrong() {
        assert_eq!(date_at("Wednesday, 06-Nov-94 08:49:37 GMT"), None);
        assert_eq!(date_at("Sun, 06 Nov 1994 08:49:37 +0200"), None);
        assert_eq!(date_at("Sun, 06 Foo 1994 08:49:37 GMT"), None);
    }
}
