//! IMA ADPCM in a WAV: four bits a sample, a quarter of 16-bit PCM and an
//! eighth of the importer's `f32` — what a sound is cooked into for a
//! download (`scrap cook --platform web`), where a game's few hundred
//! effects decoded would be most of what the page fetches. Pure code both
//! ways: the cook encodes ([`encode_wav`]), the mixer in a browser decodes
//! ([`decode_wav`]) once per sound and keeps it, exactly as long as it was
//! (the file's `fact` says; the last block's padding is cut off, so a loop
//! has no gap).
//!
//! The format is Microsoft's DVI/IMA ADPCM (`WAVE_FORMAT_IMA_ADPCM`,
//! 0x0011): blocks of [`BLOCK`] bytes a channel, each with its first sample
//! and step in a 4-byte header, then nibbles — any WAV reader that knows
//! the format reads it (symphonia's `adpcm` does).

/// Bytes of one channel's share of a block.
pub const BLOCK: usize = 256;

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871,
    5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// One channel's decoder (and encoder) state.
#[derive(Clone, Copy, Default)]
struct State {
    predictor: i32,
    index: i32,
}

impl State {
    fn decode(&mut self, nibble: u8) -> i16 {
        let step = STEPS[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 8 != 0 {
            self.predictor -= diff;
        } else {
            self.predictor += diff;
        }
        self.predictor = self.predictor.clamp(-32768, 32767);
        self.index = (self.index + INDEX[nibble as usize & 15]).clamp(0, 88);
        self.predictor as i16
    }

    fn encode(&mut self, sample: i16) -> u8 {
        let step = STEPS[self.index as usize];
        let mut diff = sample as i32 - self.predictor;
        let mut nibble = 0u8;
        if diff < 0 {
            nibble = 8;
            diff = -diff;
        }
        let mut s = step;
        if diff >= s {
            nibble |= 4;
            diff -= s;
        }
        s >>= 1;
        if diff >= s {
            nibble |= 2;
            diff -= s;
        }
        s >>= 1;
        if diff >= s {
            nibble |= 1;
        }
        // The encoder follows the decoder exactly, so errors never pile up.
        self.decode(nibble);
        nibble
    }
}

fn pcm(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Samples per channel in a block of [`BLOCK`] bytes a channel.
pub const fn samples_per_block() -> usize {
    (BLOCK - 4) * 2 + 1
}

/// Interleaved stereo `samples` (as a sound asset holds them) as an IMA
/// ADPCM WAV — mono when both channels are the same (an imported mono
/// sound, which is most).
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = samples.len() / 2;
    let mono = samples.chunks_exact(2).all(|f| f[0] == f[1]);
    let channels = if mono { 1usize } else { 2 };
    let per = samples_per_block();
    let blocks = frames.div_ceil(per).max(1);
    let channel = |c: usize, i: usize| -> i16 { if i < frames { pcm(samples[i * 2 + c.min(1)]) } else { 0 } };
    let mut data = Vec::with_capacity(blocks * BLOCK * channels);
    for b in 0..blocks {
        let first = b * per;
        let mut states = [State::default(); 2];
        for (c, st) in states.iter_mut().enumerate().take(channels) {
            let s0 = channel(c, first);
            st.predictor = s0 as i32;
            // A step that suits the block's start (the decoder is told it).
            let d = (channel(c, first + 1) as i32 - s0 as i32).abs();
            st.index = STEPS.iter().position(|s| *s >= d).unwrap_or(0).saturating_sub(2) as i32;
            data.extend_from_slice(&s0.to_le_bytes());
            data.push(st.index as u8);
            data.push(0);
        }
        // Then groups of eight samples (four bytes) a channel, in turn.
        let mut i = first + 1;
        while i < first + per {
            for (c, st) in states.iter_mut().enumerate().take(channels) {
                for k in 0..4 {
                    let lo = st.encode(channel(c, i + 2 * k));
                    let hi = st.encode(channel(c, i + 2 * k + 1));
                    data.push(lo | (hi << 4));
                }
            }
            i += 8;
        }
    }
    let block_align = (BLOCK * channels) as u16;
    let mut out = Vec::with_capacity(data.len() + 60);
    let byte_rate = sample_rate as usize * block_align as usize / per;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + 8 + 20 + 8 + 4 + 8 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&20u32.to_le_bytes());
    out.extend_from_slice(&0x0011u16.to_le_bytes());
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(byte_rate as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&(per as u16).to_le_bytes());
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// An [`encode_wav`] file (any IMA ADPCM WAV) back as its sample rate and
/// stereo frames, exactly as many as it says; `None` if it is not one.
pub fn decode_wav(bytes: &[u8]) -> Option<(u32, Vec<[f32; 2]>)> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (mut channels, mut rate, mut align, mut frames, mut data) = (0usize, 0u32, 0usize, None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32_at(at + 4)? as usize;
        let body = at + 8;
        match id {
            b"fmt " => {
                if u16_at(body)? != 0x0011 || u16_at(body + 14)? != 4 {
                    return None;
                }
                channels = u16_at(body + 2)? as usize;
                rate = u32_at(body + 4)?;
                align = u16_at(body + 12)? as usize;
            }
            b"fact" => frames = Some(u32_at(body)? as usize),
            b"data" => data = Some(bytes.get(body..(body + len).min(bytes.len()))?),
            _ => {}
        }
        at = body + len + (len & 1);
    }
    let data = data?;
    if !(1..=2).contains(&channels) || align < 4 * channels {
        return None;
    }
    let per = (align - 4 * channels) * 2 / channels + 1;
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(data.len() / align * per);
    for block in data.chunks(align) {
        if block.len() < 4 * channels {
            break;
        }
        let mut states = [State::default(); 2];
        let mut chans: [Vec<i16>; 2] = [Vec::with_capacity(per), Vec::with_capacity(per)];
        for c in 0..channels {
            let h = &block[c * 4..c * 4 + 4];
            states[c] = State { predictor: i16::from_le_bytes([h[0], h[1]]) as i32, index: (h[2] as i32).clamp(0, 88) };
            chans[c].push(states[c].predictor as i16);
        }
        let mut rest = &block[4 * channels..];
        while rest.len() >= 4 * channels {
            for c in 0..channels {
                for b in &rest[c * 4..c * 4 + 4] {
                    chans[c].push(states[c].decode(b & 15));
                    chans[c].push(states[c].decode(b >> 4));
                }
            }
            rest = &rest[4 * channels..];
        }
        let n = chans[0].len();
        for i in 0..n {
            let l = chans[0][i] as f32 / 32768.0;
            let r = if channels == 2 { chans[1].get(i).copied().unwrap_or(0) as f32 / 32768.0 } else { l };
            out.push([l, r]);
        }
    }
    if let Some(n) = frames {
        out.truncate(n);
    }
    Some((rate, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, stereo: bool) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / 32000.0;
                let l = (t * 440.0 * std::f32::consts::TAU).sin() * 0.6 + (t * 97.0 * std::f32::consts::TAU).sin() * 0.2;
                let r = if stereo { (t * 660.0 * std::f32::consts::TAU).sin() * 0.5 } else { l };
                [l, r]
            })
            .collect()
    }

    /// Signal to noise, in decibels, of `b` against `a`.
    fn snr(a: &[f32], b: &[[f32; 2]]) -> f32 {
        let (mut s, mut n) = (0.0f64, 0.0f64);
        for (i, f) in b.iter().enumerate() {
            for c in 0..2 {
                let x = a[i * 2 + c] as f64;
                s += x * x;
                n += (x - f[c] as f64).powi(2);
            }
        }
        (10.0 * (s / n.max(1e-12)).log10()) as f32
    }

    #[test]
    fn a_sound_comes_back_as_long_and_close_in_a_quarter_of_the_bytes() {
        for stereo in [false, true] {
            let frames = 32000 + 77;
            let samples = tone(frames, stereo);
            let wav = encode_wav(&samples, 32000);
            let (rate, back) = decode_wav(&wav).unwrap();
            assert_eq!(rate, 32000);
            assert_eq!(back.len(), frames, "exactly as long (stereo: {stereo})");
            let quality = snr(&samples, &back);
            assert!(quality > 22.0, "{quality} dB (stereo: {stereo})");
            let pcm16 = frames * 2 * if stereo { 2 } else { 1 };
            assert!(wav.len() * 3 < pcm16, "{} bytes against {pcm16} of 16-bit", wav.len());
        }
        assert!(decode_wav(b"RIFF\0\0\0\0WAVEjunk").is_none());
        assert!(decode_wav(&[1, 2, 3]).is_none());
    }
}
