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
    /// Seconds since 1970: for "3 h ago".
    pub when: i64,
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
    Ok(status(dir)?.files.into_iter().map(|f| f.path).collect())
}

/// Where the repository around a folder stands: its branch, how far it is
/// from the branch it follows, whether a merge is under way, and the files
/// under the folder that are not as committed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// `None` on a detached `HEAD`.
    pub branch: Option<String>,
    /// The branch it follows, `origin/main`, when it follows one.
    pub upstream: Option<String>,
    /// Commits here the upstream has not, and there that are not here.
    pub ahead: u32,
    pub behind: u32,
    /// Git is in the middle of a merge: what is committed next finishes it.
    pub merging: bool,
    pub files: Vec<FileStatus>,
}

/// One file not as committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Absolute.
    pub path: std::path::PathBuf,
    /// Relative to the repository's top, with `/`, as git names it.
    pub name: String,
    /// Git's two letters: the index's and the working tree's — `M`, `A`,
    /// `D`, `R`, `U` unmerged, `?` new to git, `.` unchanged.
    pub index: char,
    pub worktree: char,
}

impl FileStatus {
    /// In a merge that conflicted on it and has not been settled.
    pub fn conflicted(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D')
        )
    }

    /// One letter for the change: `M` changed, `A` new, `D` deleted, `R`
    /// renamed, `U` in conflict.
    pub fn letter(&self) -> char {
        if self.conflicted() {
            return 'U';
        }
        match (self.index, self.worktree) {
            ('?', _) | ('A', _) => 'A',
            ('R', _) | (_, 'R') => 'R',
            ('D', _) | (_, 'D') => 'D',
            _ => 'M',
        }
    }

    /// The letter in words.
    pub fn word(&self) -> &'static str {
        match self.letter() {
            'U' => "in conflict",
            'A' => "new",
            'D' => "deleted",
            'R' => "renamed",
            _ => "changed",
        }
    }
}

/// [`Status`] of the repository around `dir`, with the files under it.
pub fn status(dir: &Path) -> Result<Status, String> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let top = Path::new(top.trim());
    let text = git(
        dir,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    let mut status = parse_status(&text);
    for f in &mut status.files {
        f.path = top.join(&f.name);
    }
    status.merging = merging(dir);
    Ok(status)
}

fn merging(dir: &Path) -> bool {
    git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

/// `git status --porcelain=v2 --branch -z`, the paths left relative.
fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    let file = |name: &str, xy: &str| {
        let mut letters = xy.chars();
        FileStatus {
            path: name.into(),
            name: name.to_string(),
            index: letters.next().unwrap_or('.'),
            worktree: letters.next().unwrap_or('.'),
        }
    };
    while let Some(entry) = entries.next() {
        if let Some(header) = entry.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if value != "(detached)" => status.branch = Some(value.into()),
                "branch.upstream" => status.upstream = Some(value.into()),
                "branch.ab" => {
                    for n in value.split(' ') {
                        if let Some(a) = n.strip_prefix('+') {
                            status.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = n.strip_prefix('-') {
                            status.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        // The path is the last field and may hold spaces.
        let xy = entry.get(2..4).unwrap_or("..");
        let path_at = |n: usize| entry.splitn(n + 1, ' ').nth(n).map(str::to_string);
        match entry.get(..1).unwrap_or("") {
            // 1 XY sub mH mI mW hH hI path
            "1" => {
                if let Some(path) = path_at(8) {
                    status.files.push(file(&path, xy));
                }
            }
            // 2 XY sub mH mI mW hH hI score path, then the name it had
            "2" => {
                if let Some(path) = path_at(9) {
                    status.files.push(file(&path, xy));
                }
                entries.next();
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            "u" => {
                if let Some(path) = path_at(10) {
                    status.files.push(file(&path, xy));
                }
            }
            "?" => status.files.push(file(&entry[2..], "??")),
            _ => {}
        }
    }
    status
}

/// Commit `paths` — every change to them, new files and deletions too —
/// with `message`, and nothing else that is staged. Mid-merge git commits
/// the whole merge, so everything staged goes. The new commit, short.
pub fn commit(dir: &Path, paths: &[std::path::PathBuf], message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("a commit needs a message: what changed, and why".into());
    }
    if paths.is_empty() {
        return Err("nothing chosen to commit".into());
    }
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut add = vec!["add", "-A", "--"];
    add.extend(names.iter().map(String::as_str));
    git(dir, &add)?;
    let mut commit = vec!["commit", "-q", "-m", message];
    if !merging(dir) {
        commit.push("--");
        commit.extend(names.iter().map(String::as_str));
    }
    git(dir, &commit)?;
    git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string())
}

/// A file's name as git names it: relative to the repository's top.
pub fn name_in_repo(path: &Path) -> Result<String, String> {
    let (dir, name) = split(path)?;
    let prefix = git(dir, &["rev-parse", "--show-prefix"])?;
    Ok(format!("{}{name}", prefix.trim()))
}

/// Tell git a file's conflict is settled: `git add` it.
pub fn resolve(path: &Path) -> Result<(), String> {
    let (dir, name) = split(path)?;
    git(dir, &["add", "--", &name]).map(|_| ())
}

/// The files a commit changed under `dir`: git's letter (`M`, `A`, `D`,
/// `R`…) and the name, relative to the repository's top.
pub fn files_at(dir: &Path, commit: &str) -> Result<Vec<(char, String)>, String> {
    let text = git(
        dir,
        &["show", "--name-status", "--format=", commit, "--", "."],
    )?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let letter = line.chars().next()?;
            // A rename names where it went last.
            let name = line.rsplit('\t').next()?.to_string();
            Some((letter, name))
        })
        .collect())
}

/// The commit `HEAD` names in the repository around `dir`.
pub fn head(dir: &Path) -> Result<String, String> {
    git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

const LOG_FORMAT: &str = "--format=%H%x1f%an%x1f%ad%x1f%at%x1f%s";

/// The commits that touched `path`, newest first, following renames.
pub fn log(path: &Path) -> Result<Vec<Revision>, String> {
    let (dir, name) = split(path)?;
    let text = git(
        dir,
        &["log", "--follow", "--date=short", LOG_FORMAT, "--", &name],
    )?;
    Ok(revisions(&text))
}

/// The commits that touched anything under `dir`, newest first, `limit`
/// at most.
pub fn log_dir(dir: &Path, limit: usize) -> Result<Vec<Revision>, String> {
    let limit = format!("-{limit}");
    let text = git(dir, &["log", &limit, "--date=short", LOG_FORMAT, "--", "."])?;
    Ok(revisions(&text))
}

fn revisions(text: &str) -> Vec<Revision> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            Some(Revision {
                commit: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                when: parts.next()?.parse().unwrap_or(0),
                summary: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
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
    fn the_status_says_the_branch_how_far_it_is_and_each_file_once() {
        let text = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0\
# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 a b scenes/main scene.ron\0\
2 R. N... 100644 100644 100644 a b R100 b.ron\0a.ron\0\
u UU N... 100644 100644 100644 100644 a b c prefabs/door.prefab\0\
? new.png\0";
        let status = parse_status(text);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        let names: Vec<(&str, char)> = status
            .files
            .iter()
            .map(|f| (f.name.as_str(), f.letter()))
            .collect();
        assert_eq!(
            names,
            [
                ("scenes/main scene.ron", 'M'),
                ("b.ron", 'R'),
                ("prefabs/door.prefab", 'U'),
                ("new.png", 'A')
            ]
        );
        assert!(status.files[2].conflicted());
        assert_eq!(parse_status("# branch.head (detached)\0").branch, None);
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
