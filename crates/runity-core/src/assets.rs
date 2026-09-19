//! Hot reload for on-disk assets.
//!
//! [`AssetCache`] wraps a single file path. Part 3 of card #33 asked for
//! polling roughly four times a second and reloading a changed file without
//! taking the game down with it: [`AssetCache::poll`] checks the file's
//! mtime no more often than [`AssetCache::poll_interval`] and only calls the
//! caller-supplied loader when that mtime has actually moved. This module
//! knows nothing about scenes, OBJ or PNG — it is a stat and a timer. The
//! loaders for those formats belong to the caller (the Valley assembly
//! card).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// A cached value reloaded from a file whenever its mtime changes.
///
/// Polling never panics and never discards the last good value: a loader
/// error is recorded in [`AssetCache::last_error`] and the cache keeps
/// whatever it loaded last, so a typo in a scene file degrades to a stale
/// asset and a message instead of taking the whole game down.
pub struct AssetCache<T> {
    path: PathBuf,
    poll_interval: Duration,
    next_poll: Option<Instant>,
    known_modified: Option<SystemTime>,
    value: Option<T>,
    last_error: Option<String>,
}

impl<T> AssetCache<T> {
    /// A cache watching `path`, polling its mtime at most four times a
    /// second — the rate part 3 of card #33 asked for.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_poll_interval(path, Duration::from_millis(250))
    }

    /// [`AssetCache::new`] with an explicit poll interval, for a caller that
    /// wants to watch faster or slower than the 250ms default (tests, most
    /// often, want zero).
    pub fn with_poll_interval(path: impl Into<PathBuf>, poll_interval: Duration) -> Self {
        Self {
            path: path.into(),
            poll_interval,
            next_poll: None,
            known_modified: None,
            value: None,
            last_error: None,
        }
    }

    /// The file this cache watches.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How often [`AssetCache::poll`] actually reaches the filesystem.
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// The last successfully loaded value, if any load has ever succeeded.
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// The message from the most recent failed load, if any — a console or
    /// overlay reads this itself rather than the cache pushing it anywhere.
    ///
    /// Cleared by the next load that succeeds.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Check the file's mtime and reload through `reload` if it changed.
    ///
    /// Calling this every frame is the intended use: it is a no-op until
    /// [`AssetCache::poll_interval`] has passed since the last check, and
    /// even then it only calls `reload` if the mtime it reads differs from
    /// the one it saw last. Returns `true` exactly when `reload` ran and
    /// produced a new value.
    pub fn poll<F>(&mut self, reload: F) -> bool
    where
        F: Fn(&str) -> Result<T, String>,
    {
        let now = Instant::now();
        if let Some(next) = self.next_poll {
            if now < next {
                return false;
            }
        }
        self.next_poll = Some(now + self.poll_interval);

        let modified = match fs::metadata(&self.path).and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(err) => {
                self.last_error = Some(err.to_string());
                return false;
            }
        };

        if self.known_modified == Some(modified) {
            return false;
        }
        self.known_modified = Some(modified);

        let path_str = match self.path.to_str() {
            Some(path_str) => path_str,
            None => {
                self.last_error = Some("asset path is not valid UTF-8".to_string());
                return false;
            }
        };

        match reload(path_str) {
            Ok(value) => {
                self.value = Some(value);
                self.last_error = None;
                true
            }
            Err(err) => {
                self.last_error = Some(err);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch file under a per-test unique path, removed when dropped.
    struct ScratchFile {
        path: PathBuf,
    }

    impl ScratchFile {
        fn new(contents: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("runity-assets-{}-{unique}.txt", std::process::id()));
            fs::write(&path, contents).expect("write scratch file");
            Self { path }
        }

        /// Overwrite the contents and push the mtime strictly past whatever
        /// it was before, so a poll immediately after sees a real change —
        /// no sleeping for the clock to tick on its own.
        fn write(&self, contents: &str) {
            let previous = fs::metadata(&self.path).unwrap().modified().unwrap();
            fs::write(&self.path, contents).expect("rewrite scratch file");
            let file = File::options().write(true).open(&self.path).unwrap();
            let bumped = previous + Duration::from_secs(1);
            file.set_modified(bumped).expect("bump mtime");
        }
    }

    impl Drop for ScratchFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn cache<T>(file: &ScratchFile) -> AssetCache<T> {
        AssetCache::with_poll_interval(&file.path, Duration::ZERO)
    }

    #[test]
    fn a_changed_mtime_triggers_reload_and_updates_the_cached_value() {
        let file = ScratchFile::new("one");
        let mut cache = cache(&file);

        let reloaded = cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string()));
        assert!(reloaded, "first poll always loads");
        assert_eq!(cache.get().map(String::as_str), Some("one"));

        file.write("two");
        let reloaded = cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string()));
        assert!(reloaded, "the mtime moved, so this poll should reload");
        assert_eq!(cache.get().map(String::as_str), Some("two"));
    }

    #[test]
    fn an_unchanged_mtime_does_not_reload_again() {
        let file = ScratchFile::new("stable");
        let mut cache = cache(&file);
        let calls = std::cell::Cell::new(0u32);

        for _ in 0..5 {
            cache.poll(|path| {
                calls.set(calls.get() + 1);
                fs::read_to_string(path).map_err(|e| e.to_string())
            });
        }

        assert_eq!(
            calls.get(),
            1,
            "the file never changed after the first load"
        );
        assert_eq!(cache.get().map(String::as_str), Some("stable"));
    }

    #[test]
    fn a_reload_error_keeps_the_previous_value_and_records_last_error() {
        let file = ScratchFile::new("42");
        let mut cache = cache(&file);

        cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string()));
        assert_eq!(cache.get().map(String::as_str), Some("42"));
        assert_eq!(cache.last_error(), None);

        file.write("not a number");
        let reloaded = cache.poll(|path| {
            let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
            text.trim()
                .parse::<i64>()
                .map(|n| n.to_string())
                .map_err(|e| e.to_string())
        });
        assert!(!reloaded, "a parse failure is not a successful reload");
        assert_eq!(
            cache.get().map(String::as_str),
            Some("42"),
            "the previous value survives a bad reload"
        );
        assert!(cache.last_error().is_some());
    }

    #[test]
    fn a_reload_error_does_not_panic() {
        let file = ScratchFile::new("boom");
        let mut cache: AssetCache<()> = cache(&file);
        cache.poll(|_| Err("deliberately broken".to_string()));
        assert_eq!(cache.last_error(), Some("deliberately broken"));
        assert!(cache.get().is_none());
    }

    #[test]
    fn a_missing_file_records_an_error_without_panicking() {
        let mut cache: AssetCache<String> =
            AssetCache::with_poll_interval("/no/such/path/for/runity", Duration::ZERO);
        let reloaded = cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string()));
        assert!(!reloaded);
        assert!(cache.last_error().is_some());
        assert!(cache.get().is_none());
    }

    #[test]
    fn polling_faster_than_the_interval_is_a_no_op() {
        let file = ScratchFile::new("v1");
        let mut cache =
            AssetCache::<String>::with_poll_interval(&file.path, Duration::from_secs(3600));
        assert!(cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string())));

        file.write("v2");
        // The interval has not elapsed, so this poll should not even look at
        // the filesystem, let alone reload.
        let reloaded = cache.poll(|path| fs::read_to_string(path).map_err(|e| e.to_string()));
        assert!(!reloaded);
        assert_eq!(cache.get().map(String::as_str), Some("v1"));
    }
}
