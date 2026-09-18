//! What can go wrong when reading a byte stream.

use core::fmt;

/// The result of every read operation.
pub type Result<T> = core::result::Result<T, Error>;

/// A decoding failure, always with the byte offset where it happened.
///
/// Every variant is a statement about *untrusted input*: a save file edited
/// by hand, a packet from a player, a snapshot written by an older build.
/// None of them can panic the process, which matters because the network
/// path feeds this code bytes chosen by someone else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The stream ended in the middle of a value.
    UnexpectedEnd {
        /// Offset where the read started.
        position: usize,
        /// How many bytes the value needed.
        needed: usize,
        /// How many were left.
        available: usize,
    },
    /// The first four bytes are not runity's magic number.
    BadMagic {
        /// What was found instead.
        found: [u8; 4],
    },
    /// The container layout itself is from a different era of the engine.
    UnsupportedFormat {
        /// Format version found in the header.
        found: u16,
        /// Format version this build writes.
        supported: u16,
    },
    /// The payload is the wrong kind of thing — a save where a snapshot was
    /// expected, say.
    UnexpectedKind {
        /// Four-character tag found in the header.
        found: [u8; 4],
        /// Tag the caller asked for.
        expected: [u8; 4],
    },
    /// The payload's schema version is outside what the caller can migrate.
    UnsupportedVersion {
        /// Version found in the header.
        found: u32,
        /// Newest version the caller understands.
        supported: u32,
    },
    /// The checksum does not match: the bytes were truncated or damaged.
    Corrupt {
        /// Checksum stored in the header.
        expected: u32,
        /// Checksum computed over the payload.
        found: u32,
    },
    /// A length field claims more bytes than the stream holds.
    LengthOutOfRange {
        /// Offset of the length field.
        position: usize,
        /// The claimed length.
        length: u64,
        /// Bytes actually left.
        available: usize,
    },
    /// A varint ran past ten bytes, so it cannot be a `u64`.
    OverlongVarint {
        /// Offset where the varint started.
        position: usize,
    },
    /// A string field is not valid UTF-8.
    InvalidUtf8 {
        /// Offset where the string started.
        position: usize,
    },
    /// A value is structurally fine but semantically impossible — a `bool`
    /// byte that is neither 0 nor 1, an enum tag nobody defines.
    InvalidValue {
        /// Offset of the offending value.
        position: usize,
        /// What was being read.
        what: &'static str,
    },
    /// Decoding finished but bytes were left over, which usually means the
    /// reader and the writer disagree about the schema.
    TrailingData {
        /// Offset where decoding stopped.
        position: usize,
        /// How many bytes were left.
        remaining: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedEnd {
                position,
                needed,
                available,
            } => write!(
                f,
                "stream ended at byte {position}: needed {needed} more byte(s), {available} left"
            ),
            Error::BadMagic { found } => {
                write!(f, "not a runity archive: magic {}", Tag(*found))
            }
            Error::UnsupportedFormat { found, supported } => write!(
                f,
                "archive container format {found} is not supported (this build writes {supported})"
            ),
            Error::UnexpectedKind { found, expected } => {
                write!(
                    f,
                    "expected a {} archive, found {}",
                    Tag(*expected),
                    Tag(*found)
                )
            }
            Error::UnsupportedVersion { found, supported } => write!(
                f,
                "payload version {found} is newer than this build understands ({supported})"
            ),
            Error::Corrupt { expected, found } => {
                write!(
                    f,
                    "checksum mismatch: header says {expected:#010x}, payload is {found:#010x}"
                )
            }
            Error::LengthOutOfRange {
                position,
                length,
                available,
            } => write!(
                f,
                "length {length} at byte {position} exceeds the {available} byte(s) available"
            ),
            Error::OverlongVarint { position } => {
                write!(f, "overlong varint at byte {position}")
            }
            Error::InvalidUtf8 { position } => {
                write!(f, "invalid UTF-8 in the string at byte {position}")
            }
            Error::InvalidValue { position, what } => {
                write!(f, "invalid {what} at byte {position}")
            }
            Error::TrailingData {
                position,
                remaining,
            } => {
                write!(
                    f,
                    "{remaining} unread byte(s) after decoding stopped at {position}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// Prints a four-byte tag readably, falling back to hex for odd bytes.
struct Tag([u8; 4]);

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.iter().all(|b| b.is_ascii_graphic()) {
            write!(f, "\"{}\"", core::str::from_utf8(&self.0).unwrap_or("????"))
        } else {
            write!(f, "{:02x?}", self.0)
        }
    }
}
