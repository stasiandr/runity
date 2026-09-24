//! The scene file's tests: the core's lines with the modules' fields on
//! them, so they live beside the modules, not in the core.

#[cfg(test)]
mod tests {
    use crate::scene::*;
    use crate::material::Material;
    use glam::Vec3;

    #[test]
    fn a_scene_with_every_field_set_survives_a_round_trip() {
        // The round trip that matters is the one an editor does: write a
        // scene out, open it again, get the same thing. It failed for a
        // while because struct names were on and an untagged enum cannot
        // match a named struct — a file that saved cleanly and would not
        // reopen.
        let mut scene = Scene {
            entities: vec![EntityDesc {
                parts: Default::default(),
                in_part: None,
                inactive: false,
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "crate".into(),
                prefab: Default::default(),
                transform: Transform {
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(2.0),
                },
                children: vec![EntityDesc {
                    parts: Default::default(),
                    in_part: None,
                    inactive: false,
                    overrides: Default::default(),
                    components: Default::default(),
                    id: Default::default(),
                    name: "lid".into(),
                    prefab: Default::default(),
                    transform: Transform::default(),
                    children: Vec::new(),
                }
                .with(crate::scene::ModelRef("builtin:cube".into()))
                .with(MaterialRef::Named("stone".into()))
                .with(Body::None)
                .with(Collider::None)],
            }
            .with(crate::scene::ModelRef("builtin:cube".into()))
            .with(MaterialRef::Inline(Material::new(0.3, 0.2, 0.1)))
            .with(Body::Dynamic)
            .with(Collider::Box {
                half: Vec3::splat(0.5),
                center: glam::Vec3::ZERO,
            })],
            ..Default::default()
        };
        // IDs as a loaded scene would have them: code-built entities start
        // unassigned, and a round trip would otherwise compare minted IDs
        // against zeros.
        scene.assign_ids();
        let dir = std::env::temp_dir().join("scrap-scene-full");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn a_scene_survives_a_round_trip_through_the_file() {
        let mut scene = Scene {
            parts: Default::default(),
            entities: vec![EntityDesc {
                parts: Default::default(),
                in_part: None,
                inactive: false,
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "pine".into(),
                prefab: Default::default(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, -3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(1.2),
                },
                children: Vec::new(),
            }
            .with(crate::scene::ModelRef("models/pine_large.obj".into()))
            .with(MaterialRef::Named("needle".into()))
            .with(Body::Static)
            .with(Collider::None)],
        }
        .with(View::default())
        .with(Sun {
            hour: 17.5,
            intensity: 0.8,
            ..Sun::default()
        })
        .with(Fog::default())
        .with(crate::post::PostProcess {
            saturation: -30.0,
            ..Default::default()
        });
        scene.assign_ids();
        let dir = std::env::temp_dir().join("scrap-scene-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn saving_a_scene_twice_writes_the_same_bytes() {
        // DNA, postulate 2: a save with no changes is a zero diff. IDs,
        // field order and number formatting all have to be deterministic for
        // that, and any one of them drifting shows up as noise in every
        // commit an editor makes.
        let dir = std::env::temp_dir().join("scrap-scene-zero-diff");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        std::fs::write(
            &path,
            r#"(entities: [
                (name: "crate", model: "builtin:cube",
                 transform: (position: (0.1, 0.2, 0.3), rotation_deg: (0.0, 33.3, 0.0)),
                 children: [(name: "lid", model: "builtin:cube")]),
                (name: "rock", model: "builtin:sphere", material: "stone"),
            ])"#,
        )
        .unwrap();

        let first = Scene::load(&path).unwrap();
        first.save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        Scene::load(&path).unwrap().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    }

    #[test]
    fn an_entity_written_without_an_id_gets_one_and_keeps_it() {
        // A person or an agent writing a scene by hand should not have to
        // invent IDs. The first load gives them, the first save writes them,
        // and from then on they are the entity's.
        let dir = std::env::temp_dir().join("scrap-scene-ids");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        std::fs::write(
            &path,
            r#"(entities: [(name: "a", model: "m", children: [(name: "b", model: "m")])])"#,
        )
        .unwrap();

        let scene = Scene::load(&path).unwrap();
        let ids = scene.ids();
        assert!(ids.iter().all(|id| !id.is_unassigned()), "{ids:?}");
        scene.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("id: \""));
        assert_eq!(Scene::load(&path).unwrap().ids(), ids, "and keeps them");
    }

    #[test]
    fn an_entity_without_an_id_is_named_the_same_way_on_every_read() {
        // A running game reloads a hand-written scene every time it is
        // saved. If each read minted new IDs, every entity without one would
        // look new each time and be respawned, losing whatever the game had
        // done to it.
        let text = |extra: &str| {
            format!(
                r#"(entities: [{extra}
                    (name: "tree", model: "m"),
                    (name: "tree", model: "m", children: [(name: "leaf", model: "m")]),
                ])"#
            )
        };
        let read = |text: &str| {
            let mut scene: Scene = ron::from_str(text).unwrap();
            scene.assign_ids();
            scene
        };
        let first = read(&text(""));
        assert_eq!(
            first.ids(),
            read(&text("")).ids(),
            "the same file, the same IDs"
        );
        let ids = first.ids();
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "two trees with one name are still two entities: {ids:?}"
        );

        // A sibling written above them does not move their identities.
        let inserted = read(&text(r#"(name: "rock", model: "m"),"#));
        assert_eq!(inserted.ids()[1..], ids[..]);
    }

    #[test]
    fn a_repeated_id_is_re_minted_and_the_first_keeps_it() {
        // A block copy-pasted by hand, or a merge gone strange. Two entities
        // answering to one ID is how an edit lands on the wrong one.
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (id: "a1", name: "first", model: "m"),
                (id: "a1", name: "pasted", model: "m"),
            ])"#,
        )
        .unwrap();
        assert_eq!(scene.assign_ids(), 1);
        assert_eq!(scene.find("first").unwrap().id, "a1".parse().unwrap());
        assert_ne!(
            scene.find("pasted").unwrap().id,
            scene.find("first").unwrap().id
        );
    }

    #[test]
    fn an_entity_is_found_by_id_at_any_depth() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [(id: "1", name: "a", model: "m", children: [(id: "2", name: "b", model: "m")])])"#,
        )
        .unwrap();
        let deep = "2".parse().unwrap();
        assert_eq!(scene.get(deep).map(|e| e.name.as_str()), Some("b"));
        scene.get_mut(deep).unwrap().name = "renamed".into();
        assert_eq!(scene.find("renamed").unwrap().id, deep);
        assert!(scene.get(crate::EntityId::from_raw(3)).is_none());
    }

    #[test]
    fn euler_degrees_survive_the_trip_through_a_quaternion() {
        let t = Transform {
            rotation_deg: glam::Vec3::new(10.0, -35.0, 0.0),
            ..Transform::default()
        };
        let q = t.rotation();
        let mut back = Transform::default();
        back.set_rotation(q);
        assert!((back.rotation_deg - t.rotation_deg).length() < 0.01);
    }

    #[test]
    fn flatten_reaches_every_depth_and_stacks_the_transforms() {
        let scene: Scene = ron::from_str(
            r#"(entities: [(
                name: "cart", model: "m", transform: (position: (10.0, 0.0, 0.0)),
                children: [(
                    name: "wheel", model: "m", transform: (position: (1.0, 0.0, 0.0)),
                    children: [(name: "bolt", model: "m", transform: (position: (0.5, 0.0, 0.0)))],
                )],
            )])"#,
        )
        .unwrap();

        assert_eq!(scene.entities.len(), 1, "one root");
        let all = scene.flatten();
        assert_eq!(all.len(), 3, "three entities once the nesting is walked");
        let bolt = scene.find("bolt").expect("found at depth two");
        assert_eq!(bolt.transform.position.x, 0.5, "its own transform is local");
        let bolt_world = all
            .iter()
            .find(|(e, _)| e.name == "bolt")
            .map(|(_, m)| m.w_axis.x)
            .unwrap();
        assert_eq!(bolt_world, 11.5, "10 + 1 + 0.5");
    }

    #[test]
    fn a_material_is_named_or_spelled_out_and_a_typo_still_opens() {
        let named: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grass")])"#).unwrap();
        assert_eq!(
            named.entities[0].material(),
            crate::material::builtin::GRASS
        );

        let inline: Scene = ron::from_str(
            r#"(entities: [(name: "a", model: "m", material: (base_color: (0.5, 0.1, 0.1)))])"#,
        )
        .unwrap();
        assert_eq!(inline.entities[0].material().base_color, [0.5, 0.1, 0.1]);

        let typo: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grsas")])"#).unwrap();
        assert_eq!(typo.entities[0].material(), Material::default());
    }

    #[test]
    fn the_sun_rises_crosses_and_sets_and_warms_at_both_ends() {
        let at = |hour| Sun {
            hour,
            intensity: 1.0,
            ..Sun::default()
        };

        // Noon is overhead; morning and evening are low and on opposite
        // sides, which is what makes shadows point somewhere believable.
        let noon = at(12.0).direction();
        let morning = at(7.0).direction();
        let evening = at(17.0).direction();
        assert!(noon.y < morning.y && noon.y < evening.y, "noon is highest");
        assert!(
            morning.x.signum() != evening.x.signum(),
            "the sun should cross the sky, not wander back: {morning} then {evening}"
        );

        // Never exactly on the horizon: a sun lying flat lights nothing but
        // the horizon and turns every shadow into a stripe to the far plane.
        for hour in [0.0, 5.0, 6.0, 18.0, 23.0] {
            assert!(at(hour).direction().y < -0.1, "at {hour}");
        }

        // Warm low, white high. Not physics — the one cue that reads as a
        // time of day at a glance.
        assert!(at(12.0).color().z > at(7.0).color().z, "noon is the bluest");
        assert!(at(7.0).color().x >= at(7.0).color().z, "dawn is orange");
    }

    #[test]
    fn a_scene_remembers_where_it_is_looked_at_from() {
        let scene = Scene { ..Scene::default() }.with(View {
            position: Vec3::new(3.0, 9.0, -2.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            fov_deg: 35.0,
        });
        let dir = std::env::temp_dir().join("scrap-scene-view");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("view.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap().view(), scene.view());

        // And a scene written before there was a camera in the format still
        // opens, framed the way everything used to be.
        let older: Scene = ron::from_str(r#"(entities: [])"#).unwrap();
        assert_eq!(older.view(), View::default());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing_to_load() {
        // An agent writing a scene by hand should not have to spell out
        // every default, and an older file should still open.
        let text = r#"(entities: [(name: "rock", model: "models/boulder.obj")])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        assert_eq!(scene.entities[0].transform, Transform::default());
        assert_eq!(scene.entities[0].body(), Body::None);
        assert_eq!(scene.entities[0].material(), Material::default());
        assert_eq!(scene.sun().hour, 9.0);
    }
}

// ---------------------------------------------------------------------------
// A line of a scene, a prefab instance's override and a whole scene, read
// and written by hand: the core's fields by type, every other field as a
// part (crate::parts), in the order the file has them.

