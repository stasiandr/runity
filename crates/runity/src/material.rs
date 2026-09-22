//! What a surface is made of — which here is very little, on purpose.
//!
//! `docs/design/07-look.md` asks for flat shading, no specular, colour
//! instead of material: wood is not a wood texture, it is a flat brown on a
//! log-shaped thing. A material is therefore a colour and a choice of whether
//! the light reaches it at all.
//!
//! It is a type rather than a bare colour because the two shading modes have
//! to be distinguishable at a glance in a scene file, and because everything
//! that gets added later — a texture, a roughness, an emissive — has an
//! obvious place to go without changing every call site.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Whether the sun touches a surface.
///
/// Archivable as well as serde-serializable: a material is both something a
/// scene spells out in text and something the library stores as a compiled
/// asset, and those have to be the same type or the two paths drift.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub enum Shading {
    /// Lit by the sun and the hemisphere ambient. Everything in the world.
    #[default]
    Lit,
    /// Drawn at its own colour, with no light and no fog.
    ///
    /// For things that are not surfaces: gizmos, debug overlays, and the
    /// embers in `07-look.md`, which are a light rather than something lit.
    Unlit,
}

/// A surface.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct Material {
    /// Linear RGB, not sRGB. Lighting happens in linear and only the final
    /// write is encoded, so a colour picked off a screen has to be converted
    /// before it lands here — [`Material::from_srgb`] does that.
    pub base_color: [f32; 3],
    #[serde(default)]
    pub shading: Shading,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8],
            shading: Shading::Lit,
        }
    }
}

impl Material {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self {
            base_color: [r, g, b],
            shading: Shading::Lit,
        }
    }

    /// Build a material from the sRGB bytes a colour picker gives you.
    pub fn from_srgb(r: u8, g: u8, b: u8) -> Self {
        let channel = |byte: u8| srgb_to_linear(byte as f32 / 255.0);
        Self {
            base_color: [channel(r), channel(g), channel(b)],
            shading: Shading::Lit,
        }
    }

    pub const fn unlit(mut self) -> Self {
        self.shading = Shading::Unlit;
        self
    }

    pub fn color(&self) -> Vec3 {
        Vec3::from_array(self.base_color)
    }
}

impl From<&ArchivedMaterial> for Material {
    /// Read a material out of a mapped asset.
    ///
    /// A copy rather than a borrow, deliberately: a material is four numbers,
    /// and handing out a view into the library's bytes would tie every
    /// surface in the world to the lifetime of a file on disk for no saving
    /// worth having. Meshes are borrowed because a mesh is megabytes; this is
    /// sixteen bytes.
    fn from(archived: &ArchivedMaterial) -> Self {
        Self {
            base_color: [
                archived.base_color[0].to_native(),
                archived.base_color[1].to_native(),
                archived.base_color[2].to_native(),
            ],
            shading: match archived.shading {
                ArchivedShading::Unlit => Shading::Unlit,
                ArchivedShading::Lit => Shading::Lit,
            },
        }
    }
}

/// One channel, sRGB to linear, both in `0..=1`.
///
/// Public because the editor needs it and must not carry its own copy. A
/// host that reimplements this gets it slightly wrong — usually as a plain
/// `powf(2.2)`, which is close enough to look right and wrong enough that
/// colours picked in the editor do not match the ones the engine draws.
pub fn srgb_to_linear(channel: f32) -> f32 {
    let c = channel.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// One channel, linear back to sRGB: what a colour picker should be shown.
pub fn linear_to_srgb(channel: f32) -> f32 {
    let c = channel.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The materials the engine ships with, so that an example scene can be
/// written without inventing a palette first.
pub mod builtin {
    use super::Material;

    pub const WHITE: Material = Material::new(0.80, 0.80, 0.80);
    pub const GRASS: Material = Material::new(0.14, 0.20, 0.09);
    pub const EARTH: Material = Material::new(0.13, 0.10, 0.07);
    pub const BARK: Material = Material::new(0.06, 0.04, 0.02);
    pub const NEEDLE: Material = Material::new(0.04, 0.09, 0.05);
    pub const STONE: Material = Material::new(0.19, 0.20, 0.22);
    pub const EMBER: Material = Material::new(1.00, 0.35, 0.10).unlit();

    /// Look one up by the name a scene file uses.
    pub fn by_name(name: &str) -> Option<Material> {
        Some(match name {
            "white" => WHITE,
            "grass" => GRASS,
            "earth" => EARTH,
            "bark" => BARK,
            "needle" => NEEDLE,
            "stone" => STONE,
            "ember" => EMBER,
            _ => return None,
        })
    }

    pub const NAMES: [&str; 7] = [
        "white", "grass", "earth", "bark", "needle", "stone", "ember",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_through_the_conversion_the_shader_expects() {
        // Mid grey in sRGB is not 0.5 in linear, and getting this backwards
        // makes every hand-picked colour come out washed out.
        let mid = Material::from_srgb(128, 128, 128);
        assert!((mid.base_color[0] - 0.2158).abs() < 0.001);
        let black = Material::from_srgb(0, 0, 0);
        assert_eq!(black.base_color, [0.0, 0.0, 0.0]);
        let white = Material::from_srgb(255, 255, 255);
        assert!((white.base_color[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_two_colour_conversions_are_each_others_inverse() {
        // They are separate functions with separate constants, which is
        // exactly how one of them ends up subtly wrong.
        for step in 0..=20 {
            let c = step as f32 / 20.0;
            assert!(
                (srgb_to_linear(linear_to_srgb(c)) - c).abs() < 1e-4,
                "round trip broke at {c}"
            );
        }
        assert_eq!(linear_to_srgb(0.0), 0.0);
        // `1.055 * 1 - 0.055` is not exactly one in f32, and a picker that
        // demanded it were would clamp white to something just under.
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn every_builtin_material_is_reachable_by_its_name() {
        for name in builtin::NAMES {
            assert!(builtin::by_name(name).is_some(), "{name}");
        }
        assert!(builtin::by_name("chartreuse").is_none());
    }

    #[test]
    fn the_ember_is_the_one_thing_light_does_not_touch() {
        // 07-look.md puts embers on the "code" side of the line because they
        // are a light, not a surface.
        assert_eq!(builtin::EMBER.shading, Shading::Unlit);
        for name in builtin::NAMES.iter().filter(|n| **n != "ember") {
            assert_eq!(
                builtin::by_name(name).unwrap().shading,
                Shading::Lit,
                "{name} should be an ordinary lit surface"
            );
        }
    }
}
