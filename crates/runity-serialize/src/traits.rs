//! `Serialize` / `Deserialize` and the impls for the types an engine actually
//! stores.

use crate::{Error, Reader, Result, Writer};
use runity_math::{Mat3, Mat4, Noise, Quat, Rng, Vec2, Vec3, Vec4};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// A type that can be written to a [`Writer`].
///
/// Encoding is defined by the impl, not derived from memory layout: adding a
/// `#[repr]` or reordering fields in Rust never silently changes the format.
pub trait Serialize {
    /// Append `self` to the stream.
    fn serialize(&self, writer: &mut Writer);
}

/// A type that can be read back from a [`Reader`].
pub trait Deserialize: Sized {
    /// Decode one value, consuming exactly the bytes [`Serialize`] wrote.
    fn deserialize(reader: &mut Reader) -> Result<Self>;
}

/// Encode a single value into a fresh buffer.
pub fn to_bytes<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.serialize(&mut writer);
    writer.finish()
}

/// Decode a single value, requiring the whole slice to be consumed.
pub fn from_bytes<T: Deserialize>(bytes: &[u8]) -> Result<T> {
    let mut reader = Reader::new(bytes);
    let value = T::deserialize(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

// -------------------------------------------------------------- primitives

impl Serialize for u8 {
    fn serialize(&self, writer: &mut Writer) {
        writer.u8(*self);
    }
}

impl Deserialize for u8 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.u8()
    }
}

impl Serialize for i8 {
    fn serialize(&self, writer: &mut Writer) {
        writer.i8(*self);
    }
}

impl Deserialize for i8 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.i8()
    }
}

/// Integers wider than a byte go through varints: entity ids, counts and
/// resource amounts are small almost all of the time, and a network delta
/// full of them is half the size for it. Use [`Writer::u32`] directly when a
/// fixed width matters.
macro_rules! varint_integer {
    ($unsigned:ty, $signed:ty) => {
        impl Serialize for $unsigned {
            fn serialize(&self, writer: &mut Writer) {
                writer.varint(*self as u64);
            }
        }

        impl Deserialize for $unsigned {
            fn deserialize(reader: &mut Reader) -> Result<Self> {
                let position = reader.position();
                let value = reader.varint()?;
                <$unsigned>::try_from(value).map_err(|_| Error::InvalidValue {
                    position,
                    what: stringify!($unsigned),
                })
            }
        }

        impl Serialize for $signed {
            fn serialize(&self, writer: &mut Writer) {
                writer.signed(*self as i64);
            }
        }

        impl Deserialize for $signed {
            fn deserialize(reader: &mut Reader) -> Result<Self> {
                let position = reader.position();
                let value = reader.signed()?;
                <$signed>::try_from(value).map_err(|_| Error::InvalidValue {
                    position,
                    what: stringify!($signed),
                })
            }
        }
    };
}

varint_integer!(u16, i16);
varint_integer!(u32, i32);
varint_integer!(u64, i64);
varint_integer!(usize, isize);

impl Serialize for f32 {
    fn serialize(&self, writer: &mut Writer) {
        writer.f32(*self);
    }
}

impl Deserialize for f32 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.f32()
    }
}

impl Serialize for f64 {
    fn serialize(&self, writer: &mut Writer) {
        writer.f64(*self);
    }
}

impl Deserialize for f64 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.f64()
    }
}

impl Serialize for bool {
    fn serialize(&self, writer: &mut Writer) {
        writer.bool(*self);
    }
}

impl Deserialize for bool {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.bool()
    }
}

impl Serialize for char {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(*self as u64);
    }
}

impl Deserialize for char {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        let raw = reader.varint()?;
        u32::try_from(raw)
            .ok()
            .and_then(char::from_u32)
            .ok_or(Error::InvalidValue {
                position,
                what: "char",
            })
    }
}

impl Serialize for () {
    fn serialize(&self, _writer: &mut Writer) {}
}

impl Deserialize for () {
    fn deserialize(_reader: &mut Reader) -> Result<Self> {
        Ok(())
    }
}

// ------------------------------------------------------------------ strings

impl Serialize for str {
    fn serialize(&self, writer: &mut Writer) {
        writer.str(self);
    }
}

impl Serialize for String {
    fn serialize(&self, writer: &mut Writer) {
        writer.str(self);
    }
}

impl Deserialize for String {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.string()
    }
}

// ---------------------------------------------------------------- wrappers

impl<T: Serialize> Serialize for Option<T> {
    fn serialize(&self, writer: &mut Writer) {
        match self {
            None => {
                writer.u8(0);
            }
            Some(value) => {
                writer.u8(1);
                value.serialize(writer);
            }
        }
    }
}

impl<T: Deserialize> Deserialize for Option<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        match reader.u8()? {
            0 => Ok(None),
            1 => Ok(Some(T::deserialize(reader)?)),
            _ => Err(Error::InvalidValue {
                position,
                what: "Option tag",
            }),
        }
    }
}

impl<T: Serialize, E: Serialize> Serialize for core::result::Result<T, E> {
    fn serialize(&self, writer: &mut Writer) {
        match self {
            Ok(value) => {
                writer.u8(0);
                value.serialize(writer);
            }
            Err(error) => {
                writer.u8(1);
                error.serialize(writer);
            }
        }
    }
}

impl<T: Deserialize, E: Deserialize> Deserialize for core::result::Result<T, E> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        match reader.u8()? {
            0 => Ok(Ok(T::deserialize(reader)?)),
            1 => Ok(Err(E::deserialize(reader)?)),
            _ => Err(Error::InvalidValue {
                position,
                what: "Result tag",
            }),
        }
    }
}

impl<T: Serialize + ?Sized> Serialize for &T {
    fn serialize(&self, writer: &mut Writer) {
        (**self).serialize(writer);
    }
}

impl<T: Serialize + ?Sized> Serialize for Box<T> {
    fn serialize(&self, writer: &mut Writer) {
        (**self).serialize(writer);
    }
}

impl<T: Deserialize> Deserialize for Box<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Box::new(T::deserialize(reader)?))
    }
}

// -------------------------------------------------------------- collections

impl<T: Serialize> Serialize for [T] {
    fn serialize(&self, writer: &mut Writer) {
        writer.seq(self);
    }
}

impl<T: Serialize> Serialize for Vec<T> {
    fn serialize(&self, writer: &mut Writer) {
        writer.seq(self);
    }
}

impl<T: Deserialize> Deserialize for Vec<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        reader.seq()
    }
}

impl<T: Serialize, const N: usize> Serialize for [T; N] {
    fn serialize(&self, writer: &mut Writer) {
        // Fixed-size: no length prefix, the type already says how many.
        for item in self {
            item.serialize(writer);
        }
    }
}

impl<T: Deserialize, const N: usize> Deserialize for [T; N] {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::deserialize(reader)?);
        }
        items.try_into().map_err(|_| Error::InvalidValue {
            position: reader.position(),
            what: "fixed-size array",
        })
    }
}

/// Maps and sets are written in sorted order so that two runs holding the
/// same contents produce the same bytes. A `HashMap`'s iteration order is not
/// stable between runs, and a snapshot that changes for no reason is useless
/// for both diffing and desync detection.
impl<K: Serialize + Ord, V: Serialize> Serialize for BTreeMap<K, V> {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(self.len() as u64);
        for (key, value) in self {
            key.serialize(writer);
            value.serialize(writer);
        }
    }
}

impl<K: Deserialize + Ord, V: Deserialize> Deserialize for BTreeMap<K, V> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let entries: Vec<(K, V)> = reader.seq()?;
        Ok(entries.into_iter().collect())
    }
}

impl<T: Serialize + Ord> Serialize for BTreeSet<T> {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(self.len() as u64);
        for item in self {
            item.serialize(writer);
        }
    }
}

impl<T: Deserialize + Ord> Deserialize for BTreeSet<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let items: Vec<T> = reader.seq()?;
        Ok(items.into_iter().collect())
    }
}

impl<K: Serialize + Ord + core::hash::Hash + Eq, V: Serialize> Serialize for HashMap<K, V> {
    fn serialize(&self, writer: &mut Writer) {
        let mut entries: Vec<(&K, &V)> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        writer.varint(entries.len() as u64);
        for (key, value) in entries {
            key.serialize(writer);
            value.serialize(writer);
        }
    }
}

impl<K: Deserialize + core::hash::Hash + Eq, V: Deserialize> Deserialize for HashMap<K, V> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let entries: Vec<(K, V)> = reader.seq()?;
        Ok(entries.into_iter().collect())
    }
}

impl<T: Serialize + Ord + core::hash::Hash + Eq> Serialize for HashSet<T> {
    fn serialize(&self, writer: &mut Writer) {
        let mut items: Vec<&T> = self.iter().collect();
        items.sort();
        writer.varint(items.len() as u64);
        for item in items {
            item.serialize(writer);
        }
    }
}

impl<T: Deserialize + core::hash::Hash + Eq> Deserialize for HashSet<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let items: Vec<T> = reader.seq()?;
        Ok(items.into_iter().collect())
    }
}

macro_rules! tuple {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($name: Serialize),+> Serialize for ($($name,)+) {
            fn serialize(&self, writer: &mut Writer) {
                let ($($name,)+) = self;
                $($name.serialize(writer);)+
            }
        }

        #[allow(non_snake_case)]
        impl<$($name: Deserialize),+> Deserialize for ($($name,)+) {
            fn deserialize(reader: &mut Reader) -> Result<Self> {
                Ok(($($name::deserialize(reader)?,)+))
            }
        }
    };
}

tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);
tuple!(A, B, C, D, E, F);

// --------------------------------------------------------------- math types

macro_rules! math_vector {
    ($ty:ident, $($field:ident),+) => {
        impl Serialize for $ty {
            fn serialize(&self, writer: &mut Writer) {
                $(writer.f32(self.$field);)+
            }
        }

        impl Deserialize for $ty {
            fn deserialize(reader: &mut Reader) -> Result<Self> {
                Ok($ty { $($field: reader.f32()?),+ })
            }
        }
    };
}

math_vector!(Vec2, x, y);
math_vector!(Vec3, x, y, z);
math_vector!(Vec4, x, y, z, w);
math_vector!(Quat, x, y, z, w);

impl Serialize for Mat3 {
    fn serialize(&self, writer: &mut Writer) {
        self.cols.serialize(writer);
    }
}

impl Deserialize for Mat3 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Mat3 {
            cols: <[Vec3; 3]>::deserialize(reader)?,
        })
    }
}

impl Serialize for Mat4 {
    fn serialize(&self, writer: &mut Writer) {
        self.cols.serialize(writer);
    }
}

impl Deserialize for Mat4 {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Mat4 {
            cols: <[Vec4; 4]>::deserialize(reader)?,
        })
    }
}

/// Generators are saved by their state, not their seed: resuming from the
/// seed would replay rolls the world has already used.
impl Serialize for Rng {
    fn serialize(&self, writer: &mut Writer) {
        let (state, increment) = self.parts();
        writer.u64(state).u64(increment);
    }
}

impl Deserialize for Rng {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Rng::from_parts(reader.u64()?, reader.u64()?))
    }
}

/// A noise field has no state at all — only its seed, which is the whole
/// reason chunk generation can be resumed from nothing.
impl Serialize for Noise {
    fn serialize(&self, writer: &mut Writer) {
        writer.u64(self.seed());
    }
}

impl Deserialize for Noise {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Noise::from_raw_seed(reader.u64()?))
    }
}
