//! What a line of a scene says about its shape: its model and its
//! terrain. The render module draws them and the physics module collides
//! with them, so they are neither's — they are here, where both look
//! (DNA, postulate 3: what several modules need goes in a module they all
//! depend on).

use scrap_core::scene::{EntityDesc, Override};
use scrap_core::AssetLink;
use serde::{Deserialize, Serialize};

use crate::terrain::Terrain;

/// `model: "pine_large"` — a model in `assets/` by file stem, or a builtin
/// (`builtin:cone`). A group, or a prefab instance, draws nothing itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelRef(pub AssetLink);

scrap_core::impl_parts! {
    ModelRef => "model", default if |m| m.0.is_empty();
    Terrain => "terrain", fractions ["dunes.sinuosity", "dunes.barchans"];
}

/// A line's shape, read off it.
pub trait GeometryLine {
    fn model(&self) -> AssetLink;
    fn terrain(&self) -> Option<Terrain>;
    fn set_model(&mut self, model: impl Into<AssetLink>);
    /// This line with its model: `.with_model("pine")`.
    fn with_model(self, model: impl Into<AssetLink>) -> Self
    where
        Self: Sized;
}

impl GeometryLine for EntityDesc {
    fn model(&self) -> AssetLink {
        self.part::<ModelRef>().map(|m| m.0).unwrap_or_default()
    }
    fn terrain(&self) -> Option<Terrain> {
        self.part()
    }
    fn set_model(&mut self, model: impl Into<AssetLink>) {
        self.set_part(&ModelRef(model.into()))
    }
    fn with_model(self, model: impl Into<AssetLink>) -> Self {
        self.with(ModelRef(model.into()))
    }
}

/// What an override of a prefab's part says about its shape.
pub trait GeometryOverride {
    fn model(&self) -> Option<AssetLink>;
    fn terrain(&self) -> Option<Terrain>;
}

impl GeometryOverride for Override {
    fn model(&self) -> Option<AssetLink> {
        self.part::<ModelRef>().map(|m| m.0)
    }
    fn terrain(&self) -> Option<Terrain> {
        self.part()
    }
}
