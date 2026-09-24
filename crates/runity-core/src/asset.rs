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
pub const FORMAT_VERSION: u32 = 18;

/// What kind of asset a file holds: the byte in its header.
///
/// Stored in the header rather than inferred from the extension, because a
/// library reads whatever is in a directory and casting a texture's bytes to
/// a mesh is not an error any type system catches.
///
/// The kinds are the modules' (DNA, postulate 3: the core does not know
/// the word "mesh"): the module that owns a format names its kind —
/// geometry's `MESH` and `TEXTURE`, the sound module's `SOUND`, the
/// render's `MATERIAL` — and the core reads a library by the header alone.
/// Two modules taking one byte is what the engine's test of every kind
/// catches. A kind read off a header has only its byte; one a module
/// names has a name for messages too.
#[derive(Debug, Clone, Copy, Eq)]
pub struct AssetKind {
    pub byte: u8,
    pub name: &'static str,
}

impl AssetKind {
    /// A module's kind: its byte in the header, and what to call it.
    pub const fn new(byte: u8, name: &'static str) -> Self {
        Self { byte, name }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        (byte != 0).then_some(Self { byte, name: "" })
    }
}

impl PartialEq for AssetKind {
    fn eq(&self, other: &Self) -> bool {
        self.byte == other.byte
    }
}

impl std::hash::Hash for AssetKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.byte.hash(state)
    }
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.name.is_empty() {
            write!(f, "kind {}", self.byte)
        } else {
            f.write_str(self.name)
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
    sidecar_id_of(&crate::files::read_to_string(sidecar).ok()?)
}

/// The ID a sidecar's text records.
pub fn sidecar_id_of(text: &str) -> Option<AssetId> {
    #[derive(serde::Deserialize)]
    #[serde(rename = "ImportSettings")]
    struct Sidecar {
        #[serde(default)]
        id: Option<AssetId>,
    }
    ron::from_str::<Sidecar>(text).ok()?.id
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

/// What an [`AssetId`] says it expects: how [`crate::shape`] knows a field
/// holds an asset by its ID alone (a material's `base_map`).
pub const ID_EXPECTING: &str = "an asset id: up to 32 hex digits";

impl<'de> serde::Deserialize<'de> for AssetId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Text;
        impl serde::de::Visitor<'_> for Text {
            type Value = AssetId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(ID_EXPECTING)
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<AssetId, E> {
                text.parse().map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Text)
    }
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
                write!(f, "asset holds a {found}, not a {wanted}")
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

/// The fixed part of the header: magic, version, kind and padding.
const HEADER: usize = 16;

/// An asset format, as a module defines one: what kind it is, and the ID
/// and name it carries — which go in the file's header, so the core can
/// keep a library of assets it does not know the insides of.
pub trait Asset {
    fn id(&self) -> AssetId;
    fn name(&self) -> &str;
}

/// How long the whole header is: the fixed part, the ID, the name's
/// length and the name, padded so the body starts where rkyv reads from.
fn header_len(name_len: usize) -> usize {
    (HEADER + 16 + 2 + name_len).div_ceil(16) * 16
}

/// Serialize an asset into the bytes of a `.rasset` file.
pub fn to_bytes<T>(value: &T, kind: AssetKind) -> Result<Vec<u8>, AssetError>
where
    T: Asset,
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
    let name = value.name().as_bytes();
    let name = &name[..name.len().min(u16::MAX as usize)];
    let len = header_len(name.len());
    let mut out = Vec::with_capacity(len + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.push(kind.byte);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&value.id().0.to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(name);
    out.resize(len, 0);
    out.extend_from_slice(&body);
    Ok(out)
}

/// What the header says: the kind, the ID and the name — all a library
/// needs to keep an asset, without knowing its format.
pub fn head_of(bytes: &[u8]) -> Result<(AssetKind, AssetId, String), AssetError> {
    let kind = kind_of(bytes)?;
    split_header(bytes)?;
    let id = u128::from_le_bytes(
        bytes[HEADER..HEADER + 16]
            .try_into()
            .expect("sixteen bytes"),
    );
    let n = u16::from_le_bytes([bytes[HEADER + 16], bytes[HEADER + 17]]) as usize;
    let name = std::str::from_utf8(&bytes[HEADER + 18..HEADER + 18 + n])
        .map_err(|e| AssetError::Corrupt(e.to_string()))?;
    Ok((kind, AssetId(id), name.to_string()))
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
    if cfg!(target_arch = "wasm32") {
        return crate::files::read(path.as_ref()).is_ok_and(|b| split_header(&b).is_ok());
    }
    use std::io::Read;
    let mut header = [0u8; HEADER];
    std::fs::File::open(path.as_ref())
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok()
        && header[..8] == MAGIC
        && u32::from_le_bytes([header[8], header[9], header[10], header[11]]) == FORMAT_VERSION
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
    if bytes.len() < HEADER + 18 {
        return Err(AssetError::BadMagic);
    }
    let n = u16::from_le_bytes([bytes[HEADER + 16], bytes[HEADER + 17]]) as usize;
    let len = header_len(n);
    if bytes.len() < len {
        return Err(AssetError::BadMagic);
    }
    Ok(&bytes[len..])
}

/// Read a `.rasset` off disk into owned bytes whose archived form can be
/// borrowed with [`view`].
///
/// The bytes are kept, not decoded: the whole point is that the vertex data
/// is already in the layout the GPU wants, so the load path ends here and the
/// next step is an upload.
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, AssetError> {
    let bytes = crate::files::read(path.as_ref())?;
    split_header(&bytes)?;
    Ok(bytes)
}

/// Only a `.rasset`'s header off disk — its kind, ID and name — leaving
/// the body where it is: what a library opened lazily knows of an asset
/// before it is first used.
pub fn read_head(path: impl AsRef<Path>) -> Result<(AssetKind, AssetId, String), AssetError> {
    // In the browser the file is already in memory, whole.
    if cfg!(target_arch = "wasm32") {
        return head_of_prefix(&crate::files::read(path.as_ref())?);
    }
    use std::io::Read;
    let mut file = std::fs::File::open(path.as_ref())?;
    let mut head = vec![0u8; HEADER + 18];
    file.read_exact(&mut head)?;
    split_header_prefix(&head)?;
    let n = u16::from_le_bytes([head[HEADER + 16], head[HEADER + 17]]) as usize;
    head.resize(HEADER + 18 + n, 0);
    file.read_exact(&mut head[HEADER + 18..])?;
    head_of_prefix(&head)
}

/// The header checks of [`split_header`], on the header alone.
fn split_header_prefix(bytes: &[u8]) -> Result<(), AssetError> {
    if bytes.len() < HEADER + 18 || bytes[..8] != MAGIC {
        return Err(AssetError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != FORMAT_VERSION {
        return Err(AssetError::Version {
            found: version,
            expected: FORMAT_VERSION,
        });
    }
    Ok(())
}

/// [`head_of`], on the header alone.
fn head_of_prefix(bytes: &[u8]) -> Result<(AssetKind, AssetId, String), AssetError> {
    let kind = kind_of(bytes)?;
    split_header_prefix(bytes)?;
    let id = u128::from_le_bytes(
        bytes[HEADER..HEADER + 16]
            .try_into()
            .expect("sixteen bytes"),
    );
    let n = u16::from_le_bytes([bytes[HEADER + 16], bytes[HEADER + 17]]) as usize;
    let name = std::str::from_utf8(&bytes[HEADER + 18..HEADER + 18 + n])
        .map_err(|e| AssetError::Corrupt(e.to_string()))?;
    Ok((kind, AssetId(id), name.to_string()))
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
