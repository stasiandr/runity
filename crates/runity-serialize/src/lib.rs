//! Versioned binary serialization for runity.
//!
//! One format serves three jobs that are usually solved three different ways:
//!
//! * **Saves** — the world written to disk, read back by a later build.
//! * **Network** — state and commands passed between machines, where the
//!   bytes are small, hostile and frequent.
//! * **Tests** — a snapshot of the simulation compared against a stored one,
//!   which is how you notice that a "harmless" refactor changed the world.
//!
//! Using the same encoder for all three is deliberate. A save format that is
//! only exercised when a player saves rots; one that every determinism test
//! runs through is checked thousands of times a day.
//!
//! Two rules make that work:
//!
//! * **Byte-for-byte determinism.** Little-endian, no padding, floats stored
//!   as exact bit patterns, maps written in sorted key order. The same world
//!   encodes to the same bytes on every machine, so comparing two snapshots
//!   is `==` on a slice.
//! * **No trust in the input.** Every read is bounds-checked and returns
//!   [`Result`]; a truncated file or a malicious packet produces an
//!   [`Error`], never a panic and never a giant allocation.
//!
//! ```
//! use runity_serialize::{serializable, Archive, Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq)]
//! struct Settlement {
//!     name: String,
//!     residents: u32,
//!     wood: u32,
//! }
//!
//! serializable!(Settlement { name, residents, wood });
//!
//! const SAVE: [u8; 4] = *b"SAVE";
//! const VERSION: u32 = 1;
//!
//! let village = Settlement { name: "Ashford".into(), residents: 12, wood: 340 };
//! let bytes = Archive::encode_value(SAVE, VERSION, &village);
//!
//! let loaded: Settlement = Archive::read_value(&bytes, SAVE, VERSION).unwrap();
//! assert_eq!(loaded, village);
//! ```

#![forbid(unsafe_code)]

mod archive;
mod error;
mod reader;
mod traits;
mod writer;

pub use archive::{checksum, Archive, FORMAT, HEADER_LEN, MAGIC};
pub use error::{Error, Result};
pub use reader::Reader;
pub use traits::{from_bytes, to_bytes, Deserialize, Serialize};
pub use writer::Writer;

/// Implement [`Serialize`] and [`Deserialize`] for a struct, field by field.
///
/// There is no derive macro here on purpose: a proc macro would mean a
/// separate crate and, in practice, third-party dependencies. Listing the
/// fields is barely longer than an attribute and has a useful side effect —
/// the wire format is written down in one place, so changing it is a visible
/// edit rather than a consequence of renaming a field.
///
/// Fields are encoded in the order given. Never reorder or remove one without
/// bumping the payload version.
///
/// ```
/// use runity_serialize::{from_bytes, serializable, to_bytes};
/// use runity_math::Vec3;
///
/// #[derive(Debug, PartialEq)]
/// struct Resource {
///     position: Vec3,
///     kind: u8,
///     remaining: u32,
/// }
///
/// serializable!(Resource { position, kind, remaining });
///
/// let tree = Resource { position: Vec3 { x: 3.0, y: 0.0, z: -7.5 }, kind: 2, remaining: 40 };
/// assert_eq!(from_bytes::<Resource>(&to_bytes(&tree)).unwrap(), tree);
/// ```
/// Tuple structs — the usual shape of a small component — use parentheses
/// and field indices instead:
///
/// ```
/// use runity_serialize::{from_bytes, serializable, to_bytes};
///
/// #[derive(Debug, PartialEq)]
/// struct Wood(u32);
///
/// serializable!(Wood(0));
/// assert_eq!(from_bytes::<Wood>(&to_bytes(&Wood(12))).unwrap(), Wood(12));
/// ```
#[macro_export]
macro_rules! serializable {
    ($ty:ident ( $($index:tt),* $(,)? )) => {
        impl $crate::Serialize for $ty {
            fn serialize(&self, writer: &mut $crate::Writer) {
                $($crate::Serialize::serialize(&self.$index, writer);)*
            }
        }

        impl $crate::Deserialize for $ty {
            fn deserialize(reader: &mut $crate::Reader) -> $crate::Result<Self> {
                // Fields are decoded in declaration order; the indices are
                // there to name them on the way out.
                Ok(Self($({
                    let _ = $index;
                    $crate::Deserialize::deserialize(reader)?
                }),*))
            }
        }
    };

    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl $crate::Serialize for $ty {
            fn serialize(&self, writer: &mut $crate::Writer) {
                $($crate::Serialize::serialize(&self.$field, writer);)*
            }
        }

        impl $crate::Deserialize for $ty {
            fn deserialize(reader: &mut $crate::Reader) -> $crate::Result<Self> {
                Ok(Self {
                    $($field: $crate::Deserialize::deserialize(reader)?,)*
                })
            }
        }
    };
}

/// Implement [`Serialize`] and [`Deserialize`] for a fieldless enum with
/// explicit tags.
///
/// The tags are spelled out rather than taken from declaration order, so
/// inserting a variant in the middle — which happens constantly while a game
/// is being designed — cannot silently reinterpret every old save.
///
/// ```
/// use runity_serialize::{from_bytes, serializable_enum, to_bytes};
///
/// #[derive(Debug, PartialEq, Clone, Copy)]
/// enum Season {
///     Spring,
///     Summer,
///     Autumn,
///     Winter,
/// }
///
/// serializable_enum!(Season {
///     0 => Spring,
///     1 => Summer,
///     2 => Autumn,
///     3 => Winter,
/// });
///
/// assert_eq!(from_bytes::<Season>(&to_bytes(&Season::Autumn)).unwrap(), Season::Autumn);
/// // An unknown tag is an error, not a silently wrong season.
/// assert!(from_bytes::<Season>(&[9]).is_err());
/// ```
#[macro_export]
macro_rules! serializable_enum {
    ($ty:ty { $($tag:literal => $variant:ident),* $(,)? }) => {
        impl $crate::Serialize for $ty {
            fn serialize(&self, writer: &mut $crate::Writer) {
                let tag: u32 = match self {
                    $(<$ty>::$variant => $tag,)*
                };
                writer.varint(tag as u64);
            }
        }

        impl $crate::Deserialize for $ty {
            fn deserialize(reader: &mut $crate::Reader) -> $crate::Result<Self> {
                let position = reader.position();
                let tag = reader.varint()?;
                match tag {
                    $($tag => Ok(<$ty>::$variant),)*
                    _ => Err($crate::Error::InvalidValue {
                        position,
                        what: stringify!($ty),
                    }),
                }
            }
        }
    };
}
