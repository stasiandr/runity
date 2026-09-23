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
    AssetKind, MaterialAsset, MeshAsset, SoundAsset, TextureAsset,
};
use crate::material::Material;

/// Read an asset's id out of its bytes, whatever kind it is.
///
/// One function rather than a `match` repeated at every call site: adding a
/// kind used to mean finding three of these, and missing one meant an asset
/// that loaded but could not be looked up.
fn id_of(bytes: &[u8], kind: AssetKind) -> Option<AssetId> {
    match kind {
        AssetKind::Mesh => asset::view::<MeshAsset>(bytes).ok().map(|a| (&a.id).into()),
        AssetKind::Texture => asset::view::<TextureAsset>(bytes)
            .ok()
            .map(|a| (&a.id).into()),
        AssetKind::Sound => asset::view::<SoundAsset>(bytes)
            .ok()
            .map(|a| (&a.id).into()),
        AssetKind::Material => asset::view::<MaterialAsset>(bytes)
            .ok()
            .map(|a| (&a.id).into()),
    }
}

/// The name an asset was built with — its source file's stem — read from
/// the asset itself.
///
/// Not from the library file's name: that is the importer's business, and
/// it is the asset's ID (`<id>.rasset`), so that two sources with one stem
/// do not overwrite each other. The name a scene uses is the one inside.
fn name_of(bytes: &[u8], kind: AssetKind) -> Option<String> {
    let name = match kind {
        AssetKind::Mesh => asset::view::<MeshAsset>(bytes).ok()?.name.as_str(),
        AssetKind::Texture => asset::view::<TextureAsset>(bytes).ok()?.name.as_str(),
        AssetKind::Sound => asset::view::<SoundAsset>(bytes).ok()?.name.as_str(),
        AssetKind::Material => asset::view::<MaterialAsset>(bytes).ok()?.name.as_str(),
    };
    Some(name.to_string())
}

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
    /// The name inside the asset — its source's file stem — which is what a
    /// hand-written scene uses to name a model.
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
    /// A list rather than one slot, because a stem is not unique. One
    /// directory cannot hold two assets with the same stem — the importer
    /// writes `<stem>.rasset` — but a library assembled from several
    /// directories with [`Library::add`] can, and `boulder.rmat` beside a
    /// `boulder.obj` is a reasonable thing to want. With one slot the second
    /// one read replaces the first, and the symptom is an asset that is on
    /// disk and cannot be found by name.
    by_name: HashMap<String, Vec<usize>>,
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
        // The body is validated here, on the way in, so that every later
        // lookup is a cast into bytes already known to be sound.
        let id = id_of(&bytes, kind)
            .ok_or_else(|| AssetError::Corrupt(format!("{kind:?} body did not validate")))?;
        let name = name_of(&bytes, kind).unwrap_or_default();

        let index = self.entries.len();
        self.entries.push(Entry {
            bytes,
            kind,
            path: path.to_path_buf(),
            modified: modified_at(path),
            name: name.clone(),
        });
        self.by_id.insert(id, index);
        self.by_name.entry(name).or_default().push(index);
        Ok(id)
    }

    /// Add every `.rasset` in `directory` that is not in the library yet.
    ///
    /// What [`Library::reload_changed`] cannot see: it re-reads files it
    /// already has, and an asset imported while the game runs is a file it
    /// has never heard of. Unreadable files are skipped, as in
    /// [`Library::open`]; the next call tries them again.
    pub fn add_new(&mut self, directory: impl AsRef<Path>) -> Vec<Reloaded> {
        let Ok(entries) = std::fs::read_dir(directory.as_ref()) else {
            return Vec::new();
        };
        let known: std::collections::HashSet<PathBuf> =
            self.entries.iter().map(|e| e.path.clone()).collect();
        let mut fresh: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("rasset"))
            .filter(|path| !known.contains(path))
            .collect();
        fresh.sort();
        fresh
            .into_iter()
            .filter_map(|path| {
                let id = self.add(&path).ok()?;
                let kind = self.entries[self.by_id[&id]].kind;
                Some(Reloaded { id, kind, path })
            })
            .collect()
    }

    /// The name a scene uses for this asset.
    pub fn name(&self, id: AssetId) -> Option<&str> {
        Some(self.entries[*self.by_id.get(&id)?].name.as_str())
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

    /// The one asset of this kind with this file stem.
    fn named(&self, name: &str, kind: AssetKind) -> Option<&Entry> {
        self.by_name
            .get(name)?
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .find(|e| e.kind == kind)
    }

    /// The material an id names.
    ///
    /// Returned by value: a material is four numbers, and a borrow would tie
    /// every surface in a scene to the library's lifetime for nothing.
    pub fn material(&self, id: AssetId) -> Option<Material> {
        let entry = self.entries.get(*self.by_id.get(&id)?)?;
        (entry.kind == AssetKind::Material)
            .then(|| asset::view::<MaterialAsset>(&entry.bytes).ok())
            .flatten()
            .map(|a| Material::from(&a.material))
    }

    /// Look a material up by file stem, which is what a scene writes.
    ///
    /// This is the palette: `material: "mossy_stone"` in any scene finds the
    /// one asset, and changing that asset changes every scene that used it.
    pub fn material_by_name(&self, name: &str) -> Option<Material> {
        let entry = self.named(name, AssetKind::Material)?;
        asset::view::<MaterialAsset>(&entry.bytes)
            .ok()
            .map(|a| Material::from(&a.material))
    }

    /// Look a sound up by file stem.
    pub fn sound_by_name(&self, name: &str) -> Option<&ArchivedSoundAsset> {
        asset::view::<SoundAsset>(&self.named(name, AssetKind::Sound)?.bytes).ok()
    }

    /// Look a texture up by file stem.
    pub fn texture_by_name(&self, name: &str) -> Option<&ArchivedTextureAsset> {
        asset::view::<TextureAsset>(&self.named(name, AssetKind::Texture)?.bytes).ok()
    }

    /// Look an asset up by file stem.
    ///
    /// A convenience for scenes written by hand and for tests. Once the
    /// editor writes scenes, it writes ids, and this stops being on the path
    /// anything important takes.
    pub fn mesh_by_name(&self, name: &str) -> Option<&ArchivedMeshAsset> {
        asset::view::<MeshAsset>(&self.named(name, AssetKind::Mesh)?.bytes).ok()
    }

    /// Follow a link to a mesh: by its ID when it has one the library
    /// knows, by its name otherwise (docs/refs.md). By ID is what tells two
    /// `rock` models in two folders apart.
    pub fn mesh_link(&self, link: &crate::AssetLink) -> Option<&ArchivedMeshAsset> {
        link.id
            .and_then(|id| self.mesh(id))
            .or_else(|| self.mesh_by_name(link))
    }

    /// What a link names, as the library has it now: the ID and the name
    /// inside the asset. By ID first, then by name — only when the name is
    /// one asset's of that kind, since a guess between two would point a
    /// line at the wrong thing.
    pub fn find(&self, link: &crate::AssetLink, kind: AssetKind) -> Option<(AssetId, &str)> {
        if let Some(id) = link.id {
            if let Some(&index) = self.by_id.get(&id) {
                let entry = &self.entries[index];
                if entry.kind == kind {
                    return Some((id, entry.name.as_str()));
                }
            }
        }
        let named: Vec<usize> = self
            .by_name
            .get(link.as_str())?
            .iter()
            .copied()
            .filter(|&i| self.entries[i].kind == kind)
            .collect();
        match named.as_slice() {
            [one] => {
                let entry = &self.entries[*one];
                Some((id_of(&entry.bytes, entry.kind)?, entry.name.as_str()))
            }
            _ => None,
        }
    }

    /// The id of the first asset read with this file stem, of any kind.
    ///
    /// First rather than "the" because a name is not unique across kinds;
    /// anything that cares which one it is asking for calls the by-kind
    /// lookup instead.
    pub fn id_by_name(&self, name: &str) -> Option<AssetId> {
        let index = *self.by_name.get(name)?.first()?;
        let entry = self.entries.get(index)?;
        id_of(&entry.bytes, entry.kind)
    }

    /// Every asset of one kind, by the name a scene would use.
    ///
    /// What the editor's content browser lists, and what makes a palette
    /// visible: the materials in a project are whatever this returns for
    /// [`AssetKind::Material`], in the order they were read.
    pub fn names_of(&self, kind: AssetKind) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(move |e| e.kind == kind)
            .map(|e| e.name.as_str())
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
            let Some(id) = id_of(&bytes, kind) else {
                continue;
            };

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
