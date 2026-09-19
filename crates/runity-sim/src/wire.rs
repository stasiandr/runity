//! Text-based round-tripping of entity state — the same shape serves save
//! files on disk and snapshots sent over the network.
//!
//! A snapshot is a header line followed by one line per entity. The header
//! comes first and carries a format version, an RNG seed and a world name,
//! so a save-file reader and a network reader can both make sense of the
//! stream before either one is specialized on what an entity line contains.

use std::fmt;

/// Version of the [`write`]/[`read`] text format. Bump this when the header
/// or the line framing changes shape — not when a game's own entity lines
/// change, since those are outside this crate's control.
pub const WIRE_FORMAT_VERSION: u32 = 1;

/// The line written ahead of every entity line.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The [`WIRE_FORMAT_VERSION`] the snapshot was written with.
    pub version: u32,
    /// The RNG seed the world was generated or is running with.
    pub seed: u64,
    /// A human-readable name for the world, not parsed for meaning.
    pub world_name: String,
}

impl Header {
    fn write_line(&self) -> String {
        format!("{} {} {}", self.version, self.seed, self.world_name)
    }

    fn read_line(line: &str) -> Result<Self, WireError> {
        let mut parts = line.splitn(3, ' ');
        let version = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or(WireError::Header("missing format version"))?;
        let seed = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or(WireError::Header("missing seed"))?;
        let world_name = parts
            .next()
            .ok_or(WireError::Header("missing world name"))?
            .to_string();
        Ok(Header {
            version,
            seed,
            world_name,
        })
    }
}

/// Round-trips one entity's state as a single line of text.
///
/// Implementors must write floats through `Display`/`Debug` — Rust's float
/// formatting already produces the shortest string that parses back to the
/// exact same bits — never through a fixed number of decimal places, which
/// would quietly lose precision on write.
pub trait Wire: Sized {
    /// Write this entity's state as one line. Must not contain `'\n'`.
    fn write_line(&self) -> String;

    /// Parse one entity back from a line written by [`Wire::write_line`].
    fn read_line(line: &str) -> Result<Self, WireError>;
}

/// Write a full snapshot: the header, then one line per entity in order.
pub fn write<T: Wire>(entities: &[T], seed: u64, world_name: &str) -> String {
    let header = Header {
        version: WIRE_FORMAT_VERSION,
        seed,
        world_name: world_name.to_string(),
    };
    let mut out = header.write_line();
    for entity in entities {
        out.push('\n');
        out.push_str(&entity.write_line());
    }
    out
}

/// Parse a full snapshot written by [`write`]: the header, then every
/// entity line, in the order they were written.
pub fn read<T: Wire>(text: &str) -> Result<(Header, Vec<T>), WireError> {
    let mut lines = text.lines();
    let header = Header::read_line(lines.next().ok_or(WireError::MissingHeader)?)?;
    let entities = lines.map(T::read_line).collect::<Result<Vec<_>, _>>()?;
    Ok((header, entities))
}

/// Something went wrong parsing a [`write`]-formatted snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The text had no header line at all.
    MissingHeader,
    /// The header line did not have `version seed world_name`.
    Header(&'static str),
    /// An entity line did not parse; carries the implementor's own message.
    Entity(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::MissingHeader => write!(f, "missing header line"),
            WireError::Header(msg) => write!(f, "malformed header: {msg}"),
            WireError::Entity(msg) => write!(f, "malformed entity line: {msg}"),
        }
    }
}

impl std::error::Error for WireError {}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::Vec3;

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        entity: u32,
        pos: Vec3,
    }

    impl Wire for Position {
        fn write_line(&self) -> String {
            format!(
                "{} {} {} {}",
                self.entity, self.pos.x, self.pos.y, self.pos.z
            )
        }

        fn read_line(line: &str) -> Result<Self, WireError> {
            let mut parts = line.split_whitespace();
            let mut next = |what: &'static str| -> Result<&str, WireError> {
                parts
                    .next()
                    .ok_or(WireError::Entity(format!("missing {what}")))
            };
            let entity = next("entity id")?
                .parse()
                .map_err(|_| WireError::Entity("bad entity id".to_string()))?;
            let x = next("x")?
                .parse()
                .map_err(|_| WireError::Entity("bad x".to_string()))?;
            let y = next("y")?
                .parse()
                .map_err(|_| WireError::Entity("bad y".to_string()))?;
            let z = next("z")?
                .parse()
                .map_err(|_| WireError::Entity("bad z".to_string()))?;
            Ok(Position {
                entity,
                pos: Vec3::new(x, y, z),
            })
        }
    }

    #[test]
    fn header_precedes_entity_lines() {
        let entities = vec![Position {
            entity: 1,
            pos: Vec3::new(1.0, 2.0, 3.0),
        }];
        let text = write(&entities, 42, "arena");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("1 42 arena"));
        assert_eq!(lines.next(), Some("1 1 2 3"));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn header_round_trips_version_seed_and_world_name() {
        let entities: Vec<Position> = vec![];
        let text = write(&entities, 0xDEAD_BEEF, "small world");
        let (header, parsed) = read::<Position>(&text).unwrap();
        assert_eq!(header.version, WIRE_FORMAT_VERSION);
        assert_eq!(header.seed, 0xDEAD_BEEF);
        assert_eq!(header.world_name, "small world");
        assert!(parsed.is_empty());
    }

    #[test]
    fn entities_round_trip_in_order() {
        let entities = vec![
            Position {
                entity: 1,
                pos: Vec3::new(1.0, 2.0, 3.0),
            },
            Position {
                entity: 2,
                pos: Vec3::new(-4.0, 5.0, -6.0),
            },
        ];
        let text = write(&entities, 7, "w");
        let (_, parsed) = read::<Position>(&text).unwrap();
        assert_eq!(parsed, entities);
    }

    #[test]
    fn floats_round_trip_bit_for_bit() {
        let values = [
            0.0_f32,
            -0.0,
            1.0,
            0.1,
            1.0 / 3.0,
            -123.456,
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::EPSILON,
            std::f32::consts::PI,
        ];
        let entities: Vec<Position> = values
            .iter()
            .enumerate()
            .map(|(i, &v)| Position {
                entity: i as u32,
                pos: Vec3::new(v, v, v),
            })
            .collect();

        let text = write(&entities, 1, "w");
        let (_, parsed) = read::<Position>(&text).unwrap();

        for (original, roundtripped) in entities.iter().zip(parsed.iter()) {
            assert_eq!(
                original.pos.x.to_bits(),
                roundtripped.pos.x.to_bits(),
                "{} did not round-trip bit-for-bit",
                original.pos.x
            );
        }
    }

    #[test]
    fn missing_header_is_an_error() {
        assert_eq!(read::<Position>(""), Err(WireError::MissingHeader));
    }

    #[test]
    fn malformed_header_is_an_error() {
        assert_eq!(
            read::<Position>("not-a-number seed world"),
            Err(WireError::Header("missing format version"))
        );
    }

    #[test]
    fn malformed_entity_line_is_an_error() {
        let text = "1 42 arena\nnot enough fields";
        assert!(matches!(read::<Position>(text), Err(WireError::Entity(_))));
    }
}
