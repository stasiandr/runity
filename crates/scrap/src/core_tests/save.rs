//! The core's `save`, tested with the modules' fields on its lines.

    use crate::save::*;
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    #[allow(unused_imports)]
    use serde::{Deserialize, Serialize};
    #[allow(unused_imports)]
    use crate::components::Components;
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::world::{addressable, despawn_tree, NetId, NetPrefab};
    #[allow(unused_imports)]
    use crate::scene::{Scene, Transform};
    #[allow(unused_imports)]
    use crate::world::SceneId;
    #[allow(unused_imports)]
    use crate::prelude::*;
    use crate::render::MeshHandle;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Locked(bool);

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Stiffness(f32);

    const KEEP: &str = r#"(entities: [
        (id: "a1", name: "door", model: "m", components: { "locked": (true), "stiffness": (3.0) }),
        (id: "b2", name: "crate", model: "m"),
        (id: "c3", name: "barrel", model: "m"),
    ])"#;

    fn components() -> Components {
        let mut c = Components::new();
        c.register_saved::<Locked>("locked")
            .register::<Stiffness>("stiffness");
        c
    }

    fn start() -> (hecs::World, Scene) {
        let mut scene: Scene = ron::from_str(KEEP).unwrap();
        scene.assign_ids();
        let mut world = hecs::World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        components().apply(&scene, &mut world);
        (world, scene)
    }

    fn entity(world: &hecs::World, id: &str) -> Option<hecs::Entity> {
        addressable(world).get(&id.parse().unwrap()).copied()
    }

    #[test]
    fn a_saved_game_loads_back_into_a_fresh_world_from_the_same_scene() {
        let components = components();
        let (mut world, scene) = start();
        // Play: unlock the door, push the crate, smash the barrel, drop a
        // torch, and change something that is not saved.
        let door = entity(&world, "a1").unwrap();
        world.get::<&mut Locked>(door).unwrap().0 = false;
        world.get::<&mut Stiffness>(door).unwrap().0 = 99.0;
        let crate_ = entity(&world, "b2").unwrap();
        world.get::<&mut Transform>(crate_).unwrap().position.x = 4.0;
        let barrel = entity(&world, "c3").unwrap();
        despawn_tree(&mut world, barrel);
        let torch = world.spawn((Transform {
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            ..Transform::default()
        },));
        crate::net::announce(&mut world, torch, crate::net::PeerId::HOST, "torch");

        let path = std::env::temp_dir().join("scrap-save/slot1.ron");
        capture(&world, &components, &scene).write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"locked\"") && !text.contains("stiffness"),
            "{text}"
        );

        let (mut loaded, _) = start();
        let save = SaveGame::read(&path).unwrap();
        let done = restore(&mut loaded, &components, &save, |w, prefab, t| {
            (prefab == "torch").then(|| w.spawn((t,)))
        });
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!((done.spawned, done.removed), (1, 1));
        let door = entity(&loaded, "a1").unwrap();
        assert_eq!(*loaded.get::<&Locked>(door).unwrap(), Locked(false));
        assert_eq!(
            *loaded.get::<&Stiffness>(door).unwrap(),
            Stiffness(3.0),
            "not saved: from the scene"
        );
        let crate_ = entity(&loaded, "b2").unwrap();
        assert_eq!(loaded.get::<&Transform>(crate_).unwrap().position.x, 4.0);
        assert!(entity(&loaded, "c3").is_none(), "the barrel stays smashed");
        let torch = loaded
            .query::<(&NetPrefab, &Transform)>()
            .iter()
            .map(|(_, t)| t.position)
            .next()
            .expect("the torch is back");
        assert_eq!(torch, glam::Vec3::new(1.0, 2.0, 3.0));
        // And saving the loaded world says the same.
        assert_eq!(capture(&loaded, &components, &scene), save);
    }
