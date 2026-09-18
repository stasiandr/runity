//! Reading `~/.Xauthority` so we can authenticate with the X server.
//!
//! The file is a flat list of records, all integers big-endian:
//! `family:u16, address, number, name, data`, where every variable-length
//! field is `len:u16` followed by that many bytes.

use std::fs;
use std::path::PathBuf;

const FAMILY_LOCAL: u16 = 256;
pub const MIT_MAGIC_COOKIE: &str = "MIT-MAGIC-COOKIE-1";

#[derive(Debug, Clone, Default)]
pub struct AuthEntry {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u16(&mut self) -> Option<u16> {
        let b = self.bytes.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    fn blob(&mut self) -> Option<&'a [u8]> {
        let len = self.u16()? as usize;
        let b = self.bytes.get(self.at..self.at + len)?;
        self.at += len;
        Some(b)
    }
}

fn xauthority_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XAUTHORITY") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Xauthority"))
}

/// Best-effort hostname, used to match `FamilyLocal` entries.
fn hostname() -> Option<String> {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

/// Find the cookie for `display` on this host.
///
/// Returns `None` when there is no auth file or no matching entry — servers
/// that allow unauthenticated local connections are happy with that.
pub fn cookie_for_display(display: u32) -> Option<AuthEntry> {
    let bytes = fs::read(xauthority_path()?).ok()?;
    let host = hostname();
    let want_number = display.to_string();

    let mut reader = Reader {
        bytes: &bytes,
        at: 0,
    };
    let mut fallback: Option<AuthEntry> = None;

    while reader.at < bytes.len() {
        let Some(family) = reader.u16() else { break };
        let (Some(address), Some(number), Some(name), Some(data)) =
            (reader.blob(), reader.blob(), reader.blob(), reader.blob())
        else {
            break;
        };

        if name != MIT_MAGIC_COOKIE.as_bytes() {
            continue;
        }
        // An empty display number in the file means "any display".
        if !number.is_empty() && number != want_number.as_bytes() {
            continue;
        }

        let entry = AuthEntry {
            name: name.to_vec(),
            data: data.to_vec(),
        };
        let host_matches = match (&host, family) {
            (Some(h), FAMILY_LOCAL) => address == h.as_bytes(),
            _ => false,
        };
        if host_matches {
            return Some(entry);
        }
        fallback.get_or_insert(entry);
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(family: u16, address: &str, number: &str, name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = family.to_be_bytes().to_vec();
        for field in [address.as_bytes(), number.as_bytes(), name.as_bytes(), data] {
            out.extend_from_slice(&(field.len() as u16).to_be_bytes());
            out.extend_from_slice(field);
        }
        out
    }

    /// Parse a synthetic file through the same reader the real code uses.
    fn parse_all(bytes: &[u8]) -> Vec<(u16, String, String, String, Vec<u8>)> {
        let mut r = Reader { bytes, at: 0 };
        let mut out = Vec::new();
        while r.at < bytes.len() {
            let Some(family) = r.u16() else { break };
            let (Some(a), Some(n), Some(name), Some(d)) = (r.blob(), r.blob(), r.blob(), r.blob())
            else {
                break;
            };
            out.push((
                family,
                String::from_utf8_lossy(a).into_owned(),
                String::from_utf8_lossy(n).into_owned(),
                String::from_utf8_lossy(name).into_owned(),
                d.to_vec(),
            ));
        }
        out
    }

    #[test]
    fn reads_consecutive_records() {
        let mut file = record(FAMILY_LOCAL, "box", "0", MIT_MAGIC_COOKIE, &[1, 2, 3]);
        file.extend(record(0, "1.2.3.4", "1", MIT_MAGIC_COOKIE, &[9]));
        let parsed = parse_all(&file);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, "box");
        assert_eq!(parsed[0].4, vec![1, 2, 3]);
        assert_eq!(parsed[1].2, "1");
    }

    #[test]
    fn a_truncated_record_stops_parsing_instead_of_panicking() {
        let mut file = record(FAMILY_LOCAL, "box", "0", MIT_MAGIC_COOKIE, &[1]);
        file.truncate(file.len() - 1);
        assert!(parse_all(&file).is_empty());
    }

    #[test]
    fn missing_auth_file_is_not_an_error() {
        // Point at a path that cannot exist; the caller treats None as
        // "connect without authentication".
        std::env::set_var("XAUTHORITY", "/nonexistent/runity/.Xauthority");
        assert!(cookie_for_display(0).is_none());
        std::env::remove_var("XAUTHORITY");
    }
}
