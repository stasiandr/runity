//! Files looked at on a thread of their own: a game's hot reload asks
//! "did anything change?" without walking the project on the frame.
//!
//! A [`Watch`] runs a look — a fingerprint of the files it cares about,
//! a hash of their paths and stamps — every period on its own thread and
//! raises a flag when the fingerprint moves. The frame reads the flag
//! ([`Watch::changed`]) and does the real reading only then. Walking a
//! project's few thousand files takes tens to hundreds of milliseconds;
//! done on the frame every quarter second it was a stutter four times a
//! second in every game run from its project (Not Divinity's window
//! dropped a frame of ~150 ms every 0.25 s). Not in a browser: nothing
//! changes the files under a page, and it has no threads to spare.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A look at files every period, on a thread; the flag it raises.
pub struct Watch {
    changed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// Look with `look` now and every `period` after, on a thread of its
    /// own, until the watch is dropped. `None` where there are no threads.
    pub fn spawn(period: Duration, mut look: impl FnMut() -> u64 + Send + 'static) -> Option<Self> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let changed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, done) = (changed.clone(), stop.clone());
        std::thread::Builder::new()
            .name("scrap-watch".into())
            .spawn(move || {
                let mut last = look();
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    let now = look();
                    if now != last {
                        last = now;
                        flag.store(true, Ordering::Release);
                    }
                }
            })
            .ok()?;
        Some(Self { changed, stop })
    }

    /// Whether what is watched changed since the last time this said so.
    pub fn changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A fingerprint of files: their paths and when each last changed.
pub fn fingerprint<'a>(files: impl IntoIterator<Item = (&'a std::path::Path, Option<std::time::SystemTime>)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (path, stamp) in files {
        path.hash(&mut h);
        stamp.hash(&mut h);
    }
    h.finish()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn a_change_raises_the_flag_once() {
        let value = Arc::new(AtomicU64::new(1));
        let seen = value.clone();
        let watch = Watch::spawn(Duration::from_millis(5), move || seen.load(Ordering::Relaxed)).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        assert!(!watch.changed(), "nothing moved");
        value.store(2, Ordering::Relaxed);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !watch.changed() {
            assert!(std::time::Instant::now() < deadline, "the change was never seen");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(40));
        assert!(!watch.changed(), "said once");
    }
}
