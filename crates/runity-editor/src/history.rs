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

/// A file someone holds a Git LFS lock on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    /// Relative to the repository root, as git-lfs reports it.
    pub path: String,
    pub owner: String,
    pub locked_at: String,
    pub id: String,
}

/// Who holds which lock, as the LFS server says. An error, in words, when
/// the repository has no LFS server to ask.
pub fn locks(dir: &Path) -> Result<Vec<Lock>, String> {
    let text = git(dir, &["lfs", "locks", "--json"])?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("git lfs locks: {e}"))?;
    let field = |v: &serde_json::Value, key: &str| v[key].as_str().unwrap_or("").to_string();
    Ok(value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Lock {
                    path: field(item, "path"),
                    owner: item["owner"]["name"].as_str().unwrap_or("").to_string(),
                    locked_at: field(item, "locked_at"),
                    id: field(item, "id"),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Take the lock on a file before editing it, so the next person finds out
/// before they start, not after.
pub fn lock(path: &Path) -> Result<(), String> {
    let (dir, name) = split(path)?;
    git(dir, &["lfs", "lock", &name]).map(|_| ())
}

pub fn unlock(path: &Path) -> Result<(), String> {
    let (dir, name) = split(path)?;
    git(dir, &["lfs", "unlock", &name]).map(|_| ())
}
