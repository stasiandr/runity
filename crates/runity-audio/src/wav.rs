//! Reading and writing WAV.
//!
//! WAV is the one audio format worth implementing by hand: it is a header and
//! then the samples, with no compression to speak of. That makes it the
//! natural counterpart to the PNG encoder — a headless run can record what it
//! would have sounded like, and a test can listen to its own output.

use crate::sound::Sound;

/// Why a WAV file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavError {
    /// Too short to contain a header.
    TooShort,
    /// Not a RIFF/WAVE file at all.
    NotWav,
    /// A chunk claims to be longer than the file.
    Truncated {
        /// Where the chunk started.
        position: usize,
    },
    /// Compressed, or otherwise not plain PCM.
    Unsupported {
        /// The format tag found.
        format: u16,
    },
    /// A sample width this decoder does not handle.
    UnsupportedBits {
        /// Bits per sample found.
        bits: u16,
    },
    /// No samples.
    NoData,
}

impl core::fmt::Display for WavError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WavError::TooShort => write!(f, "the file is too short to be a WAV"),
            WavError::NotWav => write!(f, "not a RIFF/WAVE file"),
            WavError::Truncated { position } => {
                write!(
                    f,
                    "a chunk at byte {position} runs past the end of the file"
                )
            }
            WavError::Unsupported { format } => {
                write!(f, "format {format} is not plain PCM")
            }
            WavError::UnsupportedBits { bits } => write!(f, "{bits}-bit samples are not supported"),
            WavError::NoData => write!(f, "the file contains no samples"),
        }
    }
}

impl std::error::Error for WavError {}

/// Encode a sound as a 16-bit PCM WAV file.
///
/// Sixteen bits because it is what everything reads, and because the extra
/// range of float samples is only useful while mixing — by the time a sound
/// is a file, it has been mixed.
pub fn encode(sound: &Sound) -> Vec<u8> {
    let channels = sound.channels.max(1);
    let bits = 16u16;
    let block_align = channels * bits / 8;
    let byte_rate = sound.rate * u32::from(block_align);
    let data_length = sound.samples.len() * 2;

    let mut bytes = Vec::with_capacity(44 + data_length);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_length) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");

    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sound.rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());

    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_length as u32).to_le_bytes());
    for sample in &sound.samples {
        // Clamp before converting: a sample above 1.0 would otherwise wrap
        // around to silence-adjacent noise, which sounds like a fault in the
        // speakers rather than in the mix.
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode a PCM WAV file.
///
/// Handles 8, 16, 24 and 32-bit integer samples and 32-bit floats, because
/// those are what tools produce; anything compressed is refused rather than
/// guessed at.
pub fn decode(bytes: &[u8]) -> Result<Sound, WavError> {
    if bytes.len() < 12 {
        return Err(WavError::TooShort);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }

    let mut position = 12;
    let mut channels = 1u16;
    let mut rate = crate::sound::SAMPLE_RATE;
    let mut bits = 16u16;
    let mut format = 1u16;
    let mut data: Option<&[u8]> = None;

    // Chunks in any order, and unknown ones skipped: real files carry LIST,
    // fact and whatever else the tool that made them felt like adding.
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let length = u32::from_le_bytes([
            bytes[position + 4],
            bytes[position + 5],
            bytes[position + 6],
            bytes[position + 7],
        ]) as usize;
        let body = position + 8;
        if body + length > bytes.len() {
            return Err(WavError::Truncated { position });
        }

        match id {
            b"fmt " if length >= 16 => {
                format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
                channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]).max(1);
                rate = u32::from_le_bytes([
                    bytes[body + 4],
                    bytes[body + 5],
                    bytes[body + 6],
                    bytes[body + 7],
                ])
                .max(1);
                bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            }
            b"data" => data = Some(&bytes[body..body + length]),
            _ => {}
        }
        // Chunks are padded to an even length.
        position = body + length + (length & 1);
    }

    // 1 is PCM, 3 is IEEE float, 0xFFFE is "extensible", whose real format is
    // in an extra field we do not read — treating it as PCM is right far more
    // often than not, but guessing is how a decoder gets a reputation.
    if format != 1 && format != 3 {
        return Err(WavError::Unsupported { format });
    }
    let data = data.ok_or(WavError::NoData)?;

    let samples: Vec<f32> = match (format, bits) {
        (1, 8) => data
            .iter()
            .map(|byte| (*byte as f32 - 128.0) / 128.0)
            .collect(),
        (1, 16) => data
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32_768.0)
            .collect(),
        (1, 24) => data
            .chunks_exact(3)
            .map(|triple| {
                let value = i32::from_le_bytes([0, triple[0], triple[1], triple[2]]) >> 8;
                value as f32 / 8_388_608.0
            })
            .collect(),
        (1, 32) => data
            .chunks_exact(4)
            .map(|quad| {
                i32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]) as f32 / 2_147_483_648.0
            })
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|quad| f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
            .collect(),
        _ => return Err(WavError::UnsupportedBits { bits }),
    };

    Ok(Sound {
        samples,
        channels,
        rate,
    })
}

/// Write a sound to a file.
pub fn save(path: impl AsRef<std::path::Path>, sound: &Sound) -> std::io::Result<()> {
    std::fs::write(path, encode(sound))
}

/// Read a sound from a file.
pub fn load(path: impl AsRef<std::path::Path>) -> std::io::Result<Sound> {
    let bytes = std::fs::read(path)?;
    decode(&bytes).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
