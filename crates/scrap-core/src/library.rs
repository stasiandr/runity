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

use crate::asset::{self, AssetError, AssetId, AssetKind};

/// Read an asset's id out of its bytes, whatever kind it is.
///
/// One function rather than a `match` repeated at every call site: adding a
/// kind used to mean finding three of these, and missing one meant an asset
/// that loaded but could not be looked up.
/// The ID an archive's header gives it.
fn id_of(bytes: &[u8], _kind: AssetKind) -> Option<AssetId> {
    asset::head_of(bytes).ok().map(|(_, id, _)| id)
}

/// The name an asset was built with — its source file's stem — read from
/// the asset itself.
///
/// Not from the library file's name: that is the importer's business, and
/// it is the asset's ID (`<id>.scrasset`), so that two sources with one stem
/// do not overwrite each other. The name a scene uses is the one inside.
/// The name an archive's header gives it.
fn name_of(bytes: &[u8], _kind: AssetKind) -> Option<String> {
    asset::head_of(bytes).ok().map(|(_, _, name)| name)
}

/// One asset's bytes, plus where they came from.
struct Entry {
    /// Read when first asked for in a library opened lazily
    /// ([`Library::open_lazy`]), and given back by [`Library::release`].
    bytes: std::sync::OnceLock<Vec<u8>>,
    /// Whether the bytes can be read again from `path`: not for assets
    /// handed in as bytes (the web's).
    on_disk: bool,
    id: AssetId,
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
    crate::files::modified(path)
}

/// Everything imported, indexed by id and by name.
#[derive(Default)]
pub struct Library {
    entries: Vec<Entry>,
    by_id: HashMap<AssetId, usize>,
    /// A list rather than one slot, because a stem is not unique. One
    /// directory cannot hold two assets with the same stem — the importer
    /// writes `<stem>.scrasset` — but a library assembled from several
    /// directories with [`Library::add`] can, and `boulder.scrmat` beside a
    /// `boulder.obj` is a reasonable thing to want. With one slot the second
    /// one read replaces the first, and the symptom is an asset that is on
    /// disk and cannot be found by name.
    by_name: HashMap<String, Vec<usize>>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read every `.scrasset` in a directory.
    ///
    /// A file that is not ours, or is from an older format, is reported and
    /// skipped rather than taking the whole library down with it: one bad
    /// asset should cost one missing model, not a black screen.
    pub fn open(
        directory: impl AsRef<Path>,
    ) -> Result<(Self, Vec<(PathBuf, AssetError)>), AssetError> {
        let mut library = Self::new();
        let mut problems = Vec::new();
        for entry in crate::files::read_dir(directory.as_ref())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("scrasset") {
                continue;
            }
            if let Err(e) = library.add(&path) {
                problems.push((path, e));
            }
        }
        Ok((library, problems))
    }

    /// Every `.scrasset` in `folder` of a game's [`crate::data::Data`]:
    /// [`Library::open`] through the seam, for the web, where the host
    /// fetched them. Such assets are not polled for changes.
    pub fn open_from(
        data: &dyn crate::data::Data,
        folder: &str,
    ) -> (Self, Vec<(String, AssetError)>) {
        let mut library = Self::new();
        let mut problems = Vec::new();
        for path in data.list(folder).unwrap_or_default() {
            if !path.ends_with(".scrasset") {
                continue;
            }
            let added = data
                .read(&path)
                .map_err(AssetError::from)
                .and_then(|bytes| library.add_bytes(PathBuf::from(&path), bytes));
            if let Err(e) = added {
                problems.push((path, e));
            }
        }
        (library, problems)
    }

    /// Every `.scrasset` in a directory, but only their headers: an asset's
    /// body is read the first time something asks for it, and can be given
    /// back with [`Library::release`] once it is on the GPU — what streams
    /// a world's meshes and pictures off disk as it needs them, rather
    /// than holding every one in memory from the start. A body that turns
    /// out corrupt reads as missing, where [`Library::open`] would have
    /// named it at once.
    pub fn open_lazy(
        directory: impl AsRef<Path>,
    ) -> Result<(Self, Vec<(PathBuf, AssetError)>), AssetError> {
        let mut library = Self::new();
        let mut problems = Vec::new();
        for entry in crate::files::read_dir(directory.as_ref())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("scrasset") {
                continue;
            }
            match asset::read_head(&path) {
                Ok((kind, id, name)) => {
                    let index = library.entries.len();
                    library.entries.push(Entry {
                        bytes: std::sync::OnceLock::new(),
                        on_disk: true,
                        id,
                        kind,
                        modified: modified_at(&path),
                        path,
                        name: name.clone(),
                    });
                    library.by_id.insert(id, index);
                    library.by_name.entry(name).or_default().push(index);
                }
                Err(e) => problems.push((path, e)),
            }
        }
        Ok((library, problems))
    }

    /// An entry's bytes, read off disk now if they are not yet.
    fn loaded(entry: &Entry) -> Option<&[u8]> {
        if let Some(bytes) = entry.bytes.get() {
            return Some(bytes.as_slice());
        }
        let bytes = asset::read(&entry.path).ok()?;
        // Sound before it is kept: every later look is a cast.
        asset::kind_of(&bytes).ok().filter(|k| *k == entry.kind)?;
        let _ = entry.bytes.set(bytes);
        entry.bytes.get().map(Vec::as_slice)
    }

    /// Give back the bytes of every asset read from disk that `keep` does
    /// not want — once its mesh or picture is on the GPU, a game does not
    /// need it in memory too. Asked for again, it is read again. What was
    /// freed, in bytes.
    pub fn release(&mut self, keep: impl Fn(AssetId) -> bool) -> usize {
        let mut freed = 0;
        for entry in &mut self.entries {
            if !entry.on_disk || keep(entry.id) {
                continue;
            }
            if let Some(bytes) = entry.bytes.take() {
                freed += bytes.len();
            }
        }
        freed
    }

    /// Bytes of assets held in memory now.
    pub fn resident_bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|e| e.bytes.get())
            .map(Vec::len)
            .sum()
    }

    /// Add one asset file.
    pub fn add(&mut self, path: impl AsRef<Path>) -> Result<AssetId, AssetError> {
        let path = path.as_ref();
        let bytes = asset::read(path)?;
        let id = self.add_bytes(path.to_path_buf(), bytes)?;
        if let Some(&index) = self.by_id.get(&id) {
            self.entries[index].modified = modified_at(path);
        }
        Ok(id)
    }

    /// Add an asset's bytes, known by `path`.
    fn add_bytes(&mut self, path: PathBuf, bytes: Vec<u8>) -> Result<AssetId, AssetError> {
        asset::split_header(&bytes)?;
        let kind = asset::kind_of(&bytes)?;
        // The body is validated here, on the way in, so that every later
        // lookup is a cast into bytes already known to be sound.
        let id = id_of(&bytes, kind)
            .ok_or_else(|| AssetError::Corrupt(format!("{kind:?} body did not validate")))?;
        let name = name_of(&bytes, kind).unwrap_or_default();

        let index = self.entries.len();
        let on_disk = crate::files::is_file(&path);
        self.entries.push(Entry {
            bytes: std::sync::OnceLock::from(bytes),
            on_disk,
            id,
            kind,
            path,
            modified: None,
            name: name.clone(),
        });
        self.by_id.insert(id, index);
        self.by_name.entry(name).or_default().push(index);
        Ok(id)
    }

    /// Add every `.scrasset` in `directory` that is not in the library yet.
    ///
    /// What [`Library::reload_changed`] cannot see: it re-reads files it
    /// already has, and an asset imported while the game runs is a file it
    /// has never heard of. Unreadable files are skipped, as in
    /// [`Library::open`]; the next call tries them again.
    pub fn add_new(&mut self, directory: impl AsRef<Path>) -> Vec<Reloaded> {
        let Ok(entries) = crate::files::read_dir(directory.as_ref()) else {
            return Vec::new();
        };
        let known: std::collections::HashSet<PathBuf> =
            self.entries.iter().map(|e| e.path.clone()).collect();
        let mut fresh: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("scrasset"))
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

    /// An asset's archive, when it is of this kind: what a module reads
    /// its format from (`MeshLibrary::mesh`…).
    pub fn bytes_of(&self, id: AssetId, kind: AssetKind) -> Option<&[u8]> {
        let entry = self.entries.get(*self.by_id.get(&id)?)?;
        (entry.kind == kind).then(|| Self::loaded(entry)).flatten()
    }

    /// The archive of the one asset of this kind with this file stem.
    pub fn bytes_named(&self, name: &str, kind: AssetKind) -> Option<&[u8]> {
        self.named(name, kind).and_then(Self::loaded)
    }

    /// An asset a link names, of this kind: by its ID when the library
    /// knows it, by its name otherwise (docs/refs.md). By ID is what tells
    /// two `rock` models in two folders apart.
    pub fn bytes_linked(&self, link: &crate::AssetLink, kind: AssetKind) -> Option<&[u8]> {
        link.id
            .and_then(|id| self.bytes_of(id, kind))
            .or_else(|| self.bytes_named(link, kind))
    }

    /// The one asset of this kind with this file stem.
    fn named(&self, name: &str, kind: AssetKind) -> Option<&Entry> {
        self.by_name
            .get(name)?
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .find(|e| e.kind == kind)
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
                Some((entry.id, entry.name.as_str()))
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
        Some(self.entries.get(index)?.id)
    }

    /// Every asset of one kind, by the name a scene would use.
    ///
    /// What the editor's content browser lists, and what makes a palette
    /// visible: the materials in a project are whatever this returns for
    /// a material, in the order they were read.
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

            self.entries[index].bytes = std::sync::OnceLock::from(bytes);
            self.entries[index].id = id;
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
