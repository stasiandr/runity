//! The asset library: the one place allowed to know where a file sits.
//!
//! Scenes and assets point at each other with [`AssetId`], never with paths,
//! so something has to map an id back to bytes on disk. That something is
//! here, and keeping it in exactly one place is what makes moving a file a
//! non-event.
//!
//! Assets are read once and kept as bytes. Nothing is decoded on the way in —
//! an [`ArchivedMeshAsset`] is a view into those bytes — so "loading" a
//! library is reading files, and the cost of keeping one open is the size of
//! the files themselves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::asset::{
    self, ArchivedMeshAsset, ArchivedSoundAsset, ArchivedTextureAsset, AssetError, AssetId,
    AssetKind, MeshAsset, SoundAsset, TextureAsset,
};

/// One asset's bytes, plus where they came from.
struct Entry {
    bytes: Vec<u8>,
    kind: AssetKind,
    path: PathBuf,
    /// When the file was last written, as of the last read.
    ///
    /// Polled rather than watched. A watcher means a thread, a channel and a
    /// debounce, and the engine is a guest — the editor already has a loop
    /// and can ask. Polling a few dozen timestamps costs nothing beside a
    /// frame.
    modified: Option<std::time::SystemTime>,
    /// The file stem, which is what a hand-written scene uses to name a model
    /// before an editor exists to write ids.
    name: String,
}

/// An asset that changed on disk and has been re-read.
#[derive(Debug, Clone, PartialEq)]
pub struct Reloaded {
    pub id: AssetId,
    pub kind: AssetKind,
    pub path: PathBuf,
}

fn modified_at(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Everything imported, indexed by id and by name.
#[derive(Default)]
pub struct Library {
    entries: Vec<Entry>,
    by_id: HashMap<AssetId, usize>,
    by_name: HashMap<String, usize>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read every `.rasset` in a directory.
    ///
    /// A file that is not ours, or is from an older format, is reported and
    /// skipped rather than taking the whole library down with it: one bad
    /// asset should cost one missing model, not a black screen.
    pub fn open(
        directory: impl AsRef<Path>,
    ) -> Result<(Self, Vec<(PathBuf, AssetError)>), AssetError> {
        let mut library = Self::new();
        let mut problems = Vec::new();
        for entry in std::fs::read_dir(directory.as_ref())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rasset") {
                continue;
            }
            if let Err(e) = library.add(&path) {
                problems.push((path, e));
            }
        }
        Ok((library, problems))
    }

    /// Add one asset file.
    pub fn add(&mut self, path: impl AsRef<Path>) -> Result<AssetId, AssetError> {
        let path = path.as_ref();
        let bytes = asset::read(path)?;
        let kind = asset::kind_of(&bytes)?;
        let id = match kind {
            AssetKind::Mesh => AssetId::from(&asset::view::<MeshAsset>(&bytes)?.id),
            AssetKind::Texture => AssetId::from(&asset::view::<TextureAsset>(&bytes)?.id),
            AssetKind::Sound => AssetId::from(&asset::view::<SoundAsset>(&bytes)?.id),
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let index = self.entries.len();
        self.entries.push(Entry {
            bytes,
            kind,
            path: path.to_path_buf(),
            modified: modified_at(path),
            name: name.clone(),
        });
        self.by_id.insert(id, index);
        self.by_name.insert(name, index);
        Ok(id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn mesh(&self, id: AssetId) -> Option<&ArchivedMeshAsset> {
        let entry = self.entries.get(*self.by_id.get(&id)?)?;
        (entry.kind == AssetKind::Mesh).then(|| asset::view::<MeshAsset>(&entry.bytes).ok())?
    }

    pub fn texture(&self, id: AssetId) -> Option<&ArchivedTextureAsset> {
        let entry = self.entries.get(*self.by_id.get(&id)?)?;
        (entry.kind == AssetKind::Texture)
            .then(|| asset::view::<TextureAsset>(&entry.bytes).ok())?
    }

    pub fn sound(&self, id: AssetId) -> Option<&ArchivedSoundAsset> {
        let entry = self.entries.get(*self.by_id.get(&id)?)?;
        (entry.kind == AssetKind::Sound).then(|| asset::view::<SoundAsset>(&entry.bytes).ok())?
    }

    /// Look a sound up by file stem.
    pub fn sound_by_name(&self, name: &str) -> Option<&ArchivedSoundAsset> {
        let entry = self.entries.get(*self.by_name.get(name)?)?;
        (entry.kind == AssetKind::Sound).then(|| asset::view::<SoundAsset>(&entry.bytes).ok())?
    }

    /// Look a texture up by file stem.
    pub fn texture_by_name(&self, name: &str) -> Option<&ArchivedTextureAsset> {
        let entry = self.entries.get(*self.by_name.get(name)?)?;
        (entry.kind == AssetKind::Texture)
            .then(|| asset::view::<TextureAsset>(&entry.bytes).ok())?
    }

    /// Look an asset up by file stem.
    ///
    /// A convenience for scenes written by hand and for tests. Once the
    /// editor writes scenes, it writes ids, and this stops being on the path
    /// anything important takes.
    pub fn mesh_by_name(&self, name: &str) -> Option<&ArchivedMeshAsset> {
        let entry = self.entries.get(*self.by_name.get(name)?)?;
        (entry.kind == AssetKind::Mesh).then(|| asset::view::<MeshAsset>(&entry.bytes).ok())?
    }

    pub fn id_by_name(&self, name: &str) -> Option<AssetId> {
        let entry = self.entries.get(*self.by_name.get(name)?)?;
        match entry.kind {
            AssetKind::Mesh => asset::view::<MeshAsset>(&entry.bytes)
                .ok()
                .map(|m| AssetId::from(&m.id)),
            AssetKind::Texture => asset::view::<TextureAsset>(&entry.bytes)
                .ok()
                .map(|t| AssetId::from(&t.id)),
            AssetKind::Sound => asset::view::<SoundAsset>(&entry.bytes)
                .ok()
                .map(|s| AssetId::from(&s.id)),
        }
    }

    /// Re-read every asset whose file has changed since it was loaded.
    ///
    /// Returns what changed, so the caller can re-upload exactly those and
    /// nothing else. An asset that now fails to read keeps its old bytes:
    /// a half-written file caught mid-save should not blank a model on
    /// screen, and the next poll will pick up the finished one.
    pub fn reload_changed(&mut self) -> Vec<Reloaded> {
        let mut changed = Vec::new();
        for index in 0..self.entries.len() {
            let path = self.entries[index].path.clone();
            let now = modified_at(&path);
            if now.is_none() || now == self.entries[index].modified {
                continue;
            }
            let Ok(bytes) = asset::read(&path) else {
                continue;
            };
            let Ok(kind) = asset::kind_of(&bytes) else {
                continue;
            };
            let id = match kind {
                AssetKind::Mesh => asset::view::<MeshAsset>(&bytes)
                    .ok()
                    .map(|m| AssetId::from(&m.id)),
                AssetKind::Texture => asset::view::<TextureAsset>(&bytes)
                    .ok()
                    .map(|t| AssetId::from(&t.id)),
                AssetKind::Sound => asset::view::<SoundAsset>(&bytes)
                    .ok()
                    .map(|s| AssetId::from(&s.id)),
            };
            let Some(id) = id else { continue };

            // An id is derived from the source path, so a re-import keeps
            // it — but a file replaced by a different asset entirely would
            // change it, and the index has to follow.
            let previous = self
                .by_id
                .iter()
                .find(|(_, i)| **i == index)
                .map(|(id, _)| *id);
            if let Some(previous) = previous {
                if previous != id {
                    self.by_id.remove(&previous);
                }
            }
            self.by_id.insert(id, index);

            self.entries[index].bytes = bytes;
            self.entries[index].kind = kind;
            self.entries[index].modified = now;
            changed.push(Reloaded {
                id,
                kind,
                path: path.clone(),
            });
        }
        changed
    }

    pub fn kind(&self, id: AssetId) -> Option<AssetKind> {
        Some(self.entries.get(*self.by_id.get(&id)?)?.kind)
    }

    pub fn path(&self, id: AssetId) -> Option<&Path> {
        Some(self.entries.get(*self.by_id.get(&id)?)?.path.as_path())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }
}
