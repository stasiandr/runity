//! A mesh as the library keeps it, ready to upload: vertices, indices, the
//! parts drawn with different materials, a skin for a skeleton, and the
//! model's own colours — and a texture, the same way. The geometry
//! module's asset formats, and reading them from the library
//! ([`MeshLibrary`], [`TextureLibrary`]).

use rkyv::{Archive, Deserialize, Serialize};

use crate::asset::{Asset, AssetId, AssetKind};

/// A mesh's kind in an asset's header: this module's.
pub const MESH: AssetKind = AssetKind::new(1, "mesh");
/// A texture's kind in an asset's header: this module's.
pub const TEXTURE: AssetKind = AssetKind::new(2, "texture");
use crate::library::Library;

/// One vertex, in the layout the vertex buffer uses.
///
/// `repr(C)` because this is uploaded to the GPU as-is; the archived form and
/// the in-memory form have to agree on padding.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Archive,
    Serialize,
    Deserialize,
    bytemuck::Pod,
    bytemuck::Zeroable,
)]
#[repr(C)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A run of indices sharing one material — what becomes one draw call.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct Submesh {
    pub first_index: u32,
    pub index_count: u32,
    /// The material this run is drawn with, or `None` while materials are
    /// still the atlas the old pipeline baked.
    pub material: Option<AssetId>,
}

/// An axis-aligned box around everything in the mesh.
///
/// Computed at import rather than at load, because it is needed for culling
/// on the first frame and recomputing it would mean walking every vertex —
/// which is exactly the parsing the format exists to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Archive, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub fn of(vertices: &[Vertex]) -> Self {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(v.position[axis]);
                max[axis] = max[axis].max(v.position[axis]);
            }
        }
        // An empty mesh gets a degenerate box at the origin rather than
        // infinities, which would poison every culling test it touched.
        if vertices.is_empty() {
            min = [0.0; 3];
            max = [0.0; 3];
        }
        Bounds { min, max }
    }

    pub fn center(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }
}

/// How a texture's texels are stored: straight RGBA8, as the importer
/// writes the library, or in blocks the GPU samples as they are — BC7 on a
/// desktop's, ASTC 4x4 on a phone's or an Apple one's — as a build for a
/// platform cooks them (`scrap build --platform`). A quarter of the bytes
/// on disk, in the download and in video memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Archive, Serialize, Deserialize)]
#[rkyv(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub enum TextureCoding {
    #[default]
    Rgba8,
    Bc7,
    Astc4x4,
}

impl TextureCoding {
    /// Texels across (and down) a block: 1 for RGBA8.
    pub fn block(self) -> u32 {
        match self {
            TextureCoding::Rgba8 => 1,
            TextureCoding::Bc7 | TextureCoding::Astc4x4 => 4,
        }
    }

    /// Bytes a block takes: a texel's four for RGBA8, sixteen for BC7 and
    /// ASTC 4x4.
    pub fn block_bytes(self) -> u32 {
        match self {
            TextureCoding::Rgba8 => 4,
            TextureCoding::Bc7 | TextureCoding::Astc4x4 => 16,
        }
    }

    /// Blocks across and down a level `width` by `height`: the last ones
    /// partly past its edge.
    pub fn blocks(self, width: u32, height: u32) -> (u32, u32) {
        let b = self.block();
        (width.max(1).div_ceil(b), height.max(1).div_ceil(b))
    }

    /// The bytes of a level `width` by `height`.
    pub fn level_bytes(self, width: u32, height: u32) -> usize {
        let (x, y) = self.blocks(width, height);
        x as usize * y as usize * self.block_bytes() as usize
    }
}

impl ArchivedTextureCoding {
    /// The coding, off the archive.
    pub fn native(&self) -> TextureCoding {
        match self {
            ArchivedTextureCoding::Rgba8 => TextureCoding::Rgba8,
            ArchivedTextureCoding::Bc7 => TextureCoding::Bc7,
            ArchivedTextureCoding::Astc4x4 => TextureCoding::Astc4x4,
        }
    }
}

/// An image, ready to upload: RGBA8, one byte per channel, as imported;
/// in GPU blocks ([`TextureCoding`]) as a build for a platform cooks it.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct TextureAsset {
    pub id: AssetId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// The base level, top row first: `width * height * 4` bytes of RGBA8,
    /// or its blocks, a row of them at a time ([`TextureCoding::level_bytes`]).
    pub pixels: Vec<u8>,
    /// Levels 1 and down, each half the previous, ending at 1x1.
    ///
    /// Built at import rather than on the GPU at load. Without them a
    /// texture seen at a distance samples one texel out of many and
    /// shimmers as the camera moves — the artifact that looks like the
    /// renderer is broken and is only a missing chain.
    pub mips: Vec<TextureLevel>,
    /// Whether the values are sRGB-encoded. Colour maps are; normal maps,
    /// roughness and masks are not, and sampling those through an sRGB view
    /// bends every value in them.
    pub srgb: bool,
    /// How `pixels` and each level's are stored.
    pub coding: TextureCoding,
}


/// One step down the mip chain.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct TextureLevel {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// What binds a mesh's vertices to a skeleton.
///
/// Held apart from [`Vertex`] rather than widened into it, so a static mesh
/// pays nothing: most meshes in most scenes have no skeleton, and putting
/// four joint indices and four weights on every vertex in the world would
/// cost a third more memory and bandwidth for nothing.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct MeshSkin {
    /// Four joint indices per vertex, parallel to `vertices`.
    pub joints: Vec<[u16; 4]>,
    /// Four weights per vertex, summing to one.
    pub weights: Vec<[f32; 4]>,
    pub skeleton: crate::animation::Skeleton,
    pub clips: Vec<crate::animation::Clip>,
}

impl ArchivedMeshAsset {
    /// The skeleton and the clips, as plain values: what the animation
    /// module's `Animator` is made from. `None` for a mesh with no skin. A copy, made once when
    /// something starts animating, not in the frame.
    pub fn skin_owned(&self) -> Option<MeshSkin> {
        self.skin
            .as_ref()
            .and_then(|skin| rkyv::deserialize::<MeshSkin, rkyv::rancor::Error>(skin).ok())
    }
}

/// A mesh, ready to upload.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct MeshAsset {
    pub id: AssetId,
    /// Kept for the editor's browser and for error messages; the runtime
    /// never looks anything up by it.
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<Submesh>,
    pub bounds: Bounds,
    /// Present only when the mesh is skinned.
    pub skin: Option<MeshSkin>,
    /// The colour painted on each vertex (glTF's `COLOR_0`), parallel to
    /// `vertices` — or empty, the commonest case, when the file paints
    /// none or paints it all white.
    ///
    /// Held apart from [`Vertex`] for the reason the skin is: most meshes
    /// have none, and they pay nothing for it. Four bytes a vertex, as
    /// Unity keeps them, and as the file says them: no curve is taken off
    /// or put on, so a material's shader reads the number the artist
    /// painted (a mask more often than a colour).
    pub colors: Vec<[u8; 4]>,
    /// The model's own colours, from the file's materials: its one texture,
    /// or its materials' colours in a little palette its UVs point into.
    /// What it is drawn with when the entity's material has no map of its
    /// own — a Kenney kit's model looks as it did in its maker's tool.
    pub look: Option<TextureAsset>,
}

impl Asset for MeshAsset {
    fn id(&self) -> AssetId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl Asset for TextureAsset {
    fn id(&self) -> AssetId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Meshes out of the library.
pub trait MeshLibrary {
    fn mesh(&self, id: AssetId) -> Option<&ArchivedMeshAsset>;
    /// By file stem: a convenience for scenes written by hand and for
    /// tests.
    fn mesh_by_name(&self, name: &str) -> Option<&ArchivedMeshAsset>;
    /// Follow a link to a mesh: by its ID when it has one the library
    /// knows, by its name otherwise (docs/refs.md).
    fn mesh_link(&self, link: &crate::AssetLink) -> Option<&ArchivedMeshAsset>;
}

impl MeshLibrary for Library {
    fn mesh(&self, id: AssetId) -> Option<&ArchivedMeshAsset> {
        crate::asset::view::<MeshAsset>(self.bytes_of(id, MESH)?).ok()
    }
    fn mesh_by_name(&self, name: &str) -> Option<&ArchivedMeshAsset> {
        crate::asset::view::<MeshAsset>(self.bytes_named(name, MESH)?).ok()
    }
    fn mesh_link(&self, link: &crate::AssetLink) -> Option<&ArchivedMeshAsset> {
        crate::asset::view::<MeshAsset>(self.bytes_linked(link, MESH)?).ok()
    }
}

/// Textures out of the library.
pub trait TextureLibrary {
    fn texture(&self, id: AssetId) -> Option<&ArchivedTextureAsset>;
    fn texture_by_name(&self, name: &str) -> Option<&ArchivedTextureAsset>;
}

impl TextureLibrary for Library {
    fn texture(&self, id: AssetId) -> Option<&ArchivedTextureAsset> {
        crate::asset::view::<TextureAsset>(self.bytes_of(id, TEXTURE)?).ok()
    }
    fn texture_by_name(&self, name: &str) -> Option<&ArchivedTextureAsset> {
        crate::asset::view::<TextureAsset>(self.bytes_named(name, TEXTURE)?).ok()
    }
}
