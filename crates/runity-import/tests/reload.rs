//! Changing a file and seeing it take effect.
//!
//! Two halves, and both have to work for the workflow to exist: a changed
//! source has to be rebuilt into an asset, and a changed asset has to be
//! re-read by a library that already holds it.

use std::path::PathBuf;

use runity::{asset, Library, MeshAsset};
use runity_import::{import_file, reimport_changed, ImportSettings};

const SQUARE: &str = "\
v -1.0 0.0 -1.0
v  1.0 0.0 -1.0
v  1.0 0.0  1.0
f 1 3 2
";

const BIGGER: &str = "\
v -4.0 0.0 -4.0
v  4.0 0.0 -4.0
v  4.0 0.0  4.0
f 1 3 2
";

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runity-reload-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// File timestamps have a resolution, and a test that writes twice in a
/// microsecond can produce two files that claim the same mtime. Bumping it
/// explicitly is more honest than sleeping and hoping.
fn touch_forward(path: &PathBuf) {
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

#[test]
fn a_changed_source_is_rebuilt_from_its_sidecar() {
    let dir = temp("source");
    let source = dir.join("shape.obj");
    std::fs::write(&source, SQUARE).unwrap();
    let library = dir.join("library");

    import_file(
        &source,
        &library,
        ImportSettings {
            origin_to_base: false,
            ..ImportSettings::for_source("shape.obj")
        },
    )
    .unwrap();

    // Nothing has changed, so nothing is rebuilt.
    assert!(reimport_changed(&library, &dir).is_empty());

    std::fs::write(&source, BIGGER).unwrap();
    touch_forward(&source);

    let done = reimport_changed(&library, &dir);
    assert_eq!(done.len(), 1, "the one changed source");
    assert!(done[0].result.is_ok(), "{:?}", done[0].result);

    let bytes = asset::read(library.join("shape.rasset")).unwrap();
    let mesh = asset::view::<MeshAsset>(&bytes).unwrap();
    assert_eq!(
        mesh.bounds.max[0].to_native(),
        4.0,
        "the asset should hold the bigger shape"
    );

    // And the settings survived the round trip: the sidecar is what makes a
    // rebuild reproducible rather than a guess.
    let settings = ImportSettings::load(library.join("shape.rimport")).unwrap();
    assert!(!settings.origin_to_base);
}

#[test]
fn a_source_that_has_gone_is_reported_rather_than_deleting_the_asset() {
    let dir = temp("missing");
    let source = dir.join("shape.obj");
    std::fs::write(&source, SQUARE).unwrap();
    let library = dir.join("library");
    import_file(&source, &library, ImportSettings::for_source("shape.obj")).unwrap();

    std::fs::remove_file(&source).unwrap();
    let done = reimport_changed(&library, &dir);

    assert_eq!(done.len(), 1);
    assert!(done[0].result.is_err(), "it should say so");
    assert!(
        library.join("shape.rasset").exists(),
        "and leave the asset alone; a moved file should not blank a model"
    );
}

#[test]
fn a_library_re_reads_only_what_changed() {
    let dir = temp("library");
    let library = dir.join("library");
    for (name, text) in [("one.obj", SQUARE), ("two.obj", SQUARE)] {
        let source = dir.join(name);
        std::fs::write(&source, text).unwrap();
        import_file(
            &source,
            &library,
            ImportSettings {
                origin_to_base: false,
                ..ImportSettings::for_source(name)
            },
        )
        .unwrap();
    }

    let (mut loaded, problems) = Library::open(&library).unwrap();
    assert!(problems.is_empty());
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.reload_changed().len(), 0, "nothing has moved yet");

    // Rebuild just one of them, bigger.
    std::fs::write(dir.join("one.obj"), BIGGER).unwrap();
    touch_forward(&dir.join("one.obj"));
    reimport_changed(&library, &dir);
    touch_forward(&library.join("one.rasset"));

    let changed = loaded.reload_changed();
    assert_eq!(changed.len(), 1, "one asset, not both");
    assert_eq!(changed[0].path.file_name().unwrap(), "one.rasset");

    // The library now holds the new geometry, without being reopened.
    let mesh = loaded.mesh_by_name("one").unwrap();
    assert_eq!(mesh.bounds.max[0].to_native(), 4.0);
    assert_eq!(
        loaded.mesh_by_name("two").unwrap().bounds.max[0].to_native(),
        1.0,
        "the other one is untouched"
    );
}
