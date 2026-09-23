//! The example project's sidecars are committed, and current.
//!
//! A clone has the sources and their `.rimport`s and nothing else; the
//! library is built from them. So every source needs one, and each one has
//! to describe the file beside it — a stale hash means the first `--sync`
//! on a fresh clone rewrites a committed file, which is exactly the diff
//! nobody made that the DNA's second postulate rules out.

use std::path::{Path, PathBuf};

use runity::Project;
use runity_import::{content_hash, importable, sidecar_for, ImportSettings};

fn example() -> Project {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/runity-import is two levels down")
        .join("examples/valley");
    Project::open(root).expect("examples/valley is a project")
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if importable(&path) {
            out.push(path);
        }
    }
}

#[test]
fn every_example_source_has_a_current_sidecar() {
    let project = example();
    let mut found = Vec::new();
    sources(&project.assets(), &mut found);
    sources(&project.materials(), &mut found);
    assert!(found.len() > 20, "only {} sources", found.len());

    for source in found {
        let sidecar = sidecar_for(&source);
        let fix = "run `cargo run -p runity-cli -- sync examples/valley` and commit the .rimport";
        let settings = ImportSettings::load(&sidecar)
            .unwrap_or_else(|e| panic!("{}: {e:#}; {fix}", sidecar.display()));
        assert_eq!(
            settings.source,
            project.relative(&source).unwrap(),
            "{}: names another file; {fix}",
            sidecar.display()
        );
        assert_eq!(
            settings.hash,
            content_hash(&source).unwrap(),
            "{}: the source changed since; {fix}",
            sidecar.display()
        );
        assert!(settings.id.is_some(), "{}: no asset id", sidecar.display());
    }
}

#[test]
fn every_example_prefab_and_scene_has_an_id() {
    let project = example();
    let fix = "run `cargo run -p runity-cli -- sync examples/valley` and commit the .rimport";
    let mut found = 0;
    for (dir, extension) in [(project.prefabs(), "prefab"), (project.scenes(), "ron")] {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != extension) {
                continue;
            }
            found += 1;
            let sidecar = sidecar_for(&path);
            let settings = ImportSettings::load(&sidecar)
                .unwrap_or_else(|e| panic!("{}: {e:#}; {fix}", sidecar.display()));
            assert_eq!(settings.source, project.relative(&path).unwrap(), "{fix}");
            assert!(settings.id.is_some(), "{}: no id; {fix}", sidecar.display());
        }
    }
    assert!(found >= 3, "only {found}");
}
