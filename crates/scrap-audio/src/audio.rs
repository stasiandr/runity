//! Sound, on kira.
//!
//! The mixer is kira's; what is here is the part a game touches — playing a
//! decoded asset, placing it in the world, and moving the listener.
//!
//! Sounds are decoded at import, like everything else. A game that decodes
//! an OGG on the frame it needs a footstep stutters on the footstep, and the
//! one place that cost can be paid once is the import.

#[allow(unused_imports)]
use crate::prelude::*;
use glam::Vec3;
use kira::backend::cpal::CpalBackend;
use kira::backend::Backend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
#[cfg(not(target_arch = "wasm32"))]
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, Tween};

use crate::asset::ArchivedSoundAsset;

/// How a sound fades with distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Falloff {
    /// Inside this, a sound is at full volume.
    pub full_volume_distance: f32,
    /// Beyond this, it is silent.
    pub silence_distance: f32,
}

impl Default for Falloff {
    fn default() -> Self {
        Self {
            full_volume_distance: 1.0,
            silence_distance: 40.0,
        }
    }
}

impl Falloff {
    /// Gain in `0..=1` for a distance.
    ///
    /// Inverse-distance rather than linear: sound pressure falls with the
    /// reciprocal of distance, and a linear ramp makes everything sound like
    /// it is on a slider rather than in a place. Normalised so the far end
    /// really does reach zero, because a tail that never quite arrives means
    /// every sound in the world stays mixed in forever.
    pub fn gain(&self, distance: f32) -> f32 {
        let near = self.full_volume_distance.max(0.0001);
        let far = self.silence_distance.max(near + 0.0001);
        if distance <= near {
            return 1.0;
        }
        if distance >= far {
            return 0.0;
        }
        let raw = near / distance;
        let at_far = near / far;
        ((raw - at_far) / (1.0 - at_far)).clamp(0.0, 1.0)
    }
}

/// What a sound is playing as.
pub struct Playing(Handle);

/// Decoded and played from memory, or streamed from its compressed bytes.
enum Handle {
    Decoded(StaticSoundHandle),
    #[cfg(not(target_arch = "wasm32"))]
    Streamed(StreamingSoundHandle<kira::sound::FromFileError>),
}

impl Playing {
    pub fn stop(&mut self) {
        match &mut self.0 {
            Handle::Decoded(h) => h.stop(kira::Tween::default()),
            #[cfg(not(target_arch = "wasm32"))]
            Handle::Streamed(h) => h.stop(kira::Tween::default()),
        }
    }

    /// Faster and higher above 1, slower and lower below, as it plays.
    pub fn set_pitch(&mut self, pitch: f32) {
        let rate = kira::PlaybackRate(pitch.max(0.01) as f64);
        match &mut self.0 {
            Handle::Decoded(h) => h.set_playback_rate(rate, kira::Tween::default()),
            #[cfg(not(target_arch = "wasm32"))]
            Handle::Streamed(h) => h.set_playback_rate(rate, kira::Tween::default()),
        }
    }

    pub fn set_volume(&mut self, gain: f32) {
        let volume = gain_to_decibels(gain);
        match &mut self.0 {
            Handle::Decoded(h) => h.set_volume(volume, kira::Tween::default()),
            #[cfg(not(target_arch = "wasm32"))]
            Handle::Streamed(h) => h.set_volume(volume, kira::Tween::default()),
        }
    }
}

impl std::fmt::Debug for Playing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Playing")
    }
}

/// The mixer, and where the ears are.
///
/// Generic over kira's backend, with the real one as the default, so the
/// only place that knows about the mock is a test. Making this concrete and
/// hiding the mock behind a runtime flag was the first attempt and does not
/// work: the two managers are different types, and pretending otherwise
/// means boxing every call for the sake of a test.
pub struct Audio<B: Backend = CpalBackend> {
    manager: AudioManager<B>,
    listener: Vec3,
    /// Everything is scaled by this, so a game has one volume to expose.
    master: f32,
    /// Mixer groups by name — music, effects, voices, the interface —
    /// each a track of the mixer with its own volume, the sliders of a
    /// settings screen. Unity's Audio Mixer groups. Made the first time a
    /// group is named.
    groups: std::collections::HashMap<String, (TrackHandle, f32)>,
}

impl<B: Backend> Audio<B>
where
    // Debug rather than Display: the mock backend's error type is `()`,
    // which cannot be displayed, and a bound that excludes the test backend
    // would be a bound that excludes testing.
    B::Error: std::fmt::Debug,
    AudioManagerSettings<B>: Default,
{
    /// Open a mixer.
    ///
    /// With the default type parameter this opens the system's audio device.
    /// `Audio::<MockBackend>::new()` opens one with no device at all — for
    /// tests, for headless runs, and for a build server, which never has a
    /// sound card. A game that cannot start without an audio device is a
    /// game that cannot be tested.
    pub fn new() -> Result<Self, String> {
        let manager = AudioManager::<B>::new(AudioManagerSettings::default())
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            manager,
            listener: Vec3::ZERO,
            master: 1.0,
            groups: Default::default(),
        })
    }

    /// Everything's volume, what is playing now included.
    pub fn set_master_volume(&mut self, gain: f32) {
        self.master = gain.clamp(0.0, 1.0);
        self.manager
            .main_track()
            .set_volume(gain_to_decibels(self.master), Tween::default());
    }

    /// One group's volume, what is playing in it now included: the music
    /// slider turns the music down, not the next song.
    pub fn set_group_volume(&mut self, group: &str, gain: f32) -> Result<(), String> {
        let gain = gain.clamp(0.0, 1.0);
        let (track, volume) = self.group(group)?;
        *volume = gain;
        track.set_volume(gain_to_decibels(gain), Tween::default());
        Ok(())
    }

    /// A group's volume; 1 for one never set.
    pub fn group_volume(&self, group: &str) -> f32 {
        self.groups.get(group).map_or(1.0, |(_, v)| *v)
    }

    /// The groups named so far.
    pub fn groups(&self) -> impl Iterator<Item = &str> {
        self.groups.keys().map(String::as_str)
    }

    fn group(&mut self, name: &str) -> Result<&mut (TrackHandle, f32), String> {
        if !self.groups.contains_key(name) {
            let track = self
                .manager
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| e.to_string())?;
            self.groups.insert(name.to_string(), (track, 1.0));
        }
        Ok(self.groups.get_mut(name).expect("just made"))
    }

    /// [`Self::play`] in a mixer group.
    pub fn play_in(
        &mut self,
        group: &str,
        sound: &ArchivedSoundAsset,
        gain: f32,
    ) -> Result<Playing, String> {
        self.start(Some(group), sound, gain, false, 1.0)
    }

    /// Play, in a group or straight to the main track, once or round and
    /// round.
    fn start(
        &mut self,
        group: Option<&str>,
        sound: &ArchivedSoundAsset,
        gain: f32,
        looped: bool,
        pitch: f32,
    ) -> Result<Playing, String> {
        let rate = kira::PlaybackRate(pitch.max(0.01) as f64);
        let volume = gain_to_decibels(gain);
        // The browser has no thread to stream on: a long sound is decoded
        // whole when it starts instead.
        #[cfg(target_arch = "wasm32")]
        if !sound.encoded.is_empty() {
            let mut data =
                StaticSoundData::from_cursor(std::io::Cursor::new(sound.encoded.to_vec()))
                    .map_err(|e| format!("{}: {e}", sound.name))?
                    .volume(volume)
                    .playback_rate(rate);
            if looped {
                data = data.loop_region(0.0..);
            }
            let handle = match group {
                Some(group) => self.group(group)?.0.play(data),
                None => self.manager.play(data),
            }
            .map_err(|e| e.to_string())?;
            return Ok(Playing(Handle::Decoded(handle)));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !sound.encoded.is_empty() {
            let mut data = streamed(sound)?.volume(volume).playback_rate(rate);
            if looped {
                data = data.loop_region(0.0..);
            }
            let handle = match group {
                Some(group) => self.group(group)?.0.play(data),
                None => self.manager.play(data),
            }
            .map_err(|e| e.to_string())?;
            return Ok(Playing(Handle::Streamed(handle)));
        }
        let mut data = to_static(sound).volume(volume).playback_rate(rate);
        if looped {
            data = data.loop_region(0.0..);
        }
        let handle = match group {
            Some(group) => self.group(group)?.0.play(data),
            None => self.manager.play(data),
        }
        .map_err(|e| e.to_string())?;
        Ok(Playing(Handle::Decoded(handle)))
    }

    /// [`Self::play_at`] in a mixer group.
    pub fn play_at_in(
        &mut self,
        group: &str,
        sound: &ArchivedSoundAsset,
        position: Vec3,
        falloff: &Falloff,
    ) -> Result<Option<Playing>, String> {
        let gain = falloff.gain((position - self.listener).length());
        if gain <= 0.0 {
            return Ok(None);
        }
        self.play_in(group, sound, gain).map(Some)
    }

    pub fn master_volume(&self) -> f32 {
        self.master
    }

    /// Move the ears. Everything placed is mixed relative to this.
    pub fn set_listener(&mut self, position: Vec3) {
        self.listener = position;
    }

    pub fn listener(&self) -> Vec3 {
        self.listener
    }

    /// Play a sound at full volume, with no place in the world.
    ///
    /// For music, narration, and anything that is not coming from somewhere.
    pub fn play(&mut self, sound: &ArchivedSoundAsset, gain: f32) -> Result<Playing, String> {
        self.start(None, sound, gain, false, 1.0)
    }

    /// Play a sound somewhere in the world.
    ///
    /// Distance only, not direction: panning wants the listener's
    /// orientation, and a listener that has a position but no facing is the
    /// honest halfway point rather than a pan computed from a guess.
    pub fn play_at(
        &mut self,
        sound: &ArchivedSoundAsset,
        position: Vec3,
        falloff: &Falloff,
    ) -> Result<Option<Playing>, String> {
        let gain = falloff.gain((position - self.listener).length());
        if gain <= 0.0 {
            // Out of range: not started at all. Starting a silent sound and
            // letting it run costs a voice for as long as it lasts, and a
            // busy scene runs out of voices on things nobody can hear.
            return Ok(None);
        }
        self.play(sound, gain).map(Some)
    }
}

/// The world's [`crate::world::Sounding`] entities, played: each started
/// once it is in the world and switched on (when it says `on_start`),
/// turned up and down as the listener comes and goes, and stopped when it
/// is switched off or gone. Switched on again, it starts again, as a Unity
/// AudioSource that plays on awake does. Run once a frame, after the
/// listener has moved.
#[derive(Debug, Default)]
pub struct Sources {
    playing: std::collections::HashMap<hecs::Entity, Voice>,
    /// Asked to play by the game, for the next update.
    wanted: std::collections::HashSet<hecs::Entity>,
}

#[derive(Debug)]
struct Voice {
    /// `None` for a one-shot that was out of earshot when it started.
    sound: Option<Playing>,
    source: crate::scene::SoundSource,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sound` finds a clip by the link the source names.
    pub fn update<'a, B: Backend>(
        &mut self,
        audio: &mut Audio<B>,
        world: &hecs::World,
        sound: impl Fn(&crate::AssetLink) -> Option<&'a ArchivedSoundAsset>,
    ) -> Vec<String>
    where
        B::Error: std::fmt::Debug,
        AudioManagerSettings<B>: Default,
    {
        let mut problems = Vec::new();
        let off = crate::world::inactive_in_hierarchy(world);
        let mut seen = std::collections::HashSet::new();
        for (entity, sounding, placed) in world
            .query::<(
                hecs::Entity,
                &crate::world::Sounding,
                Option<&crate::world::WorldTransform>,
            )>()
            .iter()
        {
            if off.contains(&entity) {
                continue;
            }
            let source = &sounding.0;
            let at = placed.map_or(Vec3::ZERO, |t| t.0.w_axis.truncate());
            let gain = source.volume
                * if source.spatial {
                    Falloff {
                        full_volume_distance: source.near,
                        silence_distance: source.far,
                    }
                    .gain((at - audio.listener).length())
                } else {
                    1.0
                };
            match self.playing.get_mut(&entity) {
                // Another sound while it plays: from the start. Louder,
                // quieter, faster, carrying further — a motion turning it,
                // music hurrying at the end — is the same sound.
                Some(voice) if !same_sound(&voice.source, source) => {
                    if let Some(mut s) = voice.sound.take() {
                        s.stop();
                    }
                    self.playing.remove(&entity);
                }
                Some(voice) => {
                    if let Some(s) = &mut voice.sound {
                        if source.spatial || source.volume != voice.source.volume {
                            s.set_volume(gain);
                        }
                        if source.pitch != voice.source.pitch {
                            s.set_pitch(source.pitch);
                        }
                    }
                    voice.source = source.clone();
                    seen.insert(entity);
                    continue;
                }
                None => {}
            }
            if !source.on_start && !self.wanted.remove(&entity) {
                continue;
            }
            seen.insert(entity);
            // A one-shot nobody can hear is not started; a loop is, quiet,
            // to be heard when the listener comes near.
            let started = if gain <= 0.0 && !source.looped {
                None
            } else {
                let Some(clip) = sound(&source.clip) else {
                    problems.push(format!("sound `{}`: no such sound", source.clip));
                    self.playing.insert(
                        entity,
                        Voice {
                            sound: None,
                            source: source.clone(),
                        },
                    );
                    continue;
                };
                let group = (!source.group.is_empty()).then_some(source.group.as_str());
                match audio.start(group, clip, gain, source.looped, source.pitch) {
                    Ok(playing) => Some(playing),
                    Err(e) => {
                        problems.push(e);
                        None
                    }
                }
            };
            self.playing.insert(
                entity,
                Voice {
                    sound: started,
                    source: source.clone(),
                },
            );
        }
        // Switched off, gone, or no longer a source: quiet, and ready to
        // start again.
        self.playing.retain(|entity, voice| {
            let keep = seen.contains(entity);
            if !keep {
                if let Some(s) = &mut voice.sound {
                    s.stop();
                }
            }
            keep
        });
        problems
    }

    /// Play an entity's sound from the start at the next update — one that
    /// waits (`on_start: false`), or again: Unity's `AudioSource.Play()`.
    pub fn play(&mut self, entity: hecs::Entity) {
        if let Some(mut voice) = self.playing.remove(&entity) {
            if let Some(s) = &mut voice.sound {
                s.stop();
            }
        }
        self.wanted.insert(entity);
    }

    /// Stop it; one that plays on start stays quiet until [`Self::play`].
    pub fn stop(&mut self, entity: hecs::Entity) {
        if let Some(voice) = self.playing.get_mut(&entity) {
            if let Some(s) = &mut voice.sound {
                s.stop();
            }
            voice.sound = None;
        }
        self.wanted.remove(&entity);
    }

    /// How many sources are playing — or would be, heard from here.
    pub fn len(&self) -> usize {
        self.playing.values().filter(|v| v.sound.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether two sources are one sound, whatever its volume and reach.
fn same_sound(a: &crate::scene::SoundSource, b: &crate::scene::SoundSource) -> bool {
    a.clip == b.clip
        && a.looped == b.looped
        && a.group == b.group
        && a.spatial == b.spatial
        && a.on_start == b.on_start
}

/// Linear gain to decibels, which is what a mixer actually works in.
///
/// Silence is not "minus a lot", it is negative infinity — and `log10(0)`
/// gives exactly that, so the special case is only there to say so out loud.
fn gain_to_decibels(gain: f32) -> Decibels {
    if gain <= 0.0 {
        return Decibels(f32::NEG_INFINITY);
    }
    Decibels(20.0 * gain.clamp(0.0, 1.0).log10())
}

/// A long sound's compressed bytes, to be streamed.
#[cfg(not(target_arch = "wasm32"))]
fn streamed(
    sound: &ArchivedSoundAsset,
) -> Result<StreamingSoundData<kira::sound::FromFileError>, String> {
    StreamingSoundData::from_cursor(std::io::Cursor::new(sound.encoded.to_vec()))
        .map_err(|e| format!("{}: {e}", sound.name))
}

/// Turn an archived asset into something kira can play.
fn to_static(sound: &ArchivedSoundAsset) -> StaticSoundData {
    let rate = sound.sample_rate.to_native();
    let frames: Vec<kira::Frame> = sound
        .samples
        .chunks_exact(2)
        .map(|pair| kira::Frame {
            left: pair[0].to_native(),
            right: pair[1].to_native(),
        })
        .collect();
    StaticSoundData {
        sample_rate: rate,
        frames: frames.into(),
        settings: Default::default(),
        slice: None,
    }
}

// The tests sit mid-file, beside what they test; the module's glue follows.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use kira::backend::mock::MockBackend;

    fn silent() -> Audio<MockBackend> {
        Audio::new().expect("a mixer with no device")
    }

    /// One second of a quiet tone, as an asset would arrive.
    fn tone() -> crate::asset::SoundAsset {
        let rate = 8000;
        crate::asset::SoundAsset {
            id: crate::AssetId::from_source("tone", 0),
            name: "tone".into(),
            sample_rate: rate,
            samples: (0..rate)
                .flat_map(|i| {
                    let v = (i as f32 * 0.05).sin() * 0.2;
                    [v, v]
                })
                .collect(),
            encoded: Vec::new(),
            seconds: 1.0,
        }
    }

    fn archived() -> Vec<u8> {
        crate::asset::to_bytes(&tone(), crate::sound::SOUND).unwrap()
    }

    /// A WAV file's bytes, a second of silence at 8 kHz, mono 16-bit.
    fn wav_bytes() -> Vec<u8> {
        let (rate, frames) = (8000u32, 8000u32);
        let data = frames * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        b.resize(b.len() + data as usize, 0);
        b
    }

    #[test]
    fn a_long_sound_streams_from_its_compressed_bytes() {
        let mut audio = silent();
        let long = crate::asset::SoundAsset {
            samples: Vec::new(),
            encoded: wav_bytes(),
            seconds: 1.0,
            ..tone()
        };
        let bytes = crate::asset::to_bytes(&long, crate::sound::SOUND).unwrap();
        let sound = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        let mut music = audio.play_in("music", sound, 0.8).expect("it streams");
        music.set_volume(0.5);
        music.stop();
        let broken = crate::asset::SoundAsset {
            encoded: b"not a sound".to_vec(),
            ..long
        };
        let bytes = crate::asset::to_bytes(&broken, crate::sound::SOUND).unwrap();
        let sound = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        assert!(
            audio.play(sound, 1.0).is_err(),
            "said in words, not a crash"
        );
    }

    #[test]
    fn a_scenes_sources_play_while_they_are_switched_on() {
        use crate::scene::SoundSource;
        use crate::world::{Sounding, WorldTransform};
        let mut audio = silent();
        let bytes = archived();
        let clip = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        let find = |link: &crate::AssetLink| (*link == "tone").then_some(clip);
        let mut world = hecs::World::new();
        let at = |x: f32| WorldTransform(glam::Mat4::from_translation(Vec3::new(x, 0.0, 0.0)));
        let radio = world.spawn((
            Sounding(SoundSource {
                clip: crate::AssetLink::named("tone"),
                looped: true,
                ..Default::default()
            }),
            at(100.0),
            crate::scene::Transform::default(),
        ));
        let bang = world.spawn((
            Sounding(SoundSource {
                clip: crate::AssetLink::named("tone"),
                ..Default::default()
            }),
            at(100.0),
        ));
        let waits = world.spawn((
            Sounding(SoundSource {
                clip: crate::AssetLink::named("tone"),
                on_start: false,
                spatial: false,
                ..Default::default()
            }),
            at(0.0),
        ));
        let mut sources = Sources::new();
        assert!(sources.update(&mut audio, &world, find).is_empty());
        assert_eq!(
            sources.len(),
            1,
            "the far loop runs quiet; the far bang never starts"
        );
        sources.play(waits);
        sources.update(&mut audio, &world, find);
        assert_eq!(sources.len(), 2, "asked to, it plays");
        crate::world::set_active(&mut world, radio, false);
        sources.update(&mut audio, &world, find);
        assert_eq!(sources.len(), 1, "switched off, it stops");
        crate::world::set_active(&mut world, radio, true);
        world.despawn(bang).unwrap();
        sources.update(&mut audio, &world, find);
        assert_eq!(sources.len(), 2, "switched on, it starts again");
        let missing = world.spawn((Sounding(SoundSource {
            clip: crate::AssetLink::named("nope"),
            ..Default::default()
        }),));
        let said = sources.update(&mut audio, &world, find);
        assert!(said.iter().any(|p| p.contains("nope")), "{said:?}");
        let _ = missing;
    }

    #[test]
    fn a_sound_plays_through_a_mixer_with_no_device() {
        let mut audio = silent();
        let bytes = archived();
        let sound = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        let mut playing = audio.play(sound, 1.0).expect("it starts");
        playing.set_volume(0.5);
        playing.stop();
    }

    #[test]
    fn groups_have_their_own_volume_and_play_their_sounds() {
        let mut audio = silent();
        let bytes = archived();
        let sound = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        assert_eq!(audio.group_volume("music"), 1.0, "never set: full");
        let mut song = audio.play_in("music", sound, 1.0).expect("it starts");
        audio.set_group_volume("music", 0.25).unwrap();
        audio.set_group_volume("sfx", 2.0).unwrap();
        assert_eq!(audio.group_volume("music"), 0.25);
        assert_eq!(audio.group_volume("sfx"), 1.0, "clamped");
        let mut names: Vec<&str> = audio.groups().collect();
        names.sort();
        assert_eq!(names, ["music", "sfx"]);
        audio.set_master_volume(0.5);
        assert!(audio
            .play_at_in("sfx", sound, Vec3::ZERO, &Falloff::default())
            .unwrap()
            .is_some());
        song.stop();
    }

    #[test]
    fn a_sound_out_of_earshot_is_never_started() {
        // Starting a silent sound and letting it run costs a voice for as
        // long as it lasts, and a busy scene runs out of voices on things
        // nobody can hear.
        let mut audio = silent();
        let bytes = archived();
        let sound = crate::asset::view::<crate::asset::SoundAsset>(&bytes).unwrap();
        let falloff = Falloff {
            full_volume_distance: 1.0,
            silence_distance: 10.0,
        };
        audio.set_listener(Vec3::ZERO);
        assert!(audio
            .play_at(sound, Vec3::new(0.0, 0.0, 2.0), &falloff)
            .unwrap()
            .is_some());
        assert!(audio
            .play_at(sound, Vec3::new(0.0, 0.0, 500.0), &falloff)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_decoded_asset_knows_how_long_it_is() {
        let sound = tone();
        assert_eq!(sound.frames(), 8000);
        assert!((sound.duration_seconds() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_sound_is_full_volume_up_close_and_silent_far_away() {
        let falloff = Falloff {
            full_volume_distance: 2.0,
            silence_distance: 20.0,
        };
        assert_eq!(falloff.gain(0.0), 1.0);
        assert_eq!(falloff.gain(2.0), 1.0);
        assert_eq!(falloff.gain(20.0), 0.0, "it really reaches zero");
        assert_eq!(falloff.gain(100.0), 0.0);
    }

    #[test]
    fn falloff_is_not_a_straight_line() {
        // Sound pressure falls with the reciprocal of distance. A linear
        // ramp makes everything sound like it is on a slider.
        let falloff = Falloff {
            full_volume_distance: 1.0,
            silence_distance: 101.0,
        };
        let halfway = falloff.gain(51.0);
        assert!(
            halfway < 0.2,
            "halfway in distance should be far quieter than halfway in \
             volume, got {halfway}"
        );
    }

    #[test]
    fn a_falloff_with_nonsense_distances_does_not_divide_by_zero() {
        // A designer typing the same number twice, or zero, should get a
        // usable curve rather than a NaN that silences the whole mix.
        let broken = Falloff {
            full_volume_distance: 0.0,
            silence_distance: 0.0,
        };
        for distance in [0.0, 1.0, 100.0] {
            let gain = broken.gain(distance);
            assert!(gain.is_finite() && (0.0..=1.0).contains(&gain), "{gain}");
        }
    }

    #[test]
    fn silence_is_negative_infinity_and_not_a_very_small_number() {
        // A mixer works in decibels. Clamping silence to, say, -80 dB leaves
        // every stopped sound faintly audible in a quiet scene.
        assert_eq!(gain_to_decibels(0.0).0, f32::NEG_INFINITY);
        assert_eq!(gain_to_decibels(1.0).0, 0.0);
        assert!((gain_to_decibels(0.5).0 - -6.02).abs() < 0.01);
    }
}

/// The sound module's dresser ([`crate::world::Dress`]): the sound a line
/// makes, played by [`Sources`].
pub struct SoundDress;

impl crate::world::Dress for SoundDress {
    fn parts(&self) -> &[&'static str] {
        &["sound"]
    }

    fn dress(
        &mut self,
        line: &crate::scene::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: crate::world::Changed,
        _: &mut Vec<crate::world::Unresolved>,
    ) {
        match line.sound() {
            Some(sound) => {
                let _ = world.insert_one(entity, crate::world::Sounding(sound));
            }
            None => {
                scrap_core::world::take_off::<crate::world::Sounding>(world, entity);
            }
        }
    }
}
