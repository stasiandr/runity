//! A minimal PNG encoder — the escape hatch that makes the renderer testable
//! and screenshot-able without a windowing system, and without an image crate.
//!
//! PNG only *requires* a zlib stream; it does not require that stream to
//! actually compress. We emit stored (uncompressed) deflate blocks, so the
//! encoder is about a hundred lines: CRC-32, Adler-32, and some chunk framing.

use crate::framebuffer::Framebuffer;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
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

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in bytes {
        c = CRC_TABLE[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for byte in chunk {
            a += *byte as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Wrap raw bytes in a zlib stream made of stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // CM=8 (deflate), CINFO=7 (32K window), FLEVEL=0; 0x7801 is divisible by 31.
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    let mut chunks = data.chunks(0xFFFF).peekable();
    while let Some(block) = chunks.next() {
        let final_block = chunks.peek().is_none();
        out.push(if final_block { 1 } else { 0 });
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Encode `0xAARRGGBB` pixels as an 8-bit RGB PNG.
pub fn encode_png(width: usize, height: usize, pixels: &[u32]) -> Vec<u8> {
    assert_eq!(
        pixels.len(),
        width * height,
        "pixel count must match the image size"
    );

    // Scanlines, each prefixed with filter type 0 (None).
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for row in pixels.chunks(width) {
        raw.push(0);
        for p in row {
            raw.push((p >> 16) as u8);
            raw.push((p >> 8) as u8);
            raw.push(*p as u8);
        }
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits, truecolor RGB, no interlace

    let mut out = Vec::with_capacity(raw.len() + 1024);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Write a framebuffer to a PNG file.
pub fn save_png(path: impl AsRef<Path>, fb: &Framebuffer) -> io::Result<()> {
    let bytes = encode_png(fb.width(), fb.height(), fb.pixels());
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(&bytes)?;
    file.flush()
}

/// Write a framebuffer as binary PPM (P6) — even simpler, handy for piping.
pub fn save_ppm(path: impl AsRef<Path>, fb: &Framebuffer) -> io::Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    write!(file, "P6\n{} {}\n255\n", fb.width(), fb.height())?;
    let mut row = Vec::with_capacity(fb.width() * 3);
    for line in fb.pixels().chunks(fb.width()) {
        row.clear();
        for p in line {
            row.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        file.write_all(&row)?;
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_reference_vector() {
        // The canonical CRC-32 of "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler32_matches_the_reference_vector() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_has_a_valid_header_and_chunk_layout() {
        let png = encode_png(
            2,
            2,
            &[0xff_ff_00_00, 0xff_00_ff_00, 0xff_00_00_ff, 0xff_ff_ff_ff],
        );
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");

        // Walk the chunks and verify every CRC.
        let mut at = 8;
        let mut kinds = Vec::new();
        while at + 8 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let body = &png[at + 4..at + 8 + len];
            let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            assert_eq!(crc32(body), crc, "bad CRC on chunk {:?}", &body[..4]);
            kinds.push(String::from_utf8_lossy(&body[..4]).to_string());
            at += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
        assert_eq!(at, png.len(), "no trailing bytes");
    }

    #[test]
    fn stored_blocks_span_more_than_one_deflate_block() {
        // 40k pixels * 3 bytes + filters is well past the 65535-byte block cap.
        let png = encode_png(200, 200, &vec![0xff_12_34_56; 40_000]);
        assert!(png.len() > 65_535);
    }
}
