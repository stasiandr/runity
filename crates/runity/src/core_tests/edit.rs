//! The core's `edit`, tested with the modules' fields on its lines.

    use crate::edit::*;
    #[allow(unused_imports)]
    use std::collections::HashSet;
    #[allow(unused_imports)]
    use crate::id::EntityId;
    #[allow(unused_imports)]
    use crate::scene::{EntityDesc, Scene};
    #[allow(unused_imports)]
    use crate::prelude::*;

    #[test]
    fn a_gesture_is_one_step_however_many_edits_it_makes() {
        let mut history = History::new(Scene::default(), 64);
        history.edit().entities.push(EntityDesc::default());
        history.begin_gesture();
        for _ in 0..100 {
            history.edit().entities.push(EntityDesc::default());
        }
        history.end_gesture();
        assert_eq!(history.depth(), 2, "the first edit, and the gesture");
        history.undo();
        assert_eq!(
            history.scene().entities.len(),
            1,
            "back to before the gesture"
        );
        history.edit().entities.clear();
        assert_eq!(history.depth(), 2, "after it, edits are steps again");
    }
    use glam::Vec3;

    fn entity(name: &str, children: Vec<EntityDesc>) -> EntityDesc {
        EntityDesc {
            name: name.into(),
            children,
            ..Default::default()
        }
        .with(crate::scene::ModelRef("builtin:cube".into()))
    }

    /// root
    ///   child
    ///     grandchild
    /// other
    fn scene() -> Scene {
        let mut scene = Scene {
            entities: vec![
                entity(
                    "root",
                    vec![entity("child", vec![entity("grandchild", vec![])])],
                ),
                entity("other", vec![]),
            ],
            ..Default::default()
        };
        scene.assign_ids();
        scene
    }

    fn names(scene: &Scene) -> Vec<String> {
        scene
            .flatten()
            .into_iter()
            .map(|(e, _)| e.name.clone())
            .collect()
    }

    fn id(scene: &Scene, name: &str) -> EntityId {
        scene.find(name).unwrap_or_else(|| panic!("{name}")).id
    }

    #[test]
    fn undo_puts_back_exactly_what_was_there() {
        let mut history = History::new(scene(), 32);
        assert!(!history.can_undo());

        let before = history.scene().clone();
        let child = id(&before, "child");
        remove(history.edit(), child);
        assert_eq!(names(history.scene()), ["root", "other"]);

        assert!(history.undo());
        assert_eq!(*history.scene(), before, "byte for byte, not nearly");
        assert!(!history.can_undo());
        assert!(history.can_redo());

        assert!(history.redo());
        assert_eq!(names(history.scene()), ["root", "other"]);
    }

    #[test]
    fn a_new_edit_after_an_undo_throws_the_redo_away() {
        // Keeping it would let a redo reapply a state the new edit never
        // came from — the classic way an undo stack corrupts a document.
        let mut history = History::new(scene(), 32);
        let (other, child) = (id(history.scene(), "other"), id(history.scene(), "child"));
        remove(history.edit(), other);
        history.undo();
        assert!(history.can_redo());

        remove(history.edit(), child);
        assert!(!history.can_redo());
    }

    #[test]
    fn a_drag_is_one_step_however_many_times_it_moves() {
        // Sixty mutations a second must not be sixty undo steps.
        let mut history = History::new(scene(), 32);
        let root = id(history.scene(), "root");
        history.snapshot();
        for i in 1..=60 {
            let scene = history.scene_mut_untracked();
            scene.get_mut(root).unwrap().transform.position = Vec3::new(i as f32, 0.0, 0.0);
        }
        assert_eq!(history.scene().entities[0].transform.position.x, 60.0);

        assert!(history.undo());
        assert_eq!(history.scene().entities[0].transform.position.x, 0.0);
        assert!(!history.can_undo(), "one step, not sixty");
    }

    #[test]
    fn the_history_does_not_grow_without_bound() {
        let mut history = History::new(scene(), 3);
        for _ in 0..10 {
            history.snapshot();
        }
        for _ in 0..3 {
            assert!(history.undo());
        }
        assert!(!history.undo(), "three deep, as asked");
    }

    #[test]
    fn opening_a_document_clears_the_stacks() {
        // Undoing across an open would put another document's entities into
        // this one.
        let mut history = History::new(scene(), 32);
        let child = id(history.scene(), "child");
        remove(history.edit(), child);
        history.replace(Scene::default());
        assert!(!history.can_undo() && !history.can_redo());
    }

    #[test]
    fn removing_an_entity_takes_its_children_with_it() {
        let mut scene = scene();
        let child = id(&scene, "child");
        remove(&mut scene, child);
        assert_eq!(names(&scene), ["root", "other"], "child and grandchild too");
    }

    #[test]
    fn an_id_does_not_move_when_something_before_it_goes() {
        // The whole reason for IDs. With positions, removing "root" would
        // make "other" answer to the number "child" used to have, and the
        // next edit meant for one would land on the other.
        let mut scene = scene();
        let other = id(&scene, "other");
        let root = id(&scene, "root");
        remove(&mut scene, root);
        assert_eq!(scene.get(other).map(|e| e.name.as_str()), Some("other"));
    }

    #[test]
    fn a_duplicate_lands_beside_the_original_and_copies_the_subtree() {
        let mut scene = scene();
        let root = id(&scene, "root");
        let copy = duplicate(&mut scene, root).expect("root is duplicable");
        assert_eq!(
            names(&scene),
            [
                "root",
                "child",
                "grandchild",
                "root (1)",
                "child",
                "grandchild",
                "other"
            ]
        );
        assert_ne!(copy, root);
        assert_eq!(scene.entities[1].id, copy, "right after the original");
        // A copy of a copy takes the next free number.
        duplicate(&mut scene, copy).unwrap();
        duplicate(&mut scene, root).unwrap();
        let tops: Vec<&str> = scene.entities.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(tops, ["root", "root (3)", "root (1)", "root (2)", "other"]);
    }

    #[test]
    fn a_duplicate_is_a_new_thing_all_the_way_down() {
        // A copy that kept its children's IDs would have two grandchildren
        // answering to one name: an edit would hit whichever was found first
        // and a merge could not tell them apart.
        let mut scene = scene();
        let root = id(&scene, "root");
        duplicate(&mut scene, root).unwrap();
        let ids = scene.ids();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "every ID once: {ids:?}");
    }

    #[test]
    fn reparenting_moves_a_subtree_under_a_new_parent() {
        let mut scene = scene();
        let (other, child) = (id(&scene, "other"), id(&scene, "child"));
        assert!(
            reparent(&mut scene, other, Some(child)),
            "other under child"
        );
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
        // And it really is nested now, not merely ordered that way.
        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.get(other).unwrap().name, "other", "same entity");
    }

    #[test]
    fn nothing_can_become_its_own_ancestor() {
        // The one way a tree stops being a tree, and the first thing an
        // editor's drag-and-drop tries.
        let mut scene = scene();
        let (root, grandchild) = (id(&scene, "root"), id(&scene, "grandchild"));
        assert!(
            !reparent(&mut scene, root, Some(grandchild)),
            "root under its grandchild"
        );
        assert!(!reparent(&mut scene, root, Some(root)), "root under itself");
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
    }

    #[test]
    fn a_parent_that_is_not_there_is_refused_rather_than_losing_the_entity() {
        let mut scene = scene();
        let other = id(&scene, "other");
        assert!(!reparent(&mut scene, other, Some(EntityId::fresh())));
        assert!(scene.get(other).is_some(), "still in the scene");
    }

    #[test]
    fn adding_under_a_parent_puts_it_at_the_end_of_that_parent() {
        let mut scene = scene();
        let child = id(&scene, "child");
        let added = add(&mut scene, Some(child), entity("new", vec![])).expect("added");
        assert_eq!(
            names(&scene),
            ["root", "child", "grandchild", "new", "other"]
        );
        assert_eq!(scene.get(added).unwrap().name, "new");
        assert!(
            add(&mut scene, Some(EntityId::fresh()), entity("lost", vec![])).is_none(),
            "a parent that is not there"
        );
    }

    #[test]
    fn adding_something_whose_id_is_taken_gives_it_a_new_one() {
        // Adding the same description twice adds two things, not one thing
        // twice.
        let mut scene = scene();
        let other = scene.find("other").unwrap().clone();
        let added = add(&mut scene, None, other.clone()).unwrap();
        assert_ne!(added, other.id);
    }

    #[test]
    fn a_scatter_is_the_same_for_the_same_seed_and_keeps_its_spacing() {
        let layout = Scatter {
            radius: 8.0,
            count: 30,
            spacing: 1.5,
            seed: 7,
            ..Scatter::default()
        };
        let a = scatter(&layout);
        assert_eq!(a, scatter(&layout), "reproducible");
        assert_eq!(a.len(), 30);
        for (i, p) in a.iter().enumerate() {
            assert!(p.position.length() <= 8.0 + 1e-3);
            for q in &a[i + 1..] {
                assert!(p.position.distance(q.position) >= 1.5);
            }
        }
        assert_ne!(
            a,
            scatter(&Scatter {
                seed: 8,
                ..layout.clone()
            }),
            "and seed-dependent"
        );

        // Too full for the spacing: fewer, not a clump.
        let crowded = scatter(&Scatter {
            radius: 1.0,
            count: 50,
            spacing: 1.0,
            ..layout
        });
        assert!(crowded.len() < 50 && !crowded.is_empty());
    }

    #[test]
    fn undo_says_what_it_would_take_back() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [(id: "a1", name: "tree", model: "m"), (id: "b2", name: "hut", model: "m", children: [(id: "c3", name: "door", model: "m")])])"#,
        )
        .unwrap();
        scene.assign_ids();
        let mut history = History::new(scene, 16);
        assert_eq!(history.undo_description(), None);

        history
            .edit()
            .get_mut("a1".parse().unwrap())
            .unwrap()
            .transform
            .position
            .x = 3.0;
        assert_eq!(history.undo_description().as_deref(), Some("move `tree`"));
        history.edit().get_mut("a1".parse().unwrap()).unwrap().name = "pine".into();
        assert_eq!(
            history.undo_description().as_deref(),
            Some("rename `tree` to `pine`")
        );
        remove(history.edit(), "b2".parse().unwrap());
        assert_eq!(
            history.undo_description().as_deref(),
            Some("delete `hut`"),
            "not the door too"
        );
        history.undo();
        assert_eq!(history.redo_description().as_deref(), Some("delete `hut`"));
    }
