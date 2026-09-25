//! The core's `prefab`, tested with the modules' fields on its lines.

    use crate::prefab::*;
    #[allow(unused_imports)]
    use std::collections::{HashMap, HashSet};
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::scene::{EntityDesc, Scene};
    #[allow(unused_imports)]
    use crate::prelude::*;
    use crate::scene::Transform;
    use crate::scene::{Body, MaterialRef};
    use glam::Vec3;

    /// A scene as `Scene::load` would hand it over: parsed, with IDs.
    fn parse(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn campfire() -> EntityDesc {
        let mut desc: EntityDesc = ron::from_str(
            r#"(
                name: "campfire",
                model: "builtin:plane",
                material: "earth",
                children: [
                    (name: "ember", model: "builtin:sphere", material: "ember",
                     transform: (position: (0.0, 0.2, 0.0))),
                    (name: "stone", model: "builtin:cube", material: "stone",
                     transform: (position: (0.8, 0.0, 0.0))),
                ],
            )"#,
        )
        .unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut desc), &mut HashSet::new());
        desc
    }

    fn with_campfire() -> Prefabs {
        let mut prefabs = Prefabs::new();
        prefabs.insert("campfire", campfire());
        prefabs
    }

    fn scene_with_two_fires() -> Scene {
        parse(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "north fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, -10.0))),
                (name: "south fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, 10.0))),
            ])"#,
        )
    }

    /// Which document entity owns each expanded one, by name, in tree order.
    fn owners(document: &Scene, done: &Instanced) -> Vec<String> {
        done.scene
            .flatten()
            .iter()
            .map(|(e, _)| {
                let owner = done.owner_of(e.id).expect("every entity has an owner");
                document.get(owner).unwrap().name.clone()
            })
            .collect()
    }

    #[test]
    fn an_instance_becomes_the_whole_thing_it_names() {
        let done = instantiate(&scene_with_two_fires(), &with_campfire());
        assert!(done.problems.is_empty(), "{:?}", done.problems);

        let flat = done.scene.flatten();
        assert_eq!(flat.len(), 7, "ground plus two fires of three each");
        let names: Vec<&str> = flat.iter().map(|(e, _)| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ground",
                "north fire",
                "ember",
                "stone",
                "south fire",
                "ember",
                "stone"
            ]
        );

        // Placed by the instance, not by the prefab: the ember of the north
        // fire is ten metres away from the south one's.
        let embers: Vec<f32> = flat
            .iter()
            .filter(|(e, _)| e.name == "ember")
            .map(|(_, world)| world.w_axis.z)
            .collect();
        assert_eq!(embers, vec![-10.0, 10.0]);

        // And the prefab's own look came along.
        let ember = done.scene.find("ember").unwrap();
        assert_eq!(ember.material(), crate::material::builtin::EMBER);
    }

    #[test]
    fn everything_a_prefab_brought_belongs_to_the_instance_that_brought_it() {
        // The editor has to be able to answer "what did I just click on"
        // with something the document can move. A stone that came out of a
        // prefab has no entry of its own, so the answer is the fire.
        let scene = scene_with_two_fires();
        let done = instantiate(&scene, &with_campfire());
        assert_eq!(
            owners(&scene, &done),
            [
                "ground",
                "north fire",
                "north fire",
                "north fire",
                "south fire",
                "south fire",
                "south fire"
            ]
        );
    }

    #[test]
    fn the_same_stone_in_two_fires_is_two_entities_and_keeps_being_them() {
        // Each part of an instance has an identity of its own — what an
        // override or a network message will point at — and it does not
        // change between two expansions of the same scene.
        let scene = scene_with_two_fires();
        let prefabs = with_campfire();
        let stones = |done: &Instanced| -> Vec<EntityId> {
            done.scene
                .flatten()
                .iter()
                .filter(|(e, _)| e.name == "stone")
                .map(|(e, _)| e.id)
                .collect()
        };
        let first = stones(&instantiate(&scene, &prefabs));
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1], "two stones, two identities");
        assert_eq!(first, stones(&instantiate(&scene, &prefabs)), "and stable");

        // The instance itself keeps the document's ID: it *is* that line.
        let north = scene.find("north fire").unwrap().id;
        let done = instantiate(&scene, &prefabs);
        assert_eq!(done.scene.find("north fire").unwrap().id, north);
    }

    #[test]
    fn an_instance_places_and_recolours_without_touching_the_prefab() {
        let scene = parse(
            r#"(entities: [(
                name: "cold fire", model: "", prefab: "campfire",
                material: "stone", body: Static,
                transform: (position: (3.0, 0.0, 0.0), scale: (2.0, 2.0, 2.0)),
            )])"#,
        );
        let prefabs = with_campfire();
        let done = instantiate(&scene, &prefabs);
        let root = &done.scene.entities[0];

        assert_eq!(root.model(), "builtin:plane", "the prefab says what it is");
        assert_eq!(root.material(), crate::material::builtin::STONE);
        assert_eq!(root.body(), Body::Static);
        assert_eq!(root.transform.scale, Vec3::splat(2.0));
        assert_eq!(root.children.len(), 2, "and it still brought its children");

        // The prefab itself is untouched, so the next instance is not
        // wearing the last one's overrides.
        assert_eq!(prefabs.get("campfire").unwrap().material(), {
            crate::material::builtin::EARTH
        });
    }

    #[test]
    fn an_instance_may_have_children_of_its_own() {
        // Those do have entries in the document, so they stay selectable —
        // which is the difference between "part of the prefab" and "put
        // there beside it".
        let scene = parse(
            r#"(entities: [(
                name: "fire", model: "", prefab: "campfire",
                children: [(name: "kettle", model: "builtin:sphere")],
            )])"#,
        );
        let done = instantiate(&scene, &with_campfire());
        let names: Vec<&str> = done
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["fire", "ember", "stone", "kettle"]);
        assert_eq!(
            owners(&scene, &done),
            ["fire", "fire", "fire", "kettle"],
            "the kettle is its own entry; the prefab's parts are the fire's"
        );
    }

    #[test]
    fn a_prefab_can_be_built_out_of_prefabs() {
        let mut prefabs = with_campfire();
        prefabs.insert(
            "camp",
            ron::from_str(
                r#"(name: "camp", model: "builtin:plane", children: [
                    (name: "fire", model: "", prefab: "campfire"),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(r#"(entities: [(name: "camp", model: "", prefab: "camp")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.scene.flatten().len(), 4, "camp, fire, ember, stone");
        assert_eq!(
            owners(&scene, &done),
            ["camp"; 4],
            "all of it belongs to the one instance in the document"
        );
        let ids = done.scene.ids();
        assert_eq!(
            ids.iter().collect::<HashSet<_>>().len(),
            ids.len(),
            "and nesting did not give two parts one identity"
        );
    }

    #[test]
    fn a_link_to_a_part_holds_in_a_prefab_inside_a_prefab() {
        // A door whose component names its hinge, the door in a shed, the
        // shed in a scene and a shed's lamp naming the door: each link is
        // to a part that is there, whatever the depth.
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "door",
            ron::from_str(
                r#"(id: "00000000000000d0", name: "door", model: "", components: {
                    "door": (hinge: EntityRef("00000000000000d1")),
                }, children: [(id: "00000000000000d1", name: "hinge", model: "", components: {
                    "hinge": (door: EntityRef("00000000000000d0")),
                })])"#,
            )
            .unwrap(),
        );
        prefabs.insert(
            "shed",
            ron::from_str(
                r#"(id: "00000000000000e0", name: "shed", model: "", children: [
                    (id: "00000000000000e1", name: "door", model: "", prefab: "door"),
                    (id: "00000000000000e2", name: "lamp", model: "", components: {
                        "lamp": (watches: EntityRef("00000000000000e1")),
                    }),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [(id: "00000000000000f0", name: "shed", model: "", prefab: "shed")])"#,
        );
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let ids: HashSet<String> = done.scene.ids().iter().map(|i| i.to_string()).collect();
        let mut links = Vec::new();
        for (desc, _) in done.scene.flatten() {
            for value in desc.components.values() {
                links.extend(crate::EntityRef::find_in(value.get_ron()));
            }
        }
        assert_eq!(
            links.len(),
            3,
            "the hinge names its door, the prefab's root, too"
        );
        for link in links {
            assert!(ids.contains(&link.to_string()), "{link} is a part: {ids:?}");
        }
    }

    #[test]
    fn a_link_to_a_part_of_a_prefab_inside_a_prefab_names_it_by_its_key() {
        // A shed's lamp watches its door's hinge, and a scene's bell the
        // same hinge in the shed it placed: each names the hinge by the key
        // an override would — the door's instance within the hinge's own,
        // and in the scene that within the shed's — as Unity's importer
        // writes them (a vending machine's screen's labels). Expanded, each
        // is the hinge.
        let d1: EntityId = "00000000000000d1".parse().unwrap();
        let e1: EntityId = "00000000000000e1".parse().unwrap();
        let f0: EntityId = "00000000000000f0".parse().unwrap();
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "door",
            ron::from_str(
                r#"(id: "00000000000000d0", name: "door", model: "", children: [
                    (id: "00000000000000d1", name: "hinge", model: ""),
                ])"#,
            )
            .unwrap(),
        );
        prefabs.insert(
            "shed",
            ron::from_str(&format!(
                r#"(id: "00000000000000e0", name: "shed", model: "", children: [
                    (id: "00000000000000e1", name: "door", model: "", prefab: "door"),
                    (id: "00000000000000e2", name: "lamp", model: "", components: {{
                        "lamp": (watches: EntityRef("{}")),
                    }}),
                ])"#,
                e1.within(d1)
            ))
            .unwrap(),
        );
        let scene = parse(&format!(
            r#"(entities: [
                (id: "00000000000000f0", name: "shed", model: "", prefab: "shed"),
                (id: "00000000000000f1", name: "bell", model: "", components: {{
                    "bell": (rings: EntityRef("{}")),
                }}),
            ])"#,
            f0.within(e1.within(d1))
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert!(done.scene.broken_links().is_empty(), "{:?}", done.scene.broken_links().len());
        let hinge = done
            .scene
            .flatten()
            .into_iter()
            .find(|(d, _)| d.name == "hinge")
            .map(|(d, _)| d.id)
            .unwrap();
        for (desc, _) in done.scene.flatten() {
            for value in desc.components.values() {
                for link in crate::EntityRef::find_in(value.get_ron()) {
                    assert_eq!(link, hinge, "{} links the hinge", desc.name);
                }
            }
        }
    }

    #[test]
    fn an_override_of_the_prefabs_root_changes_the_instance_itself() {
        let prefabs = with_campfire();
        let root = prefabs
            .find(&crate::AssetLink::named("campfire"))
            .unwrap()
            .1
            .id;
        let scene = parse(&format!(
            r#"(entities: [(id: "00000000000000a1", name: "fire", model: "", prefab: "campfire",
                overrides: {{ "{root}": (inactive: true) }})])"#
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let fire = done.scene.get("a1".parse().unwrap()).unwrap();
        assert!(fire.inactive, "switched off in this instance");
        assert_eq!(fire.name, "fire");
    }

    #[test]
    fn a_link_an_override_gives_a_part_in_a_prefab_is_the_instances() {
        let mut prefabs = with_campfire();
        let ember = prefabs
            .find(&crate::AssetLink::named("campfire"))
            .unwrap()
            .1
            .children[0]
            .id;
        prefabs.insert(
            "yard",
            ron::from_str(&format!(
                r#"(id: "00000000000000b0", name: "yard", model: "", children: [
                    (id: "00000000000000b1", name: "fire", model: "", prefab: "campfire",
                     overrides: {{ "{ember}": (components: {{"warms": (who: EntityRef("00000000000000b2"))}}) }}),
                    (id: "00000000000000b2", name: "bench", model: "builtin:cube"),
                ])"#
            ))
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [(id: "00000000000000a1", name: "home", model: "", prefab: "yard")])"#,
        );
        let done = instantiate(&scene, &prefabs);
        let bench = done
            .scene
            .flatten()
            .into_iter()
            .find(|(e, _)| e.name == "bench")
            .unwrap()
            .0
            .id;
        let warms = done
            .scene
            .flatten()
            .into_iter()
            .find(|(e, _)| e.name == "ember")
            .unwrap()
            .0
            .components["warms"]
            .get_ron()
            .to_string();
        assert!(
            warms.contains(&bench.to_string()),
            "{warms} names this yard's bench {bench}"
        );
    }

    #[test]
    fn a_scene_hangs_a_thing_on_a_part_of_a_placed_prefab() {
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "statue",
            ron::from_str(
                r#"(id: "00000000000000e0", name: "statue", model: "builtin:cube", children: [
                    (id: "00000000000000e1", name: "arm", model: "builtin:cube", children: [
                        (id: "00000000000000e2", name: "hand", model: "builtin:cube"),
                    ]),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [(id: "00000000000000a1", name: "statue", model: "", prefab: "statue", children: [
                (id: "00000000000000a2", name: "torch", model: "builtin:cube", in_part: "00000000000000e2"),
                (id: "00000000000000a3", name: "plaque", model: "builtin:cube"),
                (id: "00000000000000a4", name: "lost", model: "builtin:cube", in_part: "00000000000000ff"),
            ])])"#,
        );
        let done = instantiate(&scene, &prefabs);
        let path_of = |name: &str| -> Vec<String> {
            fn walk(e: &[EntityDesc], name: &str, trail: &mut Vec<String>) -> bool {
                for x in e {
                    trail.push(x.name.clone());
                    if x.name == name || walk(&x.children, name, trail) {
                        return true;
                    }
                    trail.pop();
                }
                false
            }
            let mut trail = Vec::new();
            walk(&done.scene.entities, name, &mut trail);
            trail
        };
        assert_eq!(path_of("torch"), ["statue", "arm", "hand", "torch"]);
        assert_eq!(path_of("plaque"), ["statue", "plaque"]);
        assert_eq!(
            path_of("lost"),
            ["statue", "lost"],
            "under the instance, and said"
        );
        assert!(
            done.problems.iter().any(|p| p.entity_name == "lost"),
            "{:?}",
            done.problems
        );
    }

    #[test]
    fn an_instance_takes_away_what_a_part_has() {
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "lamp",
            ron::from_str(
                r#"(id: "00000000000000d0", name: "lamp", model: "builtin:cube", children: [
                    (id: "00000000000000d1", name: "bulb", model: "builtin:sphere",
                     light: (color: (1.0, 1.0, 1.0), intensity: 1.0, range: 5.0),
                     components: {"flicker": (rate: 2.0)}),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [(id: "00000000000000a1", name: "dark lamp", model: "", prefab: "lamp",
                overrides: { "00000000000000d1": (removed: ["light", "flicker", "model"]) })])"#,
        );
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let bulb = done
            .scene
            .flatten()
            .into_iter()
            .find(|(e, _)| e.name == "bulb")
            .unwrap()
            .0
            .clone();
        assert!(bulb.light().is_none() && bulb.components.is_empty() && bulb.model().is_empty());
        // And an edit that takes the light away says so.
        let prefab = prefabs
            .find(&crate::AssetLink::named("lamp"))
            .unwrap()
            .1
            .children[0]
            .clone();
        let dark = prefab.clone().without("light");
        assert_eq!(
            crate::scene::Override::between(&prefab, &dark).removed,
            ["light"]
        );
    }

    #[test]
    fn an_override_reaches_a_part_of_a_prefab_inside_the_prefab() {
        // Unity names a part of a nested prefab by one id in the outer
        // prefab's file; here it is the nested instance's id within the
        // part's own — what the outer prefab alone would call it.
        let mut prefabs = with_campfire();
        let ember = prefabs
            .find(&crate::AssetLink::named("campfire"))
            .unwrap()
            .1
            .children[0]
            .id;
        prefabs.insert(
            "yard",
            ron::from_str(
                r#"(id: "00000000000000b0", name: "yard", model: "", children: [
                    (id: "00000000000000b1", name: "fire", model: "", prefab: "campfire"),
                ])"#,
            )
            .unwrap(),
        );
        let key = "b1".parse::<EntityId>().unwrap().within(ember);
        let scene = parse(&format!(
            r#"(entities: [(id: "00000000000000a1", name: "home", model: "", prefab: "yard",
                overrides: {{ "{key}": (inactive: true) }})])"#
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let off: Vec<_> = done
            .scene
            .flatten()
            .into_iter()
            .filter(|(e, _)| e.inactive)
            .map(|(e, _)| e.name.clone())
            .collect();
        assert_eq!(off, ["ember"]);
        // Two deep: the yard in a street, the fire in the yard.
        prefabs.insert(
            "street",
            ron::from_str(
                r#"(id: "00000000000000c0", name: "street", model: "", children: [
                    (id: "00000000000000c1", name: "yard", model: "", prefab: "yard"),
                ])"#,
            )
            .unwrap(),
        );
        let key = "c1".parse::<EntityId>().unwrap().within(key);
        let scene = parse(&format!(
            r#"(entities: [(id: "00000000000000a2", name: "town", model: "", prefab: "street",
                overrides: {{ "{key}": (inactive: true) }})])"#
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert!(done
            .scene
            .flatten()
            .iter()
            .any(|(e, _)| e.inactive && e.name == "ember"));
    }

    #[test]
    fn a_prefab_that_contains_itself_stops_and_says_so() {
        // A file describing an infinite scene. Stopping with a message beats
        // filling memory, and beats silently drawing one level of it.
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "snake",
            ron::from_str(
                r#"(name: "snake", model: "builtin:cube", children: [
                    (name: "tail", model: "", prefab: "snake"),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(r#"(entities: [(name: "s", model: "", prefab: "snake")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(!done.problems.is_empty(), "it should complain");
        assert!(done.problems[0].reason.contains("itself"));
        assert!(done.scene.flatten().len() < 64, "and it stopped");
        assert_eq!(owners(&scene, &done), vec!["s"; done.scene.flatten().len()]);
    }

    #[test]
    fn a_missing_prefab_leaves_a_hole_rather_than_refusing_the_scene() {
        let scene = parse(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "ghost", model: "", prefab: "not_here"),
            ])"#,
        );
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.problems.len(), 1);
        assert_eq!(done.problems[0].entity_name, "ghost");
        assert_eq!(done.scene.flatten().len(), 2, "the scene still opens");
        assert_eq!(owners(&scene, &done), ["ground", "ghost"]);
    }

    #[test]
    fn a_prefab_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join("scrap-prefab-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("campfire.{EXTENSION}"));
        let fire = campfire();
        Prefabs::save(&fire, &path).unwrap();

        let (prefabs, problems) = Prefabs::open(&dir).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(prefabs.names(), vec!["campfire"]);
        assert_eq!(prefabs.get("campfire"), Some(&fire), "IDs and all");
    }

    #[test]
    fn a_prefab_that_does_not_parse_costs_one_prefab_and_not_the_directory() {
        let dir = std::env::temp_dir().join("scrap-prefab-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Prefabs::save(&campfire(), dir.join(format!("good.{EXTENSION}"))).unwrap();
        std::fs::write(dir.join(format!("bad.{EXTENSION}")), "(name: ").unwrap();

        let (prefabs, problems) = Prefabs::open(&dir).unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(prefabs.names(), vec!["good"]);
    }

    #[test]
    fn a_scene_with_no_prefabs_is_the_same_scene() {
        let mut scene = Scene {
            entities: vec![EntityDesc {
                name: "rock".into(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, 0.0),
                    ..Transform::default()
                },
                ..EntityDesc::default()
            }
            .with(crate::scene::ModelRef("builtin:sphere".into()))],
            ..Scene::default()
        };
        scene.assign_ids();
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.scene, scene);
        assert_eq!(owners(&scene, &done), ["rock"]);
    }

    #[test]
    fn an_override_changes_one_part_of_one_instance_and_the_prefab_still_reaches_the_rest() {
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "fire",
            ron::from_str(
                r#"(id: "c1", name: "fire", model: "m", children: [
                    (id: "c2", name: "ember", model: "m", material: "ember"),
                    (id: "c3", name: "stone", model: "m"),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [
                (id: "a1", name: "one", prefab: "fire",
                 overrides: { "00000000000000c2": (material: "moss", name: "cold ember") }),
                (id: "a2", name: "two", prefab: "fire",
                 overrides: { "00000000000000ff": (name: "ghost") }),
            ])"#,
        );
        let out = instantiate(&scene, &prefabs);
        let one: EntityId = "a1".parse().unwrap();
        let ember = out.scene.get(one.within("c2".parse().unwrap())).unwrap();
        assert_eq!(ember.name, "cold ember");
        assert_eq!(ember.material_ref(), MaterialRef::Named("moss".into()));
        assert_eq!(
            ember.model(),
            "m",
            "what it did not say comes from the prefab"
        );
        let two: EntityId = "a2".parse().unwrap();
        assert_eq!(
            out.scene
                .get(two.within("c2".parse().unwrap()))
                .unwrap()
                .material_ref(),
            MaterialRef::Named("ember".into()),
            "the other instance is the prefab's"
        );
        assert_eq!(out.problems.len(), 1, "{:?}", out.problems);
        assert!(out.problems[0].reason.contains("does not have"));
        assert_eq!(
            out.parts.get(&one.within("c3".parse().unwrap())),
            Some(&(one, "c3".parse().unwrap())),
            "and every part knows where it came from"
        );
    }

    /// A campfire with mossy stones, a kettle, and no fire of its own
    /// making: the variant's file is a scene line. With the base stone's id.
    fn with_variant() -> (Prefabs, EntityId) {
        let base = campfire();
        let stone = base.children[1].id;
        let mut prefabs = Prefabs::new();
        prefabs.insert("campfire", base);
        let text = format!(
            r#"(
                name: "camp kitchen",
                prefab: "campfire",
                overrides: {{ "{stone}": (material: "moss") }},
                children: [(name: "kettle", model: "builtin:sphere", transform: (position: (0.0, 0.5, 0.0)))],
            )"#
        );
        let mut variant: EntityDesc = ron::from_str(&text).unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut variant), &mut HashSet::new());
        prefabs.insert("kitchen", variant);
        (prefabs, stone)
    }

    #[test]
    fn a_variant_is_its_base_with_its_own_changes() {
        let (prefabs, _) = with_variant();
        let scene = parse(
            r#"(entities: [(name: "west", model: "", prefab: "kitchen",
                transform: (position: (5.0, 0.0, 0.0)))])"#,
        );
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let names: Vec<&str> = done
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["west", "ember", "stone", "kettle"]);
        assert_eq!(
            done.scene.find("stone").unwrap().material_ref(),
            MaterialRef::Named("moss".into()),
            "the variant's override"
        );
        assert_eq!(
            done.scene.find("ember").unwrap().material_ref(),
            MaterialRef::Named("ember".into()),
            "the base, where the variant said nothing"
        );
        let west = scene.entities[0].id;
        assert!(done
            .scene
            .flatten()
            .iter()
            .all(|(e, _)| done.owner_of(e.id) == Some(west)));
    }

    #[test]
    fn a_scene_overrides_a_variant_by_the_base_part_ids() {
        let (prefabs, stone) = with_variant();
        let scene = parse(&format!(
            r#"(entities: [(name: "west", model: "", prefab: "kitchen",
                overrides: {{ "{stone}": (material: "bark") }})])"#
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(
            done.scene.find("stone").unwrap().material_ref(),
            MaterialRef::Named("bark".into()),
            "the instance beats the variant, as the variant beats the base"
        );
        let id = done.scene.find("stone").unwrap().id;
        assert_eq!(
            done.parts.get(&id),
            Some(&(scene.entities[0].id, stone)),
            "addressed as a part of the instance, by its id in the base"
        );
    }

    #[test]
    fn a_variant_of_a_variant_and_a_variant_of_itself() {
        let (mut prefabs, _) = with_variant();
        let mut grand: EntityDesc =
            ron::from_str(r#"(name: "big kitchen", prefab: "kitchen", material: "stone")"#)
                .unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut grand), &mut HashSet::new());
        prefabs.insert("big kitchen", grand);
        let scene = parse(r#"(entities: [(name: "camp", model: "", prefab: "big kitchen")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.scene.flatten().len(), 4);
        assert_eq!(
            done.scene.find("stone").unwrap().material_ref(),
            MaterialRef::Named("moss".into())
        );

        let mut prefabs = Prefabs::new();
        let mut ouroboros: EntityDesc = ron::from_str(r#"(name: "loop", prefab: "loop")"#).unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut ouroboros), &mut HashSet::new());
        prefabs.insert("loop", ouroboros);
        let scene = parse(r#"(entities: [(name: "x", model: "", prefab: "loop")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(
            done.problems[0]
                .reason
                .contains("a prefab containing itself?"),
            "{:?}",
            done.problems
        );
    }

    #[test]
    fn a_joint_inside_a_prefab_holds_the_parts_of_its_own_instance() {
        let mut prefabs = Prefabs::new();
        let lamp: EntityDesc = ron::from_str(
            r#"(id: "00000000000000c1", name: "lamp post", model: "builtin:cube", body: Static,
                children: [(id: "00000000000000c2", name: "lamp", model: "builtin:sphere", body: Dynamic,
                    joint: Ball(to: "00000000000000c1", anchor: (0.0, 0.5, 0.0)))])"#,
        )
        .unwrap();
        prefabs.insert("lamp", lamp);
        let scene = parse(
            r#"(entities: [
                (id: "00000000000000a1", name: "west", prefab: "lamp"),
                (id: "00000000000000a2", name: "east", prefab: "lamp"),
            ])"#,
        );
        let done = instantiate(&scene, &prefabs);
        let (west, east): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
        let part: EntityId = "c2".parse().unwrap();
        let joint_of = |id: EntityId| done.scene.get(id).unwrap().joint().to().unwrap();
        // The lamp hangs from its own post — the instance's root, which
        // keeps the instance's id — not from the file's.
        assert_eq!(joint_of(west.within(part)), west);
        assert_eq!(joint_of(east.within(part)), east);
        assert!(
            done.scene.get(west).is_some(),
            "and that is an entity there"
        );
    }
