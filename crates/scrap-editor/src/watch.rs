//! What changed on disk, told by the operating system rather than looked
//! for: a count that goes up whenever a file of the open project is made,
//! written, moved or removed. The editor keeps what it read from the
//! project — the asset listing, the scene's stamps — until the count
//! moves, instead of walking the project to find out; on a big one
//! (Dacha's four thousand files) every walk was tens of milliseconds, and
//! the editor walked it several times a second and at every click.
//!
//! Left out: what is derived or the editor's own (`target/`, `build/`, the
//! hidden folders — `.scrap/`, where the editor writes its layout twice a
//! second, and `.git/`). The library is in: a built asset counts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use notify::Watcher as _;

/// Watching one project's folder.
pub(crate) struct Watch {
    pub root: PathBuf,
    changes: Arc<AtomicU64>,
    _watcher: notify::RecommendedWatcher,
}

impl Watch {
    /// Watching `root` and everything under it; `None` where the system
    /// cannot say (the caller looks for itself then).
    pub fn start(root: &Path) -> Option<Self> {
        let changes = Arc::new(AtomicU64::new(0));
        let counted = changes.clone();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let seen_root = canonical.clone();
        let plain_root = root.to_path_buf();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                // Lost track (an overflow): say something changed.
                counted.fetch_add(1, Ordering::Release);
                return;
            };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            let counts = event.paths.is_empty()
                || event.paths.iter().any(|path| {
                    let inside = path
                        .strip_prefix(&seen_root)
                        .or_else(|_| path.strip_prefix(&plain_root));
                    inside.is_ok_and(counts_for)
                });
            if counts {
                counted.fetch_add(1, Ordering::Release);
            }
        })
        .ok()?;
        watcher
            .watch(&canonical, notify::RecursiveMode::Recursive)
            .ok()?;
        Some(Self {
            root: root.to_path_buf(),
            changes,
            _watcher: watcher,
        })
    }

    /// How many changes there have been.
    pub fn changes(&self) -> u64 {
        self.changes.load(Ordering::Acquire)
    }
}

/// Whether a change at `relative` (to the project's root) is one the
/// editor reads: not derived, not its own.
fn counts_for(relative: &Path) -> bool {
    let mut parts = relative.components().map(|c| c.as_os_str().to_string_lossy());
    let Some(first) = parts.next() else {
        return true;
    };
    if matches!(first.as_ref(), "target" | "build") || first.starts_with('.') {
        return false;
    }
    !parts.any(|part| part.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_derived_or_the_editors_own_does_not_count() {
        assert!(counts_for(Path::new("prefabs/crate.prefab")));
        assert!(counts_for(Path::new("library/0a1b.scrasset")));
        assert!(!counts_for(Path::new(".scrap/studio.ron")));
        assert!(!counts_for(Path::new("target/release/x")));
        assert!(!counts_for(Path::new("assets/.DS_Store")));
    }

    #[test]
    fn a_file_written_under_the_root_is_counted() {
        let dir = std::env::temp_dir().join(format!("scrap-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".scrap")).unwrap();
        let Some(watch) = Watch::start(&dir) else { return };
        std::fs::write(dir.join(".scrap/layout.ron"), "()").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let before = watch.changes();
        std::fs::write(dir.join("crate.prefab"), "()").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while watch.changes() == before && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(watch.changes() > before, "the write was told");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
