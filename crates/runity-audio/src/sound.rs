//! Sound as data: samples, and the ways to make them.
//!
//! A game engine without an artist still needs sounds to test with, and
//! shipping placeholder `.wav` files means shipping files. Generating them is
//! a few lines of arithmetic, costs nothing in the repository, and has the
//! useful property that a test can state exactly what it expects to hear.

use runity_math::Rng;

/// Samples per second. Everything here assumes one rate throughout.
pub const SAMPLE_RATE: u32 = 44_100;

/// A block of audio: interleaved stereo, or mono.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// Samples, in `-1.0..=1.0`. Stereo is interleaved left, right.
    pub samples: Vec<f32>,
    /// 1 for mono, 2 for stereo.
    pub channels: u16,
    /// Samples per second per channel.
    pub rate: u32,
}

impl Sound {
    /// An empty mono sound.
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            channels: 1,
            rate: SAMPLE_RATE,
        }
    }

    /// A mono sound from samples.
    pub fn mono(samples: Vec<f32>, rate: u32) -> Self {
        Self {
            samples,
            channels: 1,
            rate: rate.max(1),
        }
    }

    /// A stereo sound from interleaved samples.
    pub fn stereo(samples: Vec<f32>, rate: u32) -> Self {
        Self {
            samples,
            channels: 2,
            rate: rate.max(1),
        }
    }

    /// How many frames — samples per channel — it holds.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// How long it lasts, in seconds.
    pub fn duration(&self) -> f32 {
        self.frames() as f32 / self.rate as f32
    }

    /// Whether it has no samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// One frame as a left/right pair, silent past the end.
    ///
    /// Mono is returned on both sides rather than on the left: a mono sound
    /// that plays only out of one ear is the most common bug in a first mixer.
    pub fn frame(&self, index: usize) -> (f32, f32) {
        if index >= self.frames() {
            return (0.0, 0.0);
        }
        match self.channels {
            1 => {
                let value = self.samples[index];
                (value, value)
            }
            _ => {
                let base = index * self.channels as usize;
                (self.samples[base], self.samples[base + 1])
            }
        }
    }

    /// Linearly interpolated frame, for playback at a changed pitch.
    pub fn frame_at(&self, position: f32) -> (f32, f32) {
        if position < 0.0 {
            return (0.0, 0.0);
        }
        let index = position.floor() as usize;
        let fraction = position - index as f32;
        let (left, right) = self.frame(index);
        let (next_left, next_right) = self.frame(index + 1);
        (
            left + (next_left - left) * fraction,
            right + (next_right - right) * fraction,
        )
    }

    /// The loudest sample in it.
    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .fold(0.0f32, |worst, sample| worst.max(sample.abs()))
    }

    /// Root mean square — loudness as heard, rather than the worst spike.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.samples.iter().map(|sample| sample * sample).sum();
        (sum / self.samples.len() as f32).sqrt()
    }

    /// Scale every sample.
    pub fn amplify(&mut self, gain: f32) {
        for sample in &mut self.samples {
            *sample *= gain;
        }
    }

    /// Fade in over `seconds` and out over `seconds` at the end.
    ///
    /// A sound that starts or stops mid-wave clicks, which is the first thing
    /// anyone notices about home-made audio.
    pub fn fade_edges(&mut self, seconds: f32) {
        let frames = self.frames();
        let fade = ((seconds.max(0.0) * self.rate as f32) as usize).min(frames / 2);
        if fade == 0 {
            return;
        }
        let channels = self.channels.max(1) as usize;
        for frame in 0..frames {
            let gain = if frame < fade {
                frame as f32 / fade as f32
            } else if frame >= frames - fade {
                (frames - frame) as f32 / fade as f32
            } else {
                continue;
            };
            for channel in 0..channels {
                self.samples[frame * channels + channel] *= gain;
            }
        }
    }
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

/// How a generated tone rises and falls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    /// Seconds to reach full volume.
    pub attack: f32,
    /// Seconds to fall to the sustain level.
    pub decay: f32,
    /// Level held after the decay, from 0 to 1.
    pub sustain: f32,
    /// Seconds to fall to silence at the end.
    pub release: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        // A short plucked shape: audible, and it does not click.
        Self {
            attack: 0.005,
            decay: 0.05,
            sustain: 0.6,
            release: 0.1,
        }
    }
}

impl Envelope {
    /// The gain at a point in a sound of a given length.
    pub fn gain(&self, time: f32, duration: f32) -> f32 {
        if time < 0.0 || time > duration {
            return 0.0;
        }
        let release_starts = (duration - self.release).max(0.0);
        if time >= release_starts && self.release > 0.0 {
            let through = (time - release_starts) / self.release;
            return self.sustain * (1.0 - through).clamp(0.0, 1.0);
        }
        if time < self.attack && self.attack > 0.0 {
            return time / self.attack;
        }
        let after_attack = time - self.attack;
        if after_attack < self.decay && self.decay > 0.0 {
            let through = after_attack / self.decay;
            return 1.0 + (self.sustain - 1.0) * through;
        }
        self.sustain
    }
}

/// A sine wave, the honest test tone.
pub fn sine(frequency: f32, seconds: f32, rate: u32) -> Sound {
    generate(seconds, rate, |time| {
        (time * frequency * core::f32::consts::TAU).sin()
    })
}

/// A square wave: harsher, and much easier to hear at low volume.
pub fn square(frequency: f32, seconds: f32, rate: u32) -> Sound {
    generate(seconds, rate, |time| {
        if (time * frequency).fract() < 0.5 {
            1.0
        } else {
            -1.0
        }
    })
}

/// A sawtooth.
pub fn saw(frequency: f32, seconds: f32, rate: u32) -> Sound {
    generate(seconds, rate, |time| (time * frequency).fract() * 2.0 - 1.0)
}

/// White noise from a named stream, so the same seed gives the same hiss.
pub fn noise(seed: u64, seconds: f32, rate: u32) -> Sound {
    let mut rng = Rng::named(seed, "noise");
    let frames = (seconds.max(0.0) * rate as f32) as usize;
    Sound::mono((0..frames).map(|_| rng.range(-1.0, 1.0)).collect(), rate)
}

/// Shape a sound with an envelope.
pub fn shape(sound: &Sound, envelope: Envelope) -> Sound {
    let duration = sound.duration();
    let channels = sound.channels.max(1) as usize;
    let mut shaped = sound.clone();
    for frame in 0..sound.frames() {
        let time = frame as f32 / sound.rate as f32;
        let gain = envelope.gain(time, duration);
        for channel in 0..channels {
            shaped.samples[frame * channels + channel] *= gain;
        }
    }
    shaped
}

fn generate(seconds: f32, rate: u32, mut wave: impl FnMut(f32) -> f32) -> Sound {
    let rate = rate.max(1);
    let frames = (seconds.max(0.0) * rate as f32) as usize;
    let samples = (0..frames)
        .map(|frame| wave(frame as f32 / rate as f32))
        .collect();
    Sound::mono(samples, rate)
}
