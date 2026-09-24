//! The game's files by path, wherever it runs (DNA, "Не закрывать веб":
//! reading data through a seam).
//!
//! On the desktop these are `std::fs` and nothing more. In the browser there
//! is no disk: the web host fetches the project's data before the game
//! starts and [`mount`]s it under the root the game was compiled against
//! (its `CARGO_MANIFEST_DIR`), and every read below finds it there — so
//! `LiveScene`, `Tuned`, the screens, the strings and the library read the
//! same paths in both places, and none of them needs a second way to load.
//! What a game writes (player prefs, a save) stays in memory there.
//!
//! [`Data`](crate::data::Data) is the seam for code that is handed its
//! data; this is the one under code that is handed a path.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Files put in memory, by their full normalised path.
fn mounted() -> &'static Mutex<BTreeMap<PathBuf, Vec<u8>>> {
    static FILES: OnceLock<Mutex<BTreeMap<PathBuf, Vec<u8>>>> = OnceLock::new();
    FILES.get_or_init(Default::default)
}

/// Put `files` (paths relative to `root`, forward slashes) in memory under
/// `root`: what the web host does with the project it fetched. Later reads
/// of those paths find them before the disk.
pub fn mount(root: impl AsRef<Path>, files: impl IntoIterator<Item = (String, Vec<u8>)>) {
    let root = normal(root.as_ref());
    let mut map = mounted().lock().unwrap();
    for (path, bytes) in files {
        map.insert(root.join(path), bytes);
    }
}

/// `a/./b/../c` as `a/c`: the same file however it was spelled.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn in_memory<R>(path: &Path, f: impl FnOnce(&BTreeMap<PathBuf, Vec<u8>>, &Path) -> R) -> R {
    let map = mounted().lock().unwrap();
    f(&map, &normal(path))
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: no such file", path.display()),
    )
}

/// A file's bytes.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    if let Some(bytes) = in_memory(path, |map, p| map.get(p).cloned()) {
        return Ok(bytes);
    }
    #[cfg(target_arch = "wasm32")]
    return Err(not_found(path));
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::read(path)
}

/// A file's text.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Called with every file written to memory: how the web host keeps the
/// player's files between visits (in the browser's storage).
type Written = Box<dyn Fn(&Path, &[u8]) + Send + Sync>;

fn written() -> &'static Mutex<Option<Written>> {
    static WRITTEN: OnceLock<Mutex<Option<Written>>> = OnceLock::new();
    WRITTEN.get_or_init(Default::default)
}

/// Be told of every file written to memory from now on.
pub fn on_write(f: impl Fn(&Path, &[u8]) + Send + Sync + 'static) {
    *written().lock().unwrap() = Some(Box::new(f));
}

/// Write a file: to disk on the desktop, to memory where the files are
/// mounted (a browser has nowhere else to put them).
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    if cfg!(target_arch = "wasm32") || in_memory(path, |map, p| map.contains_key(p)) {
        let path = normal(path);
        mounted()
            .lock()
            .unwrap()
            .insert(path.clone(), bytes.as_ref().to_vec());
        if let Some(f) = written().lock().unwrap().as_ref() {
            f(&path, bytes.as_ref());
        }
        return Ok(());
    }
    std::fs::write(path, bytes)
}

/// Make a folder and its parents; in memory a folder is only its files, so
/// there is nothing to make.
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    if cfg!(target_arch = "wasm32") {
        return Ok(());
    }
    std::fs::create_dir_all(path)
}

/// Whether a file is there.
pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    in_memory(path, |map, p| map.contains_key(p))
        || (!cfg!(target_arch = "wasm32") && path.is_file())
}

/// Whether a folder is there: in memory, whether any file is under it.
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    in_memory(path, |map, p| {
        map.keys().any(|k| k != p && k.starts_with(p))
    }) || (!cfg!(target_arch = "wasm32") && path.is_dir())
}

/// Whether anything is there.
pub fn exists(path: impl AsRef<Path>) -> bool {
    is_file(&path) || is_dir(&path)
}

/// What is directly in a folder — files and folders — by full path, sorted.
pub fn list(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let path = path.as_ref();
    let mut out: Vec<PathBuf> = in_memory(path, |map, p| {
        map.keys()
            .filter_map(|k| {
                let rest = k.strip_prefix(p).ok()?;
                let first = rest.components().next()?;
                Some(p.join(first))
            })
            .collect()
    });
    #[cfg(not(target_arch = "wasm32"))]
    if out.is_empty() {
        out = std::fs::read_dir(path)?
            .flatten()
            .map(|e| e.path())
            .collect();
    }
    if out.is_empty() && !is_dir(path) {
        return Err(not_found(path));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// A folder's entry, as [`read_dir`] gives it: `std::fs::DirEntry`'s
/// `path` and `file_name`, which is what reading a folder of data needs.
#[derive(Debug, Clone)]
pub struct Entry(PathBuf);

impl Entry {
    pub fn path(&self) -> PathBuf {
        self.0.clone()
    }

    pub fn file_name(&self) -> std::ffi::OsString {
        self.0.file_name().unwrap_or_default().to_os_string()
    }
}

/// [`list`] shaped like `std::fs::read_dir`, so a loop over a folder reads
/// the same over memory as over a disk.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<std::vec::IntoIter<io::Result<Entry>>> {
    Ok(list(path)?
        .into_iter()
        .map(|p| Ok(Entry(p)))
        .collect::<Vec<_>>()
        .into_iter())
}

/// When a file last changed — for reloading what was saved while the game
/// runs. `None` in memory: nothing changes a mounted file behind the game's
/// back.
pub fn modified(path: impl AsRef<Path>) -> Option<SystemTime> {
    if cfg!(target_arch = "wasm32") || in_memory(path.as_ref(), |map, p| map.contains_key(p)) {
        return None;
    }
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounted_files_read_list_and_write_like_a_disk() {
        let root = Path::new("/nowhere/scrap-files-test");
        mount(
            root,
            [
                ("scenes/main.ron".to_string(), b"(entities: [])".to_vec()),
                ("scenes/rush.ron".to_string(), b"(entities: [])".to_vec()),
                ("ui/menu.ron".to_string(), b"()".to_vec()),
            ],
        );
        assert_eq!(
            read_to_string(root.join("scenes/../scenes/main.ron")).unwrap(),
            "(entities: [])"
        );
        assert!(is_file(root.join("ui/menu.ron")));
        assert!(is_dir(root.join("scenes")) && !is_file(root.join("scenes")));
        assert_eq!(list(root).unwrap(), [root.join("scenes"), root.join("ui")]);
        assert_eq!(list(root.join("scenes")).unwrap().len(), 2);
        assert!(read(root.join("missing.ron")).is_err());
        assert!(modified(root.join("ui/menu.ron")).is_none());
        write(root.join("ui/menu.ron"), "(x)").unwrap();
        assert_eq!(read_to_string(root.join("ui/menu.ron")).unwrap(), "(x)");
    }
}
