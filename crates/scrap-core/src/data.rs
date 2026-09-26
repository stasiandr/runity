//! Where a game's data comes from (DNA, "Не закрывать веб": reading data
//! through a seam). On the desktop it is files on disk; on the web there is
//! no disk, and the host fetches the data before the game starts and hands
//! it over as bytes. Scenes, prefabs and the asset library read through
//! [`Data`] (`Scene::load_from`, `Prefabs::open_from`,
//! `Library::open_from`), so a game reads the same way wherever it runs.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

/// A game's files, by their path relative to its data (`scenes/main.ron`,
/// forward slashes).
pub trait Data {
    /// A file's bytes.
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
    /// The files directly in a folder, by their paths.
    fn list(&self, folder: &str) -> io::Result<Vec<String>>;

    /// The files in a folder and every folder under it, by their paths,
    /// sorted — `""` for all of them. What finding a kind wherever it lies
    /// needs (docs/layout.md).
    fn walk(&self, folder: &str) -> io::Result<Vec<String>> {
        self.list(folder)
    }

    /// A file's text.
    fn read_text(&self, path: &str) -> io::Result<String> {
        String::from_utf8(self.read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Files on disk under a root: the desktop, and the editor.
#[derive(Debug, Clone)]
pub struct Disk {
    pub root: PathBuf,
}

impl Disk {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Data for Disk {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.root.join(path))
    }

    fn list(&self, folder: &str) -> io::Result<Vec<String>> {
        let mut out: Vec<String> = std::fs::read_dir(self.root.join(folder))?
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| join(folder, &e.file_name().to_string_lossy()))
            .collect();
        out.sort();
        Ok(out)
    }

    fn walk(&self, folder: &str) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        let mut folders = vec![folder.to_string()];
        while let Some(here) = folders.pop() {
            for entry in std::fs::read_dir(self.root.join(&here))?.flatten() {
                let path = join(&here, &entry.file_name().to_string_lossy());
                if entry.path().is_dir() {
                    folders.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

/// Files already in memory: what a web host fetched before the game
/// started, or what a test made up.
#[derive(Debug, Clone, Default)]
pub struct Preloaded {
    files: BTreeMap<String, Vec<u8>>,
}

impl Preloaded {
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a file in, at its path.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> &mut Self {
        self.files.insert(path.into(), bytes.into());
        self
    }
}

impl Data for Preloaded {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path}: not preloaded")))
    }

    fn list(&self, folder: &str) -> io::Result<Vec<String>> {
        let prefix = if folder.is_empty() { String::new() } else { format!("{}/", folder.trim_end_matches('/')) };
        Ok(self
            .files
            .keys()
            .filter(|p| p.strip_prefix(&prefix).is_some_and(|rest| !rest.contains('/')))
            .cloned()
            .collect())
    }

    fn walk(&self, folder: &str) -> io::Result<Vec<String>> {
        let prefix = if folder.is_empty() { String::new() } else { format!("{}/", folder.trim_end_matches('/')) };
        Ok(self.files.keys().filter(|p| p.starts_with(&prefix)).cloned().collect())
    }
}

/// `folder/name`, or `name` at the top.
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{}/{name}", folder.trim_end_matches('/'))
    }
}

/// A path's file stem: `prefabs/crate.prefab` is `crate`.
pub(crate) fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split_once('.').map_or(name, |(stem, _)| stem).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preloaded_files_list_by_folder_like_a_disk() {
        let mut data = Preloaded::new();
        data.insert("scenes/main.ron", "(entities: [])")
            .insert("scenes/caves/deep.ron", "(entities: [])")
            .insert("scrap.ron", "()");
        assert_eq!(data.list("scenes").unwrap(), ["scenes/main.ron"]);
        assert_eq!(data.list("").unwrap(), ["scrap.ron"]);
        assert_eq!(data.read_text("scenes/main.ron").unwrap(), "(entities: [])");
        assert!(data.read("nowhere.ron").is_err());
        assert_eq!(stem("prefabs/crate.prefab"), "crate");
    }

    #[test]
    fn a_scene_and_its_prefabs_read_from_bytes_as_from_disk() {
        let mut data = Preloaded::new();
        data.insert("scenes/main.ron", r#"(entities: [(name: "box", prefab: "crate")])"#)
            .insert("prefabs/crate.prefab", r#"(name: "crate")"#)
            .insert(
                "prefabs/crate.prefab.scrimport",
                r#"ImportSettings(source: "prefabs/crate.prefab", id: Some("0000000000000000000000000000002a"))"#,
            );
        let scene = crate::Scene::load_from(&data, "scenes/main.ron").unwrap();
        assert!(!scene.entities[0].id.is_unassigned(), "ids assigned as on load");
        let (prefabs, problems) = crate::Prefabs::open_from(&data, "prefabs");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(prefabs.get("crate").is_some());
        assert_eq!(prefabs.id_of("crate"), Some(crate::AssetId(42)));
    }
}
