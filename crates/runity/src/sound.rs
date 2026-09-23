//! The sound a line of a scene makes: the sound module's field, `sound`, as
//! a type, and the reading of it off a line ([`SoundLine`]).

use serde::{Deserialize, Serialize};

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
