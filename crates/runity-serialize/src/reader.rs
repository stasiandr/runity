//! Walking a byte stream.

use crate::{Deserialize, Error, Result};

/// A cursor over encoded bytes.
///
/// Every method is fallible and bounds-checked; a `Reader` never panics on
/// malformed input, because the same code path decodes both a save file the
/// player may have edited and a packet another machine sent.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    version: u32,
}

impl<'a> Reader<'a> {
    /// Read `data` with schema version 0.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            version: 0,
        }
    }

    /// Read `data`, telling nested values which schema version wrote it.
    ///
    /// This is how migration works without a second copy of every struct:
    /// a `Deserialize` impl checks [`Reader::version`] and skips or defaults
    /// the fields that older builds did not write.
    pub fn versioned(data: &'a [u8], version: u32) -> Self {
        Self {
            data,
            position: 0,
            version,
        }
    }

    /// The schema version of the stream being read.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Current byte offset.
    pub fn position(&self) -> usize {
        self.position
    }

    /// How many bytes are left.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Whether the stream is exhausted.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Decode any deserializable value.
    pub fn read<T: Deserialize>(&mut self) -> Result<T> {
        T::deserialize(self)
    }

    /// Decode a length-prefixed sequence.
    pub fn seq<T: Deserialize>(&mut self) -> Result<Vec<T>> {
        let position = self.position;
        let count = self.varint()?;
        if count > self.remaining() as u64 && count > 0 {
            // Every element costs at least the byte that says it is there, so
            // a count larger than the bytes left is a lie. Checking this
            // before allocating keeps a hostile packet from asking for 16 GiB.
            return Err(Error::LengthOutOfRange {
                position,
                length: count,
                available: self.remaining(),
            });
        }
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            items.push(T::deserialize(self)?);
        }
        Ok(items)
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// A little-endian `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// A little-endian `u32`.
    pub fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A little-endian `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// A signed byte.
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    /// A little-endian `i16`.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    /// A little-endian `i32`.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    /// A little-endian `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }

    /// A 32-bit float, restored from its exact bit pattern.
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// A 64-bit float, restored from its exact bit pattern.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// A boolean. Any byte other than 0 or 1 is rejected rather than coerced,
    /// because a stream that disagrees with us here disagrees about more.
    pub fn bool(&mut self) -> Result<bool> {
        let position = self.position;
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::InvalidValue {
                position,
                what: "bool",
            }),
        }
    }

    /// A LEB128 varint.
    pub fn varint(&mut self) -> Result<u64> {
        let start = self.position;
        let mut value: u64 = 0;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            if shift == 63 && byte > 1 {
                // The tenth byte may only carry the single remaining bit.
                return Err(Error::OverlongVarint { position: start });
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(Error::OverlongVarint { position: start });
            }
        }
    }

    /// A zigzag-encoded signed varint.
    pub fn signed(&mut self) -> Result<i64> {
        let raw = self.varint()?;
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    /// Length-prefixed bytes, borrowed from the stream.
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let position = self.position;
        let length = self.varint()?;
        if length > self.remaining() as u64 {
            return Err(Error::LengthOutOfRange {
                position,
                length,
                available: self.remaining(),
            });
        }
        self.take(length as usize)
    }

    /// Exactly `count` bytes with no length prefix.
    pub fn raw(&mut self, count: usize) -> Result<&'a [u8]> {
        self.take(count)
    }

    /// A length-prefixed string, borrowed from the stream.
    pub fn str(&mut self) -> Result<&'a str> {
        let position = self.position;
        let bytes = self.bytes()?;
        core::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8 { position })
    }

    /// A length-prefixed string, copied.
    pub fn string(&mut self) -> Result<String> {
        Ok(self.str()?.to_string())
    }

    /// Skip `count` bytes — for reading past a field this build does not care
    /// about.
    pub fn skip(&mut self, count: usize) -> Result<()> {
        self.take(count).map(|_| ())
    }

    /// Assert the stream is fully consumed.
    ///
    /// Worth calling at the end of a decode: leftover bytes mean the reader
    /// and writer disagree about the schema, and finding that out here beats
    /// finding it out as a mysterious desync three minutes later.
    pub fn finish(self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::TrailingData {
                position: self.position,
                remaining: self.remaining(),
            })
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(Error::UnexpectedEnd {
                position: self.position,
                needed: count,
                available: self.remaining(),
            })?;
        if end > self.data.len() {
            return Err(Error::UnexpectedEnd {
                position: self.position,
                needed: count,
                available: self.remaining(),
            });
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }
}
