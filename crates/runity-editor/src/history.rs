//! A file's history, from git.
//!
//! DNA, postulate 2: an asset's history is visible in the editor — who
//! changed it, when, and what it looked like then. Git already knows all
//! of it; this asks.

use std::path::Path;
use std::process::Command;

/// One commit that touched a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub commit: String,
    pub author: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub summary: String,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn split(path: &Path) -> Result<(&Path, String), String> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} is not a file", path.display()))?
        .to_string_lossy()
        .into_owned();
    Ok((dir, name))
}

/// The commits that touched `path`, newest first, following renames.
pub fn log(path: &Path) -> Result<Vec<Revision>, String> {
    let (dir, name) = split(path)?;
    let text = git(
        dir,
        &[
            "log",
            "--follow",
            "--date=short",
            "--format=%H%x1f%an%x1f%ad%x1f%s",
            "--",
            &name,
        ],
    )?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            Some(Revision {
                commit: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                summary: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}

/// What `path` said at `commit`.
pub fn show(path: &Path, commit: &str) -> Result<String, String> {
    let (dir, name) = split(path)?;
    git(dir, &["show", &format!("{commit}:./{name}")])
}
