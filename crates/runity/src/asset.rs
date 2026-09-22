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

/// Bumped whenever an archived type below changes shape. An asset built by an
/// older importer is re-imported, never guessed at.
pub const FORMAT_VERSION: u32 = 1;

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

    pub fn as_hex(&self) -> String {
        format!("{:032x}", self.0)
    }
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
pub fn to_bytes<T>(value: &T) -> Result<Vec<u8>, AssetError>
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
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Check the header and hand back the body, without touching it.
///
/// Split out from [`load`] so that a caller who has mapped a file can
/// validate the first sixteen bytes without paging in the rest.
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
        let bytes = to_bytes(&mesh).unwrap();
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
        let mut bytes = to_bytes(&cube()).unwrap();
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
