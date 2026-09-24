//! `scrap rebuild-time`: how long the game takes to rebuild after an edit.
//!
//! DNA, postulate 1: iteration time is a number with a budget. This is the
//! biggest part of it for code — change one line of the game, and wait for
//! the build — measured the way it happens: the game crate is built once,
//! then its `main.rs` gets a line appended and is built again, several
//! times, and the best is the answer. The file is put back as it was.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use scrap::Project;

pub struct RebuildTime {
    /// Each timed rebuild, in order.
    pub runs: Vec<Duration>,
}

impl RebuildTime {
    pub fn best(&self) -> Duration {
        self.runs.iter().copied().min().unwrap_or_default()
    }
}

/// Puts a file's contents back when dropped, whatever happened meanwhile.
struct Restore<'a> {
    path: &'a Path,
    original: String,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let _ = std::fs::write(self.path, &self.original);
    }
}

pub fn rebuild_time(project: &Project, runs: usize) -> Result<RebuildTime> {
    let manifest = project.root().join("Cargo.toml");
    let main = project.root().join(scrap::project::SRC).join("main.rs");
    if !manifest.is_file() || !main.is_file() {
        bail!(
            "{} has no game crate (Cargo.toml and src/main.rs) to rebuild",
            project.root().display()
        );
    }
    build(&manifest).context("the first build, before anything is timed")?;

    let restore = Restore {
        path: &main,
        original: std::fs::read_to_string(&main)?,
    };
    let mut timed = Vec::with_capacity(runs);
    for run in 0..runs.max(1) {
        let edited = format!("{}\n// scrap rebuild-time {run}\n", restore.original);
        std::fs::write(&main, edited)?;
        let start = Instant::now();
        build(&manifest)?;
        timed.push(start.elapsed());
    }
    drop(restore);
    Ok(RebuildTime { runs: timed })
}

fn build(manifest: &Path) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["build", "--quiet", "--manifest-path"])
        .arg(manifest)
        .output()
        .context("running cargo")?;
    if !output.status.success() {
        bail!(
            "cargo build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
