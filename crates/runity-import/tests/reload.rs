//! Changing a file and seeing it take effect, inside a project.
//!
//! Two halves, and both have to work for the workflow to exist: a changed
//! source has to be rebuilt into an asset, and a changed asset has to be
//! re-read by a library that already holds it.
//!
//! And the rules the DNA set for the first half: the sidecar sits beside its
//! source and names it relative to the project; whether a source changed is
//! the content hash's call, not the clock's; a source that moved is found by
//! its contents and keeps its settings and its asset's ID; the library is
//! derived, so a clone with none builds all of it.

use std::path::{Path, PathBuf};

use runity::{asset, Library, MeshAsset, Project};
use runity_import::{import_into, sidecar_for, sync, Change, ImportSettings};

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

fn project(name: &str) -> (Project, PathBuf) {
    let root = std::env::temp_dir().join(format!("runity-reload-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, name).unwrap();
    (project, root)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// File timestamps have a resolution, and a test that writes twice in a
/// microsecond can produce two files that claim the same mtime. Bumping it
/// explicitly is more honest than sleeping and hoping.
fn touch_forward(path: &Path) {
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

fn flat() -> ImportSettings {
    ImportSettings {
        origin_to_base: false,
        ..ImportSettings::default()
    }
}

fn width(asset_path: &Path) -> f32 {
    let bytes = asset::read(asset_path).unwrap();
    let mesh = asset::view::<MeshAsset>(&bytes).unwrap();
    mesh.bounds.max[0].to_native()
}

#[test]
fn the_sidecar_sits_beside_its_source_and_names_it_relative_to_the_project() {
    // The library is derived and never committed, so settings kept there
    // are settings nobody keeps. Beside the source, they are committed; and
    // relative, they mean the same thing in every clone.
    let (project, root) = project("sidecar");
    let source = root.join("assets/rocks/shape.obj");
    write(&source, SQUARE);
    let report = import_into(&project, &source, Some(&flat())).unwrap();
    assert!(report.warning.is_none(), "{:?}", report.warning);

    let sidecar = sidecar_for(&source);
    assert_eq!(report.imported.sidecar, sidecar);
    let settings = ImportSettings::load(&sidecar).unwrap();
    assert_eq!(settings.source, "assets/rocks/shape.obj", "forward slashes");
    assert_eq!(settings.hash.len(), 32, "a content hash, in hex");
    assert_eq!(settings.id, Some(report.imported.id), "and the asset's ID");
    assert!(report.imported.asset.starts_with(project.library()));
}

#[test]
fn a_changed_source_is_rebuilt_from_its_sidecar() {
    let (project, root) = project("source");
    let source = root.join("assets/shape.obj");
    write(&source, SQUARE);
    let first = import_into(&project, &source, Some(&flat())).unwrap();

    // Nothing has changed, so nothing is rebuilt.
    assert!(sync(&project).is_empty());

    write(&source, BIGGER);
    touch_forward(&source);
    let done = sync(&project);
    assert_eq!(done.len(), 1, "the one changed source");
    assert_eq!(done[0].change, Change::Changed);
    assert_eq!(done[0].result, Ok(first.imported.id), "the same asset");
    assert_eq!(
        width(&first.imported.asset),
        4.0,
        "now holding the bigger shape"
    );

    // And the settings survived the rebuild: the sidecar is what makes a
    // rebuild reproducible rather than a guess.
    let settings = ImportSettings::load(sidecar_for(&source)).unwrap();
    assert!(!settings.origin_to_base);
}

#[test]
fn a_source_touched_but_not_changed_is_not_rebuilt() {
    // A checkout, a copy, a sync tool: the clock moves and not a byte does.
    // The hash decides, so nothing is rebuilt — and asked again, nothing is
    // hashed twice either, because the sidecar's clock is moved on.
    let (project, root) = project("touched");
    let source = root.join("assets/shape.obj");
    write(&source, SQUARE);
    import_into(&project, &source, None).unwrap();
    let asset = runity_import::built_for(&source, &project.library()).unwrap();
    let built = std::fs::metadata(&asset).unwrap().modified().unwrap();

    touch_forward(&source);
    assert!(sync(&project).is_empty(), "the same bytes are not a change");
    assert_eq!(
        std::fs::metadata(&asset).unwrap().modified().unwrap(),
        built,
        "and the asset was not rewritten"
    );
}

#[test]
fn a_moved_source_is_found_by_its_contents_and_keeps_its_settings_and_id() {
    // Moving a file must not turn its asset into a different one, and must
    // not throw away the settings someone chose for it.
    let (project, root) = project("moved");
    let before = root.join("assets/shape.obj");
    write(&before, SQUARE);
    let options = ImportSettings {
        scale: 0.5,
        ..flat()
    };
    let first = import_into(&project, &before, Some(&options)).unwrap();

    // Moved the way a person moves a file: without its sidecar.
    let after = root.join("assets/rocks/pebble.obj");
    std::fs::create_dir_all(after.parent().unwrap()).unwrap();
    std::fs::rename(&before, &after).unwrap();

    let done = sync(&project);
    assert_eq!(done.len(), 1, "{done:?}");
    assert_eq!(
        done[0].change,
        Change::Moved {
            from: "assets/shape.obj".into()
        }
    );
    assert_eq!(done[0].result, Ok(first.imported.id), "the same asset");

    let settings = ImportSettings::load(sidecar_for(&after)).unwrap();
    assert_eq!(settings.source, "assets/rocks/pebble.obj");
    assert_eq!(settings.scale, 0.5, "and the settings came with it");
    assert!(!sidecar_for(&before).exists(), "the old sidecar is gone");

    // The library has it once, under the new name.
    let (library, problems) = Library::open(project.library()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(library.len(), 1, "not a second copy under the old name");
    assert!(library.mesh(first.imported.id).is_some());
}

#[test]
fn a_source_that_has_gone_is_reported_rather_than_deleting_the_asset() {
    let (project, root) = project("missing");
    let source = root.join("assets/shape.obj");
    write(&source, SQUARE);
    let first = import_into(&project, &source, None).unwrap();

    std::fs::remove_file(&source).unwrap();
    let done = sync(&project);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].change, Change::Gone);
    assert!(done[0].result.is_err(), "it should say so");
    assert!(
        first.imported.asset.exists(),
        "and leave the asset alone; a moved file should not blank a model"
    );
}

#[test]
fn a_fresh_clone_with_no_library_builds_all_of_it() {
    // The library is derived and not committed. What a clone has is the
    // sources and their sidecars, and that has to be enough.
    let (project, root) = project("clone");
    for name in ["one", "two"] {
        let source = root.join(format!("assets/{name}.obj"));
        write(&source, SQUARE);
        import_into(&project, &source, Some(&flat())).unwrap();
    }
    write(&root.join("materials/clay.rmat"), r##"(color: "#8a5a3c")"##);
    import_into(&project, &root.join("materials/clay.rmat"), None).unwrap();
    std::fs::remove_dir_all(project.library()).unwrap();

    let done = sync(&project);
    assert_eq!(done.len(), 3, "{done:?}");
    assert!(done
        .iter()
        .all(|r| r.change == Change::Built && r.result.is_ok()));

    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.mesh_by_name("one").is_some());
    assert!(library.mesh_by_name("two").is_some());
    assert!(library.material_by_name("clay").is_some());
}

#[test]
fn a_model_and_a_material_with_one_name_do_not_overwrite_each_other() {
    // `stone.obj` and `stone.rmat` are both reasonable, and the library is
    // built from both folders at once.
    let (project, root) = project("same-name");
    write(&root.join("assets/stone.obj"), SQUARE);
    write(
        &root.join("materials/stone.rmat"),
        r##"(color: "#797c81")"##,
    );
    let done = sync(&project);
    assert_eq!(done.len(), 2, "{done:?}");

    let (library, problems) = Library::open(project.library()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert!(library.mesh_by_name("stone").is_some());
    assert!(library.material_by_name("stone").is_some());
}

#[test]
fn a_new_source_dropped_in_is_imported_and_given_a_sidecar() {
    let (project, root) = project("new");
    let source = root.join("assets/new.obj");
    write(&source, SQUARE);
    let done = sync(&project);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].change, Change::New);
    assert_eq!(
        ImportSettings::load(sidecar_for(&source)).unwrap().source,
        "assets/new.obj"
    );
    assert!(
        sync(&project).is_empty(),
        "and the second sync has nothing to do"
    );
}

#[test]
fn a_source_outside_the_project_is_imported_with_a_warning() {
    // Allowed, because dragging a file in from Downloads is what people do.
    // Said out loud, because a clone of the project elsewhere will not find
    // it.
    let (project, _) = project("outside");
    let elsewhere = std::env::temp_dir().join("runity-reload-elsewhere");
    let _ = std::fs::remove_dir_all(&elsewhere);
    let source = elsewhere.join("wedge.obj");
    write(&source, SQUARE);

    let report = import_into(&project, &source, None).unwrap();
    let warning = report.warning.expect("a warning");
    assert!(warning.contains("outside the project"), "{warning}");
    assert!(warning.contains("assets/"), "and the fix: {warning}");

    // Its sidecar lives in the project, not beside a file in someone's
    // downloads, and names the source absolutely.
    let sidecar = project.assets().join("wedge.obj.rimport");
    let settings = ImportSettings::load(&sidecar).unwrap();
    assert!(Path::new(&settings.source).is_absolute());
    assert!(!sidecar_for(&source).exists(), "nothing written outside");
    assert!(sync(&project).is_empty(), "and a sync still finds it");
}

#[test]
fn a_library_re_reads_only_what_changed() {
    let (project, root) = project("library");
    for name in ["one", "two"] {
        let source = root.join(format!("assets/{name}.obj"));
        write(&source, SQUARE);
        import_into(&project, &source, Some(&flat())).unwrap();
    }

    let (mut loaded, problems) = Library::open(project.library()).unwrap();
    assert!(problems.is_empty());
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.reload_changed().len(), 0, "nothing has moved yet");

    // Rebuild just one of them, bigger.
    let one = root.join("assets/one.obj");
    write(&one, BIGGER);
    touch_forward(&one);
    sync(&project);
    touch_forward(&runity_import::built_for(&one, &project.library()).unwrap());

    let changed = loaded.reload_changed();
    assert_eq!(changed.len(), 1, "one asset, not both");
    assert_eq!(
        changed[0].path,
        runity_import::built_for(&one, &project.library()).unwrap()
    );

    // The library now holds the new geometry, without being reopened.
    let mesh = loaded.mesh_by_name("one").unwrap();
    assert_eq!(mesh.bounds.max[0].to_native(), 4.0);
    assert_eq!(
        loaded.mesh_by_name("two").unwrap().bounds.max[0].to_native(),
        1.0,
        "the other one is untouched"
    );
}

#[test]
fn a_terrain_is_described_imported_as_a_mesh_and_rebuilt_when_its_seed_changes() {
    let (project, root) = project("terrain");
    let source = root.join("assets/hills.rterrain");
    let text = "(size: (100.0, 60.0), resolution: 33, height: 12.0, noise: (seed: 7, scale: 30.0))";
    write(&source, text);
    let first = import_into(&project, &source, None).unwrap();
    let (library, _) = Library::open(project.library()).unwrap();
    let mesh = library.mesh_by_name("hills").expect("named after the file");
    assert_eq!(mesh.vertices.len(), 33 * 33);
    let (min, max) = (mesh.bounds.min, mesh.bounds.max);
    assert!((min[1].to_native()).abs() < 1e-4, "lowest point at zero");
    assert!(
        (max[1].to_native() - 12.0).abs() < 1e-3,
        "highest at the height asked"
    );
    assert!(
        (max[0].to_native() - 50.0).abs() < 1e-3 && (min[2].to_native() + 30.0).abs() < 1e-3,
        "centred"
    );
    let before = asset::read(&first.imported.asset).unwrap();

    // The same file, the same hills; another seed, other hills.
    write(&source, &text.replace("seed: 7", "seed: 8"));
    touch_forward(&source);
    let done = sync(&project);
    assert_eq!(done.len(), 1);
    assert_ne!(asset::read(&first.imported.asset).unwrap(), before);
    write(&source, text);
    touch_forward(&source);
    sync(&project);
    assert_eq!(
        asset::read(&first.imported.asset).unwrap(),
        before,
        "reproducible"
    );

    write(&source, "(size: (10.0, 10.0), resolution: 1, height: 1.0)");
    touch_forward(&source);
    let done = sync(&project);
    let err = done[0].result.as_ref().unwrap_err();
    assert!(err.contains("resolution"), "{err}");
}

#[test]
fn a_painted_heightmap_shapes_the_terrain_and_repainting_it_rebuilds() {
    let (project, root) = project("heightmap");
    let paint = |value: u8| {
        // A ramp from black on the left to `value` on the right.
        let image =
            image::GrayImage::from_fn(16, 16, |x, _| image::Luma([(x * value as u32 / 15) as u8]));
        image.save(root.join("assets/ramp.png")).unwrap();
        touch_forward(&root.join("assets/ramp.png"));
    };
    paint(255);
    let source = root.join("assets/field.rterrain");
    write(
        &source,
        r#"(size: (30.0, 30.0), resolution: 16, height: 6.0, heightmap: "ramp.png")"#,
    );
    let done = sync(&project);
    assert!(done.iter().all(|r| r.result.is_ok()), "{done:?}");
    let (library, _) = Library::open(project.library()).unwrap();
    let field = library.mesh_by_name("field").unwrap();
    assert!(
        (field.bounds.max[1].to_native() - 6.0).abs() < 0.01,
        "white is the full height"
    );
    assert!(
        field.bounds.min[1].to_native().abs() < 0.01,
        "black is the ground"
    );
    let before =
        asset::read(runity_import::built_for(&source, &project.library()).unwrap()).unwrap();

    // Repaint only the image: the terrain is rebuilt, lower.
    paint(128);
    let done = sync(&project);
    assert!(
        done.iter()
            .any(|r| r.source == source && r.change == Change::Changed),
        "the terrain counts as changed: {done:?}"
    );
    let after =
        asset::read(runity_import::built_for(&source, &project.library()).unwrap()).unwrap();
    assert_ne!(before, after);
}
