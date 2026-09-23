//! Materials as assets: a palette that lives in one place.
//!
//! A colour spelled out in twenty scenes drifts in nineteen of them. The fix
//! is the one meshes and textures already have — name it once, reference it by
//! name — and this is the test that the whole path works: a `.rmat` written in
//! hex becomes an asset, the library finds it by name, a scene that names it
//! gets it, and changing the hex changes what the scene draws without anything
//! being reopened.

use std::path::{Path, PathBuf};

use runity::material::{Material, Shading};
use runity::render::MeshHandle;
use runity::{Library, Scene};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runity-palette-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a `.rmat` source and import it into `library`.
fn write_material(dir: &Path, library: &Path, stem: &str, body: &str) {
    let source = dir.join(format!("{stem}.rmat"));
    std::fs::write(&source, body).unwrap();
    runity_import::import_file(
        &source,
        library,
        runity_import::ImportSettings::for_source(format!("{stem}.rmat")),
    )
    .unwrap();
}

/// What a scene looks like when it names a material rather than spelling it
/// out. This is the readable form the whole feature is for.
fn scene_naming(material: &str) -> Scene {
    ron::from_str(&format!(
        r#"(entities: [(name: "rock", model: "builtin:sphere", material: "{material}")])"#
    ))
    .unwrap()
}

/// The material the one entity in a scene ends up drawn with.
fn surface_of(scene: &Scene, library: &Library) -> Material {
    let mut world = runity::hecs::World::new();
    let missing = runity::spawn_scene_with(
        scene,
        &mut world,
        |_| Some(MeshHandle::TEST),
        |name| library.material_by_name(name),
    );
    assert!(missing.is_empty(), "{missing:?}");
    let surface = world.query::<&runity::Surface>().iter().map(|s| s.0).next();
    surface.expect("the entity has a surface")
}

#[test]
fn a_hex_colour_becomes_a_material_a_scene_can_name() {
    let dir = temp("hex");
    let library_dir = dir.join("library");
    write_material(&dir, &library_dir, "mossy_stone", r##"(color: "#4a5a3c")"##);

    let (library, problems) = Library::open(&library_dir).unwrap();
    assert!(problems.is_empty(), "{problems:?}");

    // Hex is sRGB, and the asset holds linear: the conversion happens at
    // import, once, rather than in every hand-written scene.
    let expected = Material::from_srgb(0x4a, 0x5a, 0x3c);
    let found = library
        .material_by_name("mossy_stone")
        .expect("the material we just imported");
    assert_eq!(found, expected);
    assert!(
        found.base_color[1] > found.base_color[0],
        "a mossy green should be greenest: {:?}",
        found.base_color
    );

    // And a scene that names it draws with it.
    assert_eq!(
        surface_of(&scene_naming("mossy_stone"), &library),
        expected,
        "the palette should reach the entity's surface"
    );

    // The content browser can list the palette without knowing what else is
    // in the library.
    let materials: Vec<&str> = library
        .names_of(runity::asset::AssetKind::Material)
        .collect();
    assert_eq!(materials, vec!["mossy_stone"]);
}

#[test]
fn a_project_material_shadows_a_builtin_and_builtin_reaches_past_it() {
    // The seven builtins exist so an example scene can be written before a
    // palette does. They are not a reservation: a project that defines its own
    // `stone` means its own, and `builtin:stone` is there for when the engine's
    // was what was wanted.
    let dir = temp("shadow");
    let library_dir = dir.join("library");
    write_material(&dir, &library_dir, "stone", r##"(color: "#ff0000")"##);
    let (library, _) = Library::open(&library_dir).unwrap();

    let theirs = surface_of(&scene_naming("stone"), &library);
    assert!(
        theirs.base_color[0] > 0.9 && theirs.base_color[1] < 0.01,
        "the project's own stone is red: {:?}",
        theirs.base_color
    );
    assert_eq!(
        surface_of(&scene_naming("builtin:stone"), &library),
        runity::material::builtin::STONE,
    );

    // A name nothing answers to is plain grey, not a failure to load: a typo
    // should leave a visible mistake in the scene rather than no scene.
    assert_eq!(
        surface_of(&scene_naming("mossy_stnoe"), &library),
        Material::default()
    );
}

#[test]
fn an_unlit_material_survives_the_trip_through_the_asset() {
    // Unlit is the one thing about a material that is not a colour, so it is
    // the one thing a round trip can quietly drop.
    let dir = temp("unlit");
    let library_dir = dir.join("library");
    write_material(
        &dir,
        &library_dir,
        "ember",
        "(color: (1.0, 0.35, 0.1), unlit: true)",
    );
    let (library, _) = Library::open(&library_dir).unwrap();
    let ember = library.material_by_name("ember").unwrap();
    assert_eq!(ember.shading, Shading::Unlit);
    assert_eq!(ember.base_color, [1.0, 0.35, 0.1], "linear stays linear");
}

#[test]
fn editing_the_hex_changes_what_the_scene_draws() {
    // This is the loop the whole asset pipeline exists to make short: change
    // a colour in a text file, and a library that is already open picks it up
    // without being reopened and without the scene being touched.
    let root = temp("edit");
    std::fs::remove_dir_all(&root).unwrap();
    let project = runity::Project::create(&root, "edit").unwrap();
    let source = project.materials().join("clay.rmat");
    std::fs::write(&source, r##"(color: "#8a5a3c")"##).unwrap();
    runity_import::import_into(&project, &source, None).unwrap();
    let (mut library, _) = Library::open(project.library()).unwrap();
    let before = library.material_by_name("clay").unwrap();

    std::fs::write(&source, r##"(color: "#3c5a8a")"##).unwrap();
    touch_forward(&source);
    let done = runity_import::sync(&project);
    assert_eq!(done.len(), 1, "the one changed material");
    assert!(done[0].result.is_ok(), "{:?}", done[0].result);
    touch_forward(&runity_import::built_for(&source, &project.library()).unwrap());

    let changed = library.reload_changed();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].kind, runity::asset::AssetKind::Material);

    let after = library.material_by_name("clay").unwrap();
    assert_ne!(after, before);
    assert!(
        after.base_color[2] > after.base_color[0],
        "warm clay became cool: {:?}",
        after.base_color
    );
    assert_eq!(
        surface_of(&scene_naming("clay"), &library),
        after,
        "and a scene spawned now gets the new colour"
    );
}

#[test]
fn a_colour_that_is_not_a_colour_says_so_instead_of_importing_black() {
    // A typo in a hex code that silently became black would be indisputably
    // worse than an error: the scene would look deliberately dark.
    let dir = temp("bad");
    let source = dir.join("broken.rmat");
    std::fs::write(&source, r##"(color: "#12345")"##).unwrap();
    let err = runity_import::import_file(
        &source,
        dir.join("library"),
        runity_import::ImportSettings::for_source("broken.rmat"),
    )
    .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("hex"),
        "the error should name the problem: {message}"
    );
    assert!(
        !dir.join("library").join("broken.rmat.rasset").exists(),
        "and nothing should have been written"
    );
}

#[test]
fn the_example_palette_stands_in_for_the_builtins() {
    // The engine ships a palette so that a project has something to start
    // from and so that this path has something real to run on. It is the
    // builtins written out as assets, which means it can be checked against
    // them: import the directory, resolve the reference scene's material
    // names through it, and every colour should land where the builtin was.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/runity-import is two levels down");
    let sources = root.join("examples/valley/materials");
    let library_dir = temp("examples").join("library");

    let mut imported = 0;
    for entry in std::fs::read_dir(&sources).expect("the example palette") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rmat") {
            continue;
        }
        let relative = format!(
            "examples/valley/materials/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        // The sidecar goes beside the temporary library, not beside the
        // committed source: a test must not write into the repository.
        let sidecar = library_dir.parent().unwrap().join(format!(
            "{}.rimport",
            path.file_name().unwrap().to_string_lossy()
        ));
        runity_import::import_to(
            &path,
            &library_dir,
            &sidecar,
            runity_import::ImportSettings::for_source(relative),
        )
        .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        imported += 1;
    }
    assert!(imported >= 6, "only {imported} materials in the palette");

    let (library, problems) = Library::open(&library_dir).unwrap();
    assert!(problems.is_empty(), "{problems:?}");

    for name in runity::material::builtin::NAMES {
        let asset = library
            .material_by_name(name)
            .unwrap_or_else(|| panic!("the palette should cover the builtin {name}"));
        let builtin = runity::material::builtin::by_name(name).unwrap();
        // Hex is eight bits per channel, so the round trip through sRGB is
        // lossy — but only in the last thousandth, which is why a tolerance
        // this tight is still a real check and not a rubber stamp.
        for axis in 0..3 {
            let (a, b) = (asset.base_color[axis], builtin.base_color[axis]);
            assert!(
                (a - b).abs() < 0.005,
                "{name} channel {axis}: asset {a}, builtin {b}"
            );
        }
        assert_eq!(asset.shading, builtin.shading, "{name}");
    }

    // And at least one name the engine does not know, because that is the
    // point of a project palette.
    assert!(library.material_by_name("moss").is_some());
    assert!(runity::material::builtin::by_name("moss").is_none());

    // The reference scene names materials the palette now answers for, so
    // rendering it with a library and without one should agree.
    let scene = Scene::load(root.join("examples/valley/scenes/first-light.ron")).unwrap();
    let named = scene
        .flatten()
        .iter()
        .filter(|(e, _)| !matches!(&e.material, runity::scene::MaterialRef::Inline(_)))
        .count();
    assert!(named > 0, "the reference scene names its materials");
    for (entity, _) in scene.flatten() {
        let with = entity.material_from(|name| library.material_by_name(name));
        let without = entity.material();
        for axis in 0..3 {
            assert!(
                (with.base_color[axis] - without.base_color[axis]).abs() < 0.005,
                "{}: {with:?} through the palette, {without:?} through the builtins",
                entity.name
            );
        }
    }
}

/// File timestamps have a resolution, and writing twice inside one of its
/// ticks produces two files claiming the same mtime. Bumping it is more
/// honest than sleeping and hoping.
fn touch_forward(path: &Path) {
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

#[test]
fn an_instance_is_its_parent_with_what_it_says_changed_and_follows_the_parent() {
    let root = std::env::temp_dir().join(format!("runity-instances-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = runity::Project::create(&root, "instances").unwrap();
    let materials = project.materials();
    std::fs::write(
        materials.join("stone.rmat"),
        r##"(color: "#808080", smoothness: 0.2, metallic: 0.1)"##,
    )
    .unwrap();
    std::fs::write(
        materials.join("wet_stone.rmat"),
        r#"// Stone after rain.
(parent: "stone", smoothness: 0.8)"#,
    )
    .unwrap();
    runity_import::sync(&project);
    let read = |name: &str| {
        Library::open(project.library())
            .unwrap()
            .0
            .material_by_name(name)
            .unwrap()
    };
    let (stone, wet) = (read("stone"), read("wet_stone"));
    assert_eq!(wet.base_color, stone.base_color, "the parent's colour");
    assert_eq!(wet.metallic, stone.metallic, "and metal");
    assert_eq!(wet.smoothness, 0.8, "its own smoothness");

    // The parent changes colour: the instance follows, rebuilt by sync.
    let stone_file = materials.join("stone.rmat");
    std::fs::write(&stone_file, r##"(color: "#ff0000", smoothness: 0.2)"##).unwrap();
    touch_forward(&stone_file);
    let changed = runity_import::sync(&project);
    assert!(
        changed.iter().any(|r| r.source.ends_with("wet_stone.rmat")),
        "{changed:?}"
    );
    let wet = read("wet_stone");
    assert!(
        wet.base_color[0] > 0.9 && wet.base_color[1] < 0.1,
        "red now: {:?}",
        wet.base_color
    );
    assert_eq!(wet.smoothness, 0.8);

    // A parent that is not there says so.
    std::fs::write(materials.join("odd.rmat"), r#"(parent: "nowhere")"#).unwrap();
    let said = runity_import::sync(&project);
    let odd = said
        .iter()
        .find(|r| r.source.ends_with("odd.rmat"))
        .unwrap();
    assert!(
        odd.result
            .as_ref()
            .is_err_and(|e| e.contains("no material `nowhere`")),
        "{odd:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
