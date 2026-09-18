//! The properties the rest of the engine relies on: what goes in comes out,
//! the bytes are the same every time, and nothing hostile gets through.

use runity_math::{Mat4, Quat, Rng, Vec2, Vec3, Vec4};
use runity_serialize::{
    checksum, from_bytes, serializable, serializable_enum, to_bytes, Archive, Deserialize, Error,
    Reader, Result, Serialize, Writer, HEADER_LEN,
};
use std::collections::{BTreeMap, HashMap, HashSet};

const SAVE: [u8; 4] = *b"SAVE";
const SNAP: [u8; 4] = *b"SNAP";

#[derive(Debug, PartialEq, Clone)]
struct Villager {
    name: String,
    age: u32,
    position: Vec3,
    hunger: f32,
    carrying: Option<(u8, u32)>,
    friends: Vec<u32>,
}

serializable!(Villager {
    name,
    age,
    position,
    hunger,
    carrying,
    friends
});

#[derive(Debug, PartialEq, Clone, Copy)]
enum Job {
    Idle,
    Woodcutter,
    Mason,
    Smith,
}

serializable_enum!(Job {
    0 => Idle,
    1 => Woodcutter,
    7 => Mason,
    9 => Smith,
});

fn villager() -> Villager {
    Villager {
        name: "Мирослава".to_string(),
        age: 34,
        position: Vec3 {
            x: -12.5,
            y: 0.25,
            z: 300.125,
        },
        hunger: 0.37,
        carrying: Some((2, 17)),
        friends: vec![1, 4, 9, 900_001],
    }
}

#[test]
fn structs_survive_a_round_trip() {
    let value = villager();
    let bytes = to_bytes(&value);
    assert_eq!(from_bytes::<Villager>(&bytes).unwrap(), value);
}

#[test]
fn primitives_survive_their_extremes() {
    assert_eq!(from_bytes::<u64>(&to_bytes(&u64::MAX)).unwrap(), u64::MAX);
    assert_eq!(from_bytes::<u64>(&to_bytes(&0u64)).unwrap(), 0);
    assert_eq!(from_bytes::<i64>(&to_bytes(&i64::MIN)).unwrap(), i64::MIN);
    assert_eq!(from_bytes::<i64>(&to_bytes(&i64::MAX)).unwrap(), i64::MAX);
    assert_eq!(from_bytes::<i32>(&to_bytes(&-1i32)).unwrap(), -1);
    assert_eq!(from_bytes::<u8>(&to_bytes(&255u8)).unwrap(), 255);
    assert_eq!(from_bytes::<i8>(&to_bytes(&i8::MIN)).unwrap(), i8::MIN);
    assert_eq!(from_bytes::<char>(&to_bytes(&'ы')).unwrap(), 'ы');
    assert_eq!(from_bytes::<String>(&to_bytes(&String::new())).unwrap(), "");
    assert!(from_bytes::<bool>(&to_bytes(&true)).unwrap());
}

#[test]
fn floats_keep_their_exact_bits() {
    // Tone mapping and physics both produce values where the last bit
    // matters for reproducibility; a save that rounds them is a save that
    // desyncs.
    for value in [
        0.1f32,
        -0.0,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        let back = from_bytes::<f32>(&to_bytes(&value)).unwrap();
        assert_eq!(back.to_bits(), value.to_bits(), "{value}");
    }
    let nan = f32::from_bits(0x7fc0_1234);
    assert_eq!(
        from_bytes::<f32>(&to_bytes(&nan)).unwrap().to_bits(),
        0x7fc0_1234
    );
    assert_eq!(
        from_bytes::<f64>(&to_bytes(&-0.0f64)).unwrap().to_bits(),
        (-0.0f64).to_bits()
    );
}

#[test]
fn varints_are_as_small_as_promised() {
    assert_eq!(to_bytes(&0u32).len(), 1);
    assert_eq!(to_bytes(&127u32).len(), 1);
    assert_eq!(to_bytes(&128u32).len(), 2);
    assert_eq!(to_bytes(&16_383u32).len(), 2);
    assert_eq!(to_bytes(&16_384u32).len(), 3);
    assert_eq!(to_bytes(&u64::MAX).len(), 10);
    // Zigzag keeps small negatives small, which plain two's complement
    // would not: -1 as a raw varint would take ten bytes.
    assert_eq!(to_bytes(&-1i32).len(), 1);
    assert_eq!(to_bytes(&-64i32).len(), 1);
    assert_eq!(to_bytes(&i64::MIN).len(), 10);
}

#[test]
fn every_varint_value_round_trips() {
    let mut rng = Rng::named(1, "varint");
    for _ in 0..5_000 {
        // Cover every length class, not just the small ones.
        let bits = rng.below(65);
        let value = if bits == 64 {
            u64::MAX
        } else {
            rng.next_u64() >> (63 - bits)
        };
        let mut writer = Writer::new();
        writer.varint(value);
        let bytes = writer.finish();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.varint().unwrap(), value);
        assert_eq!(reader.remaining(), 0);
    }
}

#[test]
fn signed_varints_round_trip_across_the_whole_range() {
    let mut rng = Rng::named(2, "signed");
    for _ in 0..5_000 {
        let value = rng.next_u64() as i64 >> rng.below(64);
        let mut writer = Writer::new();
        writer.signed(value);
        let mut reader = Reader::new(writer.as_bytes());
        assert_eq!(reader.signed().unwrap(), value);
    }
}

#[test]
fn collections_and_wrappers_nest() {
    /// A deliberately awkward nesting: the point of the test is that the
    /// generic impls compose without special cases.
    type Stockpiles = Vec<Option<BTreeMap<String, Vec<(u32, f32)>>>>;

    let value: Stockpiles = vec![
        None,
        Some(BTreeMap::new()),
        Some(
            [
                ("wood".to_string(), vec![(1u32, 0.5f32), (2, 1.5)]),
                ("stone".to_string(), vec![]),
            ]
            .into_iter()
            .collect(),
        ),
    ];
    assert_eq!(
        from_bytes::<Vec<Option<BTreeMap<String, Vec<(u32, f32)>>>>>(&to_bytes(&value)).unwrap(),
        value
    );

    let empty: Vec<u32> = Vec::new();
    assert_eq!(to_bytes(&empty), vec![0]);
    assert_eq!(from_bytes::<Vec<u32>>(&[0]).unwrap(), empty);

    let array = [1u32, 2, 3, 4];
    // A fixed-size array needs no length prefix: the type carries it.
    assert_eq!(to_bytes(&array).len(), 4);
    assert_eq!(from_bytes::<[u32; 4]>(&to_bytes(&array)).unwrap(), array);

    let result: core::result::Result<u32, String> = Err("no wood".into());
    assert_eq!(
        from_bytes::<core::result::Result<u32, String>>(&to_bytes(&result)).unwrap(),
        result
    );
}

#[test]
fn enum_tags_are_explicit_and_unknown_ones_are_rejected() {
    for job in [Job::Idle, Job::Woodcutter, Job::Mason, Job::Smith] {
        assert_eq!(from_bytes::<Job>(&to_bytes(&job)).unwrap(), job);
    }
    // The tag is what the macro says, not the declaration index.
    assert_eq!(to_bytes(&Job::Mason), vec![7]);
    assert!(matches!(
        from_bytes::<Job>(&[3]),
        Err(Error::InvalidValue { .. })
    ));
}

#[test]
fn math_types_round_trip() {
    let mut rng = Rng::named(3, "math");
    for _ in 0..100 {
        let v3 = rng.unit_vector() * rng.range(-100.0, 100.0);
        assert_eq!(from_bytes::<Vec3>(&to_bytes(&v3)).unwrap(), v3);

        let v2 = Vec2 {
            x: rng.range(-9.0, 9.0),
            y: rng.range(-9.0, 9.0),
        };
        assert_eq!(from_bytes::<Vec2>(&to_bytes(&v2)).unwrap(), v2);

        let v4 = Vec4 {
            x: v3.x,
            y: v3.y,
            z: v3.z,
            w: rng.next_f32(),
        };
        assert_eq!(from_bytes::<Vec4>(&to_bytes(&v4)).unwrap(), v4);

        let q = Quat::from_axis_angle(rng.unit_vector(), rng.range(-3.0, 3.0));
        assert_eq!(from_bytes::<Quat>(&to_bytes(&q)).unwrap(), q);

        let m = Mat4::look_at(v3, Vec3::ZERO, Vec3::Y);
        assert_eq!(from_bytes::<Mat4>(&to_bytes(&m)).unwrap(), m);
    }
}

#[test]
fn hash_maps_encode_in_a_stable_order() {
    // The same contents inserted in different orders must give the same
    // bytes, or every snapshot comparison becomes a coin flip.
    let mut a: HashMap<u32, String> = HashMap::new();
    let mut b: HashMap<u32, String> = HashMap::new();
    for key in [5u32, 1, 9, 3, 7, 2] {
        a.insert(key, format!("v{key}"));
    }
    for key in [9u32, 7, 3, 2, 1, 5] {
        b.insert(key, format!("v{key}"));
    }
    assert_eq!(to_bytes(&a), to_bytes(&b));
    assert_eq!(
        from_bytes::<HashMap<u32, String>>(&to_bytes(&a)).unwrap(),
        a
    );

    let set_a: HashSet<u32> = [4u32, 1, 8].into_iter().collect();
    let set_b: HashSet<u32> = [8u32, 4, 1].into_iter().collect();
    assert_eq!(to_bytes(&set_a), to_bytes(&set_b));
    assert_eq!(
        from_bytes::<HashSet<u32>>(&to_bytes(&set_a)).unwrap(),
        set_a
    );
}

#[test]
fn encoding_is_reproducible() {
    let value = villager();
    let first = to_bytes(&value);
    for _ in 0..10 {
        assert_eq!(to_bytes(&value), first);
    }
    assert_eq!(
        Archive::encode_value(SAVE, 3, &value),
        Archive::encode_value(SAVE, 3, &value)
    );
}

#[test]
fn an_archive_carries_its_kind_and_version() {
    let value = villager();
    let bytes = Archive::encode_value(SAVE, 4, &value);

    let header = Archive::peek(&bytes).unwrap();
    assert_eq!(header.kind, SAVE);
    assert_eq!(header.kind_str(), Some("SAVE"));
    assert_eq!(header.version, 4);
    assert_eq!(header.length as usize, bytes.len() - HEADER_LEN);
    assert_eq!(header.checksum, checksum(&bytes[HEADER_LEN..]));

    assert_eq!(
        Archive::read_value::<Villager>(&bytes, SAVE, 4).unwrap(),
        value
    );
    // Reading it as something else, or with an older build, must fail.
    assert!(matches!(
        Archive::read_value::<Villager>(&bytes, SNAP, 4),
        Err(Error::UnexpectedKind { .. })
    ));
    assert!(matches!(
        Archive::read_value::<Villager>(&bytes, SAVE, 3),
        Err(Error::UnsupportedVersion {
            found: 4,
            supported: 3
        })
    ));
}

#[test]
fn the_reader_knows_which_version_wrote_the_stream() {
    // A field added in v2: old saves have to load anyway.
    #[derive(Debug, PartialEq)]
    struct Town {
        residents: u32,
        /// Added in version 2.
        market_level: u32,
    }

    impl Serialize for Town {
        fn serialize(&self, writer: &mut Writer) {
            writer.write(&self.residents).write(&self.market_level);
        }
    }

    impl Deserialize for Town {
        fn deserialize(reader: &mut Reader) -> Result<Self> {
            let residents = reader.read()?;
            let market_level = if reader.version() >= 2 {
                reader.read()?
            } else {
                0
            };
            Ok(Town {
                residents,
                market_level,
            })
        }
    }

    let old = Archive::write(SAVE, 1, |w| {
        w.write(&40u32);
    });
    let loaded = Archive::read_value::<Town>(&old, SAVE, 2).unwrap();
    assert_eq!(
        loaded,
        Town {
            residents: 40,
            market_level: 0
        }
    );

    let new = Archive::encode_value(
        SAVE,
        2,
        &Town {
            residents: 40,
            market_level: 3,
        },
    );
    assert_eq!(
        Archive::read_value::<Town>(&new, SAVE, 2).unwrap(),
        Town {
            residents: 40,
            market_level: 3
        }
    );
}

#[test]
fn corruption_is_caught_by_the_checksum() {
    let bytes = Archive::encode_value(SAVE, 1, &villager());
    for index in HEADER_LEN..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[index] ^= 0x01;
        match Archive::decode(&damaged) {
            Err(Error::Corrupt { .. }) => {}
            other => panic!("byte {index} flipped but decode said {other:?}"),
        }
    }
}

#[test]
fn a_damaged_header_is_reported_precisely() {
    let bytes = Archive::encode_value(SAVE, 1, &villager());

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        Archive::decode(&bad_magic),
        Err(Error::BadMagic { .. })
    ));

    let mut bad_format = bytes.clone();
    bad_format[4] = 99;
    assert!(matches!(
        Archive::decode(&bad_format),
        Err(Error::UnsupportedFormat { .. })
    ));

    let mut bad_length = bytes.clone();
    bad_length[14] = 0xff;
    bad_length[15] = 0xff;
    assert!(matches!(
        Archive::decode(&bad_length),
        Err(Error::LengthOutOfRange { .. })
    ));

    assert!(matches!(
        Archive::decode(&[]),
        Err(Error::UnexpectedEnd { .. })
    ));
    assert!(matches!(
        Archive::decode(b"RNTY"),
        Err(Error::UnexpectedEnd { .. })
    ));
}

#[test]
fn truncation_anywhere_is_an_error_and_never_a_panic() {
    let bytes = Archive::encode_value(SAVE, 1, &villager());
    for length in 0..bytes.len() {
        let result = Archive::read_value::<Villager>(&bytes[..length], SAVE, 1);
        assert!(
            result.is_err(),
            "prefix of {length} byte(s) decoded successfully"
        );
    }
    // Same for a bare payload with no header to catch it.
    let payload = to_bytes(&villager());
    for length in 0..payload.len() {
        assert!(
            from_bytes::<Villager>(&payload[..length]).is_err(),
            "{length}"
        );
    }
}

#[test]
fn leftover_bytes_are_reported() {
    let mut bytes = to_bytes(&42u32);
    bytes.push(0);
    assert!(matches!(
        from_bytes::<u32>(&bytes),
        Err(Error::TrailingData { remaining: 1, .. })
    ));
}

#[test]
fn a_hostile_length_does_not_allocate_the_world() {
    // A sequence header claiming four billion elements, followed by nothing.
    let mut writer = Writer::new();
    writer.varint(4_000_000_000);
    let bytes = writer.finish();
    assert!(matches!(
        from_bytes::<Vec<u64>>(&bytes),
        Err(Error::LengthOutOfRange { .. })
    ));

    let mut writer = Writer::new();
    writer.varint(u64::MAX);
    writer.raw(b"short");
    assert!(matches!(
        from_bytes::<String>(writer.as_bytes()),
        Err(Error::LengthOutOfRange { .. })
    ));
}

#[test]
fn malformed_values_are_rejected_rather_than_coerced() {
    assert!(matches!(
        from_bytes::<bool>(&[2]),
        Err(Error::InvalidValue { what: "bool", .. })
    ));
    assert!(matches!(
        from_bytes::<Option<u8>>(&[5, 0]),
        Err(Error::InvalidValue { .. })
    ));

    // Eleven continuation bytes cannot be a u64.
    let overlong = [0x80u8; 11];
    assert!(matches!(
        from_bytes::<u64>(&overlong),
        Err(Error::OverlongVarint { .. })
    ));

    // A varint that fits in u64 but not in the target type.
    let mut writer = Writer::new();
    writer.varint(70_000);
    assert!(matches!(
        from_bytes::<u16>(writer.as_bytes()),
        Err(Error::InvalidValue { .. })
    ));

    // Invalid UTF-8 in a string field.
    let mut writer = Writer::new();
    writer.bytes(&[0xff, 0xfe]);
    assert!(matches!(
        from_bytes::<String>(writer.as_bytes()),
        Err(Error::InvalidUtf8 { .. })
    ));

    // A surrogate is not a char.
    let mut writer = Writer::new();
    writer.varint(0xD800);
    assert!(matches!(
        from_bytes::<char>(writer.as_bytes()),
        Err(Error::InvalidValue { .. })
    ));
}

#[test]
fn random_bytes_never_panic_the_decoder() {
    // The network path decodes whatever arrives. Fuzz it lightly: the only
    // acceptable outcomes are a value or an error.
    let mut rng = Rng::named(7, "fuzz");
    for _ in 0..20_000 {
        let length = rng.below(48) as usize;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(rng.below(256) as u8);
        }
        let _ = from_bytes::<Villager>(&bytes);
        let _ = from_bytes::<Vec<(u32, String)>>(&bytes);
        let _ = from_bytes::<HashMap<String, Vec3>>(&bytes);
        let _ = from_bytes::<Job>(&bytes);
        let _ = Archive::read_value::<Villager>(&bytes, SAVE, 1);
    }
}

#[test]
fn mutating_a_valid_stream_never_panics_either() {
    // Bit flips inside an otherwise well-formed save: closer to real
    // corruption than uniform noise, and much more likely to hit a decoder
    // path that assumes the previous field was sane.
    let original = to_bytes(&villager());
    let mut rng = Rng::named(8, "bitflip");
    for _ in 0..20_000 {
        let mut bytes = original.clone();
        let flips = 1 + rng.below(3) as usize;
        for _ in 0..flips {
            let index = rng.below(bytes.len() as u32) as usize;
            bytes[index] ^= 1 << rng.below(8);
        }
        let _ = from_bytes::<Villager>(&bytes);
    }
}

#[test]
fn a_big_world_round_trips() {
    let mut rng = Rng::named(9, "world");
    let villagers: Vec<Villager> = (0..5_000)
        .map(|i| Villager {
            name: format!("villager {i}"),
            age: rng.below(90),
            position: rng.unit_vector() * rng.range(0.0, 500.0),
            hunger: rng.next_f32(),
            carrying: if rng.chance(0.4) {
                Some((rng.below(4) as u8, rng.below(64)))
            } else {
                None
            },
            friends: (0..rng.below(6)).map(|_| rng.below(5_000)).collect(),
        })
        .collect();

    let bytes = Archive::encode_value(SNAP, 1, &villagers);
    let loaded: Vec<Villager> = Archive::read_value(&bytes, SNAP, 1).unwrap();
    assert_eq!(loaded, villagers);
    // And re-encoding the decoded world gives the identical file: that is
    // the property a determinism test actually checks.
    assert_eq!(Archive::encode_value(SNAP, 1, &loaded), bytes);
}

#[test]
fn a_writer_can_be_reused_without_leaking_the_last_frame() {
    let mut writer = Writer::new();
    writer.write(&1u32);
    let first = writer.as_bytes().to_vec();
    writer.clear();
    assert!(writer.is_empty());
    writer.write(&1u32);
    assert_eq!(writer.as_bytes(), first.as_slice());
}

#[test]
fn an_empty_payload_is_a_valid_archive() {
    let bytes = Archive::encode(SAVE, 0, &[]);
    assert_eq!(bytes.len(), HEADER_LEN);
    let (header, reader) = Archive::decode(&bytes).unwrap();
    assert_eq!(header.length, 0);
    assert!(reader.is_empty());
    reader.finish().unwrap();
}

#[test]
fn errors_say_where_and_what() {
    let message = from_bytes::<Villager>(&[3, b'a', b'b'])
        .unwrap_err()
        .to_string();
    assert!(message.contains("byte"), "{message}");

    let bad = Archive::decode(b"XXXX000000000000000000")
        .unwrap_err()
        .to_string();
    assert!(bad.contains("magic"), "{bad}");
}
