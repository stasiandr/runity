//! Crash reports: when a game panics, what happened is written down in the
//! player's folder before the process goes — what, where, the stack, which
//! build — and the next start finds it ([`pending`]). Sending it anywhere
//! (Sentry) is the `reports` feature's, and the player's to allow: nothing
//! here touches the network.

use std::path::{Path, PathBuf};

/// The folder under the player's where reports wait.
pub const DIR: &str = "crashes";

/// One crash, as written.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// Seconds since 1970, UTC.
    pub at: u64,
    /// What the panic said.
    pub message: String,
    /// `file:line:column`.
    pub location: String,
    pub backtrace: String,
    /// The game's name and the build's version.
    pub game: String,
    pub version: String,
    pub os: String,
}

/// Write down the next panic in `dir` (made if missing), then let the
/// usual message print. `version` is the game's own, what a report groups
/// by.
pub fn install_in(dir: PathBuf, game: &str, version: &str) {
    let (game, version) = (game.to_string(), version.to_string());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let report = Report {
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            message,
            location,
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
            game: game.clone(),
            version: version.clone(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        };
        if let Ok(path) = write(&dir, &report) {
            eprintln!("crash report: {}", path.display());
        }
        previous(info);
    }));
}

/// [`install_in`] the player's folder for `game`.
pub fn install(game: &str, version: &str) {
    if let Ok(dir) = crate::player_prefs::user_dir(game) {
        install_in(dir.join(DIR), game, version);
    }
}

fn write(dir: &Path, report: &Report) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}-{}.ron", report.at, std::process::id()));
    let text =
        ron::ser::to_string_pretty(report, Default::default()).map_err(std::io::Error::other)?;
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Reports waiting in `dir`, oldest first, with where each is.
pub fn pending_in(dir: &Path) -> Vec<(PathBuf, Report)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(PathBuf, Report)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            Some((p, ron::from_str(&text).ok()?))
        })
        .collect();
    out.sort_by_key(|(_, r)| r.at);
    out
}

/// The reports the last runs of `game` left.
pub fn pending(game: &str) -> Vec<(PathBuf, Report)> {
    crate::player_prefs::user_dir(game)
        .map(|d| pending_in(&d.join(DIR)))
        .unwrap_or_default()
}

/// Done with a report: sent, or declined.
pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_leaves_a_report_the_next_start_finds() {
        let dir = std::env::temp_dir().join(format!("scrap-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        install_in(dir.clone(), "dacha", "0.9.1");
        let result = std::panic::catch_unwind(|| panic!("the well ran dry"));
        let _ = std::panic::take_hook();
        assert!(result.is_err());
        let found = pending_in(&dir);
        assert_eq!(found.len(), 1, "{found:?}");
        let (path, report) = &found[0];
        assert_eq!(report.message, "the well ran dry");
        assert!(report.location.contains("crash.rs"), "{}", report.location);
        assert_eq!(
            (report.game.as_str(), report.version.as_str()),
            ("dacha", "0.9.1")
        );
        assert!(!report.backtrace.is_empty());
        forget(path);
        assert!(pending_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
