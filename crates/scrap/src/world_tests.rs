#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use crate::prelude::*;
    #[allow(unused_imports)]
    use crate::world::*;
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::material::Material;
    #[allow(unused_imports)]
    use crate::render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, TextureHandle};
    #[allow(unused_imports)]
    use crate::scene::{Body, EntityDesc, Scene, Transform};
    #[allow(unused_imports)]
    use hecs::World;
    #[allow(unused_imports)]
    use std::collections::{HashMap, HashSet};

    #[test]
    fn at_night_the_moon_lights_from_above_and_the_sky_from_under_the_horizon() {
        let day = scene_lighting(&crate::scene::Sun {
            hour: 13.0,
            ..crate::scene::Sun::default()
        });
        assert_eq!(day.night, 0.0);
        assert!(day.sky_sun.is_none());
        let night = scene_lighting(&crate::scene::Sun {
            hour: 23.0,
            intensity: 1.2,
            ..crate::scene::Sun::default()
        });
        assert_eq!(night.night, 1.0);
        assert!(
            night.sun_direction.y < -0.1,
            "the moon is up: {}",
            night.sun_direction
        );
        assert!(
            night.sun_color.z > night.sun_color.x,
            "and cold: {}",
            night.sun_color
        );
        assert!(night.sun_intensity < day.sun_intensity * 0.15, "and dim");
        let (sun, _) = night.sky_sun.expect("the sky lit by the sun where it is");
        assert!(
            sun.y > 0.0,
            "under the horizon, its light travels up: {sun}"
        );
    }

    #[test]
    fn sand_underfoot_lights_what_faces_down_warm_and_bright() {
        let grass = scene_lighting(&crate::scene::Sun::default());
        let sand = scene_lighting(&crate::scene::Sun {
            ground: [0.78, 0.6, 0.38],
            ..crate::scene::Sun::default()
        });
        assert!(
            sand.ground_color.x > grass.ground_color.x * 3.0,
            "{} vs {}",
            sand.ground_color,
            grass.ground_color
        );
        assert!(
            sand.ground_color.x > sand.ground_color.z * 1.5,
            "warm: {}",
            sand.ground_color
        );
        // With the sun down to a glimmer the ground has little to send back.
        let night = scene_lighting(&crate::scene::Sun {
            intensity: 0.05,
            ground: [0.78, 0.6, 0.38],
            ..crate::scene::Sun::default()
        });
        assert!(
            night.ground_color.x < sand.ground_color.x * 0.2,
            "{}",
            night.ground_color
        );
    }
    use glam::Vec3;

    /// An entity with nothing set, for `..blank()` in the tests below.
    fn blank() -> EntityDesc {
        EntityDesc {
            parts: Default::default(),
            in_part: None,
            inactive: false,
            overrides: Default::default(),
            components: Default::default(),
            id: Default::default(),
            name: String::new(),
            prefab: Default::default(),
            transform: Transform::default(),
            children: Vec::new(),
        }
        .with(crate::scene::ModelRef("m".into()))
        .with(Body::None)
        .with(crate::scene::Collider::None)
    }

    fn scene_with(models: &[&str]) -> Scene {
        Scene {
            entities: models
                .iter()
                .enumerate()
                .map(|(i, model)| {
                    EntityDesc {
                        parts: Default::default(),
                        in_part: None,
                        inactive: false,
                        overrides: Default::default(),
                        components: Default::default(),
                        id: Default::default(),
                        name: format!("thing {i}"),
                        prefab: Default::default(),
                        transform: Transform {
                            position: Vec3::new(i as f32, 0.0, 0.0),
                            ..Default::default()
                        },
                        children: Vec::new(),
                    }
                    .with(crate::scene::ModelRef((*model).into()))
                    .with(Body::Static)
                    .with(crate::scene::Collider::None)
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_scene_becomes_entities_and_then_a_draw_list() {
        let scene = scene_with(&["pine", "pine", "rock"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        assert!(missing.is_empty());

        let frame = build_frame(
            &world,
            Camera::default(),
            Lighting::default(),
            FogSettings::default(),
        );
        assert_eq!(frame.draws.len(), 3);
        // The clear colour is the fog colour, so the horizon and the far
        // distance meet instead of showing a seam.
        assert_eq!(frame.clear_color, frame.fog.color);
    }

    #[test]
    fn a_child_is_placed_relative_to_its_parent() {
        // A cart at x = 10 with a wheel at x = 1 puts the wheel at 11, and
        // that is the whole point of a hierarchy: place the cart, and the
        // wheel comes along.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "cart".into(),
                transform: Transform {
                    position: Vec3::new(10.0, 0.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "wheel".into(),
                    transform: Transform {
                        position: Vec3::new(1.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }
                .with(crate::scene::ModelRef("m".into()))],
                ..blank()
            }
            .with(crate::scene::ModelRef("m".into()))],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        let mut positions: Vec<f32> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.x)
            .collect();
        positions.sort_by(f32::total_cmp);
        assert_eq!(positions, vec![10.0, 11.0]);
    }

    #[test]
    fn a_parents_rotation_carries_its_children_around_with_it() {
        // Translation alone would pass even if the child's matrix were being
        // added rather than multiplied. A quarter turn tells them apart.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "turntable".into(),
                transform: Transform {
                    rotation_deg: Vec3::new(0.0, 90.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "arm".into(),
                    transform: Transform {
                        position: Vec3::new(2.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }
                .with(crate::scene::ModelRef("m".into()))],
                ..blank()
            }
            .with(crate::scene::ModelRef("m".into()))],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // +X rotated 90° about Y lands on -Z in a right-handed system.
        let arm = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.truncate())
            .find(|p| p.length() > 0.5)
            .expect("the arm is offset from its parent");
        assert!(arm.x.abs() < 1e-4, "expected the arm off the X axis: {arm}");
        assert!((arm.z + 2.0).abs() < 1e-4, "expected z = -2, got {arm}");
    }

    #[test]
    fn moving_a_parent_moves_everything_under_it() {
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                children: vec![EntityDesc {
                    name: "child".into(),
                    transform: Transform {
                        position: Vec3::new(0.0, 3.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "grandchild".into(),
                        transform: Transform {
                            position: Vec3::new(0.0, 3.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }
                    .with(crate::scene::ModelRef("m".into()))],
                    ..blank()
                }
                .with(crate::scene::ModelRef("m".into()))],
                ..blank()
            }
            .with(crate::scene::ModelRef("m".into()))],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // Shove the root sideways and re-resolve.
        let root = world
            .query::<(hecs::Entity, &Transform)>()
            .iter()
            .find(|(_, t)| t.position == Vec3::ZERO)
            .map(|(e, _)| e)
            .expect("the root sits at the origin");
        world.get::<&mut Transform>(root).unwrap().position = Vec3::new(5.0, 0.0, 0.0);
        apply_hierarchy(&mut world);

        let mut heights: Vec<(f32, f32)> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| (placed.0.w_axis.x, placed.0.w_axis.y))
            .collect();
        heights.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(heights, vec![(5.0, 0.0), (5.0, 3.0), (5.0, 6.0)]);
    }

    #[test]
    fn a_child_whose_model_is_missing_keeps_its_own_children_in_place() {
        // Dropping the branch would move the grandchild, which is a worse
        // failure than a hole where one model should be.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                children: vec![EntityDesc {
                    name: "broken".into(),
                    transform: Transform {
                        position: Vec3::new(4.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "fine".into(),
                        transform: Transform {
                            position: Vec3::new(1.0, 0.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }
                    .with(crate::scene::ModelRef("m".into()))],
                    ..blank()
                }
                .with(crate::scene::ModelRef("missing".into()))],
                ..blank()
            }
            .with(crate::scene::ModelRef("m".into()))],
            ..Default::default()
        };
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);

        let drawn: Vec<f32> = world
            .query::<(&WorldTransform, &Model)>()
            .iter()
            .map(|(placed, _)| placed.0.w_axis.x)
            .collect();
        assert!(
            drawn.contains(&5.0),
            "the grandchild should still stand at 4 + 1, got {drawn:?}"
        );
    }

    #[test]
    fn a_model_that_cannot_be_resolved_is_reported_not_skipped_in_silence() {
        let scene = scene_with(&["pine", "missing"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].model, "missing");
        assert_eq!(missing[0].entity_name, "thing 1");

        // The entity still exists — it just has nothing to draw. Dropping it
        // would move anything parented to it, and a hole where one model
        // should be is a smaller failure than a subtree that silently moved.
        assert_eq!(world.len(), 2);
        assert_eq!(world.query::<&Model>().iter().count(), 1);
    }

    // --- live reload ------------------------------------------------------

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn spawned(scene: &Scene) -> World {
        let mut world = World::new();
        spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        world
    }

    fn entity(world: &World, id: &str) -> hecs::Entity {
        let id: EntityId = id.parse().unwrap();
        world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("no entity {id}"))
    }

    fn patch(before: &Scene, after: &Scene, world: &mut World) -> Patched {
        patch_scene(before, after, world, |_| Some(MeshHandle::TEST), |_| None)
    }

    /// Something the game put on an entity, which no file knows about.
    #[derive(Debug, PartialEq)]
    struct Health(u32);

    const DOOR: &str = r#"(entities: [
        (id: "d", name: "door", model: "m", transform: (position: (0.0, 0.0, 0.0))),
        (id: "a1", name: "wall", model: "m", material: "stone"),
    ])"#;

    #[test]
    fn a_reload_writes_only_what_the_file_changed() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let door = entity(&world, "d");
        // The game swings the door open and gives it a component of its own.
        world
            .insert_one(
                door,
                Transform {
                    position: Vec3::X,
                    ..Transform::default()
                },
            )
            .unwrap();
        world.insert_one(door, Health(3)).unwrap();

        // Someone recolours the wall.
        let after = scene(&DOOR.replace(r#"material: "stone""#, r#"material: "bark""#));
        let done = patch(&before, &after, &mut world);
        assert_eq!(
            (done.spawned, done.updated, done.despawned),
            (0, 1, 0),
            "{done:?}"
        );

        assert_eq!(
            world.get::<&Transform>(door).unwrap().position,
            Vec3::X,
            "the door stays where the game put it"
        );
        assert_eq!(*world.get::<&Health>(door).unwrap(), Health(3));
        let wall = entity(&world, "a1");
        assert_eq!(
            world.get::<&Surface>(wall).unwrap().0,
            crate::material::builtin::BARK,
            "and the wall has its new colour"
        );
    }

    #[test]
    fn a_field_the_file_changed_wins_over_the_game() {
        // The file is the source: editing the door's position means you want
        // it there, whatever the game had done with it.
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let door = entity(&world, "d");
        world
            .insert_one(
                door,
                Transform {
                    position: Vec3::X,
                    ..Transform::default()
                },
            )
            .unwrap();

        let after = scene(&DOOR.replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 5.0)"));
        patch(&before, &after, &mut world);
        assert_eq!(
            world.get::<&Transform>(door).unwrap().position,
            Vec3::new(0.0, 0.0, 5.0)
        );
        assert_eq!(
            world.get::<&WorldTransform>(door).unwrap().0.w_axis.z,
            5.0,
            "and its world transform follows"
        );
    }

    #[test]
    fn lines_added_and_removed_are_spawned_and_despawned() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        // The game hangs a torch on the wall, and spawns a bird of its own.
        let wall = entity(&world, "a1");
        let torch = world.spawn((Transform::default(), Parent(wall)));
        let bird = world.spawn((Transform::default(),));

        let after = scene(
            r#"(entities: [
                (id: "d", name: "door", model: "m", transform: (position: (0.0, 0.0, 0.0))),
                (id: "b2", name: "roof", model: "m"),
            ])"#,
        );
        let done = patch(&before, &after, &mut world);
        assert_eq!((done.spawned, done.despawned), (1, 2), "{done:?}");
        assert!(!world.contains(wall));
        assert!(
            !world.contains(torch),
            "what hung off the wall goes with it"
        );
        assert!(world.contains(bird), "what the game made on its own stays");
        entity(&world, "b2");
    }

    #[test]
    fn a_reparented_line_moves_under_its_new_parent() {
        let before = scene(
            r#"(entities: [(id: "a", name: "a", model: "m", transform: (position: (10.0, 0.0, 0.0))), (id: "b", name: "b", model: "m")])"#,
        );
        let mut world = spawned(&before);
        let after = scene(
            r#"(entities: [(id: "a", name: "a", model: "m", transform: (position: (10.0, 0.0, 0.0)), children: [(id: "b", name: "b", model: "m")])])"#,
        );
        let done = patch(&before, &after, &mut world);
        assert_eq!(done.updated, 1, "{done:?}");
        let (a, b) = (entity(&world, "a"), entity(&world, "b"));
        assert_eq!(world.get::<&Parent>(b).unwrap().0, a);
        assert_eq!(world.get::<&WorldTransform>(b).unwrap().0.w_axis.x, 10.0);
    }

    #[test]
    fn a_walker_with_footprints_leaves_them_in_the_frame_and_retuned_starts_afresh() {
        let text = r#"(entities: [
            (id: "e7", name: "walker", model: "m", transform: (position: (0.0, 0.9, 0.0)), footprints: (feet: 0.9)),
        ])"#;
        let before = scene(text);
        let mut world = spawned(&before);
        let walker = entity(&world, "e7");
        for i in 0..=120 {
            world.get::<&mut WorldTransform>(walker).unwrap().0 =
                glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.9, -0.05 * i as f32));
            crate::footprints::run_footprints(&mut world, 1.0 / 60.0);
        }
        let frame = build_frame(
            &world,
            Camera::default(),
            Lighting::default(),
            FogSettings::default(),
        );
        let prints: Vec<_> = frame
            .decals
            .iter()
            .filter(|d| d.shape == crate::decals::DecalShape::Footprint)
            .collect();
        assert_eq!(prints.len(), 8, "six metres at 0.75 a stride");
        assert!(
            prints.iter().all(|d| d.transform.w_axis.y.abs() < 1e-4),
            "at its feet, not its middle"
        );
        assert!(!frame.puffs.is_empty(), "and dust");
        // Retuned in the file: a fresh trail.
        let after = scene(&text.replace("feet: 0.9", "feet: 0.9, stride: 0.5"));
        patch(&before, &after, &mut world);
        let trail = world.get::<&crate::footprints::Trail>(walker).unwrap();
        assert_eq!(trail.prints(), 0);
        assert_eq!(trail.settings.stride, 0.5);
    }

    #[test]
    fn the_same_file_twice_changes_nothing() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        assert!(patch(&before, &before.clone(), &mut world).is_empty());
    }

    #[test]
    fn a_model_nothing_answers_to_is_reported_and_leaves_nothing_drawn() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let after = scene(&DOOR.replace(
            r#"name: "door", model: "m""#,
            r#"name: "door", model: "gone""#,
        ));
        let done = patch_scene(
            &before,
            &after,
            &mut world,
            |name| (name == "m").then_some(MeshHandle::TEST),
            |_| None,
        );
        assert_eq!(done.missing.len(), 1);
        assert_eq!(done.missing[0].model, "gone");
        assert!(world.get::<&Model>(entity(&world, "d")).is_err());
    }

    #[test]
    fn two_scenes_in_one_world_leave_each_other_alone() {
        let village = scene(DOOR);
        let forest = scene(r#"(entities: [(id: "f1", name: "oak", model: "m")])"#);
        let mut world = spawned(&village);
        spawn_scene(&forest, &mut world, |_| Some(MeshHandle::TEST));

        // The village reloads with the wall gone: the forest is not its.
        let smaller = scene(r#"(entities: [(id: "d", name: "door", model: "m")])"#);
        let done = patch(&village, &smaller, &mut world);
        assert_eq!(done.despawned, 1, "{done:?}");
        entity(&world, "f1");

        // And the village unloads alone.
        let done = patch(&smaller, &Scene::default(), &mut world);
        assert_eq!(done.despawned, 1);
        entity(&world, "f1");
    }

    #[test]
    fn a_camera_on_the_player_sees_from_its_eyes_and_follows_it() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (id: "00000000000000a1", name: "player", model: "builtin:cube",
                 transform: (position: (3.0, 0.0, 0.0), rotation_deg: (0.0, 90.0, 0.0)),
                 children: [(id: "00000000000000a2", name: "eyes", transform: (position: (0.0, 1.6, 0.0)),
                             camera: (fov_deg: 70.0))]),
            ])"#,
        )
        .unwrap();
        scene.assign_ids();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let camera = camera_of(&world).expect("a camera");
        assert!(
            (camera.position - Vec3::new(3.0, 1.6, 0.0)).length() < 1e-4,
            "{:?}",
            camera.position
        );
        let looking = (camera.target - camera.position).normalize();
        assert!(
            (looking - Vec3::X).length() < 1e-4,
            "where the player faces: {looking:?}"
        );
        assert_eq!(camera.fov_y_degrees, 70.0);

        let player = world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == scene.entities[0].id)
            .map(|(e, _)| e)
            .unwrap();
        world.get::<&mut Transform>(player).unwrap().position.z = 10.0;
        apply_hierarchy(&mut world);
        assert!(
            (camera_of(&world).unwrap().position.z - 10.0).abs() < 1e-4,
            "it follows"
        );

        // A second camera that asks for it wins.
        world.spawn((
            CameraLens(crate::scene::Lens {
                fov_deg: 40.0,
                priority: 1,
                ortho: Some(8.0),
                follow: None,
                blend: None,
            }),
            WorldTransform(glam::Mat4::IDENTITY),
        ));
        assert_eq!(camera_of(&world).unwrap().fov_y_degrees, 40.0);
        assert_eq!(camera_of(&world).unwrap().ortho, Some(8.0));

        // A camera following a player keeps behind it, softly, and looks.
        let mut world = World::new();
        let player_id: crate::id::EntityId = "00000000000000a7".parse().unwrap();
        let player = world.spawn((
            SceneId(player_id),
            WorldTransform(glam::Mat4::from_translation(glam::Vec3::new(
                10.0, 0.0, 0.0,
            ))),
        ));
        let lens = crate::scene::Lens {
            fov_deg: 60.0,
            priority: 0,
            ortho: None,
            follow: Some(crate::scene::Follow {
                target: player_id,
                offset: glam::Vec3::new(0.0, 3.0, -6.0),
                damping: 0.3,
                look: true,
                look_damping: 0.0,
                look_offset: glam::Vec3::ZERO,
                dead_zone: 0.0,
            }),
            blend: None,
        };
        world.spawn((
            CameraLens(lens),
            crate::scene::Transform::default(),
            WorldTransform(glam::Mat4::IDENTITY),
        ));
        follow_cameras(&mut world, 1.0 / 60.0);
        let early = camera_of(&world).unwrap().position;
        assert!(
            early.x > 0.1 && early.x < 5.0,
            "on its way, softly: {early}"
        );
        for _ in 0..120 {
            follow_cameras(&mut world, 1.0 / 60.0);
        }
        let camera = camera_of(&world).unwrap();
        assert!(
            (camera.position - glam::Vec3::new(10.0, 3.0, -6.0)).length() < 0.01,
            "{}",
            camera.position
        );
        let looking = (camera.target - camera.position).normalize();
        let at_player = (glam::Vec3::new(10.0, 0.0, 0.0) - camera.position).normalize();
        assert!(looking.dot(at_player) > 0.999, "looks at it");
        let _ = player;
        assert!(camera_of(&World::new()).is_none());
    }

    #[test]
    fn a_thing_on_a_bone_goes_where_the_pose_puts_the_bone() {
        use crate::animation::{Joint, PoseTransform, Skeleton};
        let bind = glam::Mat4::from_translation(glam::Vec3::new(1.0, 0.0, 0.0));
        let skeleton = std::sync::Arc::new(Skeleton {
            joints: vec![Joint {
                name: "hand".into(),
                parent: None,
                inverse_bind: bind.inverse().to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        });
        let animator = crate::Animator::new(skeleton, std::sync::Arc::new(Vec::new()));
        // The pose lifts the hand to (1, 2, 0) in the model.
        let hand = glam::Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 0.0));
        let mut world = World::new();
        let at = |x: f32, y: f32, z: f32| Transform {
            position: glam::Vec3::new(x, y, z),
            ..Transform::default()
        };
        let body = world.spawn((
            at(10.0, 0.0, 0.0),
            animator,
            Posed(vec![hand * bind.inverse()]),
        ));
        let spade = world.spawn((at(0.0, 0.0, 0.5), Parent(body), OnBone("hand".into())));
        let beside = world.spawn((at(0.0, 0.0, 0.5), Parent(body)));
        crate::animator::hold_on_bones(&mut world);
        apply_hierarchy(&mut world);
        let place = |e| world.get::<&WorldTransform>(e).unwrap().0.w_axis.truncate();
        assert_eq!(place(spade), glam::Vec3::new(11.0, 2.0, 0.5));
        assert_eq!(
            place(beside),
            glam::Vec3::new(10.0, 0.0, 0.5),
            "not on a bone: the body"
        );
    }

    #[test]
    fn a_post_volume_is_all_there_inside_and_fades_out_over_its_blend() {
        let dark = crate::post::PostProcess {
            exposure: -2.0,
            ..Default::default()
        };
        let mut world = World::new();
        world.spawn((
            WorldTransform(glam::Mat4::IDENTITY),
            PostVolumeBox(crate::scene::PostVolume {
                size: glam::Vec3::splat(4.0),
                blend_distance: 2.0,
                priority: 0,
                post: dark,
            }),
        ));
        let exposure_at = |x: f32| {
            let mut frame = Frame::default();
            frame.camera.position = glam::Vec3::new(x, 0.0, 0.0);
            let outside = frame.post.exposure;
            post_volumes(&mut frame, &world);
            (frame.post.exposure, outside)
        };
        assert_eq!(exposure_at(1.0).0, -2.0, "inside");
        let (half, outside) = exposure_at(3.0);
        assert!(
            (half - (outside + (-2.0 - outside) * 0.5)).abs() < 1e-4,
            "{half}"
        );
        assert_eq!(exposure_at(10.0).0, outside, "far away: the scene's");
    }

    #[test]
    fn a_thing_switched_off_hides_with_all_under_it_until_switched_on() {
        let scene: crate::scene::Scene = ron::from_str(
            r#"(entities: [
                (id: "0000000000000001", name: "shed", model: "builtin:cube", inactive: true, children: [
                    (id: "0000000000000002", name: "shelf", model: "builtin:cube"),
                ]),
                (id: "0000000000000003", name: "well", model: "builtin:cube"),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        apply_hierarchy(&mut world);
        let drawn = |world: &World| {
            build_frame(
                world,
                Camera::default(),
                Lighting::default(),
                FogSettings::default(),
            )
            .draws
            .len()
        };
        assert_eq!(drawn(&world), 1, "the well alone");
        let shed = crate::net::addressable(&world)[&crate::id::EntityId::from_raw(1)];
        let shelf = crate::net::addressable(&world)[&crate::id::EntityId::from_raw(2)];
        assert!(!is_active(&world, shelf), "off by its parent");
        set_active(&mut world, shed, true);
        assert_eq!(drawn(&world), 3);
        assert!(is_active(&world, shelf));
    }
}