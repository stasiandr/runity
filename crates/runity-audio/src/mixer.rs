//! Playing several sounds at once, and placing them in the world.
//!
//! The mixer owes the rest of the engine one thing: given how much time has
//! passed, produce the samples that should have been heard. It does not know
//! about sound cards, and that is deliberate — the same mixer feeds a real
//! device, a file recorded during a headless run, and a test that asserts the
//! footsteps got quieter as the villager walked away.

use runity_math::Vec3;

use crate::sound::Sound;

/// A sound held by the mixer, ready to be played.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundId(usize);

/// A sound that is currently playing.
///
/// Carries a generation, so holding on to the handle of a footstep that
/// finished long ago cannot turn the volume down on somebody else's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId {
    index: usize,
    generation: u32,
}

/// Where the ears are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    /// Where they are.
    pub position: Vec3,
    /// Which way they face.
    pub forward: Vec3,
    /// Which way is to their right, for deciding which ear a sound is in.
    pub right: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
        }
    }
}

impl Listener {
    /// Face the listener from a camera's position and target.
    pub fn look_at(position: Vec3, target: Vec3) -> Self {
        let forward = (target - position).normalized();
        let right = forward.cross(Vec3::Y).normalized();
        Self {
            position,
            forward,
            right,
        }
    }
}

/// How a sound fades with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attenuation {
    /// Distance at which a sound is at full volume.
    pub reference: f32,
    /// Distance past which it is silent.
    pub maximum: f32,
}

impl Default for Attenuation {
    fn default() -> Self {
        Self {
            reference: 2.0,
            maximum: 60.0,
        }
    }
}

impl Attenuation {
    /// Volume at a distance, from 1 down to 0.
    ///
    /// Inverse distance rather than linear: sound really does fall away
    /// quickly at first and then slowly, and a linear fade is the thing that
    /// makes game audio sound like a volume slider being dragged.
    pub fn gain(&self, distance: f32) -> f32 {
        if distance >= self.maximum {
            return 0.0;
        }
        let reference = self.reference.max(1e-3);
        let near = (reference / distance.max(reference)).clamp(0.0, 1.0);
        // Taper the last stretch to zero so a sound does not cut off audibly
        // as it passes the maximum.
        let span = (self.maximum - reference).max(1e-3);
        let taper = (1.0 - (distance - reference).max(0.0) / span).clamp(0.0, 1.0);
        near * taper
    }
}

/// One playing sound.
#[derive(Clone, Debug)]
pub struct Voice {
    sound: SoundId,
    /// Playback position, in frames of the source sound.
    position: f64,
    generation: u32,
    /// How loud, before distance is taken into account.
    pub volume: f32,
    /// Playback rate. 2.0 is an octave up and half as long.
    pub pitch: f32,
    /// Stereo placement for a sound with no position: -1 left, 1 right.
    pub pan: f32,
    /// Whether it starts again when it ends.
    pub looping: bool,
    /// Whether it is temporarily silent and not advancing.
    pub paused: bool,
    /// Where it is in the world, if it is anywhere.
    pub emitter: Option<Vec3>,
    /// How it fades with distance.
    pub attenuation: Attenuation,
    finished: bool,
}

impl Voice {
    /// Whether the sound has played out.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// How far through it is, in seconds of the source.
    pub fn elapsed(&self, rate: u32) -> f32 {
        self.position as f32 / rate.max(1) as f32
    }
}

/// Holds the sounds, the voices and the listener.
#[derive(Debug)]
pub struct Mixer {
    sounds: Vec<Sound>,
    voices: Vec<Voice>,
    generation: u32,
    /// Overall volume.
    pub master: f32,
    /// Where the ears are.
    pub listener: Listener,
    rate: u32,
    rendered: u64,
}

impl Mixer {
    /// A mixer at a sample rate.
    pub fn new(rate: u32) -> Self {
        Self {
            sounds: Vec::new(),
            voices: Vec::new(),
            generation: 1,
            master: 0.8,
            listener: Listener::default(),
            rate: rate.max(1),
            rendered: 0,
        }
    }

    /// The sample rate everything is mixed at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How many frames have been produced since the mixer was made.
    pub fn rendered(&self) -> u64 {
        self.rendered
    }

    /// Hand a sound to the mixer, getting back a handle to play it with.
    pub fn add(&mut self, sound: Sound) -> SoundId {
        self.sounds.push(sound);
        SoundId(self.sounds.len() - 1)
    }

    /// A sound the mixer holds.
    pub fn sound(&self, id: SoundId) -> Option<&Sound> {
        self.sounds.get(id.0)
    }

    /// Start a sound with no position — music, or a click.
    pub fn play(&mut self, sound: SoundId) -> VoiceId {
        self.start(sound, None)
    }

    /// Start a sound somewhere in the world.
    pub fn play_at(&mut self, sound: SoundId, position: Vec3) -> VoiceId {
        self.start(sound, Some(position))
    }

    fn start(&mut self, sound: SoundId, emitter: Option<Vec3>) -> VoiceId {
        let voice = Voice {
            sound,
            position: 0.0,
            generation: self.generation,
            volume: 1.0,
            pitch: 1.0,
            pan: 0.0,
            looping: false,
            paused: false,
            emitter,
            attenuation: Attenuation::default(),
            finished: false,
        };
        self.generation = self.generation.wrapping_add(1).max(1);

        // Reuse a finished slot rather than growing forever: footsteps are the
        // most common sound in a game and the most numerous.
        if let Some(index) = self.voices.iter().position(|voice| voice.finished) {
            let generation = voice.generation;
            self.voices[index] = voice;
            return VoiceId { index, generation };
        }
        self.voices.push(voice);
        VoiceId {
            index: self.voices.len() - 1,
            generation: self.generation.wrapping_sub(1).max(1),
        }
    }

    /// Adjust a playing sound, if it is still playing.
    pub fn voice_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        match self.voices.get_mut(id.index) {
            Some(voice) if voice.generation == id.generation && !voice.finished => Some(voice),
            _ => None,
        }
    }

    /// Look at a playing sound.
    pub fn voice(&self, id: VoiceId) -> Option<&Voice> {
        match self.voices.get(id.index) {
            Some(voice) if voice.generation == id.generation && !voice.finished => Some(voice),
            _ => None,
        }
    }

    /// Stop one sound.
    pub fn stop(&mut self, id: VoiceId) {
        if let Some(voice) = self.voice_mut(id) {
            voice.finished = true;
        }
    }

    /// Stop everything.
    pub fn stop_all(&mut self) {
        for voice in &mut self.voices {
            voice.finished = true;
        }
    }

    /// How many sounds are playing.
    pub fn active(&self) -> usize {
        self.voices.iter().filter(|voice| !voice.finished).count()
    }

    /// Produce interleaved stereo samples, replacing whatever is in `output`.
    ///
    /// The buffer's length decides how much time passes, which is what a
    /// sound device asks for and what a recording wants: time in the mixer is
    /// counted in samples produced, never in wall clock, so a headless run
    /// sounds the same as a real one.
    pub fn render(&mut self, output: &mut [f32]) {
        for sample in output.iter_mut() {
            *sample = 0.0;
        }
        let frames = output.len() / 2;
        let listener = self.listener;
        let master = self.master;

        for voice in &mut self.voices {
            if voice.finished || voice.paused {
                continue;
            }
            let Some(sound) = self.sounds.get(voice.sound.0) else {
                voice.finished = true;
                continue;
            };
            if sound.is_empty() {
                voice.finished = true;
                continue;
            }

            // Distance and direction are taken once per buffer rather than per
            // sample: a buffer is a few milliseconds, and nothing moves far
            // enough in that to be worth the arithmetic.
            let (gain, pan) = match voice.emitter {
                None => (1.0, voice.pan),
                Some(emitter) => {
                    let offset = emitter - listener.position;
                    let distance = offset.length();
                    let gain = voice.attenuation.gain(distance);
                    let pan = if distance > 1e-4 {
                        listener.right.dot(offset / distance)
                    } else {
                        0.0
                    };
                    (gain, pan)
                }
            };
            if gain <= 0.0 {
                // Silent, but still advancing: a sound that is inaudible now
                // must not restart when the listener walks back into range.
                voice.position += frames as f64 * f64::from(voice.pitch.max(0.0));
                Self::wrap(voice, sound);
                continue;
            }

            // Equal-power panning: a sound in the middle is as loud as one
            // fully to one side, which constant-gain panning gets wrong by
            // three decibels.
            let angle = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * core::f32::consts::PI;
            let (left_gain, right_gain) = (angle.cos(), angle.sin());
            let step = f64::from(voice.pitch.max(0.0));
            let volume = voice.volume * gain * master;

            for frame in 0..frames {
                let (left, right) = sound.frame_at(voice.position as f32);
                output[frame * 2] += left * volume * left_gain;
                output[frame * 2 + 1] += right * volume * right_gain;
                voice.position += step;
                if !Self::wrap(voice, sound) {
                    break;
                }
            }
        }

        // Soft clipping, so a loud moment compresses instead of tearing. Hard
        // clipping sounds like a broken speaker; this sounds like a loud game.
        for sample in output.iter_mut() {
            *sample = soft_clip(*sample);
        }
        self.rendered += frames as u64;
    }

    /// Render a stretch of time into a sound, for recording a headless run.
    pub fn render_seconds(&mut self, seconds: f32) -> Sound {
        let frames = (seconds.max(0.0) * self.rate as f32) as usize;
        let mut samples = vec![0.0; frames * 2];
        self.render(&mut samples);
        Sound::stereo(samples, self.rate)
    }

    /// Advance a voice past the end of its sound, looping or finishing.
    /// Returns whether it is still playing.
    fn wrap(voice: &mut Voice, sound: &Sound) -> bool {
        let frames = sound.frames() as f64;
        if voice.position < frames {
            return true;
        }
        if voice.looping && frames > 0.0 {
            voice.position %= frames;
            true
        } else {
            voice.finished = true;
            false
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new(crate::sound::SAMPLE_RATE)
    }
}

/// Squash samples toward the limit instead of chopping them off at it.
fn soft_clip(sample: f32) -> f32 {
    const KNEE: f32 = 0.7;
    let magnitude = sample.abs();
    if magnitude <= KNEE {
        return sample;
    }
    let over = magnitude - KNEE;
    let squashed = KNEE + (1.0 - KNEE) * (over / (over + (1.0 - KNEE)));
    squashed.min(1.0) * sample.signum()
}
