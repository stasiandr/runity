//! The sound a line of a scene makes: the sound module's field, `sound`, as
//! a type, and the reading of it off a line ([`SoundLine`]).

use serde::{Deserialize, Serialize};

use crate::asset::{Asset, AssetId, AssetKind};

/// A sound's kind in an asset's header: this module's.
pub const SOUND: AssetKind = AssetKind::new(3, "sound");
use crate::library::Library;

#[allow(unused_imports)]
use crate::defaults::*;
use crate::scene::EntityDesc;

/// A sound this entity makes — the radio on the table, the fire's
/// crackle, the music of the menu: Unity's AudioSource. `sound: (clip:
/// "radio", looped: true)` plays the sound asset `radio` from here, from the
/// moment the entity is in the world and switched on, fading with distance
/// from the listener between `near` (1 m, full) and `far` (40 m, silent).
/// `spatial: false` is everywhere at `volume`: music. `pitch` 0.8 plays it
/// slower and lower. `on_start: false`
/// waits for the game to start it. `group` is the mixer group its volume
/// slider is (see [`crate::audio::Audio::set_group_volume`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundSource {
    pub clip: crate::AssetLink,
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub volume: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub looped: bool,
    /// Faster and higher above 1, slower and lower below.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub pitch: f32,
    #[serde(default = "yes_sound", skip_serializing_if = "is_true")]
    pub on_start: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default = "yes_sound", skip_serializing_if = "is_true")]
    pub spatial: bool,
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub near: f32,
    #[serde(default = "forty", skip_serializing_if = "is_forty")]
    pub far: f32,
}

impl Default for SoundSource {
    fn default() -> Self {
        Self {
            clip: crate::AssetLink::default(),
            volume: 1.0,
            looped: false,
            pitch: 1.0,
            on_start: true,
            group: String::new(),
            spatial: true,
            near: 1.0,
            far: 40.0,
        }
    }
}

fn yes_sound() -> bool {
    true
}

fn forty() -> f32 {
    40.0
}

fn is_forty(v: &f32) -> bool {
    *v == 40.0
}

crate::impl_parts! {
    SoundSource => "sound";
}

/// The sound a line of a scene makes, read off it.
pub trait SoundLine {
    fn sound(&self) -> Option<SoundSource>;
}

impl SoundLine for EntityDesc {
    fn sound(&self) -> Option<SoundSource> {
        self.part()
    }
}

/// A sound the entity makes, from its line's `sound`; played by
/// [`crate::audio::Sources`].
#[derive(Debug, Clone, PartialEq)]
pub struct Sounding(pub crate::scene::SoundSource);

/// Decoded audio, ready to hand to the mixer.
///
/// Decoded at import for the same reason meshes are: a game that decodes OGG
/// on the frame it needs a footstep stutters on the footstep. The cost is
/// disk — a minute of stereo is about twenty megabytes — which is why music
/// will eventually want streaming and why sound effects never will.
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SoundAsset {
    pub id: AssetId,
    pub name: String,
    pub sample_rate: u32,
    /// Interleaved stereo. Mono sources are duplicated at import, so the
    /// mixer has one layout and no branch. Empty for a long one, which
    /// keeps its `encoded` bytes instead.
    pub samples: Vec<f32>,
    /// A long sound — music, a wind that blows all level — as its file
    /// was, compressed, played by streaming it: decoded, a ten-minute track
    /// would be two hundred megabytes. Short ones are decoded at import,
    /// so a footstep never waits on a decoder. Empty for those.
    pub encoded: Vec<u8>,
    /// How long it plays.
    pub seconds: f32,
}

/// Sounds longer than this keep their file's compressed bytes and stream.
pub const LONG_SOUND_SECONDS: f32 = 10.0;

impl SoundAsset {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration_seconds(&self) -> f32 {
        if self.samples.is_empty() {
            return self.seconds;
        }
        self.frames() as f32 / self.sample_rate.max(1) as f32
    }
}


impl Asset for SoundAsset {
    fn id(&self) -> AssetId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Sounds out of the library.
pub trait SoundLibrary {
    fn sound(&self, id: AssetId) -> Option<&ArchivedSoundAsset>;
    fn sound_by_name(&self, name: &str) -> Option<&ArchivedSoundAsset>;
    /// The sound a link names — a scene's `sound: (clip: ...)`.
    fn sound_of(&self, link: &crate::AssetLink) -> Option<&ArchivedSoundAsset>;
}

impl SoundLibrary for Library {
    fn sound(&self, id: AssetId) -> Option<&ArchivedSoundAsset> {
        crate::asset::view::<SoundAsset>(self.bytes_of(id, SOUND)?).ok()
    }
    fn sound_by_name(&self, name: &str) -> Option<&ArchivedSoundAsset> {
        crate::asset::view::<SoundAsset>(self.bytes_named(name, SOUND)?).ok()
    }
    fn sound_of(&self, link: &crate::AssetLink) -> Option<&ArchivedSoundAsset> {
        let (id, _) = self.find(link, SOUND)?;
        self.sound(id)
    }
}
