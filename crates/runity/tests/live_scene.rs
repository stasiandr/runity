//! Editing a project's files while the game runs.
//!
//! The loop the first postulate is about, end to end on a real renderer:
//! the game has a world with its own state in it, someone saves a scene or
//! a new asset lands in the library, and the next reload shows the change
//! without the game losing what it had.

use std::path::Path;

use runity::asset::{self, AssetKind};
use runity::{builtin, Gpu, LiveScene, Model, OffscreenTarget, Project, Renderer, SceneId};

/// Something the game put on an entity, which no file knows about.
#[derive(Debug, PartialEq)]
struct Lit(bool);

const SCENE: &str = r#"(entities: [
    (id: "a1", name: "campfire", model: "builtin:cube", material: "stone"),
    (id: "b2", name: "rock", model: "rock"),
])"#;

fn project(name: &str) -> Project {
    let root = std::env::temp_dir().join(format!("runity-live-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, name).unwrap();
    write(&project.scenes().join("main.ron"), SCENE);
    project
}

/// Write a file and push its clock forward, so two writes inside one tick
/// of the filesystem's clock still look like two.
fn write(path: &Path, contents: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

/// A mesh asset called `rock`, a cube of the given size.
fn rock(project: &Project, size: f32) {
    let mut mesh = builtin::cube(size);
    mesh.name = "rock".into();
    mesh.id = runity::AssetId::from_source("assets/rock.obj", 0);
    let bytes = asset::to_bytes(&mesh, AssetKind::Mesh).unwrap();
    write(&project.library().join("rock.obj.rasset"), bytes);
}

fn entity(world: &hecs::World, id: &str) -> hecs::Entity {
    let id: runity::EntityId = id.parse().unwrap();
    world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .find(|(_, s)| s.0 == id)
        .map(|(e, _)| e)
        .unwrap()
}

#[test]
fn a_saved_scene_reaches_the_running_world_and_the_game_keeps_its_state() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("scene");
    let path = project.scenes().join("main.ron");

    let (mut live, problems) = LiveScene::open(&path).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let mut world = hecs::World::new();
    let missing = live.spawn(&mut world, &gpu, &mut renderer).missing;
    assert_eq!(missing.len(), 1, "no rock in the library yet: {missing:?}");
    let campfire = entity(&world, "a1");
    world.insert_one(campfire, Lit(true)).unwrap();

    // Nothing changed, nothing done.
    assert!(live.reload(&mut world, &gpu, &mut renderer).is_empty());

    // Someone moves the campfire in a text editor.
    write(
        &path,
        SCENE.replace(
            r#"name: "campfire", model"#,
            r#"name: "campfire", transform: (position: (3.0, 0.0, 0.0)), model"#,
        ),
    );
    let done = live.reload(&mut world, &gpu, &mut renderer);
    let patched = done.patched.expect("the scene changed");
    assert_eq!(patched.updated, 1, "{patched:?}");
    assert_eq!(
        world
            .get::<&runity::Transform>(campfire)
            .unwrap()
            .position
            .x,
        3.0
    );
    assert_eq!(
        *world.get::<&Lit>(campfire).unwrap(),
        Lit(true),
        "still lit"
    );

    // Half a line saved: said so, and the world keeps what it had.
    write(&path, "(entities: [(id: \"a1\", name: ");
    let done = live.reload(&mut world, &gpu, &mut renderer);
    assert_eq!(done.problems.len(), 1, "{done:?}");
    assert!(
        done.problems[0].contains("main.ron"),
        "names the file: {done:?}"
    );
    assert!(world.contains(campfire));
    assert_eq!(live.scene().entities.len(), 2, "the last good version");
}

#[test]
fn an_asset_imported_while_the_game_runs_is_picked_up_and_replaced() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("asset");
    let (mut live, _) = LiveScene::open(project.scenes().join("main.ron")).unwrap();
    let mut world = hecs::World::new();
    live.spawn(&mut world, &gpu, &mut renderer);
    let stone = entity(&world, "b2");
    assert!(world.get::<&Model>(stone).is_err(), "waiting for its mesh");

    // The importer writes the rock into the library.
    rock(&project, 1.0);
    let done = live.reload(&mut world, &gpu, &mut renderer);
    assert_eq!(done.assets.len(), 1, "{done:?}");
    let first = world.get::<&Model>(stone).expect("it arrived").0;

    // And re-imports it, bigger: the entity draws the new upload.
    rock(&project, 3.0);
    let done = live.reload(&mut world, &gpu, &mut renderer);
    assert_eq!(done.assets.len(), 1, "{done:?}");
    let second = world.get::<&Model>(stone).unwrap().0;
    assert_ne!(first, second, "a new upload, swapped in");
    let bounds = live
        .library()
        .unwrap()
        .mesh_by_name("rock")
        .unwrap()
        .bounds
        .max[0];
    assert_eq!(bounds.to_native(), 1.5);
}

/// A component a prefab part carries.
#[derive(Debug, PartialEq, serde::Deserialize)]
struct Warmth(f32);

#[test]
fn a_prefab_spawned_at_run_time_is_whole_and_outlives_a_reload() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("spawn");
    write(
        &project.prefabs().join("campfire.prefab"),
        r#"(id: "c1", name: "campfire", model: "builtin:cube",
            children: [(id: "c2", name: "ember", model: "builtin:sphere",
                        transform: (position: (0.0, 1.0, 0.0)),
                        components: { "warmth": (3.5) })])"#,
    );
    let mut components = runity::Components::new();
    components.register::<Warmth>("warmth");
    let path = project.scenes().join("main.ron");
    let (live, _) = LiveScene::open(&path).unwrap();
    let mut live = live.with_components(components);
    let mut world = hecs::World::new();
    live.spawn(&mut world, &gpu, &mut renderer);

    let at = runity::Transform {
        position: runity::glam::Vec3::new(10.0, 0.0, 0.0),
        ..runity::Transform::default()
    };
    let fire = live
        .spawn_prefab("campfire", at, None, &mut world, &gpu, &mut renderer)
        .unwrap();
    assert!(fire.problems.is_empty(), "{:?}", fire.problems);
    assert!(
        world.get::<&SceneId>(fire.root).is_err(),
        "the game's, not the file's"
    );
    let ember = world
        .query::<(hecs::Entity, &Warmth, &runity::world::WorldTransform)>()
        .iter()
        .map(|(e, w, t)| (e, w.0, t.0.w_axis))
        .next()
        .expect("the ember, with its component");
    assert_eq!(ember.1, 3.5);
    assert_eq!((ember.2.x, ember.2.y), (10.0, 1.0), "placed under the root");

    // Someone edits the scene; the fire the game lit stays lit.
    write(&path, SCENE.replace("\"stone\"", "\"moss\""));
    let done = live.reload(&mut world, &gpu, &mut renderer);
    assert!(done.patched.is_some());
    assert!(world.contains(fire.root) && world.contains(ember.0));

    let err = live
        .spawn_prefab("campfir", at, None, &mut world, &gpu, &mut renderer)
        .unwrap_err();
    assert!(err.contains("did you mean `campfire`?"), "{err}");
}
