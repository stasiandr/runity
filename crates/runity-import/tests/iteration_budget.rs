//! How long `sync` takes to find and rebuild one changed source in a
//! project with hundreds that did not change.
//!
//! DNA, postulate 1: the time from saving an asset to seeing it is a number
//! with a budget. Most of a project is untouched at any moment, so what a
//! sync costs is mostly what it costs to decide that — a `stat` per file,
//! not a hash per file — plus the one import. Best of several runs, with a
//! budget for a slow CI runner, in a debug build.

use std::time::{Duration, Instant};

use runity::Project;

const SOURCES: usize = 300;

#[test]
fn one_changed_material_among_hundreds_is_rebuilt_within_budget() {
    let root = std::env::temp_dir().join("runity-budget-sync");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "budget").unwrap();
    for i in 0..SOURCES {
        let path = project.materials().join(format!("m{i:03}.rmat"));
        std::fs::write(path, format!("(color: \"#{:06x}\")", i * 997)).unwrap();
    }
    assert_eq!(runity_import::sync(&project).len(), SOURCES, "first build");
    assert!(
        runity_import::sync(&project).is_empty(),
        "nothing left to do"
    );

    let changed = project.materials().join("m150.rmat");
    let mut took = Duration::MAX;
    for run in 0..5u32 {
        std::fs::write(&changed, format!("(color: \"#{:06x}\")", run * 4099 + 1)).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(2 + u64::from(run));
        std::fs::File::options()
            .write(true)
            .open(&changed)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let start = Instant::now();
        let done = runity_import::sync(&project);
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
