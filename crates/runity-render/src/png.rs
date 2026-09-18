//! A minimal PNG encoder — the escape hatch that makes the renderer testable
//! and screenshot-able without a windowing system, and without an image crate.
//!
//! PNG only *requires* a zlib stream; it does not require that stream to
//! actually compress. We emit stored (uncompressed) deflate blocks, so the
//! encoder is about a hundred lines: CRC-32, Adler-32, and some chunk framing.

use crate::framebuffer::Framebuffer;
use std::fmt;
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
pub(crate) fn zlib_stored(data: &[u8]) -> Vec<u8> {
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

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A decoded image, in the same `0xAARRGGBB` layout as [`Framebuffer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    NotPng,
    Truncated,
    BadCrc { chunk: [u8; 4] },
    Unsupported(&'static str),
    Malformed(&'static str),
    Inflate(crate::inflate::InflateError),
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::NotPng => write!(f, "not a PNG file"),
            PngError::Truncated => write!(f, "PNG file ends mid-chunk"),
            PngError::BadCrc { chunk } => {
                write!(
                    f,
                    "CRC mismatch in chunk {}",
                    String::from_utf8_lossy(chunk)
                )
            }
            PngError::Unsupported(what) => write!(f, "unsupported PNG feature: {what}"),
            PngError::Malformed(what) => write!(f, "malformed PNG: {what}"),
            PngError::Inflate(e) => write!(f, "PNG image data: {e}"),
        }
    }
}

impl std::error::Error for PngError {}

impl From<crate::inflate::InflateError> for PngError {
    fn from(e: crate::inflate::InflateError) -> Self {
        PngError::Inflate(e)
    }
}

/// Paeth predictor (PNG spec, filter type 4).
#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i16, b as i16, c as i16);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Undo the per-scanline filter. `stride` is the distance in bytes to the
/// pixel on the left.
fn unfilter(filter: u8, row: &mut [u8], previous: &[u8], stride: usize) -> Result<(), PngError> {
    match filter {
        0 => {}
        1 => {
            for i in stride..row.len() {
                row[i] = row[i].wrapping_add(row[i - stride]);
            }
        }
        2 => {
            for i in 0..row.len() {
                row[i] = row[i].wrapping_add(previous[i]);
            }
        }
        3 => {
            for i in 0..row.len() {
                let left = if i >= stride {
                    row[i - stride] as u16
                } else {
                    0
                };
                let up = previous[i] as u16;
                row[i] = row[i].wrapping_add(((left + up) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let left = if i >= stride { row[i - stride] } else { 0 };
                let up = previous[i];
                let up_left = if i >= stride { previous[i - stride] } else { 0 };
                row[i] = row[i].wrapping_add(paeth(left, up, up_left));
            }
        }
        _ => return Err(PngError::Malformed("unknown scanline filter")),
    }
    Ok(())
}

/// Decode an 8-bit PNG: greyscale, RGB, palette, or either with alpha.
///
/// Interlaced and 16-bit images are rejected rather than half-handled.
pub fn decode_png(bytes: &[u8]) -> Result<Image, PngError> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err(PngError::NotPng);
    }

    let mut at = 8;
    let mut header: Option<(usize, usize, u8, u8)> = None;
    let mut palette: Vec<u32> = Vec::new();
    let mut compressed: Vec<u8> = Vec::new();

    while at + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let end = at.checked_add(12 + length).ok_or(PngError::Truncated)?;
        if end > bytes.len() {
            return Err(PngError::Truncated);
        }
        let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        let data = &bytes[at + 8..at + 8 + length];
        let expected = u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap());
        if crc32(&bytes[at + 4..end - 4]) != expected {
            return Err(PngError::BadCrc { chunk: kind });
        }

        match &kind {
            b"IHDR" => {
                if data.len() < 13 {
                    return Err(PngError::Malformed("short IHDR"));
                }
                let width = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                let height = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
                let (bit_depth, color_type, interlace) = (data[8], data[9], data[12]);
                if width == 0 || height == 0 {
                    return Err(PngError::Malformed("zero-sized image"));
                }
                if bit_depth != 8 {
                    return Err(PngError::Unsupported("only 8 bits per channel"));
                }
                if interlace != 0 {
                    return Err(PngError::Unsupported("interlaced images"));
                }
                header = Some((width, height, color_type, bit_depth));
            }
            b"PLTE" => {
                palette = data
                    .chunks_exact(3)
                    .map(|c| {
                        0xff00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
                    })
                    .collect();
            }
            b"IDAT" => compressed.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at = end;
    }

    let (width, height, color_type, _) = header.ok_or(PngError::Malformed("no IHDR"))?;
    let channels: usize = match color_type {
        0 => 1, // greyscale
        2 => 3, // RGB
        3 => 1, // palette index
        4 => 2, // greyscale + alpha
        6 => 4, // RGBA
        _ => return Err(PngError::Unsupported("unknown color type")),
    };
    if color_type == 3 && palette.is_empty() {
        return Err(PngError::Malformed("palette image without a PLTE chunk"));
    }

    let raw = crate::inflate::zlib_decompress(&compressed)?;
    let row_bytes = width * channels;
    if raw.len() < height * (row_bytes + 1) {
        return Err(PngError::Malformed(
            "image data is shorter than the header claims",
        ));
    }

    let mut pixels = Vec::with_capacity(width * height);
    let mut previous = vec![0u8; row_bytes];
    let mut row = vec![0u8; row_bytes];
    for y in 0..height {
        let start = y * (row_bytes + 1);
        let filter = raw[start];
        row.copy_from_slice(&raw[start + 1..start + 1 + row_bytes]);
        unfilter(filter, &mut row, &previous, channels)?;

        for pixel in row.chunks_exact(channels) {
            let argb = match color_type {
                0 => {
                    let v = pixel[0] as u32;
                    0xff00_0000 | (v << 16) | (v << 8) | v
                }
                2 => {
                    0xff00_0000
                        | ((pixel[0] as u32) << 16)
                        | ((pixel[1] as u32) << 8)
                        | pixel[2] as u32
                }
                3 => *palette
                    .get(pixel[0] as usize)
                    .ok_or(PngError::Malformed("palette index out of range"))?,
                4 => {
                    let v = pixel[0] as u32;
                    ((pixel[1] as u32) << 24) | (v << 16) | (v << 8) | v
                }
                _ => {
                    ((pixel[3] as u32) << 24)
                        | ((pixel[0] as u32) << 16)
                        | ((pixel[1] as u32) << 8)
                        | pixel[2] as u32
                }
            };
            pixels.push(argb);
        }
        std::mem::swap(&mut previous, &mut row);
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Read and decode a PNG file.
pub fn load_png(path: impl AsRef<Path>) -> io::Result<Image> {
    let bytes = std::fs::read(path)?;
    decode_png(&bytes).map_err(io::Error::other)
}

impl Image {
    /// Copy this image into a framebuffer of the same size.
    pub fn to_framebuffer(&self) -> Framebuffer {
        let mut fb = Framebuffer::new(self.width, self.height);
        fb.pixels_mut().copy_from_slice(&self.pixels);
        fb
    }
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

    #[test]
    fn our_own_png_round_trips_through_the_decoder() {
        let pixels: Vec<u32> = (0..12 * 7u32)
            .map(|i| 0xff00_0000 | (i * 0x0003_0507))
            .collect();
        let encoded = encode_png(12, 7, &pixels);
        let image = decode_png(&encoded).expect("our own file decodes");
        assert_eq!((image.width, image.height), (12, 7));
        // The encoder writes RGB, so alpha comes back opaque.
        let expected: Vec<u32> = pixels.iter().map(|p| p | 0xff00_0000).collect();
        assert_eq!(image.pixels, expected);
    }

    /// Files written by a real zlib, with every scanline filter in turn — none
    /// of which our own encoder ever produces.
    #[test]
    fn a_compressed_rgba_png_decodes() {
        let image = decode_png(include_bytes!("../tests/data/fixture_rgba.png")).expect("decodes");
        assert_eq!((image.width, image.height), (16, 16));
        for y in 0..16u32 {
            for x in 0..16u32 {
                let expected = (((255 - x * 4) & 0xff) << 24)
                    | ((x * 16) << 16)
                    | ((y * 16) << 8)
                    | ((x ^ y) * 8);
                assert_eq!(
                    image.pixels[(y * 16 + x) as usize],
                    expected,
                    "pixel ({x},{y}) came out wrong — a scanline filter is misapplied"
                );
            }
        }
    }

    #[test]
    fn greyscale_and_palette_pngs_decode() {
        let grey = decode_png(include_bytes!("../tests/data/fixture_gray.png")).expect("decodes");
        for y in 0..8u32 {
            for x in 0..8u32 {
                let v = (x * 32 + y) & 0xff;
                assert_eq!(
                    grey.pixels[(y * 8 + x) as usize],
                    0xff00_0000 | (v << 16) | (v << 8) | v
                );
            }
        }

        let palette =
            decode_png(include_bytes!("../tests/data/fixture_palette.png")).expect("decodes");
        const TABLE: [u32; 4] = [0xff_ff0000, 0xff_00ff00, 0xff_0000ff, 0xff_ffff00];
        for y in 0..8usize {
            for x in 0..8usize {
                assert_eq!(palette.pixels[y * 8 + x], TABLE[(x + y) % 4]);
            }
        }
    }

    #[test]
    fn broken_files_are_rejected_with_a_reason() {
        assert_eq!(decode_png(b"not a png at all"), Err(PngError::NotPng));

        let good = encode_png(2, 2, &[0; 4]);
        assert_eq!(decode_png(&good[..20]), Err(PngError::Truncated));

        // Flip a byte inside IHDR: the chunk CRC must catch it.
        let mut corrupt = good.clone();
        corrupt[18] ^= 0xff;
        assert_eq!(
            decode_png(&corrupt),
            Err(PngError::BadCrc { chunk: *b"IHDR" })
        );
    }

    #[test]
    fn a_decoded_image_becomes_a_framebuffer() {
        let image = decode_png(include_bytes!("../tests/data/fixture_gray.png")).expect("decodes");
        let fb = image.to_framebuffer();
        assert_eq!((fb.width(), fb.height()), (8, 8));
        assert_eq!(fb.pixels(), image.pixels);
    }
}
