//! What a surface is made of: URP's Lit material, with its names.
//!
//! A colour, how metallic and how smooth (the metallic workflow), what it
//! emits, how see-through it is and how it is blended, whether it is cut
//! out, which faces are drawn, and whether it takes highlights, reflections
//! and shadows. Everything past the colour defaults to what the valley
//! asks for (`docs/design/07-look.md`): not metallic, not smooth — a matte
//! surface, flat colour on a characteristic shape, which is what a
//! material that says only its colour still looks like. Anything else is
//! a line in the material's file away.

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
    /// Lit, with a one-metre grid drawn on it in world space: the greybox
    /// surface. Every face shows its size in metres whatever the object's
    /// scale, so a corridor reads as three metres wide without measuring
    /// it. Unity's prototyping materials, without a texture or UVs.
    Grid,
}

/// Whether a surface hides what is behind it: URP's Surface Type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub enum SurfaceType {
    #[default]
    Opaque,
    /// Blended over what is behind it, drawn after everything solid, far
    /// ones first.
    Transparent,
}

/// How a transparent surface combines with what is behind it: URP's Blending
/// Mode.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub enum Blend {
    /// Glass, water, smoke: the surface over the background by its alpha.
    #[default]
    Alpha,
    /// Alpha, with the colour already scaled by it: a reflection on glass
    /// keeps its brightness where the glass is clear.
    Premultiply,
    /// Added: fire, sparks, glows.
    Additive,
    /// Multiplied into the background: stains, tinted glass.
    Multiply,
}

/// Which faces are drawn: URP's Render Face.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub enum RenderFace {
    #[default]
    Front,
    /// The inside: a room seen from within a box.
    Back,
    /// Both: a leaf, a sheet of cloth, a flag.
    Both,
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
    /// 0 a dielectric (wood, stone, cloth, paint), 1 a metal.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub metallic: f32,
    /// 0 matte, 1 a mirror. The highlight's sharpness and how much of the
    /// sky it reflects.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub smoothness: f32,
    /// Light the surface gives off, linear and times its intensity — past
    /// 1.0 it blooms. An ember, a screen, a lit window.
    #[serde(default, skip_serializing_if = "is_black")]
    pub emission: [f32; 3],
    /// How opaque, 0 to 1: only a [`SurfaceType::Transparent`] surface is
    /// see-through by it; an opaque one uses it for `alpha_clip`.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub alpha: f32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub surface: SurfaceType,
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend: Blend,
    /// Alpha Clipping: pixels less opaque than this are not drawn at all —
    /// a leaf cut out of a quad, a chain-link fence. 0 is off.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub alpha_clip: f32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub render_face: RenderFace,
    /// The sun's and the lamps' highlights on it.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub specular_highlights: bool,
    /// The sky seen in it.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub environment_reflections: bool,
    /// Shadows fall on it.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub receive_shadows: bool,
    /// URP's Base Map: the colour, times `base_color`. A texture asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_map: Option<crate::asset::AssetId>,
    /// URP's Normal Map, in tangent space; imported linear, not as colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_map: Option<crate::asset::AssetId>,
    /// HDRP's mask map, which URP's Lit reads as its metallic and occlusion
    /// maps: red scales `metallic`, green is occlusion, alpha scales
    /// `smoothness`. Imported linear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_map: Option<crate::asset::AssetId>,
    /// URP's Emission Map, times `emission`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission_map: Option<crate::asset::AssetId>,
    /// Its own shader — water, glass, something that moves — as
    /// [`crate::asset::shader_id`] names it: a `surface` function in
    /// `shaders/<name>.wgsl` that changes what the standard one worked out
    /// before it is lit. `None` is the standard one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<crate::asset::AssetId>,
    /// Eight numbers of its own for its shader to read (`in.params`): what
    /// makes two materials on one shader different — a speed, a tint.
    #[serde(default, skip_serializing_if = "is_zeros")]
    pub params: [f32; 8],
    /// How strongly the normal map bends the surface.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub normal_scale: f32,
    /// How much the mask's occlusion darkens, 0 to 1.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub occlusion_strength: f32,
    /// URP's Tiling and Offset: the maps repeat this many times, shifted.
    #[serde(default = "no_tiling", skip_serializing_if = "is_no_tiling")]
    pub tiling: [f32; 2],
    #[serde(default, skip_serializing_if = "is_no_offset")]
    pub offset: [f32; 2],
}

fn no_tiling() -> [f32; 2] {
    [1.0, 1.0]
}
fn is_no_tiling(t: &[f32; 2]) -> bool {
    *t == [1.0, 1.0]
}
fn is_no_offset(o: &[f32; 2]) -> bool {
    *o == [0.0, 0.0]
}

fn is_zeros(v: &[f32; 8]) -> bool {
    v.iter().all(|x| *x == 0.0)
}

fn is_zero(x: &f32) -> bool {
    *x == 0.0
}
fn one() -> f32 {
    1.0
}
fn is_one(x: &f32) -> bool {
    *x == 1.0
}
fn is_black(c: &[f32; 3]) -> bool {
    *c == [0.0; 3]
}
fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_default<T: Default + PartialEq>(x: &T) -> bool {
    *x == T::default()
}

impl Default for Material {
    fn default() -> Self {
        Self::new(0.8, 0.8, 0.8)
    }
}

impl Material {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self {
            base_color: [r, g, b],
            shading: Shading::Lit,
            metallic: 0.0,
            smoothness: 0.0,
            emission: [0.0; 3],
            alpha: 1.0,
            surface: SurfaceType::Opaque,
            blend: Blend::Alpha,
            alpha_clip: 0.0,
            render_face: RenderFace::Front,
            specular_highlights: true,
            environment_reflections: true,
            receive_shadows: true,
            base_map: None,
            normal_map: None,
            mask_map: None,
            emission_map: None,
            shader: None,
            params: [0.0; 8],
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            tiling: [1.0, 1.0],
            offset: [0.0, 0.0],
        }
    }

    /// The textures it draws with, besides its numbers.
    pub fn maps(&self) -> impl Iterator<Item = crate::asset::AssetId> {
        [
            self.base_map,
            self.normal_map,
            self.mask_map,
            self.emission_map,
        ]
        .into_iter()
        .flatten()
    }

    /// Whether it is blended over what is behind it.
    pub fn is_transparent(&self) -> bool {
        self.surface == SurfaceType::Transparent
    }

    /// Build a material from the sRGB bytes a colour picker gives you.
    pub fn from_srgb(r: u8, g: u8, b: u8) -> Self {
        let channel = |byte: u8| srgb_to_linear(byte as f32 / 255.0);
        Self::new(channel(r), channel(g), channel(b))
    }

    pub const fn unlit(mut self) -> Self {
        self.shading = Shading::Unlit;
        self
    }

    /// The same colour, with the metre grid on it.
    pub const fn grid(mut self) -> Self {
        self.shading = Shading::Grid;
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
        let v3 =
            |c: &[rkyv::rend::f32_le; 3]| [c[0].to_native(), c[1].to_native(), c[2].to_native()];
        Self {
            base_color: v3(&archived.base_color),
            shading: match archived.shading {
                ArchivedShading::Unlit => Shading::Unlit,
                ArchivedShading::Lit => Shading::Lit,
                ArchivedShading::Grid => Shading::Grid,
            },
            metallic: archived.metallic.to_native(),
            smoothness: archived.smoothness.to_native(),
            emission: v3(&archived.emission),
            alpha: archived.alpha.to_native(),
            surface: match archived.surface {
                ArchivedSurfaceType::Opaque => SurfaceType::Opaque,
                ArchivedSurfaceType::Transparent => SurfaceType::Transparent,
            },
            blend: match archived.blend {
                ArchivedBlend::Alpha => Blend::Alpha,
                ArchivedBlend::Premultiply => Blend::Premultiply,
                ArchivedBlend::Additive => Blend::Additive,
                ArchivedBlend::Multiply => Blend::Multiply,
            },
            alpha_clip: archived.alpha_clip.to_native(),
            render_face: match archived.render_face {
                ArchivedRenderFace::Front => RenderFace::Front,
                ArchivedRenderFace::Back => RenderFace::Back,
                ArchivedRenderFace::Both => RenderFace::Both,
            },
            specular_highlights: archived.specular_highlights,
            environment_reflections: archived.environment_reflections,
            receive_shadows: archived.receive_shadows,
            base_map: archived.base_map.as_ref().map(crate::asset::AssetId::from),
            normal_map: archived
                .normal_map
                .as_ref()
                .map(crate::asset::AssetId::from),
            mask_map: archived.mask_map.as_ref().map(crate::asset::AssetId::from),
            emission_map: archived
                .emission_map
                .as_ref()
                .map(crate::asset::AssetId::from),
            shader: archived.shader.as_ref().map(crate::asset::AssetId::from),
            params: std::array::from_fn(|i| archived.params[i].to_native()),
            normal_scale: archived.normal_scale.to_native(),
            occlusion_strength: archived.occlusion_strength.to_native(),
            tiling: [
                archived.tiling[0].to_native(),
                archived.tiling[1].to_native(),
            ],
            offset: [
                archived.offset[0].to_native(),
                archived.offset[1].to_native(),
            ],
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
    /// Unlit, and glowing: it gives off a little more light than it is
    /// coloured, so it blooms at night and not at noon.
    pub const EMBER: Material = {
        let mut m = Material::new(1.00, 0.35, 0.10).unlit();
        m.emission = [0.6, 0.2, 0.05];
        m
    };
    /// Light grey with the metre grid: what a greybox is built from.
    pub const GRID: Material = Material::new(0.45, 0.46, 0.48).grid();

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
            "grid" => GRID,
            _ => return None,
        })
    }

    pub const NAMES: [&str; 8] = [
        "white", "grass", "earth", "bark", "needle", "stone", "ember", "grid",
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
        assert_eq!(builtin::GRID.shading, Shading::Grid);
        for name in builtin::NAMES
            .iter()
            .filter(|n| !["ember", "grid"].contains(n))
        {
            assert_eq!(
                builtin::by_name(name).unwrap().shading,
                Shading::Lit,
                "{name} should be an ordinary lit surface"
            );
        }
    }
}
