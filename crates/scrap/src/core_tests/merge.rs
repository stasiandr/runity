//! The core's `merge`, tested with the modules' fields on its lines.

    use crate::merge::*;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashMap, HashSet};
    #[allow(unused_imports)]
    use std::fmt;
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::scene::{EntityDesc, Scene};
    #[allow(unused_imports)]
    use crate::prelude::*;

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    const BASE: &str = r#"(entities: [
        (id: "a1", name: "tree", model: "m", transform: (position: (0.0, 0.0, 0.0))),
        (id: "b2", name: "rock", model: "m", material: "stone"),
        (id: "c3", name: "hut", model: "m", children: [(id: "d4", name: "door", model: "m")]),
    ])"#;

    #[test]
    fn different_entities_changed_on_each_side_merge_cleanly() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)"));
        let theirs = scene(&BASE.replace("\"stone\"", "\"moss\""));
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let tree = merged.scene.find("tree").unwrap();
        assert_eq!(tree.transform.position.x, 5.0);
        assert_eq!(
            merged.scene.find("rock").unwrap().material_ref(),
            crate::scene::MaterialRef::Named("moss".into())
        );
    }

    #[test]
    fn different_fields_of_one_entity_merge_and_the_same_field_conflicts_in_words() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)"));
        let theirs = scene(
            &BASE
                .replace("\"tree\"", "\"pine\"")
                .replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)"),
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        let tree = merged.scene.get("a1".parse().unwrap()).unwrap();
        assert_eq!(tree.name, "pine", "their rename");
        assert_eq!(tree.transform.position.x, 5.0, "our move, kept");
        assert_eq!(merged.conflicts.len(), 1);
        let said = merged.conflicts[0].to_string();
        assert!(said.contains("both changed its position"), "{said}");
        assert!(said.contains("kept ours"), "{said}");
    }

    #[test]
    fn additions_on_both_sides_and_a_deletion_all_land() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace(
            r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#,
            r#"(id: "b2", name: "rock", model: "m", material: "stone"), (id: "e5", name: "bush", model: "m"),"#,
        ));
        let theirs = scene(
            &BASE
                .replace(r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#, "")
                .replace(
                    r#"[(id: "d4", name: "door", model: "m")]"#,
                    r#"[(id: "d4", name: "door", model: "m"), (id: "f6", name: "window", model: "m")]"#,
                ),
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let names: Vec<&str> = merged
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["tree", "bush", "hut", "door", "window"]);
    }

    #[test]
    fn deleting_what_the_other_side_changed_keeps_it_and_says_so() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace(
            r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#,
            "",
        ));
        let theirs = scene(&BASE.replace("\"stone\"", "\"moss\""));
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(
            merged.scene.find("rock").is_some(),
            "kept: a delete can be redone"
        );
        assert_eq!(merged.conflicts.len(), 1);
        let said = merged.conflicts[0].to_string();
        assert!(
            said.contains("ours deleted it, theirs changed it; kept it"),
            "{said}"
        );
    }

    #[test]
    fn components_merge_one_by_one() {
        let base = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: false) })])"#,
        );
        let ours = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: true) })])"#,
        );
        let theirs = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: false), "loot": ("chest") })])"#,
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let gate = &merged.scene.entities[0];
        assert_eq!(gate.components["door"].get_ron(), "(open: true)");
        assert_eq!(gate.components["loot"].get_ron(), "(\"chest\")");
    }

    #[test]
    fn two_moves_that_would_make_a_loop_do_not() {
        let base = scene(
            r#"(entities: [(id: "a1", name: "a", model: "m"), (id: "b2", name: "b", model: "m")])"#,
        );
        let ours = scene(
            r#"(entities: [(id: "b2", name: "b", model: "m", children: [(id: "a1", name: "a", model: "m")])])"#,
        );
        let theirs = scene(
            r#"(entities: [(id: "a1", name: "a", model: "m", children: [(id: "b2", name: "b", model: "m")])])"#,
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert_eq!(
            merged.scene.flatten().len(),
            2,
            "both still there: {:?}",
            merged.scene
        );
        assert!(!merged.conflicts.is_empty());
    }

    #[test]
    fn a_conflict_can_be_settled_theirs_way_field_by_field() {
        let base = scene(BASE);
        let ours = scene(
            &BASE
                .replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)")
                .replace("\"stone\"", "\"bark\""),
        );
        let theirs = scene(
            &BASE
                .replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)")
                .replace("\"stone\"", "\"moss\""),
        );
        let mut merged = merge_scenes(&base, &ours, &theirs);
        assert_eq!(merged.conflicts.len(), 2);
        let position = merged
            .conflicts
            .iter()
            .find(|c| c.field == "its position")
            .unwrap()
            .clone();
        assert!(position.take_theirs(&mut merged.scene, &theirs));
        assert_eq!(merged.scene.find("tree").unwrap().transform.position.z, 9.0);
        assert_eq!(
            merged.scene.find("rock").unwrap().material_ref(),
            crate::scene::MaterialRef::Named("bark".into()),
            "the other conflict is still ours"
        );
    }

    #[test]
    fn a_diff_names_what_was_added_removed_and_which_field_changed() {
        let before = scene(BASE);
        let after = scene(
            &BASE
                .replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)")
                .replace(r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#, "")
                .replace(
                    r#"[(id: "d4", name: "door", model: "m")]"#,
                    r#"[(id: "d4", name: "door", model: "m"), (id: "f6", name: "window", model: "m")]"#,
                ),
        );
        let changes = diff_scenes(&before, &after);
        let said: Vec<String> = changes.iter().map(|c| c.to_string()).collect();
        assert_eq!(changes.len(), 3, "{said:?}");
        assert!(said.iter().any(|s| s.starts_with("added `window`")), "{said:?}");
        assert!(said.iter().any(|s| s.starts_with("removed `rock`")), "{said:?}");
        let moved = changes
            .iter()
            .find(|c| c.is_field(Some("a1".parse().unwrap()), "its position"))
            .expect("the tree's move");
        assert!(moved.to_string().contains("→"), "{moved}");
        assert!(diff_scenes(&before, &before).is_empty(), "no change, no lines");
    }

    #[test]
    fn a_conflict_taken_theirs_can_be_taken_ours_again_and_says_which_side_it_holds() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)"));
        let theirs = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)"));
        let merged = merge_scenes(&base, &ours, &theirs);
        let conflict = &merged.conflicts[0];
        let side = |scene: &Scene| {
            conflict.side(scene, &diff_scenes(&ours, scene), &diff_scenes(&theirs, scene))
        };
        let mut now = merged.scene.clone();
        assert_eq!(side(&now), Some(Side::Ours), "the merge keeps ours");
        assert!(conflict.take_theirs(&mut now, &theirs));
        assert_eq!(side(&now), Some(Side::Theirs));
        assert!(conflict.take_ours(&mut now, &ours));
        assert_eq!(side(&now), Some(Side::Ours));
        assert_eq!(now.find("tree").unwrap().transform.position.x, 5.0);
        now.get_mut("a1".parse().unwrap()).unwrap().transform.position.y = 3.0;
        assert_eq!(side(&now), None, "a hand edit is neither side");
    }
