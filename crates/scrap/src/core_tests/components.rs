//! The core's `components`, tested with the modules' fields on its lines.

    use crate::components::*;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashMap};
    #[allow(unused_imports)]
    use std::fmt;
    #[allow(unused_imports)]
    use hecs::World;
    #[allow(unused_imports)]
    use ron::value::RawValue;
    #[allow(unused_imports)]
    use serde::de::DeserializeOwned;
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::scene::{EntityDesc, Scene};
    #[allow(unused_imports)]
    use crate::world::SceneId;
    #[allow(unused_imports)]
    use crate::prelude::*;
    use crate::render::MeshHandle;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Door {
        open_angle: f32,
        #[serde(default)]
        locked: bool,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Loot(String);

    fn components() -> Components {
        let mut components = Components::new();
        components.register::<Door>("door").register::<Loot>("loot");
        components
    }

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn spawned(scene: &Scene) -> (World, hecs::Entity) {
        let mut world = World::new();
        crate::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let entity = world.query::<hecs::Entity>().iter().next().unwrap();
        (world, entity)
    }

    const GATE: &str = r#"(entities: [(id: "a1", name: "gate", model: "m",
        components: { "door": (open_angle: 90.0), "loot": ("chest") })])"#;

    #[test]
    fn a_scene_line_brings_the_games_components() {
        let gate = scene(GATE);
        let (mut world, entity) = spawned(&gate);
        assert!(components().apply(&gate, &mut world).is_empty());
        assert_eq!(
            *world.get::<&Door>(entity).unwrap(),
            Door {
                open_angle: 90.0,
                locked: false
            }
        );
        assert_eq!(world.get::<&Loot>(entity).unwrap().0, "chest");
    }

    #[test]
    fn a_reload_writes_changed_components_and_keeps_the_rest() {
        let before = scene(GATE);
        let (mut world, entity) = spawned(&before);
        let components = components();
        components.apply(&before, &mut world);
        // The game swings the door; someone edits the loot table.
        world.get::<&mut Door>(entity).unwrap().open_angle = 12.0;
        let after = scene(&GATE.replace("(\"chest\")", "(\"barrel\")"));
        assert!(components.patch(&before, &after, &mut world).is_empty());
        assert_eq!(world.get::<&Door>(entity).unwrap().open_angle, 12.0);
        assert_eq!(world.get::<&Loot>(entity).unwrap().0, "barrel");

        // And a component taken out of the file is taken off the entity.
        let bare = scene(
            &GATE
                .replace(r#", "loot": ("barrel")"#, "")
                .replace(r#", "loot": ("chest")"#, ""),
        );
        components.patch(&after, &bare, &mut world);
        assert!(world.get::<&Loot>(entity).is_err());
        assert!(world.get::<&Door>(entity).is_ok());
    }

    #[test]
    fn a_name_nobody_registered_and_a_value_that_does_not_fit_are_said() {
        let odd = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m",
                components: { "dor": (open_angle: 1.0), "loot": (42) })])"#,
        );
        let (mut world, _) = spawned(&odd);
        let problems = components().apply(&odd, &mut world);
        assert_eq!(problems.len(), 2, "{problems:?}");
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert!(text[0].contains("did you mean `door`?"), "{text:?}");
        assert!(
            text[1].starts_with("`gate` (00000000000000a1): component `loot`"),
            "{text:?}"
        );
    }

    #[test]
    fn component_text_survives_a_save_byte_for_byte() {
        let text = "(\n    entities: [\n        (\n            id: \"00000000000000a1\",\n            name: \"gate\",\n            model: \"m\",\n            components: {\n                \"door\": (open_angle: 90.0,   locked: true),\n            },\n        ),\n    ],\n)\n";
        let dir = std::env::temp_dir().join("scrap-components-save");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("scene.ron");
        std::fs::write(&path, text).unwrap();
        let loaded = Scene::load(&path).unwrap();
        assert_eq!(
            loaded.entities[0].components["door"].get_ron(),
            "(open_angle: 90.0,   locked: true)",
            "as written, spacing and all"
        );
        loaded.save(&path).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        Scene::load(&path).unwrap().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once, "stable");
        assert!(
            once.contains("(open_angle: 90.0,   locked: true)"),
            "{once}"
        );
    }
