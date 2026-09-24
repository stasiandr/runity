//! Editing a project's files while the game runs.
//!
//! The loop the first postulate is about, end to end on a real renderer:
//! the game has a world with its own state in it, someone saves a scene or
//! a new asset lands in the library, and the next reload shows the change
//! without the game losing what it had.

#[allow(unused_imports)]
use runity::prelude::*;
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

#[test]
fn two_scenes_share_a_world_and_each_reloads_and_unloads_only_its_own() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("additive");
    let (village, forest) = (
        project.scenes().join("village.ron"),
        project.scenes().join("forest.ron"),
    );
    write(
        &village,
        r#"(entities: [(id: "c1", name: "well", model: "builtin:cube")])"#,
    );
    write(
        &forest,
        r#"(entities: [(id: "d1", name: "oak", model: "builtin:cone"), (id: "d2", name: "elm", model: "builtin:cone")])"#,
    );
    let mut world = hecs::World::new();
    let (mut one, _) = LiveScene::open(&village).unwrap();
    let (mut two, _) = LiveScene::open(&forest).unwrap();
    one.spawn(&mut world, &gpu, &mut renderer);
    two.spawn(&mut world, &gpu, &mut renderer);
    let well = entity(&world, "c1");

    // The forest loses a tree: the village is not touched.
    write(
        &forest,
        r#"(entities: [(id: "d1", name: "oak", model: "builtin:cone")])"#,
    );
    let done = two.reload(&mut world, &gpu, &mut renderer);
    assert_eq!(done.patched.unwrap().despawned, 1);
    assert!(world.contains(well));
    assert!(one.reload(&mut world, &gpu, &mut renderer).is_empty());

    // Leaving the forest takes the forest, and only the forest.
    assert_eq!(two.unload(&mut world), 1);
    assert!(world.contains(well));
    assert_eq!(world.query::<&SceneId>().iter().count(), 1);
}

/// A door as the game was built with it...
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct DoorBefore {
    open: bool,
}

/// ...and after a patch gave it a field: another type, laid out
/// differently, under the same name.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct DoorAfter {
    open: bool,
    #[serde(default)]
    creaks: bool,
}

/// Scenery, not saved: starts from the scene again.
#[derive(Debug, PartialEq, serde::Deserialize)]
struct Tint(f32);

#[test]
fn after_a_hot_patch_the_world_starts_again_under_new_types_keeping_what_a_save_keeps() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("reinstance");
    let path = project.scenes().join("main.ron");
    write(
        &path,
        r#"(entities: [
            (id: "a1", name: "door", model: "builtin:cube",
             components: { "door": (open: false), "tint": (0.5) }),
        ])"#,
    );
    let mut before = runity::Components::new();
    before
        .register_saved::<DoorBefore>("door")
        .register::<Tint>("tint");
    let (live, _) = LiveScene::open(&path).unwrap();
    let mut live = live.with_components(before);
    let mut world = hecs::World::new();
    live.spawn(&mut world, &gpu, &mut renderer);

    // The game plays: the door opens, it moves, its tint changes.
    let door = entity(&world, "a1");
    world.get::<&mut DoorBefore>(door).unwrap().open = true;
    world
        .get::<&mut runity::Transform>(door)
        .unwrap()
        .position
        .x = 3.0;
    world.get::<&mut Tint>(door).unwrap().0 = 0.9;

    let mut after = runity::Components::new();
    after
        .register_saved::<DoorAfter>("door")
        .register::<Tint>("tint");
    let restored = live.reinstance(&mut world, after, &gpu, &mut renderer);
    assert!(restored.problems.is_empty(), "{:?}", restored.problems);

    let door = entity(&world, "a1");
    assert_eq!(
        *world.get::<&DoorAfter>(door).unwrap(),
        DoorAfter {
            open: true,
            creaks: false
        },
        "the open door, in its new shape"
    );
    assert!(
        world.get::<&DoorBefore>(door).is_err(),
        "nothing of the old type"
    );
    assert_eq!(
        world.get::<&runity::Transform>(door).unwrap().position.x,
        3.0
    );
    assert_eq!(world.get::<&Tint>(door).unwrap().0, 0.5, "scenery restarts");
    assert_eq!(
        world.query::<&SceneId>().iter().count(),
        1,
        "one door, not two"
    );
}

#[test]
fn a_game_changes_level_keeping_what_it_spawned_itself() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 8, 8);
    let mut renderer = Renderer::new(&gpu, &target);
    let project = project("switch");
    write(
        &project.scenes().join("cave.ron"),
        r#"(entities: [(id: "c1", name: "stalagmite", model: "builtin:cone")])"#,
    );
    let (mut live, _) = LiveScene::open(project.scenes().join("main.ron")).unwrap();
    let mut world = hecs::World::new();
    live.spawn(&mut world, &gpu, &mut renderer);
    let player = world.spawn((Lit(true),));

    let e = live
        .switch("cvae", &mut world, &gpu, &mut renderer)
        .unwrap_err();
    assert!(e.to_string().contains("did you mean `cave`?"), "{e}");
    assert_eq!(world.query::<&SceneId>().iter().count(), 2, "still on main");

    live.switch("cave", &mut world, &gpu, &mut renderer)
        .unwrap();
    let lines: Vec<String> = world
        .query::<&SceneId>()
        .iter()
        .map(|s| s.0.to_string())
        .collect();
    assert_eq!(
        lines,
        ["00000000000000c1"],
        "main's lines gone, the cave's in"
    );
    assert!(world.contains(player), "the game's own things cross over");
    assert!(live.path().ends_with("scenes/cave.ron"));
}
