//! The engine's own asset format: what an import produces and the runtime
//! reads.
//!
//! The rule is the one Unreal uses, for the same reason: **nothing the game
//! loads is a source format**. An `.obj` or a `.png` is what a kit hands us;
//! the editor imports it once into a `.rasset`, and from then on that is the
//! only thing the runtime knows how to open. The payoff is not tidiness — it
//! is that loading stops being parsing. A mesh arrives in the vertex layout
//! the GPU wants, and the load path is a read plus a cast.
//!
//! Three consequences worth stating, because they are easy to give away by
//! accident:
//!
//! * **The runtime never links an importer.** `gltf`, `tobj` and `image` live
//!   in `runity-import`, which the editor and the command line use and the
//!   shipped game does not. That is a smaller binary on a phone and no
//!   parsing code on the hot path.
//! * **References are ids, not paths.** Moving or renaming a file on disk
//!   cannot break a scene. The path is a fact about today's disk; the
//!   [`AssetId`] is a fact about the asset.
//! * **Scenes are not assets.** A scene stays RON text, because people and
//!   agents edit scenes and `git` has to show what changed. An asset is
//!   compiled output that nobody edits by hand, so it is binary. Source and
//!   build, not two flavors of the same thing.

use std::path::Path;

use rkyv::{Archive, Deserialize, Serialize};

/// Every `.rasset` starts with these bytes, so a wrong or corrupt file is
/// refused with a sentence rather than a cast into nonsense.
pub const MAGIC: [u8; 8] = *b"RUNITY\0\x01";

/// Bumped whenever an archived type below changes shape, or the header does.
/// An asset built by an older importer is re-imported, never guessed at.
pub const FORMAT_VERSION: u32 = 15;

/// What kind of asset a file holds.
///
/// Stored in the header rather than inferred from the extension, because a
/// library reads whatever is in a directory and casting a texture's bytes to
/// a mesh is not an error any type system catches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AssetKind {
    Mesh = 1,
    Texture = 2,
    Sound = 3,
    Material = 4,
}

impl AssetKind {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(AssetKind::Mesh),
            2 => Some(AssetKind::Texture),
            3 => Some(AssetKind::Sound),
            4 => Some(AssetKind::Material),
            _ => None,
        }
    }
}

/// A stable name for an asset, assigned once at import and never reused.
///
/// It is what a scene stores and what one asset uses to point at another.
/// Paths deliberately do not appear in either: the editor keeps a
/// human-readable index beside the library, and that index is the only place
/// allowed to know where a file currently sits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Archive, Serialize, Deserialize,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct AssetId(pub u128);

/// The top bits of a camera's picture's id: "REND".
const RENDER_TARGET: u128 = 0x5245_4e44;

impl AssetId {
    /// Derive an id from the source path and a salt.
    ///
    /// Deterministic on purpose: importing the same file twice gives the same
    /// id, so a re-import updates an asset instead of orphaning it and
    /// minting a new one. It is not a random UUID for exactly that reason.
    pub fn from_source(source: &str, salt: u64) -> Self {
        // FNV-1a, widened. Not a cryptographic hash and does not need to be —
        // it needs to be stable across machines and runs, which a `DefaultHasher`
        // explicitly is not.
        let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
        for byte in source.as_bytes().iter().chain(&salt.to_le_bytes()) {
            hash ^= *byte as u128;
            hash = hash.wrapping_mul(PRIME);
        }
        AssetId(hash)
    }

    /// The id a camera's picture goes by (`render_texture`), for a
    /// material to show it: `base_map: "render:mirror"`. Marked in its top
    /// bits, so it is never taken for a texture the library should have.
    pub fn render_target(name: &str) -> Self {
        let hash = Self::from_source(&format!("render:{name}"), 0).0;
        AssetId((RENDER_TARGET << 96) | (hash & ((1u128 << 96) - 1)))
    }

    /// Whether this is a camera's picture rather than an imported texture.
    pub fn is_render_target(&self) -> bool {
        self.0 >> 96 == RENDER_TARGET
    }

    pub fn as_hex(&self) -> String {
        format!("{:032x}", self.0)
    }
}

/// The ID a `.rimport` sidecar holds, read without the importer: what a
/// prefab's or a scene's identity is (docs/refs.md). `None` when the file
/// is missing, does not parse, or has no ID yet.
pub fn sidecar_id(sidecar: impl AsRef<std::path::Path>) -> Option<AssetId> {
    #[derive(serde::Deserialize)]
    #[serde(rename = "ImportSettings")]
    struct Sidecar {
        #[serde(default)]
        id: Option<AssetId>,
    }
    let text = std::fs::read_to_string(sidecar).ok()?;
    ron::from_str::<Sidecar>(&text).ok()?.id
}

/// Where a file's sidecar is: beside it, `<file>.rimport`.
pub fn sidecar_of(file: &std::path::Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".rimport");
    std::path::PathBuf::from(name)
}

impl From<&ArchivedAssetId> for AssetId {
    /// The archived form is a distinct type, so reading an id out of a
    /// mapped asset needs one conversion. It is a widening of one integer,
    /// not a deserialization of the asset.
    fn from(archived: &ArchivedAssetId) -> Self {
        AssetId(archived.0.to_native())
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_hex())
    }
}

impl std::str::FromStr for AssetId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim();
        if digits.is_empty() || digits.len() > 32 || !digits.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!("asset id {text:?} is not up to 32 hex digits"));
        }
        u128::from_str_radix(digits, 16)
            .map(AssetId)
            .map_err(|e| e.to_string())
    }
}

// Text, for the `.rimport` sidecar: an asset's ID is minted once, written
// there, and kept when the source moves — which is what lets a scene and a
// library go on meaning the same asset. Hex rather than a number, because a
// 128-bit integer is exactly what text tools and models mangle.
impl serde::Serialize for AssetId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for AssetId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

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

/// An image, ready to upload: straight RGBA8, one byte per channel.
///
/// Uncompressed for now. Block compression (BC7 on desktop, ASTC on mobile)
/// is the obvious next step and belongs at import, where it is paid for once
/// rather than every load — but it is a per-platform decision, and the
/// pipeline has to exist before it is worth making.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct TextureAsset {
    pub id: AssetId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, top row first. The base level.
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
}

/// Decoded audio, ready to hand to the mixer.
///
/// Decoded at import for the same reason meshes are: a game that decodes OGG
/// on the frame it needs a footstep stutters on the footstep. The cost is
/// disk — a minute of stereo is about twenty megabytes — which is why music
/// will eventually want streaming and why sound effects never will.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct MaterialAsset {
    pub id: AssetId,
    pub name: String,
    pub material: crate::material::Material,
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
    /// The skeleton and the clips, as plain values: what an [`Animator`]
    /// is made from. `None` for a mesh with no skin. A copy, made once when
    /// something starts animating, not in the frame.
    ///
    /// [`Animator`]: crate::Animator
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
    /// The model's own colours, from the file's materials: its one texture,
    /// or its materials' colours in a little palette its UVs point into.
    /// What it is drawn with when the entity's material has no map of its
    /// own — a Kenney kit's model looks as it did in its maker's tool.
    pub look: Option<TextureAsset>,
}

/// Errors that mean "do not cast these bytes".
#[derive(Debug)]
pub enum AssetError {
    Io(std::io::Error),
    /// Not a `.rasset` at all.
    BadMagic,
    /// A `.rasset` from a different format version — re-import it.
    Version {
        found: u32,
        expected: u32,
    },
    /// The header names a kind this build does not know.
    UnknownKind(u8),
    /// Asked for one kind, the file holds another.
    WrongKind {
        found: AssetKind,
        wanted: AssetKind,
    },
    /// The body did not survive validation.
    Corrupt(String),
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::Io(e) => write!(f, "{e}"),
            AssetError::BadMagic => write!(f, "not a runity asset"),
            AssetError::Version { found, expected } => write!(
                f,
                "asset format v{found}, this build reads v{expected} — re-import it"
            ),
            AssetError::UnknownKind(byte) => {
                write!(f, "asset kind {byte} is not one this build knows")
            }
            AssetError::WrongKind { found, wanted } => {
                write!(f, "asset holds a {found:?}, not a {wanted:?}")
            }
            AssetError::Corrupt(e) => write!(f, "corrupt asset: {e}"),
        }
    }
}

impl std::error::Error for AssetError {}

impl From<std::io::Error> for AssetError {
    fn from(e: std::io::Error) -> Self {
        AssetError::Io(e)
    }
}

/// Header size: magic plus version plus the padding that keeps the body at an
/// alignment rkyv is happy to read from.
const HEADER: usize = 16;

/// Serialize an asset into the bytes of a `.rasset` file.
pub fn to_bytes<T>(value: &T, kind: AssetKind) -> Result<Vec<u8>, AssetError>
where
    T: for<'a> Serialize<
        rkyv::api::high::HighSerializer<
            rkyv::util::AlignedVec,
            rkyv::ser::allocator::ArenaHandle<'a>,
            rkyv::rancor::Error,
        >,
    >,
{
    let body = rkyv::to_bytes::<rkyv::rancor::Error>(value)
        .map_err(|e| AssetError::Corrupt(e.to_string()))?;
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.push(kind as u8);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Whether the `.rasset` at `path` was written by another format version
/// — and so has to be built again. `false` for one that cannot be read at
/// all: that is somebody else's problem, not a version's.
pub fn stale_format(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut header = [0u8; 12];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if file.read_exact(&mut header).is_err() || header[..8] != MAGIC {
        return false;
    }
    u32::from_le_bytes([header[8], header[9], header[10], header[11]]) != FORMAT_VERSION
}

/// The id a material's own shader goes by: `shader: "water"` in a
/// material is `shaders/water.wgsl` in the project.
pub fn shader_id(name: &str) -> AssetId {
    AssetId::from_source(&format!("shader:{name}"), 0)
}

/// What kind of asset these bytes hold, from the header alone.
pub fn kind_of(bytes: &[u8]) -> Result<AssetKind, AssetError> {
    if bytes.len() < HEADER || bytes[..8] != MAGIC {
        return Err(AssetError::BadMagic);
    }
    AssetKind::from_byte(bytes[12]).ok_or(AssetError::UnknownKind(bytes[12]))
}

/// Whether the `.rasset` at `path` was written in this build's format: its
/// sixteen header bytes alone are read. A library built before a format
/// change is out of date even though no source changed.
pub fn is_current(path: impl AsRef<Path>) -> bool {
    use std::io::Read;
    let mut header = [0u8; HEADER];
    std::fs::File::open(path.as_ref())
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok()
        && split_header(&header).is_ok()
}

/// Check the header and hand back the body, without touching it.
///
/// Split out so that a caller who has mapped a file can validate the first
/// sixteen bytes without paging in the rest.
pub fn split_header(bytes: &[u8]) -> Result<&[u8], AssetError> {
    if bytes.len() < HEADER || bytes[..8] != MAGIC {
        return Err(AssetError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != FORMAT_VERSION {
        return Err(AssetError::Version {
            found: version,
            expected: FORMAT_VERSION,
        });
    }
    Ok(&bytes[HEADER..])
}

/// Read a `.rasset` off disk into owned bytes whose archived form can be
/// borrowed with [`view`].
///
/// The bytes are kept, not decoded: the whole point is that the vertex data
/// is already in the layout the GPU wants, so the load path ends here and the
/// next step is an upload.
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, AssetError> {
    let bytes = std::fs::read(path.as_ref())?;
    split_header(&bytes)?;
    Ok(bytes)
}

/// Borrow the archived asset inside bytes produced by [`read`].
pub fn view<T>(bytes: &[u8]) -> Result<&T::Archived, AssetError>
where
    T: Archive,
    T::Archived: for<'a> rkyv::bytecheck::CheckBytes<
        rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>,
    >,
{
    let body = split_header(bytes)?;
    rkyv::access::<T::Archived, rkyv::rancor::Error>(body)
        .map_err(|e| AssetError::Corrupt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> MeshAsset {
        let vertices: Vec<Vertex> = (0..8)
            .map(|i| Vertex {
                position: [
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                ],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
            })
            .collect();
        MeshAsset {
            id: AssetId::from_source("models/cube.obj", 0),
            name: "cube".into(),
            bounds: Bounds::of(&vertices),
            vertices,
            indices: (0..12u32).collect(),
            skin: None,
            look: None,
            submeshes: vec![Submesh {
                first_index: 0,
                index_count: 12,
                material: None,
            }],
        }
    }

    #[test]
    fn an_asset_is_read_back_without_being_decoded() {
        let mesh = cube();
        let bytes = to_bytes(&mesh, AssetKind::Mesh).unwrap();
        let archived = view::<MeshAsset>(&bytes).unwrap();
        assert_eq!(archived.vertices.len(), 8);
        assert_eq!(archived.name.as_str(), "cube");
        assert_eq!(archived.id, mesh.id);
        // Reading a coordinate costs no parse and no allocation.
        assert_eq!(archived.vertices[1].position[0], 1.0);
    }

    #[test]
    fn the_same_source_always_gets_the_same_id() {
        // A re-import has to update the asset in place. If ids were random,
        // every re-import would orphan every scene that referenced it.
        assert_eq!(
            AssetId::from_source("models/pine_large.obj", 0),
            AssetId::from_source("models/pine_large.obj", 0)
        );
        assert_ne!(
            AssetId::from_source("models/pine_large.obj", 0),
            AssetId::from_source("models/pine_small.obj", 0)
        );
    }

    #[test]
    fn a_file_that_is_not_ours_is_refused_rather_than_cast() {
        let png = b"\x89PNG\r\n\x1a\n and then some".to_vec();
        assert!(matches!(view::<MeshAsset>(&png), Err(AssetError::BadMagic)));
    }

    #[test]
    fn an_older_format_asks_for_a_re_import_instead_of_guessing() {
        let mut bytes = to_bytes(&cube(), AssetKind::Mesh).unwrap();
        bytes[8] = 0; // pretend it was written by format v0
        match view::<MeshAsset>(&bytes) {
            Err(AssetError::Version { found, expected }) => {
                assert_eq!((found, expected), (0, FORMAT_VERSION));
            }
            Err(e) => panic!("expected a version error, got {e}"),
            Ok(_) => panic!("a v0 asset was read as if it were current"),
        }
    }

    #[test]
    fn an_empty_mesh_gets_a_box_at_the_origin_not_infinities() {
        let b = Bounds::of(&[]);
        assert_eq!(b.min, [0.0; 3]);
        assert_eq!(b.max, [0.0; 3]);
        assert_eq!(b.center(), [0.0; 3]);
    }
}
