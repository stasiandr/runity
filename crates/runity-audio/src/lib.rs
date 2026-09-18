//! Sound for runity: generating it, mixing it, placing it, and writing it out.
//!
//! The mixer knows nothing about sound cards. That is the same decision the
//! renderer makes about windows, and it buys the same thing: a headless run
//! can record exactly what it would have sounded like, and a test can assert
//! that the footsteps got quieter as the villager walked away — without a
//! device, a driver, or anyone listening.
//!
//! What is missing is the last step: handing those samples to the operating
//! system. That is three more platform backends' worth of FFI, and until it
//! exists this crate is honest about being the half that can be tested.

#![forbid(unsafe_code)]

pub mod mixer;
pub mod sound;
pub mod wav;

pub use mixer::{Attenuation, Listener, Mixer, SoundId, Voice, VoiceId};
pub use sound::{noise, saw, shape, sine, square, Envelope, Sound, SAMPLE_RATE};
pub use wav::{decode, encode, load, save, WavError};
