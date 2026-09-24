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

/// Every file under `dir` that is not as it was committed — changed,
/// added, new to git — as absolute paths: what the Project and the
/// Hierarchy mark with a dot. An error outside a repository.
pub fn uncommitted(dir: &Path) -> Result<std::collections::HashSet<std::path::PathBuf>, String> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let top = Path::new(top.trim());
    let text = git(
        dir,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."],
    )?;
    Ok(status_paths(&text)
        .into_iter()
        .map(|p| top.join(p))
        .collect())
}

/// The paths of `git status --porcelain=v1 -z`, relative to the top of
/// the repository: `XY path`, a rename followed by the name it had.
fn status_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        let Some(path) = entry.get(3..) else { continue };
        out.push(path.to_string());
        if entry.starts_with('R') || entry.starts_with('C') {
            // The old name: gone from the tree, nothing to mark.
            entries.next();
        }
    }
    out
}

/// The commit `HEAD` names in the repository around `dir`.
pub fn head(dir: &Path) -> Result<String, String> {
    git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
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
    Ok(lock_list(&value))
}

/// The locks someone other than this user holds — what `git lfs locks
/// --verify` calls theirs — each with the file as an absolute path, so it
/// can be compared with a file whatever folder the project sits in within
/// the repository.
pub fn others_locks(dir: &Path) -> Result<Vec<(std::path::PathBuf, Lock)>, String> {
    let text = git(dir, &["lfs", "locks", "--verify", "--json"])?;
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let top = Path::new(top.trim());
    Ok(theirs(&text)?
        .into_iter()
        .map(|lock| (top.join(&lock.path), lock))
        .collect())
}

/// The `theirs` half of `git lfs locks --verify --json`.
fn theirs(text: &str) -> Result<Vec<Lock>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("git lfs locks: {e}"))?;
    Ok(lock_list(&value["theirs"]))
}

fn lock_list(value: &serde_json::Value) -> Vec<Lock> {
    let field = |v: &serde_json::Value, key: &str| v[key].as_str().unwrap_or("").to_string();
    value
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
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_lists_each_path_once_and_a_rename_by_its_new_name() {
        let text = " M scenes/main.ron\0?? prefabs/new.prefab\0R  b.ron\0a.ron\0A  x.png\0";
        assert_eq!(
            status_paths(text),
            ["scenes/main.ron", "prefabs/new.prefab", "b.ron", "x.png"]
        );
    }

    #[test]
    fn a_verified_listing_is_split_into_ours_and_theirs() {
        let text = r#"{
            "ours": [{"id": "1", "path": "game/assets/rock.png", "owner": {"name": "me"}, "locked_at": "2026-09-01T10:00:00Z"}],
            "theirs": [{"id": "2", "path": "game/assets/tree.png", "owner": {"name": "ana"}, "locked_at": "2026-09-02T10:00:00Z"}]
        }"#;
        let theirs = theirs(text).unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].path, "game/assets/tree.png");
        assert_eq!(theirs[0].owner, "ana");
        assert!(super::theirs("{}").unwrap().is_empty());
    }
}
