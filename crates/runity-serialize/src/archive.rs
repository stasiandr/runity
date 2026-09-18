//! A self-describing container: magic, versions and a checksum around a
//! payload.
//!
//! Raw [`Writer`] output is fine inside one process, but the moment bytes
//! outlive the build that wrote them — a save from last month, a replay from
//! a patch ago — you need to know what you are looking at *before* decoding.
//! That is what the header is for: it says which kind of payload follows,
//! which schema version wrote it, and whether it survived the trip intact.

use crate::{Deserialize, Error, Reader, Result, Serialize, Writer};

/// Every runity archive starts with these four bytes.
pub const MAGIC: [u8; 4] = *b"RNTY";

/// Version of the container layout itself. Bumped only if this header
/// changes — payload schemas have their own version.
pub const FORMAT: u16 = 1;

/// Bytes before the payload: magic, format, kind, version, length, checksum.
pub const HEADER_LEN: usize = 22;

/// The header of an encoded archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Archive {
    /// A four-character tag naming what the payload is, such as `*b"SAVE"`.
    pub kind: [u8; 4],
    /// The payload's schema version, chosen by whoever defined the payload.
    pub version: u32,
    /// Payload length in bytes.
    pub length: u32,
    /// CRC-32 of the payload.
    pub checksum: u32,
}

impl Archive {
    /// Wrap an already-encoded payload.
    pub fn encode(kind: [u8; 4], version: u32, payload: &[u8]) -> Vec<u8> {
        let mut writer = Writer::with_capacity(HEADER_LEN + payload.len());
        writer
            .raw(&MAGIC)
            .u16(FORMAT)
            .raw(&kind)
            .u32(version)
            .u32(payload.len() as u32)
            .u32(checksum(payload))
            .raw(payload);
        writer.finish()
    }

    /// Encode a value and wrap it in one step.
    pub fn encode_value<T: Serialize + ?Sized>(kind: [u8; 4], version: u32, value: &T) -> Vec<u8> {
        let mut writer = Writer::new();
        value.serialize(&mut writer);
        Self::encode(kind, version, writer.as_bytes())
    }

    /// Encode whatever the closure writes and wrap it.
    pub fn write(kind: [u8; 4], version: u32, body: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut writer = Writer::new();
        body(&mut writer);
        Self::encode(kind, version, writer.as_bytes())
    }

    /// Parse the header and hand back a reader positioned on the payload.
    ///
    /// The returned reader carries the payload's schema version, so nested
    /// `Deserialize` impls can migrate without being told separately.
    pub fn decode(bytes: &[u8]) -> Result<(Archive, Reader<'_>)> {
        let mut header = Reader::new(bytes);
        let magic = header.raw(4)?;
        if magic != MAGIC {
            return Err(Error::BadMagic {
                found: [magic[0], magic[1], magic[2], magic[3]],
            });
        }
        let format = header.u16()?;
        if format != FORMAT {
            return Err(Error::UnsupportedFormat {
                found: format,
                supported: FORMAT,
            });
        }
        let kind_bytes = header.raw(4)?;
        let kind = [kind_bytes[0], kind_bytes[1], kind_bytes[2], kind_bytes[3]];
        let version = header.u32()?;
        let length = header.u32()?;
        let stored = header.u32()?;

        let payload = &bytes[HEADER_LEN..];
        if (length as usize) > payload.len() {
            return Err(Error::LengthOutOfRange {
                position: HEADER_LEN,
                length: u64::from(length),
                available: payload.len(),
            });
        }
        let payload = &payload[..length as usize];

        let found = checksum(payload);
        if found != stored {
            return Err(Error::Corrupt {
                expected: stored,
                found,
            });
        }

        let archive = Archive {
            kind,
            version,
            length,
            checksum: stored,
        };
        Ok((archive, Reader::versioned(payload, version)))
    }

    /// Decode, insisting on a particular kind and a version this build can
    /// still read.
    ///
    /// Older versions are allowed through — that is what migration is for;
    /// *newer* ones are refused, because a build cannot invent fields it has
    /// never heard of.
    pub fn open(bytes: &[u8], kind: [u8; 4], newest: u32) -> Result<(Archive, Reader<'_>)> {
        let (archive, reader) = Self::decode(bytes)?;
        if archive.kind != kind {
            return Err(Error::UnexpectedKind {
                found: archive.kind,
                expected: kind,
            });
        }
        if archive.version > newest {
            return Err(Error::UnsupportedVersion {
                found: archive.version,
                supported: newest,
            });
        }
        Ok((archive, reader))
    }

    /// Decode a whole archive into one value, checking kind, version and that
    /// nothing is left over.
    pub fn read_value<T: Deserialize>(bytes: &[u8], kind: [u8; 4], newest: u32) -> Result<T> {
        let (_, mut reader) = Self::open(bytes, kind, newest)?;
        let value = T::deserialize(&mut reader)?;
        reader.finish()?;
        Ok(value)
    }

    /// Read just the header, without touching the payload.
    ///
    /// Lets a load menu list saves — name, version, size — without decoding
    /// several megabytes of world for each one.
    pub fn peek(bytes: &[u8]) -> Result<Archive> {
        Self::decode(bytes).map(|(archive, _)| archive)
    }

    /// The kind tag as text, when it is printable.
    pub fn kind_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.kind).ok()
    }
}

/// CRC-32 (the IEEE polynomial, as used by PNG and zip).
///
/// Not a security measure — it catches truncation and bit rot, not tampering.
/// A save edited on purpose will have a perfectly valid checksum, which is
/// why every decode path is bounds-checked as well.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        let index = ((crc ^ u32::from(*byte)) & 0xff) as usize;
        crc = TABLE[index] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}
