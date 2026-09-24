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


/// A material's kind in an asset's header: this module's.
pub const MATERIAL: crate::asset::AssetKind = crate::asset::AssetKind::new(4, "material");

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
    /// Water: a surface of waves reflecting the sky and what is round it,
    /// the colour of its depth below, foam where it meets the shore. Its
    /// `base_color` is the deep water's, `wind` how high the waves run,
    /// `clarity` how far down one sees, `foam` how much there is. Put on a
    /// level plane, transparent. See `water.rs`'s notes in the shader.
    Water,
    /// Sand: lit, with ripples the wind has laid across it (their crests
    /// square to the scene's wind), grains that glint in the sun, and in a
    /// strong wind sand drifting over it in streaks. All drawn by the
    /// shader in world space, so it needs no texture and a dune of any
    /// size has ripples the same size. Its `base_color` is the sand's.
    Sand,
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
/// How a base map is laid on a surface.
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
pub enum ScreenMap {
    /// On the mesh's UVs, as every map is.
    #[default]
    Off,
    /// By where the pixel is on the screen.
    Screen,
    /// By where it is on the screen, left and right swapped: what a camera
    /// reflected in a plane took, seen in that plane.
    Mirror,
}

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
    /// Textures of its own for its shader, by the names the shader reads
    /// them by (Unity's property names, `_Road_Texture`): the shader's
    /// `// runity:textures` line puts up to four of them in its slots, for
    /// `texture_at` in its `surface`.
    #[serde(default, skip_serializing_if = "MaterialTextures::is_empty")]
    #[rkyv(with = TextureEntries)]
    pub textures: MaterialTextures,
    /// The base map laid on the screen rather than on the mesh's UVs: a
    /// camera's picture seen through the surface — a mirror's reflection
    /// ([`ScreenMap::Mirror`] flips it) or a portal's view.
    #[serde(default, skip_serializing_if = "is_default")]
    pub screen_map: ScreenMap,
    /// Drawn over everything, walls and all — a marker seen from afar, a
    /// highlight through a wall. Only for a transparent surface; Unity's
    /// ZTest Always.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_top: bool,
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
    /// How much it sways in the scene's wind: 0 a rock, about 1 a tree,
    /// more for grass. See [`crate::foliage`].
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wind: f32,
    /// How much light comes through it from behind, 0 to 1: a leaf, a
    /// blade of grass, a paper lantern.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub translucency: f32,
    /// For [`Shading::Water`]: metres one sees down through it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub clarity: f32,
    /// For [`Shading::Water`]: how much foam where it meets the shore, 0 to 1.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub foam: f32,
    /// Clay: dark, smooth mud while wet; as it dries (the weather's
    /// `drying`), lighter, in patches, and cracking into curling plates.
    /// Dry clay with no weather at all is cracked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clay: bool,
    /// Light scattered under the surface — skin, wax, a leaf, marble — as
    /// the colour it comes out (linear); black is none. It carries past the
    /// edge of the lit side, tinted, and through what is thin enough
    /// toward the sun: an ear, a petal, a fingertip against the light.
    #[serde(default, skip_serializing_if = "is_black")]
    pub subsurface: [f32; 3],
    /// How far light travels under it, metres, before it is gone: a few
    /// millimetres for skin, a centimetre or two for wax.
    #[serde(default = "subsurface_reach", skip_serializing_if = "is_subsurface_reach")]
    pub subsurface_radius: f32,
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
            textures: MaterialTextures::NONE,
            screen_map: ScreenMap::Off,
            on_top: false,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            tiling: [1.0, 1.0],
            offset: [0.0, 0.0],
            wind: 0.0,
            translucency: 0.0,
            clay: false,
            subsurface: [0.0; 3],
            subsurface_radius: 0.01,
            clarity: 0.0,
            foam: 0.0,
        }
    }

    /// The textures it draws with, besides its numbers: its four maps and
    /// those it hands its own shader.
    pub fn maps(&self) -> impl Iterator<Item = crate::asset::AssetId> {
        [
            self.base_map,
            self.normal_map,
            self.mask_map,
            self.emission_map,
        ]
        .into_iter()
        .flatten()
        .chain(self.textures.ids())
    }

    /// Whether it is blended over what is behind it.
    pub fn is_transparent(&self) -> bool {
        self.surface == SurfaceType::Transparent || self.shading == Shading::Water
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
                ArchivedShading::Water => Shading::Water,
                ArchivedShading::Sand => Shading::Sand,
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
            textures: MaterialTextures::from_archived(&archived.textures),
            on_top: archived.on_top,
            screen_map: match archived.screen_map {
                ArchivedScreenMap::Off => ScreenMap::Off,
                ArchivedScreenMap::Screen => ScreenMap::Screen,
                ArchivedScreenMap::Mirror => ScreenMap::Mirror,
            },
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
            wind: archived.wind.to_native(),
            translucency: archived.translucency.to_native(),
            clarity: archived.clarity.to_native(),
            foam: archived.foam.to_native(),
            clay: archived.clay,
            subsurface: [
                archived.subsurface[0].to_native(),
                archived.subsurface[1].to_native(),
                archived.subsurface[2].to_native(),
            ],
            subsurface_radius: archived.subsurface_radius.to_native(),
        }
    }
}

/// One texture a material hands its shader: the name the shader reads it
/// by, and the texture.
#[derive(Debug, Clone, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(derive(Debug))]
pub struct TextureEntry {
    pub name: String,
    pub texture: crate::asset::AssetId,
}

/// The textures a material hands its own shader, by name
/// ([`Material::textures`]).
///
/// A number standing for the set rather than the set itself, so that a
/// material stays `Copy`: a frame copies one into every draw, and a map
/// of strings there would be an allocation a draw for the materials that
/// have any. Equal sets are the same number — each is kept once, sorted by
/// name, for the life of the process — so comparing two materials still
/// compares their textures. What a set holds is looked up only where a
/// draw is prepared and where textures are uploaded. A set is never
/// forgotten: a material edited a hundred times leaves a hundred short
/// lists behind, which is nothing next to its textures.
///
/// Written as a map in text, `{"_Road": "<texture id>"}`, and as a list of
/// [`TextureEntry`] in an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MaterialTextures(u32);

/// Every set of textures named so far: [`MaterialTextures`] `n` is the
/// `n - 1`th; 0 is none.
static TEXTURE_SETS: std::sync::RwLock<Vec<std::sync::Arc<[TextureEntry]>>> =
    std::sync::RwLock::new(Vec::new());

impl MaterialTextures {
    /// None of its own: what a material without a shader of its own has.
    pub const NONE: Self = Self(0);

    /// The set of these, by name. A name said twice keeps its last texture.
    pub fn new(
        entries: impl IntoIterator<Item = (impl Into<String>, crate::asset::AssetId)>,
    ) -> Self {
        let mut by_name = std::collections::BTreeMap::new();
        for (name, texture) in entries {
            by_name.insert(name.into(), texture);
        }
        if by_name.is_empty() {
            return Self::NONE;
        }
        let set: Vec<TextureEntry> = by_name
            .into_iter()
            .map(|(name, texture)| TextureEntry { name, texture })
            .collect();
        let find = |sets: &[std::sync::Arc<[TextureEntry]>]| {
            sets.iter()
                .position(|s| **s == *set)
                .map(|i| Self(i as u32 + 1))
        };
        if let Some(found) = find(&TEXTURE_SETS.read().unwrap_or_else(|e| e.into_inner())) {
            return found;
        }
        let mut sets = TEXTURE_SETS.write().unwrap_or_else(|e| e.into_inner());
        // Another thread may have put it in between the two locks.
        if let Some(found) = find(&sets) {
            return found;
        }
        sets.push(set.into());
        Self(sets.len() as u32)
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// What it holds, sorted by name.
    pub fn entries(&self) -> std::sync::Arc<[TextureEntry]> {
        if self.0 == 0 {
            return std::sync::Arc::new([]);
        }
        TEXTURE_SETS.read().unwrap_or_else(|e| e.into_inner())[self.0 as usize - 1].clone()
    }

    /// The texture it hands its shader as `name`.
    pub fn get(&self, name: &str) -> Option<crate::asset::AssetId> {
        if self.0 == 0 {
            return None;
        }
        self.entries()
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.texture)
    }

    /// Every texture in it.
    pub fn ids(&self) -> Vec<crate::asset::AssetId> {
        if self.0 == 0 {
            return Vec::new();
        }
        self.entries().iter().map(|e| e.texture).collect()
    }

    /// The set an asset holds.
    pub fn from_archived(entries: &rkyv::vec::ArchivedVec<ArchivedTextureEntry>) -> Self {
        Self::new(entries.iter().map(|e| {
            (
                e.name.as_str().to_string(),
                crate::asset::AssetId::from(&e.texture),
            )
        }))
    }
}

impl Serialize for MaterialTextures {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let entries = self.entries();
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for e in entries.iter() {
            map.serialize_entry(&e.name, &e.texture)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for MaterialTextures {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map =
            std::collections::BTreeMap::<String, crate::asset::AssetId>::deserialize(deserializer)?;
        Ok(Self::new(map))
    }
}

/// How [`MaterialTextures`] goes into an asset: as the list it stands for.
pub struct TextureEntries;

impl rkyv::with::ArchiveWith<MaterialTextures> for TextureEntries {
    type Archived = rkyv::vec::ArchivedVec<ArchivedTextureEntry>;
    type Resolver = rkyv::vec::VecResolver;

    fn resolve_with(
        field: &MaterialTextures,
        resolver: Self::Resolver,
        out: rkyv::Place<Self::Archived>,
    ) {
        rkyv::vec::ArchivedVec::resolve_from_len(field.entries().len(), resolver, out);
    }
}

impl<S> rkyv::with::SerializeWith<MaterialTextures, S> for TextureEntries
where
    S: rkyv::rancor::Fallible + ?Sized,
    Vec<TextureEntry>: rkyv::Serialize<S, Resolver = rkyv::vec::VecResolver>,
{
    fn serialize_with(
        field: &MaterialTextures,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        rkyv::Serialize::serialize(&field.entries().to_vec(), serializer)
    }
}

impl<D>
    rkyv::with::DeserializeWith<
        rkyv::vec::ArchivedVec<ArchivedTextureEntry>,
        MaterialTextures,
        D,
    > for TextureEntries
where
    D: rkyv::rancor::Fallible + ?Sized,
{
    fn deserialize_with(
        field: &rkyv::vec::ArchivedVec<ArchivedTextureEntry>,
        _: &mut D,
    ) -> Result<MaterialTextures, D::Error> {
        Ok(MaterialTextures::from_archived(field))
    }
}

fn subsurface_reach() -> f32 {
    0.01
}
fn is_subsurface_reach(r: &f32) -> bool {
    *r == 0.01
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

/// A surface, as an asset in its own right.
///
/// Materials were inline in scenes first, and inline is where a palette goes
/// to die: the same brown spelled out in twenty scenes drifts in nineteen of
/// them, and changing it means a find-and-replace across text files. As an
/// asset it is named once, referenced by name, and edited in one place — the
/// same deal meshes and textures already have.
///
/// It holds a [`Material`](crate::material::Material) rather than repeating
/// its fields, so adding a roughness later is one change rather than two
/// definitions to keep in step.
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MaterialAsset {
    pub id: crate::asset::AssetId,
    pub name: String,
    pub material: Material,
}


impl crate::asset::Asset for MaterialAsset {
    fn id(&self) -> crate::asset::AssetId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Materials out of the library: the palette. Returned by value — a
/// material is a few numbers, and a borrow would tie every surface in a
/// scene to the library's lifetime for nothing.
pub trait MaterialLibrary {
    fn material(&self, id: crate::asset::AssetId) -> Option<Material>;
    /// By file stem, which is what a scene writes: `material:
    /// "mossy_stone"` in any scene finds the one asset.
    fn material_by_name(&self, name: &str) -> Option<Material>;
    /// Follow a link to a material: by ID first, then by name.
    fn material_link(&self, link: &crate::AssetLink) -> Option<Material>;
}

impl MaterialLibrary for crate::library::Library {
    fn material(&self, id: crate::asset::AssetId) -> Option<Material> {
        let bytes = self.bytes_of(id, MATERIAL)?;
        crate::asset::view::<MaterialAsset>(bytes).ok().map(|a| Material::from(&a.material))
    }
    fn material_by_name(&self, name: &str) -> Option<Material> {
        let bytes = self.bytes_named(name, MATERIAL)?;
        crate::asset::view::<MaterialAsset>(bytes).ok().map(|a| Material::from(&a.material))
    }
    fn material_link(&self, link: &crate::AssetLink) -> Option<Material> {
        let bytes = self.bytes_linked(link, MATERIAL)?;
        crate::asset::view::<MaterialAsset>(bytes).ok().map(|a| Material::from(&a.material))
    }
}
