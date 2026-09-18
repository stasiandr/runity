//! What the mixer is supposed to sound like, asserted rather than listened to.

use runity_audio::{
    decode, encode, mixer::Attenuation, noise, saw, shape, sine, square, Envelope, Listener, Mixer,
    Sound, WavError, SAMPLE_RATE,
};
use runity_math::{vec3, Vec3};

/// Loudness of the left and right halves of an interleaved buffer.
fn channels(samples: &[f32]) -> (f32, f32) {
    let left: f32 = samples.iter().step_by(2).map(|s| s * s).sum();
    let right: f32 = samples.iter().skip(1).step_by(2).map(|s| s * s).sum();
    let frames = (samples.len() / 2).max(1) as f32;
    ((left / frames).sqrt(), (right / frames).sqrt())
}

// ------------------------------------------------------------------- sounds

#[test]
fn a_tone_is_the_length_and_loudness_it_claims() {
    let tone = sine(440.0, 0.5, SAMPLE_RATE);
    assert_eq!(tone.frames(), SAMPLE_RATE as usize / 2);
    assert!((tone.duration() - 0.5).abs() < 1e-3);
    assert!((tone.peak() - 1.0).abs() < 0.01);
    // A sine's RMS is its peak over root two, which is a good check that the
    // samples really are a sine and not, say, a square.
    assert!((tone.rms() - 0.707).abs() < 0.01, "rms {}", tone.rms());

    let harsh = square(440.0, 0.1, SAMPLE_RATE);
    assert!(
        (harsh.rms() - 1.0).abs() < 0.01,
        "a square is as loud as it is tall"
    );
    assert!(saw(100.0, 0.1, SAMPLE_RATE).rms() < harsh.rms());
}

#[test]
fn noise_is_reproducible_and_unlike_a_tone() {
    let first = noise(7, 0.1, SAMPLE_RATE);
    let second = noise(7, 0.1, SAMPLE_RATE);
    assert_eq!(first, second, "the same seed is the same hiss");
    assert_ne!(first, noise(8, 0.1, SAMPLE_RATE));
    assert!(first.rms() > 0.4 && first.rms() < 0.7);
}

#[test]
fn an_envelope_rises_and_falls_without_clicking() {
    let envelope = Envelope {
        attack: 0.01,
        decay: 0.02,
        sustain: 0.5,
        release: 0.05,
    };
    let shaped = shape(&sine(440.0, 0.3, SAMPLE_RATE), envelope);

    // Silence at both ends is what stops the click.
    assert!(shaped.samples[0].abs() < 1e-3);
    assert!(shaped.samples[shaped.samples.len() - 1].abs() < 1e-3);
    // And the middle is at the sustain level.
    let middle = shaped.samples[shaped.frames() / 2].abs();
    assert!(middle <= 0.51, "{middle}");
    assert_eq!(envelope.gain(-1.0, 0.3), 0.0);
    assert_eq!(envelope.gain(0.4, 0.3), 0.0, "past the end is silence");
}

#[test]
fn fading_the_edges_removes_the_click() {
    let mut tone = square(200.0, 0.2, SAMPLE_RATE);
    assert!(
        tone.samples[0].abs() > 0.9,
        "a square starts at full deflection"
    );
    tone.fade_edges(0.01);
    assert!(tone.samples[0].abs() < 0.05);
    assert!(tone.samples[tone.samples.len() - 1].abs() < 0.05);
    assert!(
        tone.samples[tone.frames() / 2].abs() > 0.9,
        "the middle is untouched"
    );
}

#[test]
fn a_mono_sound_plays_out_of_both_ears() {
    // The most common first-mixer bug: mono ending up entirely on the left.
    let tone = sine(440.0, 0.01, SAMPLE_RATE);
    let (left, right) = tone.frame(10);
    assert_eq!(left, right);
    assert_ne!(left, 0.0);
}

#[test]
fn sampling_between_frames_interpolates() {
    let sound = Sound::mono(vec![0.0, 1.0], SAMPLE_RATE);
    assert_eq!(sound.frame_at(0.0).0, 0.0);
    assert!((sound.frame_at(0.5).0 - 0.5).abs() < 1e-6);
    assert_eq!(sound.frame_at(1.0).0, 1.0);
    assert_eq!(sound.frame_at(50.0), (0.0, 0.0), "past the end is silence");
    assert_eq!(sound.frame_at(-1.0), (0.0, 0.0));
}

// --------------------------------------------------------------------- wav

#[test]
fn a_sound_survives_a_wav_round_trip() {
    let original = shape(&sine(440.0, 0.25, SAMPLE_RATE), Envelope::default());
    let decoded = decode(&encode(&original)).expect("our own file should decode");

    assert_eq!(decoded.channels, 1);
    assert_eq!(decoded.rate, original.rate);
    assert_eq!(decoded.frames(), original.frames());
    // Sixteen bits, so the samples come back within a quantisation step.
    for (before, after) in original.samples.iter().zip(&decoded.samples) {
        assert!((before - after).abs() < 1e-4, "{before} became {after}");
    }
}

#[test]
fn stereo_survives_too_and_keeps_its_sides_apart() {
    let mut samples = Vec::new();
    for frame in 0..1_000 {
        samples.push((frame as f32 / 1_000.0) * 0.5); // left ramps up
        samples.push(-0.25); // right is constant
    }
    let stereo = Sound::stereo(samples, SAMPLE_RATE);
    let decoded = decode(&encode(&stereo)).unwrap();

    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.frames(), 1_000);
    assert!((decoded.frame(999).0 - 0.4995).abs() < 1e-3);
    assert!((decoded.frame(999).1 + 0.25).abs() < 1e-3);
}

#[test]
fn samples_beyond_the_limit_are_clamped_rather_than_wrapped() {
    // A sample above 1.0 wrapping to a negative integer is the loudest, ugliest
    // bug in audio, and it sounds like broken hardware.
    let loud = Sound::mono(vec![3.0, -3.0, 0.5], SAMPLE_RATE);
    let decoded = decode(&encode(&loud)).unwrap();
    assert!((decoded.samples[0] - 1.0).abs() < 1e-3);
    assert!((decoded.samples[1] + 1.0).abs() < 1e-3);
    assert!((decoded.samples[2] - 0.5).abs() < 1e-3);
}

#[test]
fn rubbish_is_refused_with_a_reason() {
    assert_eq!(decode(&[]), Err(WavError::TooShort));
    assert_eq!(decode(b"NOTAWAVFILE!"), Err(WavError::NotWav));

    let good = encode(&sine(440.0, 0.01, SAMPLE_RATE));
    // Truncated in the middle of the data chunk.
    assert!(matches!(
        decode(&good[..good.len() - 10]),
        Err(WavError::Truncated { .. })
    ));

    // A compressed format we cannot read.
    let mut compressed = good.clone();
    compressed[20] = 17; // ADPCM
    assert!(matches!(
        decode(&compressed),
        Err(WavError::Unsupported { .. })
    ));

    // Every truncation, as a fuzz: none may panic.
    for length in 0..good.len() {
        let _ = decode(&good[..length]);
    }
}

#[test]
fn other_sample_widths_are_read() {
    // Handmade 8-bit and 32-bit-float files, since that is what other tools
    // produce and what a decoder actually has to face.
    let make = |format: u16, bits: u16, data: Vec<u8>| {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&format.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(SAMPLE_RATE * u32::from(bits) / 8).to_le_bytes());
        bytes.extend_from_slice(&(bits / 8).to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        bytes
    };

    let eight = decode(&make(1, 8, vec![128, 255, 0])).unwrap();
    assert!(eight.samples[0].abs() < 0.01, "128 is silence in 8-bit");
    assert!(eight.samples[1] > 0.9 && eight.samples[2] < -0.9);

    let floats: Vec<u8> = [0.25f32, -0.5]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let decoded = decode(&make(3, 32, floats)).unwrap();
    assert_eq!(decoded.samples, vec![0.25, -0.5]);
}

#[test]
fn an_unknown_chunk_is_stepped_over() {
    // Real files carry LIST, fact and whatever else the tool felt like.
    let tone = sine(440.0, 0.01, SAMPLE_RATE);
    let good = encode(&tone);
    let mut with_junk = good[..12].to_vec();
    with_junk.extend_from_slice(b"LIST");
    with_junk.extend_from_slice(&6u32.to_le_bytes());
    with_junk.extend_from_slice(b"INFOxy");
    with_junk.extend_from_slice(&good[12..]);
    let total = (with_junk.len() - 8) as u32;
    with_junk[4..8].copy_from_slice(&total.to_le_bytes());

    let decoded = decode(&with_junk).expect("the junk chunk should be skipped");
    assert_eq!(decoded.frames(), tone.frames());
}

// ------------------------------------------------------------------- mixer

fn mixer_with_tone() -> (Mixer, runity_audio::SoundId) {
    let mut mixer = Mixer::new(SAMPLE_RATE);
    mixer.master = 1.0;
    let tone = mixer.add(sine(440.0, 0.1, SAMPLE_RATE));
    (mixer, tone)
}

#[test]
fn a_played_sound_comes_out_of_the_mixer() {
    let (mut mixer, tone) = mixer_with_tone();
    let mut silence = vec![0.0; 512];
    mixer.render(&mut silence);
    assert_eq!(
        silence.iter().fold(0.0f32, |a, b| a.max(b.abs())),
        0.0,
        "nothing is playing"
    );

    mixer.play(tone);
    assert_eq!(mixer.active(), 1);
    let mut output = vec![0.0; 512];
    mixer.render(&mut output);
    assert!(
        output.iter().any(|sample| sample.abs() > 0.1),
        "something should be audible"
    );
}

#[test]
fn a_sound_finishes_and_frees_its_voice() {
    let (mut mixer, tone) = mixer_with_tone();
    let voice = mixer.play(tone);
    let _ = mixer.render_seconds(0.5);
    assert_eq!(mixer.active(), 0);
    assert!(mixer.voice(voice).is_none(), "the handle is stale now");

    for _ in 0..50 {
        mixer.play(tone);
        let _ = mixer.render_seconds(0.2);
    }
    assert_eq!(mixer.active(), 0);
}

#[test]
fn looping_keeps_going() {
    let (mut mixer, tone) = mixer_with_tone();
    let voice = mixer.play(tone);
    mixer.voice_mut(voice).unwrap().looping = true;

    let recorded = mixer.render_seconds(1.0);
    assert_eq!(mixer.active(), 1, "a loop does not finish");
    assert!(recorded.rms() > 0.3, "rms {}", recorded.rms());
}

#[test]
fn volume_and_pitch_do_what_they_say() {
    let (mut mixer, tone) = mixer_with_tone();

    let quiet = mixer.play(tone);
    mixer.voice_mut(quiet).unwrap().volume = 0.25;
    let quiet_sound = mixer.render_seconds(0.05);

    let loud = mixer.play(tone);
    mixer.voice_mut(loud).unwrap().volume = 1.0;
    let loud_sound = mixer.render_seconds(0.05);
    assert!(loud_sound.rms() > quiet_sound.rms() * 3.0);

    let fast = mixer.play(tone);
    mixer.voice_mut(fast).unwrap().pitch = 2.0;
    let _ = mixer.render_seconds(0.06);
    assert_eq!(
        mixer.active(),
        0,
        "a tenth of a second at double speed is gone by 0.06"
    );
}

#[test]
fn a_sound_to_the_right_is_louder_in_the_right_ear() {
    let (mut mixer, tone) = mixer_with_tone();
    mixer.listener = Listener::look_at(Vec3::ZERO, vec3(0.0, 0.0, -1.0));

    let voice = mixer.play_at(tone, vec3(3.0, 0.0, 0.0));
    mixer.voice_mut(voice).unwrap().looping = true;
    let recorded = mixer.render_seconds(0.05);
    let (left, right) = channels(&recorded.samples);
    assert!(right > left * 2.0, "left {left}, right {right}");

    mixer.stop_all();
    let voice = mixer.play_at(tone, vec3(-3.0, 0.0, 0.0));
    mixer.voice_mut(voice).unwrap().looping = true;
    let recorded = mixer.render_seconds(0.05);
    let (left, right) = channels(&recorded.samples);
    assert!(left > right * 2.0, "left {left}, right {right}");
}

#[test]
fn a_sound_in_front_is_in_both_ears_equally() {
    let (mut mixer, tone) = mixer_with_tone();
    mixer.listener = Listener::look_at(Vec3::ZERO, vec3(0.0, 0.0, -1.0));
    let voice = mixer.play_at(tone, vec3(0.0, 0.0, -3.0));
    mixer.voice_mut(voice).unwrap().looping = true;

    let recorded = mixer.render_seconds(0.05);
    let (left, right) = channels(&recorded.samples);
    assert!(
        (left - right).abs() < left * 0.05,
        "left {left}, right {right}"
    );
}

#[test]
fn walking_away_makes_a_sound_quieter() {
    let loudness_at = |distance: f32| {
        let (mut mixer, tone) = mixer_with_tone();
        let voice = mixer.play_at(tone, vec3(0.0, 0.0, -distance));
        mixer.voice_mut(voice).unwrap().looping = true;
        mixer.render_seconds(0.05).rms()
    };

    let near = loudness_at(2.0);
    let middle = loudness_at(10.0);
    let far = loudness_at(40.0);
    assert!(near > middle && middle > far, "{near} {middle} {far}");
    assert_eq!(
        loudness_at(80.0),
        0.0,
        "past the maximum distance it is silent"
    );
}

#[test]
fn a_sound_out_of_range_still_runs_out_of_time() {
    // Otherwise a sound started far away restarts from the beginning the
    // moment the listener walks into range.
    let (mut mixer, tone) = mixer_with_tone();
    let voice = mixer.play_at(tone, vec3(0.0, 0.0, -500.0));
    assert!(mixer.voice(voice).is_some());
    let _ = mixer.render_seconds(0.2);
    assert!(
        mixer.voice(voice).is_none(),
        "it should have played out unheard"
    );
}

#[test]
fn attenuation_has_the_shape_it_claims() {
    let attenuation = Attenuation {
        reference: 2.0,
        maximum: 50.0,
    };
    assert!(
        (attenuation.gain(0.0) - 1.0).abs() < 1e-3,
        "inside the reference is full volume"
    );
    assert!((attenuation.gain(2.0) - 1.0).abs() < 1e-3);
    assert!(
        attenuation.gain(4.0) < 0.6,
        "twice the reference is much quieter"
    );
    assert_eq!(attenuation.gain(50.0), 0.0);
    assert_eq!(attenuation.gain(1_000.0), 0.0);
    let early = attenuation.gain(2.0) - attenuation.gain(6.0);
    let late = attenuation.gain(20.0) - attenuation.gain(24.0);
    assert!(early > late * 3.0, "early {early}, late {late}");
}

#[test]
fn many_loud_sounds_compress_rather_than_tear() {
    let mut mixer = Mixer::new(SAMPLE_RATE);
    mixer.master = 1.0;
    let tone = mixer.add(sine(440.0, 0.5, SAMPLE_RATE));
    for _ in 0..12 {
        mixer.play(tone);
    }
    let recorded = mixer.render_seconds(0.05);
    assert!(recorded.peak() <= 1.0, "peak {}", recorded.peak());
    assert!(
        recorded.rms() > 0.5,
        "and it is still loud: {}",
        recorded.rms()
    );
}

#[test]
fn the_same_sequence_of_calls_produces_the_same_samples() {
    let render = || {
        let mut mixer = Mixer::new(SAMPLE_RATE);
        mixer.master = 0.9;
        let tone = mixer.add(sine(330.0, 0.2, SAMPLE_RATE));
        let hiss = mixer.add(noise(3, 0.2, SAMPLE_RATE));
        let a = mixer.play_at(tone, vec3(2.0, 0.0, -1.0));
        mixer.voice_mut(a).unwrap().looping = true;
        mixer.play(hiss);
        mixer.render_seconds(0.3)
    };
    assert_eq!(render(), render());
}

#[test]
fn a_paused_voice_neither_sounds_nor_advances() {
    let (mut mixer, tone) = mixer_with_tone();
    let voice = mixer.play(tone);
    mixer.voice_mut(voice).unwrap().paused = true;

    let silence = mixer.render_seconds(0.2);
    assert_eq!(silence.peak(), 0.0);
    assert!(
        mixer.voice(voice).is_some(),
        "a paused sound has not finished"
    );

    mixer.voice_mut(voice).unwrap().paused = false;
    let audible = mixer.render_seconds(0.05);
    assert!(audible.peak() > 0.1, "and it picks up where it was");
}

#[test]
fn a_stale_handle_cannot_touch_a_new_sound() {
    let (mut mixer, tone) = mixer_with_tone();
    let old = mixer.play(tone);
    mixer.stop(old);
    let new = mixer.play(tone);

    assert!(mixer.voice(old).is_none());
    assert!(
        mixer.voice_mut(old).is_none(),
        "the finished handle must not reach the new voice"
    );
    assert!(mixer.voice(new).is_some());
}

#[test]
fn an_empty_sound_does_not_hang_the_mixer() {
    let mut mixer = Mixer::new(SAMPLE_RATE);
    let nothing = mixer.add(Sound::new());
    let voice = mixer.play(nothing);
    let silence = mixer.render_seconds(0.05);
    assert_eq!(silence.peak(), 0.0);
    assert!(mixer.voice(voice).is_none());
}

#[test]
fn time_in_the_mixer_is_counted_in_samples() {
    // Not in wall clock: a headless run has to sound the same as a real one.
    let (mut mixer, _) = mixer_with_tone();
    let _ = mixer.render_seconds(0.5);
    assert_eq!(mixer.rendered(), (SAMPLE_RATE / 2) as u64);
    let _ = mixer.render_seconds(0.5);
    assert_eq!(mixer.rendered(), SAMPLE_RATE as u64);
}
