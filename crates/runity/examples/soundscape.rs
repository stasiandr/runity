//! A walk past the village, recorded as a WAV — the audio layer with no
//! renderer attached.
//!
//! ```text
//! cargo run --release --example soundscape          # writes soundscape.wav
//! ```
//!
//! There is no sound device behind the mixer yet, which is exactly why this
//! example exists: the same bargain the renderer makes lets a headless run
//! produce what it *would* have sounded like, and that file can be listened
//! to, diffed, or asserted about in a test.

use runity::audio::{mixer::Attenuation, noise, saw, shape, sine, square, wav};
use runity::prelude::*;

/// Places that make a noise, and what they sound like.
struct Emitter {
    position: Vec3,
    sound: SoundId,
    /// Seconds between repeats; zero for a continuous loop.
    interval: f32,
    next: f32,
    reach: f32,
}

fn main() -> std::io::Result<()> {
    let mut mixer = Mixer::new(runity::audio::SAMPLE_RATE);
    mixer.master = 0.9;

    // Every sound is generated: a placeholder asset that costs no file.
    let axe = mixer.add(shape(
        &square(90.0, 0.18, mixer.rate()),
        Envelope {
            attack: 0.001,
            decay: 0.05,
            sustain: 0.25,
            release: 0.12,
        },
    ));
    let hammer = mixer.add(shape(
        &saw(160.0, 0.12, mixer.rate()),
        Envelope {
            attack: 0.001,
            decay: 0.03,
            sustain: 0.2,
            release: 0.08,
        },
    ));
    let bell = mixer.add(shape(
        &sine(660.0, 1.2, mixer.rate()),
        Envelope {
            attack: 0.005,
            decay: 0.3,
            sustain: 0.35,
            release: 0.8,
        },
    ));
    let stream = mixer.add({
        let mut water = noise(11, 2.0, mixer.rate());
        // Rolled off and quiet, so it reads as water rather than as static.
        smooth(&mut water.samples, 6);
        water.amplify(0.35);
        water.fade_edges(0.05);
        water
    });

    // The woodcutters are on one side of the path and the workshop on the
    // other, so walking it sweeps the village from ear to ear.
    let mut emitters = vec![
        Emitter {
            position: vec3(-14.0, 0.0, -8.0),
            sound: axe,
            interval: 1.1,
            next: 0.3,
            reach: 40.0,
        },
        Emitter {
            position: vec3(-11.0, 0.0, -10.0),
            sound: axe,
            interval: 1.7,
            next: 0.9,
            reach: 40.0,
        },
        Emitter {
            position: vec3(10.0, 0.0, 9.0),
            sound: hammer,
            interval: 0.6,
            next: 0.1,
            reach: 40.0,
        },
        Emitter {
            position: vec3(14.0, 0.0, 8.0),
            sound: bell,
            interval: 5.0,
            next: 2.5,
            reach: 60.0,
        },
        Emitter {
            position: vec3(26.0, 0.0, 14.0),
            sound: stream,
            interval: 0.0,
            next: 0.0,
            reach: 30.0,
        },
    ];

    // The listener walks from one end of the village to the other.
    let walk = |time: f32| vec3(-25.0 + time * 4.0, 1.6, 0.0);
    let seconds = 12.0;
    let step = 1.0 / 60.0;
    let mut recording = Sound::stereo(Vec::new(), mixer.rate());
    let mut time = 0.0f32;

    while time < seconds {
        let position = walk(time);
        let ahead = walk(time + 0.1);
        mixer.listener = Listener::look_at(position, ahead);

        for emitter in &mut emitters {
            if emitter.interval <= 0.0 {
                // A continuous sound is started once and left looping.
                if emitter.next == 0.0 {
                    let voice = mixer.play_at(emitter.sound, emitter.position);
                    if let Some(voice) = mixer.voice_mut(voice) {
                        voice.looping = true;
                        voice.attenuation = Attenuation {
                            reference: 3.0,
                            maximum: emitter.reach,
                        };
                    }
                    emitter.next = f32::INFINITY;
                }
                continue;
            }
            if time >= emitter.next {
                emitter.next = time + emitter.interval;
                let voice = mixer.play_at(emitter.sound, emitter.position);
                if let Some(voice) = mixer.voice_mut(voice) {
                    voice.attenuation = Attenuation {
                        reference: 2.0,
                        maximum: emitter.reach,
                    };
                    // A little variation, so repeats do not sound mechanical.
                    voice.pitch = 0.92 + ((time * 7.0).sin() * 0.08).abs();
                }
            }
        }

        let frames = (step * mixer.rate() as f32) as usize;
        let start = recording.samples.len();
        recording.samples.resize(start + frames * 2, 0.0);
        mixer.render(&mut recording.samples[start..]);
        time += step;
    }

    // Normalize before writing: a recording that peaks at a fifth of full
    // scale is correct and inaudible, which is not a useful artifact.
    let peak = recording.peak();
    if peak > 0.0 {
        recording.amplify(0.9 / peak);
    }

    let path = std::env::var("RUNITY_SOUND").unwrap_or_else(|_| "soundscape.wav".to_string());
    wav::save(&path, &recording)?;

    // The woodcutters are passed first, on the left; the workshop and the
    // stream come later, on the right. If positional audio works, the
    // recording says so.
    let (early_left, early_right) = balance(&recording, 0.05, 0.35);
    let (late_left, late_right) = balance(&recording, 0.65, 0.95);
    let early_bias = early_left / (early_left + early_right).max(1e-6);
    let late_bias = late_right / (late_left + late_right).max(1e-6);

    println!(
        "wrote {path}: {:.1}s, peak {:.2}",
        recording.duration(),
        recording.peak()
    );
    println!(
        "  passing the woodcutters: {:.0}% in the left ear",
        early_bias * 100.0
    );
    println!(
        "  passing the workshop:    {:.0}% in the right ear",
        late_bias * 100.0
    );

    assert!(
        early_bias > 0.6,
        "the woodcutters should be on the left: {early_bias}"
    );
    assert!(
        late_bias > 0.6,
        "and the workshop on the right: {late_bias}"
    );
    Ok(())
}

/// Loudness of each channel over a fraction of the recording.
fn balance(sound: &Sound, from: f32, to: f32) -> (f32, f32) {
    let frames = sound.frames();
    let range = (frames as f32 * from) as usize..(frames as f32 * to) as usize;
    let (mut left, mut right) = (0.0f32, 0.0f32);
    for frame in range {
        let (l, r) = sound.frame(frame);
        left += l * l;
        right += r * r;
    }
    (left.sqrt(), right.sqrt())
}

/// A cheap low-pass: average each sample with its neighbours.
fn smooth(samples: &mut [f32], passes: usize) {
    for _ in 0..passes {
        for index in 1..samples.len() {
            samples[index] = (samples[index] + samples[index - 1]) * 0.5;
        }
    }
}
