//! The bare core of runity (DNA, postulate 3; docs/modules.md): what every
//! module agrees on and nothing has an alternative to — the world and its
//! stable IDs, the transform and the hierarchy, the registry of components,
//! scenes and prefabs, assets and their reload, saves, input as device
//! events, and the seam modules dress a spawned entity through.
//!
//! No GPU, no physics, no sound, no network: those are modules, and a
//! headless server, a test or an agent runs on this alone. A line's
//! fields that belong to a module are kept here as their text
//! ([`parts`]), so a scene opens and saves byte for byte whichever modules
//! a build has.

pub use glam;
pub use hecs;
pub use ron;

pub mod asset;
pub mod components;
pub mod crash;
#[doc(hidden)]
pub mod defaults;
pub mod edit;
pub mod id;
pub mod input;
pub mod layers;
pub mod library;
pub mod links;
pub mod merge;
pub mod module;
pub mod parts;
pub mod perf;
pub mod player_prefs;
pub mod prefab;
pub mod project;
pub mod ron_edit;
#[doc(hidden)]
pub mod ron_text;
pub mod save;
pub mod scene;
pub mod shape;
pub mod spelling;
pub mod strings;
pub mod time;
pub mod timers;
pub mod tuned;
pub mod wind;
pub mod world;

pub use asset::{AssetError, AssetId};
pub use components::{ComponentProblem, Components};
pub use edit::History;
pub use id::{EntityId, EntityRef};
pub use input::{Input, InputEvent, Key, MouseButton};
pub use library::{Library, Reloaded};
pub use links::{AssetLink, MaterialLink, ModelLink, PrefabLink, SceneLink, SoundLink, TextureLink};
pub use perf::{FrameSummary, FrameTimes};
pub use prefab::{Instanced, Prefabs};
pub use project::{Project, ProjectError};
pub use scene::{EntityDesc, Scene, Transform};
pub use time::{Time, TimeSettings};
pub use tuned::Tuned;
