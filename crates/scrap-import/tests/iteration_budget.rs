//! How long `sync` takes to find and rebuild one changed source in a
//! project with hundreds that did not change.
//!
//! DNA, postulate 1: the time from saving an asset to seeing it is a number
//! with a budget. Most of a project is untouched at any moment, so what a
//! sync costs is mostly what it costs to decide that — a `stat` per file,
//! not a hash per file — plus the one import. Best of several runs, with a
//! budget for a slow CI runner, in a debug build.

use std::time::{Duration, Instant};

use scrap::Project;

/// `std::fs::write`, the folders on the way made first: a new project has
/// only the folders its layout needs (docs/layout.md).
#[allow(dead_code)]
fn write_all(path: impl AsRef<std::path::Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

const SOURCES: usize = 300;

#[test]
fn one_changed_material_among_hundreds_is_rebuilt_within_budget() {
    let root = std::env::temp_dir().join("scrap-budget-sync");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "budget").unwrap();
    for i in 0..SOURCES {
        let path = project.materials().join(format!("m{i:03}.scrmat"));
        write_all(path, format!("(color: \"#{:06x}\")", i * 997)).unwrap();
    }
    assert_eq!(scrap_import::sync(&project).len(), SOURCES, "first build");
    assert!(
        scrap_import::sync(&project).is_empty(),
        "nothing left to do"
    );

    let changed = project.materials().join("m150.scrmat");
    let mut took = Duration::MAX;
    for run in 0..5u32 {
        write_all(&changed, format!("(color: \"#{:06x}\")", run * 4099 + 1)).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(2 + u64::from(run));
        std::fs::File::options()
            .write(true)
            .open(&changed)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let start = Instant::now();
        let done = scrap_import::sync(&project);
        took = took.min(start.elapsed());
        assert_eq!(done.len(), 1, "{done:?}");
    }
    let budget = Duration::from_millis(100);
    eprintln!("sync, 1 of {SOURCES} changed: {took:.2?} (budget {budget:?})");
    assert!(
        took <= budget,
        "sync took {took:.2?}, over its budget of {budget:?}"
    );
}
