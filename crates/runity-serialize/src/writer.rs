//! Building a byte stream.

use crate::Serialize;

/// An append-only buffer of encoded values.
///
/// Encoding is little-endian and fixed-width for floats (so bit patterns
/// survive exactly, NaNs included), and LEB128 varints for lengths and
/// integers that are usually small. Nothing is aligned or padded: two runs
/// that write the same values produce byte-identical output, which is what
/// lets a snapshot double as a test fixture and a desync detector.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    /// An empty stream.
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// An empty stream with room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Encode any serializable value.
    pub fn write<T: Serialize + ?Sized>(&mut self, value: &T) -> &mut Self {
        value.serialize(self);
        self
    }

    /// Encode a slice as a length-prefixed sequence.
    pub fn seq<T: Serialize>(&mut self, items: &[T]) -> &mut Self {
        self.varint(items.len() as u64);
        for item in items {
            item.serialize(self);
        }
        self
    }

    /// Encode an iterator whose length is known up front.
    ///
    /// Takes the count separately because `Iterator` alone cannot promise
    /// one; passing a wrong count would corrupt the stream, so it is checked
    /// in debug builds.
    pub fn seq_of<T: Serialize>(
        &mut self,
        count: usize,
        items: impl IntoIterator<Item = T>,
    ) -> &mut Self {
        self.varint(count as u64);
        let mut written = 0;
        for item in items {
            item.serialize(self);
            written += 1;
        }
        debug_assert_eq!(
            written, count,
            "seq_of was promised {count} items but got {written}"
        );
        self
    }

    /// A single byte.
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    /// A little-endian `u16`.
    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A little-endian `u32`.
    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A little-endian `u64`.
    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A signed byte.
    pub fn i8(&mut self, value: i8) -> &mut Self {
        self.u8(value as u8)
    }

    /// A little-endian `i16`.
    pub fn i16(&mut self, value: i16) -> &mut Self {
        self.u16(value as u16)
    }

    /// A little-endian `i32`.
    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.u32(value as u32)
    }

    /// A little-endian `i64`.
    pub fn i64(&mut self, value: i64) -> &mut Self {
        self.u64(value as u64)
    }

    /// A 32-bit float, written as its exact bit pattern.
    pub fn f32(&mut self, value: f32) -> &mut Self {
        self.u32(value.to_bits())
    }

    /// A 64-bit float, written as its exact bit pattern.
    pub fn f64(&mut self, value: f64) -> &mut Self {
        self.u64(value.to_bits())
    }

    /// A boolean as one byte.
    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.u8(u8::from(value))
    }

    /// A LEB128 varint: one byte for values below 128, ten at the very worst.
    pub fn varint(&mut self, mut value: u64) -> &mut Self {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.bytes.push(byte);
                return self;
            }
            self.bytes.push(byte | 0x80);
        }
    }

    /// A signed varint, zigzag-encoded so that small negatives stay small.
    pub fn signed(&mut self, value: i64) -> &mut Self {
        self.varint(((value << 1) ^ (value >> 63)) as u64)
    }

    /// Length-prefixed bytes.
    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.varint(value.len() as u64);
        self.bytes.extend_from_slice(value);
        self
    }

    /// Bytes with no length prefix — for payloads whose extent is known from
    /// elsewhere, like an archive body.
    pub fn raw(&mut self, value: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(value);
        self
    }

    /// A length-prefixed UTF-8 string.
    pub fn str(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }

    /// How many bytes have been written.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Forget everything written, keeping the allocation. Handy for a
    /// per-frame network scratch buffer.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Take the finished stream.
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

impl From<Writer> for Vec<u8> {
    fn from(writer: Writer) -> Vec<u8> {
        writer.bytes
    }
}
