//! The editor's session, driven the way an editor drives it.
//!
//! These were the C boundary's tests, and most of them never were about the
//! boundary: they are about what a click, a drag, an undo or pressing play
//! does to a document. They came across when the boundary went. The ones
//! that were about the boundary — a name truncated into a buffer, a null
//! handle, a layer the host had not made yet — went with it.

use std::path::{Path, PathBuf};

use runity::gizmo::{Handle, Tool};
use runity::glam::Vec3;
use runity::{EntityId, Material, Transform};
use runity_editor::{EditError, SceneReload, Session, Snap};

const SCENE: &str = r#"(
    entities: [
        (name: "ground", model: "builtin:plane", transform: (scale: (20.0, 1.0, 20.0))),
        (
            name: "crate",
            model: "builtin:cube",
            transform: (position: (0.0, 0.5, 0.0)),
            children: [(name: "lid", model: "builtin:cube", transform: (position: (0.0, 0.6, 0.0)))],
        ),
    ],
)"#;

/// A scene inside a freshly made project, the way every scene lives.
fn scene_file(name: &str, text: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("runity-editor-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    runity::Project::create(&root, name).unwrap();
    let path = root.join("scenes/scene.ron");
    std::fs::write(&path, text).unwrap();
    path
}

/// A scene in a plain folder, in no project at all.
fn loose_scene_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runity-editor-loose-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    std::fs::write(&path, SCENE).unwrap();
    path
}

/// The project folder a test scene sits in.
fn root_of(scene: &Path) -> PathBuf {
    scene.parent().and_then(Path::parent).unwrap().to_path_buf()
}

/// A session with the test scene open, or `None` on a machine with no
/// adapter at all — which says so and skips rather than failing.
fn open(name: &str) -> Option<(Session, PathBuf)> {
    open_with(name, SCENE)
}

fn new_session() -> Option<Session> {
    match Session::offscreen(192, 128) {
        Ok(session) => Some(session),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn open_with(name: &str, text: &str) -> Option<(Session, PathBuf)> {
    let mut session = new_session()?;
    let path = scene_file(name, text);
    let skipped = session.open_scene(&path).expect("the scene opens");
    assert!(skipped.is_empty(), "{skipped:?}");
    Some((session, path))
}

fn transform(position: [f32; 3], rotation_deg: [f32; 3], scale: [f32; 3]) -> Transform {
    Transform {
        position: Vec3::from_array(position),
        rotation_deg: Vec3::from_array(rotation_deg),
        scale: Vec3::from_array(scale),
    }
}

/// An entity's ID, by name — what a person clicking in the tree would get.
fn id(session: &Session, name: &str) -> EntityId {
    session
        .find(name)
        .unwrap_or_else(|| panic!("no {name} in the scene"))
}

/// Sweep outward from the middle of the view for a handle, rather than
/// guessing a pixel: where the handles land depends on the projection, and
/// a test that hard-codes that is testing arithmetic it does not own.
/// Returns the handle grabbed and how far right of centre it was.
fn grab_right_of_centre(session: &mut Session) -> Option<(Handle, u32)> {
    let (width, height) = session.size();
    let (cx, cy) = (width / 2, height / 2);
    for r in 4..(width / 2) {
        if let Some(handle) = session.gizmo_begin(cx + r, cy).unwrap() {
            return Some((handle, r));
        }
    }
    None
}

#[test]
fn a_scene_opens_and_lists_its_entities_including_children() {
    let Some((session, _)) = open("list") else {
        return;
    };
    assert_eq!(session.entity_count(), 3, "ground, crate, lid");
    let names: Vec<String> = session
        .entities()
        .into_iter()
        .map(|id| session.entity_name(id).unwrap())
        .collect();
    assert_eq!(names, ["ground", "crate", "lid"], "in tree order");
    assert_eq!(session.entity_name(EntityId::fresh()), None);
}

#[test]
fn a_transform_round_trips_and_reaches_the_file() {
    let Some((mut session, path)) = open("transform") else {
        return;
    };
    let crate_id = id(&session, "crate");
    let wanted = transform([1.0, 2.0, 3.0], [0.0, 45.0, 0.0], [2.0, 2.0, 2.0]);
    session.set_transform(crate_id, wanted).unwrap();
    assert_eq!(session.transform(crate_id), Some(wanted));

    session.save_scene(None).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("45"), "the rotation reached the file");

    // And the child moved with its parent, which is the thing an editor's
    // inspector has to get right or nothing nests.
    let reopened = runity::Scene::load(&path).unwrap();
    let lid = reopened
        .flatten()
        .into_iter()
        .find(|(e, _)| e.name == "lid")
        .map(|(_, world)| world.w_axis)
        .unwrap();
    assert!((lid.y - 3.2).abs() < 1e-4, "2.0 + 0.6 * 2.0, got {}", lid.y);
}

#[test]
fn an_edit_marks_the_document_modified_and_saving_or_undoing_clears_it() {
    let Some((mut session, path)) = open("modified") else {
        return;
    };
    assert_eq!(session.scene_path(), Some(path.as_path()));
    assert!(
        !session.is_modified(),
        "as opened, it is what the file says"
    );
    let crate_id = id(&session, "crate");
    let moved = transform([4.0, 0.0, 0.0], [0.0; 3], [1.0; 3]);
    session.set_transform(crate_id, moved).unwrap();
    assert!(session.is_modified());
    session.undo().unwrap();
    assert!(
        !session.is_modified(),
        "undone back to the file is not a change"
    );
    session.redo().unwrap();
    session.save_scene(None).unwrap();
    assert!(!session.is_modified(), "saved");
}

#[test]
fn an_edit_to_an_entity_that_is_not_there_says_which() {
    // The reason travels with the failure now, and it has to be one an
    // agent can act on from the text alone.
    let Some((mut session, _)) = open("missing") else {
        return;
    };
    let nobody = EntityId::fresh();
    let err = session
        .set_transform(nobody, Transform::default())
        .unwrap_err();
    assert_eq!(err, EditError::NoEntity(nobody));
    assert!(err.to_string().contains(&nobody.to_string()), "{err}");
    assert!(
        !session.can_undo(),
        "a refused edit must not leave an empty step on the undo stack"
    );
}

#[test]
fn clicking_on_a_thing_finds_it_and_clicking_on_sky_does_not() {
    let Some((mut session, _)) = open("pick") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 1.0, 5.0), Vec3::new(0.0, 0.5, 0.0));
    let (width, height) = session.size();
    assert!(
        session.pick(width / 2, height / 2).is_some(),
        "something is under the middle of the view"
    );
    // Straight up, where there is neither crate nor ground.
    assert_eq!(session.pick(width / 2, 0), None, "nothing is up there");
}

#[test]
fn rendering_produces_a_frame_that_can_be_read() {
    let Some((mut session, _)) = open("render") else {
        return;
    };
    session.render();
    let pixels = session.frame_pixels();
    assert_eq!(pixels.len(), 192 * 128 * 4);
    assert!(pixels.iter().any(|b| *b != 0), "something was drawn");
}

#[test]
fn the_gizmo_moves_the_thing_it_is_on_and_writes_it_to_the_scene() {
    let Some((mut session, path)) = open("gizmo") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 1.0, 6.0), Vec3::new(0.0, 0.5, 0.0));

    // Nothing selected: no handles to find.
    assert_eq!(session.selected(), None);
    assert_eq!(session.gizmo_hover(96, 64), None);

    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    assert_eq!(session.selected(), Some(crate_id));

    let height = session.size().1;
    let width = session.size().0;
    let grab_x = (0..width)
        .find(|x| session.gizmo_begin(*x, height / 2).unwrap().is_some())
        .expect("an arm crosses the middle of the view");

    let before = session.transform(crate_id).unwrap();

    // Grabbing and not moving must not move anything.
    assert!(session.gizmo_drag(grab_x, height / 2).unwrap());
    assert_eq!(
        session.transform(crate_id).unwrap(),
        before,
        "a grab on its own is not a move"
    );

    // Now drag sideways.
    assert!(session.gizmo_drag(grab_x + 30, height / 2).unwrap());
    session.gizmo_end();

    let after = session.transform(crate_id).unwrap();
    assert_ne!(after.position, before.position, "the crate moved");
    assert_eq!(
        (after.rotation_deg, after.scale),
        (before.rotation_deg, before.scale),
        "and only its position changed"
    );

    // And it survives a save and reopen: a drag has to end up in a file.
    session.save_scene(None).unwrap();
    let reopened = runity::Scene::load(&path).unwrap();
    let moved = reopened.find("crate").unwrap().transform.position;
    assert!((moved - after.position).length() < 1e-4);
}

#[test]
fn dragging_a_child_keeps_it_in_its_parent() {
    // The gizmo sits at a world position and the file holds a local one.
    // Writing the world position straight back yanks a child out of its
    // parent by however far the parent is offset.
    let Some((mut session, _)) = open("gizmo-child") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 1.5, 6.0), Vec3::new(0.0, 1.0, 0.0));
    let lid = id(&session, "lid");
    session.select(Some(lid)).unwrap();

    let (width, height) = session.size();
    let Some(grab_x) = (0..width).find(|x| session.gizmo_begin(*x, height / 2).unwrap().is_some())
    else {
        return;
    };
    let before = session.transform(lid).unwrap();
    assert!(session.gizmo_drag(grab_x, height / 2).unwrap());
    let after = session.transform(lid).unwrap();

    // The crate sits at y = 0.5 and the lid at y = 0.6 above it. A drag
    // that does not undo the parent would rewrite the lid's local y as 1.1.
    assert!(
        (after.position.y - before.position.y).abs() < 1e-3,
        "the lid's local position should be unchanged by a grab, was {} and is now {}",
        before.position.y,
        after.position.y
    );
}

#[test]
fn resizing_changes_what_it_reports_and_renders() {
    let Some((mut session, _)) = open("resize") else {
        return;
    };
    assert_eq!(session.size(), (192, 128));
    session.resize(64, 48);
    assert_eq!(session.size(), (64, 48));

    // And the frame that comes out is the new size. An offscreen target
    // cannot be resized in place, so reporting the new size while still
    // rendering the old one is the failure this catches.
    session.render();
    assert_eq!(session.frame_pixels().len(), 64 * 48 * 4);
}

#[test]
fn editing_and_undoing_round_trips() {
    let Some((mut session, _)) = open("edit") else {
        return;
    };
    assert_eq!(session.entity_count(), 3);
    assert!(!session.can_undo(), "nothing yet");

    let added = session.add(None, "builtin:sphere").unwrap();
    assert_eq!(
        session.entities().last(),
        Some(&added),
        "appended at the top level"
    );
    assert_eq!(session.entity_count(), 4);
    assert!(session.can_undo());

    // Duplicating the crate copies its child too, as new things.
    let crate_id = id(&session, "crate");
    let copy = session.duplicate(crate_id).unwrap();
    assert_ne!(copy, crate_id);
    assert_eq!(session.entity_count(), 6);

    assert!(session.undo().unwrap());
    assert_eq!(session.entity_count(), 4);
    assert!(session.undo().unwrap());
    assert_eq!(session.entity_count(), 3);
    assert!(!session.undo().unwrap(), "back to the start");

    assert!(session.can_redo());
    assert!(session.redo().unwrap());
    assert_eq!(session.entity_count(), 4);
}

#[test]
fn a_whole_drag_undoes_in_one_step() {
    // The thing an editor gets wrong: sixty mutations a second becoming
    // sixty undo steps, so undo appears not to work at all.
    let Some((mut session, _)) = open("drag-undo") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 1.0, 6.0), Vec3::new(0.0, 0.5, 0.0));
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    let before = session.transform(crate_id).unwrap();

    let (width, height) = session.size();
    let Some(grab_x) = (0..width).find(|x| session.gizmo_begin(*x, height / 2).unwrap().is_some())
    else {
        return;
    };
    for step in 1..=40 {
        session.gizmo_drag(grab_x + step, height / 2).unwrap();
    }
    session.gizmo_end();
    assert_ne!(session.transform(crate_id).unwrap(), before, "it moved");

    assert!(session.undo().unwrap());
    assert_eq!(
        session.transform(crate_id).unwrap(),
        before,
        "one undo restores the whole gesture"
    );
    assert!(!session.can_undo(), "and only one");
}

#[test]
fn reparenting_refuses_to_make_a_loop() {
    let Some((mut session, _)) = open("reparent") else {
        return;
    };
    // The lid is the crate's child; putting the crate under the lid would
    // make a cycle, and a cycle in a scene tree is unrecoverable.
    let (crate_id, lid) = (id(&session, "crate"), id(&session, "lid"));
    assert!(!session.reparent(crate_id, Some(lid)).unwrap());
    assert_eq!(session.entity_count(), 3);
    assert!(!session.can_undo(), "a refused move costs no undo step");

    // Moving the lid to the top level is fine, and it is still the lid.
    assert!(session.reparent(lid, None).unwrap());
    assert_eq!(session.entity_count(), 3);
    assert_eq!(session.entity_name(lid).as_deref(), Some("lid"));
}

#[test]
fn a_selection_survives_edits_to_everything_else() {
    // By ID, a selection points at the same thing after a delete elsewhere,
    // a reparent or an undo. When it was a position in a list it had to be
    // dropped after every one of those, or it would point at whatever slid
    // into the gap.
    let Some((mut session, _)) = open("selection") else {
        return;
    };
    let (ground, crate_id, lid) = (
        id(&session, "ground"),
        id(&session, "crate"),
        id(&session, "lid"),
    );
    session.select(Some(lid)).unwrap();

    session.delete(ground).unwrap();
    assert_eq!(session.selected(), Some(lid), "a delete elsewhere");
    assert!(session.reparent(lid, None).unwrap());
    assert_eq!(session.selected(), Some(lid), "a reparent of it");
    assert!(session.undo().unwrap());
    assert!(session.undo().unwrap());
    assert_eq!(session.selected(), Some(lid), "two undos");

    // Deleting what is selected does clear it, and takes the children too.
    session.delete(crate_id).unwrap();
    assert_eq!(session.selected(), None, "the lid went with the crate");
    assert_eq!(session.entity_count(), 1);
}

#[test]
fn the_ids_a_scene_is_saved_with_are_the_ids_it_opens_with() {
    // What makes an ID worth having: a selection, a message or a merge
    // that names an entity today names the same one after a save.
    let Some((mut session, path)) = open("ids") else {
        return;
    };
    let before = session.entities();
    session.save_scene(None).unwrap();
    session.open_scene(&path).unwrap();
    assert_eq!(session.entities(), before);
}

#[test]
fn a_colour_can_be_tuned_on_one_object_and_taken_back() {
    // The thing a person actually wants an editor for: nudge a colour, look
    // at it, undo it. A scene file cannot do this, which is why the editor
    // exists at all.
    let Some((mut session, path)) = open("material") else {
        return;
    };
    let crate_id = id(&session, "crate");
    let before = session.material(crate_id).unwrap();
    assert_eq!(before, Material::default(), "the default grey, lit");

    let wanted = Material::new(0.31, 0.12, 0.05);
    session.set_material(crate_id, wanted).unwrap();
    assert_eq!(session.material(crate_id), Some(wanted));
    assert_eq!(
        session.material_name(crate_id),
        None,
        "a tuned colour belongs to the object, not to a name"
    );

    // It reaches the file, and it comes back.
    session.save_scene(None).unwrap();
    let reopened = runity::Scene::load(&path).unwrap();
    assert_eq!(
        reopened.find("crate").unwrap().material().base_color[0],
        0.31
    );

    assert!(session.undo().unwrap());
    assert_eq!(
        session.material(crate_id),
        Some(before),
        "one step, all the way back"
    );
}

#[test]
fn an_entity_can_be_pointed_at_the_palette_and_reports_what_it_became() {
    let Some((mut session, path)) = open("palette") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.set_material_name(crate_id, "stone").unwrap();
    assert_eq!(session.material_name(crate_id).as_deref(), Some("stone"));

    // The colour reported is the colour stone is, not the word: an
    // inspector's swatch has to match what is on screen.
    assert_eq!(
        session.material(crate_id),
        Some(runity::material::builtin::STONE),
        "the name should resolve to a colour"
    );

    // And the file keeps the name rather than the colour, which is the whole
    // point of a palette.
    session.save_scene(None).unwrap();
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("\"stone\""));

    // An empty name is refused: clearing a link means giving a colour.
    assert!(matches!(
        session.set_material_name(crate_id, ""),
        Err(EditError::EmptyName(_))
    ));
    assert_eq!(
        session.material_name(crate_id).as_deref(),
        Some("stone"),
        "nothing changed"
    );
}

#[test]
fn the_palette_offers_the_builtins_and_the_librarys_own() {
    let Some((mut session, _)) = open("palette-list") else {
        return;
    };

    // With no library it is the builtins, each with a colour to draw a
    // swatch with.
    let builtins = session.palette();
    assert_eq!(builtins.len(), runity::material::builtin::NAMES.len());
    assert!(builtins.iter().any(|(name, _)| name == "ember"));
    assert!(builtins
        .iter()
        .all(|(_, m)| m.base_color.iter().all(|c| (0.0..=1.0).contains(c))));

    // Point it at a library holding two materials: one new, one shadowing a
    // builtin. The palette should gain one entry, not two, and `stone`
    // should now be the project's.
    let library = std::env::temp_dir().join("runity-editor-palette-library");
    let _ = std::fs::remove_dir_all(&library);
    std::fs::create_dir_all(&library).unwrap();
    write_material(&library, "moss", Material::new(0.1, 0.3, 0.05));
    write_material(&library, "stone", Material::new(0.9, 0.0, 0.0));
    let skipped = session.set_library(&library).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");

    assert_eq!(
        session.palette().len(),
        builtins.len() + 1,
        "one new name, and `stone` listed once rather than twice"
    );

    let crate_id = id(&session, "crate");
    session.set_material_name(crate_id, "stone").unwrap();
    let reported = session.material(crate_id).unwrap();
    assert!(
        reported.base_color[0] > 0.8 && reported.base_color[1] < 0.01,
        "the project's stone, not the engine's: {reported:?}"
    );

    session
        .set_material_name(crate_id, "builtin:stone")
        .unwrap();
    assert_eq!(
        session.material(crate_id),
        Some(runity::material::builtin::STONE),
        "and the engine's is still reachable"
    );
}

/// Write a material asset straight into a library directory, without the
/// importer: this is about what the session does with a library, not about
/// how a library is made.
fn write_material(directory: &Path, name: &str, material: Material) {
    let bytes = runity::asset::to_bytes(
        &runity::MaterialAsset {
            id: runity::AssetId::from_source(name, 0),
            name: name.into(),
            material,
        },
        runity::asset::AssetKind::Material,
    )
    .unwrap();
    std::fs::write(directory.join(format!("{name}.rasset")), bytes).unwrap();
}

#[test]
fn opening_a_scene_looks_where_the_scene_says_and_keeping_a_view_is_a_decision() {
    let Some((mut session, path)) = open("camera") else {
        return;
    };

    // The test scene carries no view, so it opens at the format's default
    // rather than at whatever the last document left behind.
    let default = runity::View::default();
    assert_eq!(session.camera().position, default.position);

    // Flying around is not an edit: nothing is dirtied and nothing is saved.
    let moved = Vec3::new(4.0, 9.0, 1.0);
    let at = Vec3::new(0.0, 1.0, 0.0);
    session.set_camera(moved, at);
    session.save_scene(None).unwrap();
    assert_eq!(
        runity::Scene::load(&path).unwrap().view,
        default,
        "looking around should not have changed the file"
    );

    // Keeping it is. And it survives the round trip.
    session.capture_camera().unwrap();
    session.save_scene(None).unwrap();
    let kept = runity::Scene::load(&path).unwrap().view;
    assert_eq!((kept.position, kept.target), (moved, at));

    // One undoable step, like anything else typed into an inspector.
    assert!(session.undo().unwrap());
    session.save_scene(None).unwrap();
    assert_eq!(runity::Scene::load(&path).unwrap().view, default);

    // And opening a scene with a view in it points the camera there.
    let framed = path.parent().unwrap().join("framed.ron");
    std::fs::write(
        &framed,
        r#"(view: (position: (1.0, 2.0, 3.0), target: (0.0, 0.0, 0.0), fov_deg: 40.0), entities: [])"#,
    )
    .unwrap();
    session.open_scene(&framed).unwrap();
    assert_eq!(session.camera().position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(session.camera().target, Vec3::ZERO);
}

#[test]
fn a_thing_arranged_once_becomes_a_thing_placed_many_times() {
    let Some((mut session, path)) = open("prefab") else {
        return;
    };
    // The crate has a lid, so it is a two-entity arrangement: exactly the
    // kind of thing that should stop being copied by hand.
    assert_eq!(session.entity_count(), 3);
    let crate_id = id(&session, "crate");
    session.make_prefab(crate_id, "crate").unwrap();

    // The scene now holds one row where there were two, and it says what it
    // is an instance of.
    assert_eq!(
        session.entity_count(),
        2,
        "the lid lives in the prefab file now"
    );
    assert_eq!(session.entity_prefab(crate_id).as_deref(), Some("crate"));
    let written = root_of(&path).join("prefabs/crate.prefab");
    assert!(
        written.exists(),
        "{} should have been written",
        written.display()
    );

    // Place a second one.
    let placed = session.add_instance(None, "crate").unwrap();
    assert_eq!(session.entity_count(), 3);
    assert_eq!(session.entity_prefab(placed).as_deref(), Some("crate"));

    // A prefab nothing answers to is refused rather than left as an entity
    // with no model and no explanation.
    assert_eq!(
        session.add_instance(None, "not_a_prefab"),
        Err(EditError::UnknownPrefab("not_a_prefab".into()))
    );

    // It is listed to offer.
    assert_eq!(session.prefab_names(), vec!["crate".to_string()]);

    // And the scene reopens: the reference is what was saved, and opening
    // finds the prefabs beside it without being told where they are.
    session.save_scene(None).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("prefab"),
        "the scene keeps the reference:\n{text}"
    );
    // `name: "lid"`, not "lid" — which also appears inside `collider`.
    assert!(
        !text.contains(r#"name: "lid""#),
        "and not a copy of what is in it"
    );
    session.open_scene(&path).unwrap();
    assert_eq!(session.prefab_names(), vec!["crate".to_string()]);
}

#[test]
fn clicking_something_a_prefab_brought_selects_the_instance() {
    // The rule that makes an instance one thing: a click on the lid is a
    // click on the crate, because the crate is what the document can move.
    let Some((mut session, _)) = open("prefab-pick") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 1.2, 6.0), Vec3::new(0.0, 0.9, 0.0));

    // The lid sits above the crate, so a ray through the upper middle finds
    // the lid first. Before it is a prefab, that is its own entity.
    let (x, y) = (96u32, 52u32);
    let (crate_id, lid) = (id(&session, "crate"), id(&session, "lid"));
    assert_eq!(
        session.pick(x, y),
        Some(lid),
        "the lid, which has its own row"
    );

    session.make_prefab(crate_id, "crate").unwrap();
    assert_eq!(
        session.pick(x, y),
        Some(crate_id),
        "the same pixel now finds the instance, not a part of it"
    );
}

#[test]
fn the_rotate_tool_turns_the_thing_it_is_on_and_undoes_in_one_step() {
    let Some((mut session, path)) = open("rotate") else {
        return;
    };
    // Looking down at the crate from above, so the flat Y ring is the one
    // facing the camera and the one a ray can cross.
    session.set_camera(Vec3::new(0.0, 8.0, 0.01), Vec3::new(0.0, 0.5, 0.0));
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.set_tool(Tool::Rotate);
    assert_eq!(session.tool(), Tool::Rotate);

    // Grab the Y ring where it crosses the screen, then drag a quarter of
    // the way around it. The exact pixels do not matter; what matters is
    // that the rotation changes and nothing else does.
    let (handle, radius) = grab_right_of_centre(&mut session).expect("a ring to grab");
    assert_eq!(handle, Handle::Y, "the Y ring, seen from above");
    let (width, height) = session.size();
    assert!(session.gizmo_drag(width / 2, height / 2 + radius).unwrap());
    session.gizmo_end();

    let turned = session.transform(crate_id).unwrap();
    assert!(
        (turned.rotation_deg.y.abs() - 90.0).abs() < 5.0,
        "a quarter turn around the ring should be about ninety degrees, got {}",
        turned.rotation_deg.y
    );
    assert_eq!(turned.position, Vec3::new(0.0, 0.5, 0.0), "it did not move");
    assert_eq!(turned.scale, Vec3::ONE, "and did not grow");

    // The whole gesture is one step, like a move drag.
    assert!(session.undo().unwrap());
    let back = session.transform(crate_id).unwrap().rotation_deg.y;
    assert!(back.abs() < 1e-3, "back to where it started: {back}");

    // And what is written is the local rotation, which is what reopens.
    assert!(session.redo().unwrap());
    session.save_scene(None).unwrap();
    let reopened = runity::Scene::load(&path).unwrap();
    let yaw = reopened.find("crate").unwrap().transform.rotation_deg.y;
    assert!((yaw.abs() - 90.0).abs() < 5.0, "got {yaw}");
}

#[test]
fn the_scale_tool_stretches_one_axis_and_leaves_the_rest() {
    let Some((mut session, _)) = open("scale") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 0.5, 6.0), Vec3::new(0.0, 0.5, 0.0));
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.set_tool(Tool::Scale);

    let (handle, radius) = grab_right_of_centre(&mut session).expect("an arm to grab");
    assert_eq!(handle, Handle::X, "the X arm");

    // Drag outward: the same arm, twice as far from the middle.
    let (width, height) = session.size();
    assert!(session
        .gizmo_drag(width / 2 + radius * 2, height / 2)
        .unwrap());
    session.gizmo_end();

    let stretched = session.transform(crate_id).unwrap();
    assert!(
        stretched.scale.x > 1.2,
        "x should have grown, got {}",
        stretched.scale.x
    );
    assert_eq!(
        (stretched.scale.y, stretched.scale.z),
        (1.0, 1.0),
        "y and z are untouched"
    );
    assert_eq!(
        stretched.position,
        Vec3::new(0.0, 0.5, 0.0),
        "and it did not move"
    );
}

/// A scene with something to drop: a floor, and a crate above it.
const FALLING: &str = r#"(
    entities: [
        (name: "floor", model: "builtin:plane", transform: (scale: (20.0, 1.0, 20.0)),
         body: Static, collider: Box(half: (10.0, 0.05, 10.0))),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 4.0, 0.0)),
         body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5))),
    ],
)"#;

#[test]
fn pressing_play_drops_the_crate_and_stopping_puts_it_back() {
    let Some((mut session, _)) = open_with("play", FALLING) else {
        return;
    };
    assert!(!session.is_playing());

    // Where it is in the world, which during play is not what the document
    // says.
    let crate_id = id(&session, "crate");
    let height = |session: &Session| session.world_position(crate_id).unwrap().y;
    assert!(
        (height(&session) - 4.0).abs() < 1e-3,
        "it starts up in the air"
    );

    session.play();
    assert!(session.is_playing());

    // Two seconds of frames. The count of fixed steps is what the engine
    // decides, not what the caller asks for: sixty a second at the default
    // rate.
    let steps: u32 = (0..120).map(|_| session.step(1.0 / 60.0)).sum();
    assert!(
        (100..=140).contains(&steps),
        "about two seconds of fixed steps, got {steps}"
    );
    let landed = height(&session);
    assert!(
        (landed - 0.55).abs() < 0.2,
        "the crate should be resting on the floor, got {landed}"
    );

    // The document never moved: play is a preview, not an edit.
    assert!(
        (session.transform(crate_id).unwrap().position.y - 4.0).abs() < 1e-3,
        "the file still says 4"
    );
    assert!(!session.can_undo(), "and nothing to undo");

    // Editing while playing is refused rather than thrown away later, and
    // the refusal says why.
    let err = session
        .set_transform(crate_id, Transform::default())
        .unwrap_err();
    assert_eq!(err, EditError::Playing);
    assert!(err.to_string().contains("playing"), "{err}");

    assert!(session.stop());
    assert!(!session.is_playing());
    assert!((height(&session) - 4.0).abs() < 1e-3, "back up in the air");
    assert_eq!(session.step(0.1), 0, "and stopped");

    // Editing works again, and stopping did not cost a step of real history.
    session
        .set_transform(crate_id, Transform::default())
        .unwrap();
    assert!(session.can_undo());
    assert!(session.undo().unwrap());
    assert!((session.transform(crate_id).unwrap().position.y - 4.0).abs() < 1e-3);
}

#[test]
fn pause_holds_the_crate_in_the_air_and_step_moves_it_one_step() {
    use runity::input::{Input, InputEvent as E, Key};
    let Some((mut session, _)) = open_with("pause", FALLING) else {
        return;
    };
    let crate_id = id(&session, "crate");
    let height = |session: &Session| session.world_position(crate_id).unwrap().y;
    assert!(!session.pause(true), "nothing to pause before play");
    assert!(!session.step_once());

    // Ctrl P: play. Half a second of falling.
    let mut input = Input::new();
    // A chord: the modifiers and the key go down in one frame, up in the next.
    fn chord(
        session: &mut Session,
        input: &mut Input,
        held: &[Key],
        key: Key,
    ) -> Vec<&'static str> {
        let mut down: Vec<E> = held.iter().map(|k| E::KeyDown(*k)).collect();
        down.push(E::KeyDown(key));
        let did = view_frame(session, input, (1.0, 1.0), &down);
        let up: Vec<E> = down
            .iter()
            .map(|e| match e {
                E::KeyDown(k) => E::KeyUp(*k),
                e => e.clone(),
            })
            .collect();
        view_frame(session, input, (1.0, 1.0), &up);
        did
    }
    let did = chord(&mut session, &mut input, &[Key::LeftControl], Key::P);
    assert_eq!(did, ["play"]);
    for _ in 0..30 {
        session.step(1.0 / 60.0);
    }
    let falling = height(&session);
    assert!(falling < 4.0, "{falling}");

    // The Inspector says where it is now; the file still says 4.
    let shown = session
        .inspect(crate_id)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "position")
        .unwrap()
        .value;
    let y: Vec3 = runity::ron::from_str(&shown).unwrap();
    assert!(
        (y.y - falling).abs() < 1e-3,
        "{shown} while falling at {falling}"
    );
    assert!((session.transform(crate_id).unwrap().position.y - 4.0).abs() < 1e-3);

    // Ctrl Shift P: held in the air, however long the frames are.
    let did = chord(
        &mut session,
        &mut input,
        &[Key::LeftControl, Key::LeftShift],
        Key::P,
    );
    assert_eq!(did, ["pause"]);
    assert!(session.is_paused());
    for _ in 0..30 {
        assert_eq!(session.step(1.0 / 60.0), 0);
    }
    assert_eq!(height(&session), falling);

    // Ctrl Alt P: one step further, and still paused.
    let did = chord(
        &mut session,
        &mut input,
        &[Key::LeftControl, Key::LeftAlt],
        Key::P,
    );
    assert_eq!(did, ["step"]);
    let stepped = height(&session);
    assert!(stepped < falling, "{stepped} after {falling}");
    assert!(session.is_paused());
    assert_eq!(session.step(1.0), 0);

    // Resumed, it goes on falling; Ctrl P stops and puts it back.
    session.pause(false);
    assert!(session.step(1.0 / 60.0) <= 1, "no catching up on the pause");
    let did = chord(&mut session, &mut input, &[Key::LeftControl], Key::P);
    assert_eq!(did, ["stop"]);
    assert!(!session.is_playing() && !session.is_paused());
    assert!((height(&session) - 4.0).abs() < 1e-3);
}

#[test]
fn a_drag_with_snapping_on_lands_on_the_grid() {
    let Some((mut session, _)) = open("snap") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    let grid = Snap {
        meters: 0.5,
        degrees: 15.0,
        scale: 0.25,
    };
    session.set_snap(grid);
    assert_eq!(session.snap(), grid);

    // Grab the X arm and drag it somewhere off-grid.
    session.set_camera(Vec3::new(0.0, 1.0, 6.0), Vec3::new(0.0, 0.5, 0.0));
    let (handle, radius) = grab_right_of_centre(&mut session).expect("an arm to grab");
    assert_eq!(handle, Handle::X);
    let (width, height) = session.size();
    assert!(session
        .gizmo_drag(width / 2 + radius + 37, height / 2)
        .unwrap());
    session.gizmo_end();

    let x = session.transform(crate_id).unwrap().position;
    assert!(x.x != 0.0, "it should have moved at all: {x}");
    assert!(
        (x.x / 0.5 - (x.x / 0.5).round()).abs() < 1e-4,
        "x should be a multiple of half a metre, got {}",
        x.x
    );
    // And the other two are still exactly where they were: snapping must
    // not drag an untouched axis onto the grid behind your back.
    assert_eq!((x.y, x.z), (0.5, 0.0));

    // Nonsense turns an axis off rather than becoming a NaN in every drag.
    session.set_snap(Snap {
        meters: f32::NAN,
        degrees: -3.0,
        scale: 0.0,
    });
    assert_eq!(session.snap(), Snap::default());
}

#[test]
fn focusing_frames_the_selection_whatever_size_it_is() {
    let Some((mut session, _)) = open("focus") else {
        return;
    };
    // Start looking somewhere else entirely.
    session.set_camera(Vec3::new(40.0, 30.0, 40.0), Vec3::new(40.0, 0.0, 0.0));
    assert!(
        !session.focus_selected(),
        "nothing selected, nothing to focus"
    );

    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    assert!(session.focus_selected());
    let camera = session.camera();
    assert_eq!(
        Some(camera.target),
        session.world_position(crate_id),
        "it looks at the thing"
    );
    let distance = (camera.position - camera.target).length();
    assert!(
        (1.0..12.0).contains(&distance),
        "a metre-wide crate should be framed from a few metres, got {distance}"
    );

    // The big one is framed from further away, which is the whole point of
    // sizing it from what is selected.
    session
        .set_transform(crate_id, transform([0.0, 5.0, 0.0], [0.0; 3], [10.0; 3]))
        .unwrap();
    assert!(session.focus_selected());
    let camera = session.camera();
    let bigger = (camera.position - camera.target).length();
    assert!(
        bigger > distance * 3.0,
        "ten times the size should be much further back: {distance} then {bigger}"
    );
}

#[test]
fn a_tuned_colour_becomes_a_material_every_scene_can_name() {
    let Some((mut session, path)) = open("save-material") else {
        return;
    };
    // The project's library: nothing to set, it is where the project says.
    let library = root_of(&path).join("library");

    // Tune a colour on one object, the way a slider would.
    let crate_id = id(&session, "crate");
    let wanted = Material::new(0.31, 0.12, 0.05);
    session.set_material(crate_id, wanted).unwrap();
    session.save_material(crate_id, "clay").unwrap();

    // What came out is a file a person could have written.
    let source = root_of(&path).join("materials/clay.rmat");
    let text = std::fs::read_to_string(&source).expect("the .rmat source");
    assert!(
        text.contains("color: \"#"),
        "sRGB hex, not a dump of floats: {text}"
    );
    assert!(
        library.join("clay.rmat.rasset").exists(),
        "and it was imported"
    );

    // The entity now names it, and still draws the colour — to within the
    // eight bits a hex code has.
    assert_eq!(session.material_name(crate_id).as_deref(), Some("clay"));
    let drawn = session.material(crate_id).unwrap();
    for axis in 0..3 {
        assert!(
            (drawn.base_color[axis] - wanted.base_color[axis]).abs() < 0.005,
            "channel {axis}: {} against {}",
            drawn.base_color[axis],
            wanted.base_color[axis]
        );
    }

    // And it is in the palette, so the next object can be given the same
    // colour by name rather than by eye.
    assert!(session.palette().iter().any(|(name, _)| name == "clay"));

    // Editing the source and asking for a reload changes what is drawn,
    // with nothing reopened: the loop the whole asset pipeline is for.
    std::fs::write(&source, "(color: \"#3c5a8a\")\n").unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    std::fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_eq!(session.reload_assets(), 1);
    let after = session.material(crate_id).unwrap();
    assert!(
        after.base_color[2] > after.base_color[0],
        "warm clay should have become cool: {after:?}"
    );
}

#[test]
fn a_model_dropped_on_the_editor_becomes_something_a_scene_can_use() {
    let Some((mut session, path)) = open("import") else {
        return;
    };
    let library = root_of(&path).join("library");

    // A triangle is enough: what is being tested is the path, not the
    // parser, which has its own tests next door.
    let source = path.parent().unwrap().join("wedge.obj");
    std::fs::write(
        &source,
        "v -1.0 0.0 -1.0\nv  1.0 0.0 -1.0\nv  1.0 0.0  1.0\nf 1 3 2\n",
    )
    .unwrap();
    session.import(&source).unwrap();
    assert!(library.join("wedge.obj.rasset").exists());

    // And a scene can use it straight away, by the name the file had.
    let added = session.add(None, "wedge").unwrap();
    assert!(session.world_position(added).is_some());

    // A scene in no project has no library to import into, and says so
    // rather than writing somewhere surprising.
    let Some(mut loose) = new_session() else {
        return;
    };
    loose.open_scene(loose_scene_file("import")).unwrap();
    let err = loose.import(&source).unwrap_err();
    assert_eq!(err, EditError::NoLibrary);
    assert!(err.to_string().contains("library"), "{err}");
}

#[test]
fn a_scene_in_no_project_still_edits_and_says_what_it_cannot_do() {
    // Prefabs and materials are written into a project; a scene lying in a
    // plain folder has nowhere to put them. It still opens and edits — and
    // the refusal names the fix.
    let Some(mut session) = new_session() else {
        return;
    };
    session.open_scene(loose_scene_file("refuse")).unwrap();
    assert!(session.project().is_none());
    let crate_id = id(&session, "crate");
    session
        .set_transform(crate_id, Transform::default())
        .unwrap();

    let err = session.make_prefab(crate_id, "crate").unwrap_err();
    assert_eq!(err, EditError::NotInProject);
    assert!(err.to_string().contains("runity.ron"), "{err}");
    assert_eq!(
        session.save_material(crate_id, "clay"),
        Err(EditError::NotInProject)
    );
}

#[test]
fn opening_a_scene_finds_its_project() {
    let Some((session, path)) = open("project") else {
        return;
    };
    let project = session.project().expect("the scene is in a project");
    assert_eq!(
        std::path::absolute(project.root()).unwrap(),
        std::path::absolute(root_of(&path)).unwrap()
    );
    assert_eq!(project.name(), "project");
}

// --- files changed underneath ---------------------------------------------

/// Write a file the way another program would, with its clock pushed on so
/// the change is visible inside one tick of the filesystem's clock.
fn write_elsewhere(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

#[test]
fn a_scene_changed_on_disk_shows_up_and_undo_takes_it_back() {
    // An agent edits the text while the editor is open. The editor shows it
    // without being told, and a person who did not want it presses undo.
    let Some((mut session, path)) = open("reload") else {
        return;
    };
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Unchanged);
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    write_elsewhere(&path, &text.replace("(0.0, 0.5, 0.0)", "(4.0, 0.5, 0.0)"));
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Reloaded);
    assert_eq!(session.transform(crate_id).unwrap().position.x, 4.0);
    assert_eq!(session.selected(), Some(crate_id), "selection survives");
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Unchanged);

    assert!(session.undo().unwrap());
    assert_eq!(session.transform(crate_id).unwrap().position.x, 0.0);
}

#[test]
fn a_scene_changed_on_both_sides_is_a_conflict_and_nothing_is_lost() {
    let Some((mut session, path)) = open("conflict") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session
        .set_transform(crate_id, transform([1.0, 0.5, 0.0], [0.0; 3], [1.0; 3]))
        .unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let theirs = text.replace("\"ground\"", "\"meadow\"");
    write_elsewhere(&path, &theirs);
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Conflict);
    assert_eq!(
        session.transform(crate_id).unwrap().position.x,
        1.0,
        "the session's edit stays"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        theirs,
        "and so does the file"
    );
}

#[test]
fn saving_is_not_mistaken_for_someone_else_changing_the_file() {
    let Some((mut session, _path)) = open("own-save") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session
        .set_transform(crate_id, transform([2.0, 0.5, 0.0], [0.0; 3], [1.0; 3]))
        .unwrap();
    session.save_scene(None).unwrap();
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Unchanged);
    assert!(session.can_undo(), "and the history is untouched");
}

#[test]
fn a_prefab_changed_on_disk_reaches_its_instances() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "prefab-reload",
        r#"(entities: [(id: "a1", name: "fire", prefab: "campfire")])"#,
    );
    let prefab = root_of(&path).join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        r#"(name: "campfire", model: "builtin:cube", children: [(id: "c1", name: "ember", model: "builtin:sphere")])"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    assert_eq!(session.spawned_count(), 2);

    write_elsewhere(
        &prefab,
        r#"(name: "campfire", model: "builtin:cube", children: [
            (id: "c1", name: "ember", model: "builtin:sphere"),
            (id: "c2", name: "stone", model: "builtin:cube"),
        ])"#,
    );
    assert_eq!(session.reload_scene().unwrap(), SceneReload::Reloaded);
    assert_eq!(
        session.spawned_count(),
        3,
        "the new stone is in the instance"
    );
}

#[test]
fn a_game_component_is_set_as_text_saved_as_written_and_undone_in_one_step() {
    let Some((mut session, path)) = open("component") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session
        .set_component(crate_id, "loot", Some("(table: \"barrel\", rolls: 2)"))
        .unwrap();
    session.save_scene(None).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("\"loot\": (table: \"barrel\", rolls: 2)"),
        "{saved}"
    );

    // Not RON: refused, in words, with nothing recorded.
    let err = session
        .set_component(crate_id, "loot", Some("(table: "))
        .unwrap_err();
    assert!(err.to_string().contains("loot"), "{err}");
    assert!(session.undo().unwrap(), "the one real step");
    assert!(session.scene().get(crate_id).unwrap().components.is_empty());
}

#[test]
fn a_scatter_is_one_group_one_step_and_the_same_for_the_same_seed() {
    let Some((mut session, _)) = open("scatter") else {
        return;
    };
    let before = session.entity_count();
    let layout = runity::edit::Scatter {
        radius: 6.0,
        count: 12,
        spacing: 1.0,
        seed: 3,
        ..Default::default()
    };
    let group = session
        .scatter(None, "builtin:cone", Vec3::new(5.0, 0.0, 0.0), &layout)
        .unwrap();
    assert_eq!(session.entity_count(), before + 13, "a group and twelve");
    let first: Vec<Transform> = session
        .scene()
        .get(group)
        .unwrap()
        .children
        .iter()
        .map(|c| c.transform)
        .collect();

    assert!(session.undo().unwrap());
    assert_eq!(
        session.entity_count(),
        before,
        "one step takes all of it back"
    );

    let again = session
        .scatter(None, "builtin:cone", Vec3::new(5.0, 0.0, 0.0), &layout)
        .unwrap();
    let second: Vec<Transform> = session
        .scene()
        .get(again)
        .unwrap()
        .children
        .iter()
        .map(|c| c.transform)
        .collect();
    assert_eq!(first, second, "the same seed, the same layout");
}

#[test]
fn the_scenes_history_is_visible_and_a_past_version_can_be_restored() {
    let Some((mut session, path)) = open("history") else {
        return;
    };
    let root = root_of(&path);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output();
        match out {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    };
    if !git(&["init", "-q"]) {
        eprintln!("skipping: no git");
        return;
    }
    git(&["config", "user.email", "a@runity"]);
    git(&["config", "user.name", "Ada"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the crate at the origin"]);

    let crate_id = id(&session, "crate");
    session
        .set_transform(crate_id, transform([6.0, 0.5, 0.0], [0.0; 3], [1.0; 3]))
        .unwrap();
    session.save_scene(None).unwrap();
    git(&["commit", "-q", "-am", "move the crate"]);

    let revisions = session.scene_history().unwrap();
    assert_eq!(revisions.len(), 2, "{revisions:?}");
    assert_eq!(revisions[0].summary, "move the crate", "newest first");
    assert_eq!(revisions[1].author, "Ada");

    let old = session.scene_at(&revisions[1].commit).unwrap();
    assert_eq!(old.get(crate_id).unwrap().transform.position.x, 0.0);
    assert_eq!(
        session.transform(crate_id).unwrap().position.x,
        6.0,
        "looking is not restoring"
    );

    session.restore_revision(&revisions[1].commit).unwrap();
    assert_eq!(session.transform(crate_id).unwrap().position.x, 0.0);
    assert!(session.undo().unwrap(), "and restoring is one undo step");
    assert_eq!(session.transform(crate_id).unwrap().position.x, 6.0);
}

#[test]
fn a_merge_conflict_is_listed_with_its_values_and_settled_theirs_way() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "conflicts",
        "(entities: [(id: \"00000000000000a1\", name: \"pine\", model: \"builtin:cone\", transform: (position: (0.0, 0.0, 0.0)))])\n",
    );
    let root = root_of(&path);
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !git(&["init", "-q", "-b", "main"]) {
        eprintln!("skipping: no git");
        return;
    }
    git(&["config", "user.email", "a@runity"]);
    git(&["config", "user.name", "Ada"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "start"]);
    git(&["checkout", "-q", "-b", "theirs"]);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)")).unwrap();
    git(&["commit", "-q", "-am", "theirs"]);
    git(&["checkout", "-q", "main"]);
    std::fs::write(&path, text.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)")).unwrap();
    git(&["commit", "-q", "-am", "ours"]);
    assert!(!git(&["merge", "-q", "theirs"]), "a conflict");
    // What the merge driver leaves: ours, loadable.
    git(&["checkout", "--ours", "--", "scenes/scene.ron"]);

    session.open_scene(&path).unwrap();
    let conflicts = session.merge_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert!(conflicts[0]
        .to_string()
        .contains("both changed its position"));
    let pine = id(&session, "pine");
    assert_eq!(session.transform(pine).unwrap().position.x, 5.0);

    session.take_theirs(0).unwrap();
    let now = session.transform(pine).unwrap().position;
    assert_eq!((now.x, now.z), (0.0, 9.0), "theirs, taken");
    assert!(session.undo().unwrap(), "one step");
    assert_eq!(session.transform(pine).unwrap().position.x, 5.0);
}

#[test]
fn editing_a_part_of_an_instance_overrides_it_in_that_instance_only() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "override",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "fire two", prefab: "campfire", transform: (position: (4.0, 0.0, 0.0))),
        ])"#,
    );
    let prefab = root_of(&path).join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        r#"(id: "00000000000000c1", name: "campfire", model: "builtin:cube",
            children: [(id: "00000000000000c2", name: "ember", model: "builtin:sphere", material: "ember")])"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();

    // The ember of the first fire, as the expanded scene names it.
    let one: EntityId = "00000000000000a1".parse().unwrap();
    let ember: EntityId = one.within("00000000000000c2".parse().unwrap());
    session.set_material_name(ember, "moss").unwrap();
    session
        .set_transform(ember, transform([0.0, 2.0, 0.0], [0.0; 3], [1.0; 3]))
        .unwrap();

    assert_eq!(session.material_name(ember).as_deref(), Some("moss"));
    assert_eq!(session.transform(ember).unwrap().position.y, 2.0);
    let two: EntityId = "00000000000000a2".parse().unwrap();
    let other_ember = two.within("00000000000000c2".parse().unwrap());
    assert_eq!(
        session.material_name(other_ember).as_deref(),
        Some("ember"),
        "the other instance keeps the prefab's"
    );

    session.save_scene(None).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("overrides"), "{saved}");
    assert!(
        saved.contains("\"00000000000000c2\""),
        "keyed by the part's id in the prefab: {saved}"
    );
    assert!(
        std::fs::read_to_string(&prefab)
            .unwrap()
            .contains("\"ember\""),
        "the prefab untouched"
    );

    // Two edits, two steps; undoing both leaves no override.
    assert!(session.undo().unwrap());
    assert!(session.undo().unwrap());
    assert_eq!(session.material_name(ember).as_deref(), Some("ember"));
    assert!(session.scene().get(one).unwrap().overrides.is_empty());
}

#[test]
fn a_selection_of_several_moves_duplicates_and_deletes_as_one_step() {
    let Some((mut session, _)) = open("multi") else {
        return;
    };
    let (ground, crate_id, lid) = (
        id(&session, "ground"),
        id(&session, "crate"),
        id(&session, "lid"),
    );
    session.select(Some(crate_id)).unwrap();
    session.add_to_selection(lid).unwrap();
    session.add_to_selection(ground).unwrap();
    assert_eq!(session.selection(), vec![crate_id, lid, ground]);

    // The lid rides on the crate: moved once, with it, not twice.
    session
        .translate_selection(Vec3::new(0.0, 0.0, 3.0))
        .unwrap();
    assert_eq!(session.transform(crate_id).unwrap().position.z, 3.0);
    assert_eq!(
        session.transform(lid).unwrap().position.z,
        0.0,
        "local to the crate"
    );
    assert_eq!(session.transform(ground).unwrap().position.z, 3.0);
    assert!(session.undo().unwrap());
    assert_eq!(
        session.transform(crate_id).unwrap().position.z,
        0.0,
        "one step back"
    );

    let before = session.entity_count();
    let copies = session.duplicate_selection().unwrap();
    assert_eq!(copies.len(), 2, "the crate (with its lid) and the ground");
    assert_eq!(session.entity_count(), before + 3);
    assert_eq!(session.selection(), copies, "the copies are selected");

    assert_eq!(session.delete_selection().unwrap(), 2);
    assert_eq!(session.entity_count(), before);
    assert!(session.selection().is_empty());
}

#[test]
fn copied_entities_paste_as_new_things_here_or_in_another_scene() {
    let Some((mut session, _)) = open("clipboard") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    let text = session.copy_selection();
    assert!(
        text.contains("\"crate\"") && text.contains("\"lid\""),
        "{text}"
    );

    let pasted = session.paste(&text, None).unwrap();
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], crate_id, "a new id");
    let copy = session.scene().get(pasted[0]).unwrap();
    assert_eq!(copy.children.len(), 1);
    assert_ne!(copy.children[0].id, id(&session, "lid"), "all the way down");

    // Into another scene, and from hand-written text too.
    let Some((mut other, _)) = open("clipboard-other") else {
        return;
    };
    let before = other.entity_count();
    other.paste(&text, None).unwrap();
    other
        .paste(r#"(name: "stump", model: "builtin:cylinder")"#, None)
        .unwrap();
    assert_eq!(other.entity_count(), before + 3);
    let err = other.paste("(name: ", None).unwrap_err();
    assert!(err.to_string().contains("not entities in RON"), "{err}");
    assert!(other.undo().unwrap() && other.undo().unwrap());
    assert_eq!(other.entity_count(), before, "a step each");
}

#[test]
fn a_level_answers_whether_the_exit_can_be_reached() {
    let Some((mut session, _)) = open_with(
        "path",
        r#"(entities: [
            (id: "00000000000000a1", name: "floor", model: "builtin:cube",
             transform: (position: (0.0, -0.1, 0.0), scale: (20.0, 0.2, 20.0)),
             body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
            (id: "00000000000000a2", name: "wall", model: "builtin:cube",
             transform: (position: (0.0, 1.0, -2.0), scale: (0.5, 2.0, 16.0)),
             body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
        ])"#,
    ) else {
        return;
    };
    let (spawn, exit) = (Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
    let settings = runity::navigation::NavSettings::default();
    let path = session
        .find_path(spawn, exit, settings)
        .expect("round the wall");
    assert!(path.iter().any(|p| p.z > 6.0), "through the gap: {path:?}");
    let before = session.scene().clone();

    // Close the gap: no way, and asking changed nothing.
    let wall = id(&session, "wall");
    session
        .set_transform(wall, transform([0.0, 1.0, 0.0], [0.0; 3], [0.5, 2.0, 20.0]))
        .unwrap();
    assert!(session.find_path(spawn, exit, settings).is_none());
    assert!(session.undo().unwrap());
    assert_eq!(session.scene(), &before);
}

#[test]
fn dropping_to_the_ground_rests_things_on_the_real_shape_below() {
    let Some((mut session, _)) = open_with(
        "drop",
        r#"(entities: [
            (id: "00000000000000a1", name: "ground", model: "builtin:plane", transform: (scale: (20.0, 1.0, 20.0))),
            (id: "00000000000000a2", name: "ramp", model: "builtin:ramp",
             transform: (position: (5.0, 1.0, 0.0), scale: (2.0, 2.0, 4.0))),
            (id: "00000000000000b1", name: "crate", model: "builtin:cube", transform: (position: (-3.0, 6.0, 0.0))),
            (id: "00000000000000b2", name: "box", model: "builtin:cube",
             transform: (position: (5.0, 8.0, 0.0), scale: (0.4, 0.4, 0.4))),
        ])"#,
    ) else {
        return;
    };
    let (crate_id, box_id) = (id(&session, "crate"), id(&session, "box"));
    session.select(Some(crate_id)).unwrap();
    session.add_to_selection(box_id).unwrap();
    assert_eq!(session.drop_to_ground().unwrap(), 2);

    let y = session.transform(crate_id).unwrap().position.y;
    assert!(
        (y - 0.5).abs() < 0.01,
        "a unit cube on a plane with no collider of its own: {y}"
    );
    let y = session.transform(box_id).unwrap().position.y;
    assert!(
        y > 0.5 && y < 2.2,
        "on the ramp's slope, not on the box around the ramp (top at 2): {y}"
    );
    assert!(session.undo().unwrap(), "one step for both");
    assert_eq!(session.transform(crate_id).unwrap().position.y, 6.0);
}

#[test]
fn the_view_orbits_pans_and_zooms_without_touching_the_document() {
    let Some((mut session, _)) = open("view") else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
    session.orbit(90.0, 0.0);
    let eye = session.camera().position;
    assert!(
        (eye.length() - 10.0).abs() < 1e-3 && eye.z.abs() < 1e-3,
        "a quarter turn round: {eye}"
    );
    session.zoom(0.5);
    assert!((session.camera().position.length() - 5.0).abs() < 1e-3);
    session.pan(1.0, 2.0);
    let camera = session.camera();
    assert!((camera.target.y - 2.0).abs() < 1e-3, "{:?}", camera.target);
    assert!(!session.can_undo(), "the view is not an edit");
}

#[test]
fn applying_overrides_writes_them_into_the_prefab_for_every_instance() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "apply",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "fire two", prefab: "campfire", transform: (position: (4.0, 0.0, 0.0))),
        ])"#,
    );
    let prefab = root_of(&path).join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        "// The camp's fire.\n(id: \"00000000000000c1\", name: \"campfire\", model: \"builtin:cube\",\n    children: [(id: \"00000000000000c2\", name: \"ember\", model: \"builtin:sphere\", material: \"ember\")])\n",
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let (one, two): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
    let part: EntityId = "c2".parse().unwrap();
    session.set_material_name(one.within(part), "moss").unwrap();
    assert_eq!(
        session.material_name(two.within(part)).as_deref(),
        Some("ember")
    );

    assert_eq!(session.apply_overrides(one).unwrap(), 1);
    assert_eq!(
        session.material_name(two.within(part)).as_deref(),
        Some("moss"),
        "every instance has it now"
    );
    assert!(session.scene().get(one).unwrap().overrides.is_empty());
    let text = std::fs::read_to_string(&prefab).unwrap();
    assert!(
        text.starts_with("// The camp's fire."),
        "its text kept: {text}"
    );
    assert!(text.contains("material: \"moss\""), "{text}");

    // Revert: an override made and dropped in one step.
    session.set_material_name(two.within(part), "bark").unwrap();
    session.revert_overrides(two).unwrap();
    assert_eq!(
        session.material_name(two.within(part)).as_deref(),
        Some("moss")
    );
}

#[test]
fn sculpting_a_terrain_writes_a_readable_line_and_reshapes_it_at_once() {
    let Some((mut session, path)) = open_with(
        "sculpt",
        r#"(entities: [(id: "00000000000000a1", name: "valley", model: "hills", transform: (position: (100.0, 0.0, 0.0)))])"#,
    ) else {
        return;
    };
    let source = root_of(&path).join("assets/hills.rterrain");
    std::fs::write(
        &source,
        "// Flat, to begin with.\n(size: (40.0, 40.0), resolution: 41, height: 0.0)\n",
    )
    .unwrap();
    session.reload_assets();
    let valley = id(&session, "valley");

    session
        .sculpt(valley, Vec3::new(105.0, 0.0, 5.0), 4.0, 3.0, false)
        .unwrap();
    let text = std::fs::read_to_string(&source).unwrap();
    assert!(text.starts_with("// Flat, to begin with."), "{text}");
    assert!(
        text.contains("Raise(at:(5.0,5.0),radius:4.0,by:3.0)"),
        "in the terrain's own space: {text}"
    );

    session
        .sculpt(valley, Vec3::new(95.0, 0.0, 0.0), 6.0, 1.5, true)
        .unwrap();
    let text = std::fs::read_to_string(&source).unwrap();
    assert!(text.contains("Flatten("), "{text}");
    assert_eq!(
        text.matches("Raise(").count(),
        1,
        "the first stroke is still there once"
    );

    // The ground rose where the stroke was: a crate dropped there sits up.
    let crate_id = session.add(None, "builtin:cube").unwrap();
    session
        .set_transform(crate_id, transform([105.0, 20.0, 5.0], [0.0; 3], [0.2; 3]))
        .unwrap();
    session.select(Some(crate_id)).unwrap();
    session.drop_to_ground().unwrap();
    let y = session.transform(crate_id).unwrap().position.y;
    assert!((y - 3.1).abs() < 0.15, "on the raised ground: {y}");
}

#[test]
fn renaming_a_material_keeps_the_open_scene_and_its_undo_pointing_at_it() {
    let text = r#"(
    entities: [
        (name: "ground", model: "builtin:plane", material: "clay"),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.5, 0.0)), material: "clay"),
    ],
)"#;
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file("rename", text);
    let root = root_of(&path);
    std::fs::write(root.join("materials/clay.rmat"), "(color: \"#b4643c\")\n").unwrap();
    runity_import::sync(&runity::Project::open(&root).unwrap());
    session.open_scene(&path).unwrap();
    let crate_id = id(&session, "crate");
    let clay = session.material(crate_id).unwrap();

    // An unsaved edit, so there is something to undo across the rename.
    session.rename(crate_id, "box").unwrap();
    let used = session.asset_usages("materials/clay.rmat").unwrap();
    assert_eq!(used.len(), 2, "{used:?}");

    let done = session
        .rename_asset("materials/clay.rmat", "materials/terracotta.rmat")
        .unwrap();
    assert_eq!(done.rewritten, [("scenes/scene.ron".to_string(), 2)]);
    assert_eq!(
        session.material_name(crate_id).as_deref(),
        Some("terracotta")
    );
    assert_eq!(session.material(crate_id), Some(clay), "drawn the same");
    assert_eq!(
        session.entity_name(crate_id).as_deref(),
        Some("box"),
        "the edit kept"
    );
    assert_eq!(
        session.reload_scene().unwrap(),
        SceneReload::Unchanged,
        "its own rewrite is not someone else's edit"
    );

    session.undo().unwrap();
    assert_eq!(session.entity_name(crate_id).as_deref(), Some("crate"));
    assert_eq!(
        session.material_name(crate_id).as_deref(),
        Some("terracotta"),
        "undo does not bring back a name that is gone"
    );
    session.save_scene(None).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("clay"));
}

#[test]
fn an_instance_becomes_a_variant_and_its_overrides_apply_to_the_variant_only() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "variant",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "fire two", prefab: "campfire", transform: (position: (4.0, 0.0, 0.0))),
        ])"#,
    );
    let root = root_of(&path);
    let base = root.join("prefabs/campfire.prefab");
    let base_text = "(id: \"00000000000000c1\", name: \"campfire\", model: \"builtin:cube\",\n    children: [(id: \"00000000000000c2\", name: \"ember\", model: \"builtin:sphere\", material: \"ember\")])\n";
    std::fs::write(&base, base_text).unwrap();
    session.open_scene(&path).unwrap();
    let (one, two): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
    let part: EntityId = "c2".parse().unwrap();

    // A mossy fire, saved as what the next fire placed will be.
    session.set_material_name(one.within(part), "moss").unwrap();
    session.make_variant(one, "mossy").unwrap();
    let variant = std::fs::read_to_string(root.join("prefabs/mossy.prefab")).unwrap();
    assert!(variant.contains("prefab: \"campfire\""), "{variant}");
    assert!(variant.contains("\"00000000000000c2\""), "{variant}");
    assert_eq!(session.entity_prefab(one).as_deref(), Some("mossy"));
    assert!(session.scene().get(one).unwrap().overrides.is_empty());
    assert_eq!(
        session.material_name(one.within(part)).as_deref(),
        Some("moss"),
        "drawn as before"
    );
    assert_eq!(
        session.material_name(two.within(part)).as_deref(),
        Some("ember")
    );

    // An override on the variant's instance, applied: into the variant,
    // never into the base the other fire still uses.
    session
        .set_transform(
            one.within(part),
            transform([0.0, 1.5, 0.0], [0.0; 3], [1.0; 3]),
        )
        .unwrap();
    assert_eq!(session.apply_overrides(one).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&base).unwrap(), base_text);
    let variant = std::fs::read_to_string(root.join("prefabs/mossy.prefab")).unwrap();
    assert!(variant.contains("1.5"), "{variant}");
    assert!(
        variant.contains("moss"),
        "the earlier override kept: {variant}"
    );
    assert_eq!(session.transform(one.within(part)).unwrap().position.y, 1.5);

    // A second instance of the variant is mossy from the start.
    let three = session.add_instance(None, "mossy").unwrap();
    assert_eq!(
        session.material_name(three.within(part)).as_deref(),
        Some("moss")
    );
    assert!(
        session.make_variant(two, "mossy").is_err(),
        "the name is taken"
    );
}

#[test]
fn an_asset_the_open_scene_uses_cannot_be_deleted_even_unsaved() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file("delete-asset", SCENE);
    let root = root_of(&path);
    std::fs::write(root.join("materials/clay.rmat"), "(color: \"#b4643c\")\n").unwrap();
    runity_import::sync(&runity::Project::open(&root).unwrap());
    session.open_scene(&path).unwrap();
    let listed = session.assets().unwrap();
    assert!(listed
        .iter()
        .any(|e| e.file == "materials/clay.rmat" && e.uses == 0));

    // Used only by an edit not saved yet.
    let lid = id(&session, "lid");
    session.set_material_name(lid, "clay").unwrap();
    let e = session.delete_asset("materials/clay.rmat").unwrap_err();
    assert!(
        e.to_string().contains("used by the open scene — `lid`"),
        "{e}"
    );

    session
        .duplicate_asset("materials/clay.rmat", "materials/brick.rmat")
        .unwrap();
    session.set_material_name(lid, "brick").unwrap();
    session.delete_asset("materials/clay.rmat").unwrap();
    assert!(!root.join("materials/clay.rmat").exists());
    assert_eq!(
        session.material(lid),
        Some(runity::Material::from_srgb(0xb4, 0x64, 0x3c)),
        "drawn with the copy"
    );
}

#[test]
fn a_search_finds_lines_and_prefab_parts_alike() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "search",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "gate", model: "builtin:cube", components: { "door": (locked: true) }),
        ])"#,
    );
    std::fs::write(
        root_of(&path).join("prefabs/campfire.prefab"),
        r#"(id: "00000000000000c1", name: "campfire", model: "builtin:cube",
            children: [(id: "00000000000000c2", name: "ember", model: "builtin:sphere", material: "ember")])"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let one: EntityId = "a1".parse().unwrap();
    assert_eq!(session.search("p:campfire").unwrap(), [one]);
    assert_eq!(
        session.search("m:ember").unwrap(),
        [one.within("c2".parse().unwrap())]
    );
    assert_eq!(session.search("c:door").unwrap(), ["a2".parse().unwrap()]);
    assert!(session.search("c:").is_err());
}

#[test]
fn a_prefab_opens_edits_and_saves_like_a_scene_and_its_instances_follow() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "prefab-mode",
        r#"(entities: [(id: "00000000000000a1", name: "fire one", prefab: "campfire")])"#,
    );
    let root = root_of(&path);
    let prefab = root.join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        "// The camp's fire.\n(id: \"00000000000000c1\", name: \"campfire\", model: \"builtin:cube\",\n    children: [(id: \"00000000000000c2\", name: \"ember\", model: \"builtin:sphere\", material: \"ember\")])\n",
    )
    .unwrap();
    std::fs::write(
        root.join("prefabs/mossy.prefab"),
        "(id: \"00000000000000d1\", name: \"mossy\", prefab: \"campfire\")\n",
    )
    .unwrap();
    session.open_scene(&path).unwrap();

    session.open_prefab("campfire").unwrap();
    assert!(session.is_prefab());
    let base: EntityId = "c1".parse().unwrap();
    let ember: EntityId = "c2".parse().unwrap();
    assert_eq!(session.selected(), Some(base), "the root, framed");
    session.set_material_name(ember, "moss").unwrap();
    let stone = session
        .add_entity(
            Some(base),
            runity::EntityDesc {
                name: "stone".into(),
                model: "builtin:cube".into(),
                ..Default::default()
            },
        )
        .unwrap();
    session.save_scene(None).unwrap();
    let text = std::fs::read_to_string(&prefab).unwrap();
    assert!(text.starts_with("// The camp's fire."), "{text}");
    assert!(
        text.contains("\"moss\"") && text.contains("\"stone\""),
        "{text}"
    );

    // A second root cannot be written into one prefab.
    let loose = session
        .add_entity(
            None,
            runity::EntityDesc {
                name: "loose".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let e = session.save_scene(None).unwrap_err();
    assert!(e.to_string().contains("a prefab is one thing"), "{e}");
    session.delete(loose).unwrap();

    // A variant in Prefab Mode: a part edited is an override in the variant.
    session.open_prefab("mossy").unwrap();
    let variant: EntityId = "d1".parse().unwrap();
    assert_eq!(
        session.material_name(variant.within(ember)).as_deref(),
        Some("moss"),
        "the base as just saved"
    );
    session
        .set_material_name(variant.within(ember), "bark")
        .unwrap();
    session.save_scene(None).unwrap();
    let text = std::fs::read_to_string(root.join("prefabs/mossy.prefab")).unwrap();
    assert!(
        text.contains("overrides") && text.contains("\"bark\""),
        "{text}"
    );
    assert!(!std::fs::read_to_string(&prefab).unwrap().contains("bark"));

    // Back in the scene, the instance is what the prefab now says.
    session.open_scene(&path).unwrap();
    assert!(!session.is_prefab());
    let one: EntityId = "a1".parse().unwrap();
    assert_eq!(
        session.material_name(one.within(ember)).as_deref(),
        Some("moss")
    );
    assert!(session.entity_name(one.within(stone)).is_some());
}

#[test]
fn problems_are_known_as_soon_as_an_edit_makes_them() {
    let Some((mut session, _)) = open("problems") else {
        return;
    };
    assert!(session.problems().is_empty(), "{:?}", session.problems());
    let lid = id(&session, "lid");
    session.set_material_name(lid, "stnoe").unwrap();
    session
        .add_entity(
            None,
            runity::EntityDesc {
                name: "tower".into(),
                model: "builtin:cub".into(),
                ..Default::default()
            },
        )
        .unwrap();
    session
        .add_entity(
            None,
            runity::EntityDesc {
                name: "fire".into(),
                prefab: "campfire".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let said: Vec<String> = session.problems().iter().map(ToString::to_string).collect();
    assert_eq!(said.len(), 3, "{said:#?}");
    assert!(
        said.iter()
            .any(|p| p.contains("`fire` (an instance of `campfire`): no prefab by that name")),
        "{said:#?}"
    );
    assert!(
        said.iter()
            .any(|p| p.contains("no model named `builtin:cub` — did you mean `builtin:cube`?")),
        "{said:#?}"
    );
    assert!(
        said.iter().any(|p| p
            .contains("no material named `stnoe` — it draws plain grey — did you mean `stone`?")),
        "{said:#?}"
    );
    assert!(session.problems().iter().any(|p| p.entity == Some(lid)));
    session.undo().unwrap();
    session.undo().unwrap();
    session.undo().unwrap();
    assert!(session.problems().is_empty(), "undone, gone");
}

#[test]
fn the_scene_view_draws_its_grid_until_told_not_to() {
    let text = r#"(entities: [(name: "floor", model: "builtin:plane", material: "white",
        transform: (scale: (40.0, 1.0, 40.0)))])"#;
    let Some((mut session, _)) = open_with("view-grid", text) else {
        return;
    };
    assert!(session.show_grid(), "on by default, as Unity's");
    session.set_camera(Vec3::new(0.3, 8.0, 0.2), Vec3::new(0.3, 0.0, 0.0));
    let spread = |session: &mut Session| {
        session.render();
        let (width, height) = session.size();
        let pixels = session.frame_pixels();
        let row = (height / 2) as usize;
        let luma: Vec<u32> = (0..width as usize)
            .map(|x| {
                let i = (row * width as usize + x) * 4;
                pixels[i] as u32 + pixels[i + 1] as u32 + pixels[i + 2] as u32
            })
            .collect();
        luma.iter().max().unwrap() - luma.iter().min().unwrap()
    };
    let with = spread(&mut session);
    session.set_show_grid(false);
    let without = spread(&mut session);
    assert!(
        with > 60 && without < 12,
        "{with} with the grid, {without} without"
    );
}

#[test]
fn a_grid_surface_shows_its_metres_and_a_plain_one_does_not() {
    // The same floor, seen from above: plain, then with the metre grid.
    let row_contrast = |material: &str| -> Option<(u32, usize)> {
        let text = format!(
            r#"(entities: [(name: "floor", model: "builtin:plane", material: "{material}",
                transform: (scale: (40.0, 1.0, 40.0)))])"#
        );
        let (mut session, _) = open_with(&format!("grid-{material}"), &text)?;
        // The material's lines, not the Scene view's.
        session.set_show_grid(false);
        session.set_camera(Vec3::new(0.3, 8.0, 0.2), Vec3::new(0.3, 0.0, 0.0));
        session.render();
        let (width, height) = session.size();
        let pixels = session.frame_pixels();
        let row = (height / 2) as usize;
        let luma: Vec<u32> = (0..width as usize)
            .map(|x| {
                let i = (row * width as usize + x) * 4;
                pixels[i] as u32 + pixels[i + 1] as u32 + pixels[i + 2] as u32
            })
            .collect();
        let (lo, hi) = (*luma.iter().min()?, *luma.iter().max()?);
        // Dark dips along the row: grid lines crossed.
        let mean = luma.iter().sum::<u32>() / luma.len() as u32;
        let dips = luma
            .windows(2)
            .filter(|w| w[0] >= mean * 9 / 10 && w[1] < mean * 9 / 10)
            .count();
        Some((hi - lo, dips))
    };
    let Some((plain, plain_dips)) = row_contrast("white") else {
        return;
    };
    let (grid, grid_dips) = row_contrast("grid").unwrap();
    assert!(
        plain < 12 && plain_dips == 0,
        "a plain floor is even: {plain}, {plain_dips}"
    );
    assert!(grid > 60, "lines darker than the floor: {grid}");
    assert!(
        grid_dips >= 4,
        "a line per metre across the view: {grid_dips}"
    );
}

#[test]
fn colliders_can_be_shown_as_outlines_coloured_by_body() {
    let text = r#"(entities: [
        (name: "floor", model: "builtin:plane", material: "white", transform: (scale: (12.0, 1.0, 12.0))),
        (name: "wall", model: "", body: Static, collider: Box(half: (0.5, 0.5, 0.5)),
         transform: (position: (-1.5, 1.0, 0.0), scale: (1.0, 2.0, 3.0))),
        (name: "ball", model: "builtin:sphere", body: Dynamic, collider: Sphere(radius: 0.5),
         transform: (position: (1.5, 1.0, 0.0))),
        (name: "zone", model: "", body: Trigger, collider: Capsule(half_height: 0.5, radius: 0.5),
         transform: (position: (0.0, 1.0, 2.5))),
    ])"#;
    let Some((mut session, _)) = open_with("colliders", text) else {
        return;
    };
    session.resize(480, 320);
    session.set_camera(Vec3::new(0.0, 5.0, 8.0), Vec3::new(0.0, 0.8, 0.0));
    let count = |session: &Session, pick: fn(u8, u8, u8) -> bool| {
        session
            .frame_pixels()
            .chunks(4)
            .filter(|p| pick(p[0], p[1], p[2]))
            .count()
    };
    let green = |r: u8, g: u8, b: u8| g as i32 > r as i32 + 60 && g as i32 > b as i32 + 60;
    let yellow = |r: u8, g: u8, b: u8| r > 180 && g > 160 && (b as i32) < r as i32 - 60;
    session.render();
    assert_eq!(count(&session, green), 0, "no outlines unless asked");
    session.set_show_colliders(true);
    session.render();
    if let Ok(dir) = std::env::var("RUNITY_SHOT_DIR") {
        let (w, h) = session.size();
        let img = image::RgbaImage::from_raw(w, h, session.frame_pixels().to_vec()).unwrap();
        img.save(std::path::Path::new(&dir).join("colliders.png"))
            .unwrap();
    }
    assert!(
        count(&session, green) > 30,
        "the invisible wall's box, in green"
    );
    assert!(count(&session, yellow) > 10, "the zone, in yellow");
}

#[test]
fn an_unpacked_instance_is_plain_entities_and_stops_following_the_prefab() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "unpack",
        r#"(entities: [(id: "00000000000000a1", name: "fire", prefab: "campfire",
            overrides: { "00000000000000c2": (material: "moss") })])"#,
    );
    let prefab = root_of(&path).join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        r#"(id: "00000000000000c1", name: "campfire", model: "builtin:cube",
            children: [(id: "00000000000000c2", name: "ember", model: "builtin:sphere", material: "ember")])"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let fire: EntityId = "a1".parse().unwrap();
    let ember = fire.within("c2".parse().unwrap());

    session.unpack_prefab(fire).unwrap();
    let line = session.scene().get(fire).unwrap();
    assert!(line.prefab.is_empty() && line.overrides.is_empty());
    assert_eq!(line.model, "builtin:cube");
    let part = session
        .scene()
        .get(ember)
        .expect("the part is a line of the scene now");
    assert_eq!(
        part.material,
        runity::scene::MaterialRef::Named("moss".into()),
        "with the override applied"
    );
    // One step: undo is the instance again, redo unpacks it again.
    assert!(session.undo().unwrap());
    assert_eq!(session.scene().get(fire).unwrap().prefab, "campfire");
    assert!(session.redo().unwrap());

    // Saved, it no longer follows the prefab file.
    session.save_scene(None).unwrap();
    let text = std::fs::read_to_string(&prefab).unwrap();
    std::fs::write(&prefab, text.replace("builtin:cube", "builtin:cone")).unwrap();
    session.open_scene(&path).unwrap();
    assert_eq!(session.scene().get(fire).unwrap().model, "builtin:cube");
    assert!(
        session.unpack_prefab(fire).is_err(),
        "nothing left to unpack"
    );
}

#[test]
fn greybox_cubes_become_the_real_prefab_where_they_stood() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "replace",
        r#"(entities: [
            (id: "00000000000000a1", name: "crate", model: "builtin:cube", material: "grid", transform: (position: (1.0, 0.5, 0.0))),
            (id: "00000000000000a2", name: "crate", model: "builtin:cube", material: "grid", transform: (position: (4.0, 0.5, 2.0), rotation_deg: (0.0, 30.0, 0.0))),
            (id: "00000000000000a3", name: "wall", model: "builtin:cube", material: "grid"),
        ])"#,
    );
    std::fs::write(
        root_of(&path).join("prefabs/barrel.prefab"),
        r#"(id: "00000000000000c1", name: "barrel", model: "builtin:cylinder", material: "bark")"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    assert_eq!(session.select_matching("crate").unwrap(), 2);
    assert_eq!(session.replace_with_prefab("barrel").unwrap(), 2);
    let second: EntityId = "a2".parse().unwrap();
    let line = session.scene().get(second).unwrap();
    assert_eq!(line.prefab, "barrel");
    assert_eq!(line.name, "crate");
    assert_eq!(
        line.transform.rotation_deg.y, 30.0,
        "where it stood, as it stood"
    );
    assert!(line.model.is_empty());
    assert_eq!(
        session.scene().get("a3".parse().unwrap()).unwrap().prefab,
        "",
        "the wall stays"
    );
    assert!(session.undo().unwrap());
    assert!(
        session.scene().get(second).unwrap().prefab.is_empty(),
        "one step"
    );
    assert!(session.replace_with_prefab("nothing").is_err());
}

#[test]
fn the_hierarchy_and_the_inspector_are_data_a_window_draws() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "panels",
        r#"(entities: [
            (id: "00000000000000a1", name: "camp", model: "", children: [
                (id: "00000000000000a2", name: "tent", model: "builtin:cube"),
            ]),
            (id: "00000000000000a3", name: "fire", prefab: "campfire"),
        ])"#,
    );
    std::fs::write(
        root_of(&path).join("prefabs/campfire.prefab"),
        r#"(id: "00000000000000c1", name: "campfire", model: "builtin:cube",
            children: [(id: "00000000000000c2", name: "ember", model: "builtin:sphere", material: "ember")])"#,
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let names = |session: &Session| -> Vec<(String, usize)> {
        session
            .hierarchy()
            .into_iter()
            .map(|r| (r.name, r.depth))
            .collect()
    };
    // The scene open, the instance closed.
    assert_eq!(
        names(&session),
        [("camp".into(), 0), ("tent".into(), 1), ("fire".into(), 0)]
    );
    let (camp, fire): (EntityId, EntityId) = ("a1".parse().unwrap(), "a3".parse().unwrap());
    session.set_open(camp, false);
    session.set_open(fire, true);
    let rows = session.hierarchy();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["camp", "fire", "ember"]
    );
    assert!(rows[2].part && rows[1].prefab.as_deref() == Some("campfire"));
    assert!(!session.can_undo(), "folding is not an edit");

    // The Inspector: fields as text, set by name, one step each.
    let tent: EntityId = "a2".parse().unwrap();
    let field = |session: &Session, id, name: &str| {
        session
            .inspect(id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap()
    };
    assert_eq!(field(&session, tent, "model").value, "builtin:cube");
    session
        .set_field(tent, "position", "(2.0, 0.0, 1.0)")
        .unwrap();
    session.set_field(tent, "material", "stone").unwrap();
    session
        .set_field(tent, "components.door", "(locked: true)")
        .unwrap();
    assert_eq!(session.transform(tent).unwrap().position.x, 2.0);
    assert_eq!(field(&session, tent, "material").value, "\"stone\"");
    assert_eq!(
        field(&session, tent, "components.door").value,
        "(locked: true)"
    );
    assert_eq!(session.undo_steps().len(), 3);
    let e = session
        .set_field(tent, "position", "(2.0, oops)")
        .unwrap_err();
    assert!(e.to_string().contains("position"), "{e}");
    assert_eq!(session.undo_steps().len(), 3, "a typo costs no step");
    let e = session.set_field(tent, "colour", "red").unwrap_err();
    assert!(e.to_string().contains("no field `colour`"), "{e}");

    // A prefab's part: its fields say which this instance overrides.
    let ember = fire.within("c2".parse().unwrap());
    assert!(!field(&session, ember, "material").overridden);
    session.set_field(ember, "material", "moss").unwrap();
    assert!(field(&session, ember, "material").overridden);
    assert!(!field(&session, ember, "name").overridden);
}

/// One frame of input to the Scene view: the mouse at `at`, then events.
fn view_frame(
    session: &mut Session,
    input: &mut runity::input::Input,
    at: (f32, f32),
    events: &[runity::input::InputEvent],
) -> Vec<&'static str> {
    input.begin_frame();
    input.handle(&runity::input::InputEvent::MouseMoved { x: at.0, y: at.1 });
    for e in events {
        input.handle(e);
    }
    session.scene_view(input, 1.0 / 60.0).unwrap()
}

#[test]
fn the_scene_view_does_what_a_mouse_and_keyboard_ask() {
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let Some((mut session, _)) = open("scene-view") else {
        return;
    };
    let mut input = Input::new();
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let (w, h) = session.size();
    let centre = (w as f32 / 2.0, h as f32 / 2.0);

    // A click on empty sky clears; a click on the crate selects it.
    view_frame(
        &mut session,
        &mut input,
        (1.0, 1.0),
        &[E::MouseDown(M::Left)],
    );
    view_frame(&mut session, &mut input, (1.0, 1.0), &[E::MouseUp(M::Left)]);
    assert_eq!(session.selected(), None);
    let did = view_frame(&mut session, &mut input, centre, &[E::MouseDown(M::Left)]);
    view_frame(&mut session, &mut input, centre, &[E::MouseUp(M::Left)]);
    assert!(did.contains(&"select") || did.contains(&"grab"), "{did:?}");
    assert!(session.selected().is_some());
    session.select(Some(crate_id)).unwrap();

    // Tools by letter.
    view_frame(&mut session, &mut input, centre, &[E::KeyDown(Key::E)]);
    assert_eq!(session.tool(), runity::gizmo::Tool::Rotate);
    view_frame(
        &mut session,
        &mut input,
        centre,
        &[E::KeyUp(Key::E), E::KeyDown(Key::W)],
    );
    assert_eq!(session.tool(), runity::gizmo::Tool::Move);
    view_frame(&mut session, &mut input, centre, &[E::KeyUp(Key::W)]);

    // A handle dragged is one undo step.
    let (_, r) = grab_right_of_centre(&mut session).expect("a handle to the right");
    session.gizmo_end();
    let grab = (centre.0 + r as f32, centre.1);
    let before = session.transform(crate_id).unwrap().position;
    let did = view_frame(&mut session, &mut input, grab, &[E::MouseDown(M::Left)]);
    assert!(did.contains(&"grab"), "{did:?}");
    let did = view_frame(&mut session, &mut input, (grab.0 + 20.0, grab.1), &[]);
    assert!(did.contains(&"drag"), "{did:?}");
    view_frame(
        &mut session,
        &mut input,
        (grab.0 + 20.0, grab.1),
        &[E::MouseUp(M::Left)],
    );
    assert_ne!(session.transform(crate_id).unwrap().position, before);

    // Ctrl Z takes the drag back in one; Ctrl D duplicates; Delete deletes.
    let did = view_frame(
        &mut session,
        &mut input,
        centre,
        &[E::KeyDown(Key::LeftControl), E::KeyDown(Key::Z)],
    );
    assert!(did.contains(&"undo"), "{did:?}");
    assert_eq!(session.transform(crate_id).unwrap().position, before);
    let count = session.entity_count();
    view_frame(
        &mut session,
        &mut input,
        centre,
        &[E::KeyUp(Key::Z), E::KeyDown(Key::D)],
    );
    assert!(session.entity_count() > count);
    view_frame(
        &mut session,
        &mut input,
        centre,
        &[E::KeyUp(Key::D), E::KeyUp(Key::LeftControl)],
    );
    let did = view_frame(&mut session, &mut input, centre, &[E::KeyDown(Key::Delete)]);
    assert!(did.contains(&"delete"), "{did:?}");
    assert_eq!(session.entity_count(), count);
    view_frame(&mut session, &mut input, centre, &[E::KeyUp(Key::Delete)]);

    // The wheel zooms, the middle button pans; neither is an edit.
    let steps = session.undo_steps().len();
    let far = (session.camera().position - session.camera().target).length();
    view_frame(
        &mut session,
        &mut input,
        centre,
        &[E::Scroll { x: 0.0, y: 3.0 }],
    );
    let near = (session.camera().position - session.camera().target).length();
    assert!(near < far * 0.8, "{far} -> {near}");
    let target = session.camera().target;
    view_frame(&mut session, &mut input, centre, &[E::MouseDown(M::Middle)]);
    view_frame(&mut session, &mut input, (centre.0 + 30.0, centre.1), &[]);
    assert_ne!(session.camera().target, target, "panned");
    view_frame(&mut session, &mut input, centre, &[E::MouseUp(M::Middle)]);
    assert_eq!(session.undo_steps().len(), steps, "the view is not an edit");

    view_frame(&mut session, &mut input, centre, &[E::KeyDown(Key::Escape)]);
    assert!(session.selection().is_empty());
}

#[test]
fn hidden_things_are_neither_drawn_nor_picked_and_a_box_takes_what_it_touches() {
    use runity::glam::Vec2;
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let Some((mut session, path)) = open("visibility") else {
        return;
    };
    let (crate_id, lid, ground) = (
        id(&session, "crate"),
        id(&session, "lid"),
        id(&session, "ground"),
    );
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let (w, h) = session.size();
    let (cx, cy) = (w / 2, h / 2);
    assert!(matches!(session.pick(cx, cy), Some(i) if i == crate_id || i == lid));
    let row = |s: &Session, id| s.hierarchy().into_iter().find(|r| r.id == id).unwrap();

    // Hidden with what is under it: not picked, marked in the tree, and
    // nothing the scene or its history knows about.
    session.set_hidden(&[crate_id], true).unwrap();
    assert!(!matches!(session.pick(cx, cy), Some(i) if i == crate_id || i == lid));
    assert!(row(&session, crate_id).hidden && row(&session, lid).hidden);
    assert!(!row(&session, ground).hidden);
    assert!(!session.can_undo(), "a view setting, not an edit");
    session.render();
    session.save_scene(None).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("hidden"));
    session.set_hidden(&[crate_id], false).unwrap();

    // Isolated: only it and what is under it.
    session.isolate(&[crate_id]).unwrap();
    assert!(row(&session, ground).hidden && !row(&session, lid).hidden);
    let taken = session
        .select_in_rect(Vec2::ZERO, Vec2::new(w as f32, h as f32), false)
        .unwrap();
    assert!(!taken.contains(&ground), "{taken:?}");
    session.show_all();

    // A box over the whole view takes everything drawn in it.
    let taken = session
        .select_in_rect(Vec2::ZERO, Vec2::new(w as f32, h as f32), false)
        .unwrap();
    for want in [crate_id, lid, ground] {
        assert!(taken.contains(&want), "{taken:?}");
    }
    assert_eq!(session.selection().len(), taken.len());

    // The same by hand: dragged from empty sky, drawn while held.
    let mut input = Input::new();
    session.select(None).unwrap();
    view_frame(
        &mut session,
        &mut input,
        (1.0, 1.0),
        &[E::MouseDown(M::Left)],
    );
    let far = (w as f32 - 1.0, h as f32 - 1.0);
    view_frame(&mut session, &mut input, far, &[]);
    assert_eq!(session.marquee(), Some((Vec2::ONE, Vec2::from(far))));
    let did = view_frame(&mut session, &mut input, far, &[E::MouseUp(M::Left)]);
    assert!(did.contains(&"box select"), "{did:?}");
    assert!(session.marquee().is_none());
    assert!(session.selection().contains(&crate_id));

    // H hides the selection and shows it again; Shift H isolates.
    session.select(Some(crate_id)).unwrap();
    let did = view_frame(&mut session, &mut input, far, &[E::KeyDown(Key::H)]);
    assert!(did.contains(&"hide"), "{did:?}");
    assert_eq!(session.hidden(), [crate_id]);
    view_frame(&mut session, &mut input, far, &[E::KeyUp(Key::H)]);
    let did = view_frame(&mut session, &mut input, far, &[E::KeyDown(Key::H)]);
    assert!(did.contains(&"show"), "{did:?}");
    assert!(session.hidden().is_empty());
    view_frame(&mut session, &mut input, far, &[E::KeyUp(Key::H)]);
    let did = view_frame(
        &mut session,
        &mut input,
        far,
        &[E::KeyDown(Key::LeftShift), E::KeyDown(Key::H)],
    );
    assert!(did.contains(&"isolate"), "{did:?}");
    assert_eq!(session.isolated(), [crate_id]);
}

#[test]
fn an_axis_view_is_a_plan_that_pans_zooms_and_picks() {
    use runity_editor::Side;
    let Some((mut session, _)) = open("axis-views") else {
        return;
    };
    let (crate_id, lid) = (id(&session, "crate"), id(&session, "lid"));
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let (w, h) = session.size();

    // From above: the lid is what the middle of the plan shows.
    session.look_from(Side::Top);
    assert!(session.is_orthographic());
    let camera = session.camera();
    assert!(camera.position.y > camera.target.y + 10.0);
    assert_eq!(session.pick(w / 2, h / 2), Some(lid));
    session.render();
    let centre = ((h / 2 * w + w / 2) * 4) as usize;
    let sky = session.frame_pixels()[4..8].to_vec();
    assert_ne!(session.frame_pixels()[centre..centre + 4], sky[..]);

    // Zoom shows more, not nearer; pan slides the plan.
    let before = session.camera();
    session.zoom(2.0);
    assert_eq!(session.camera().position, before.position);
    assert!(session.camera().ortho.unwrap() > before.ortho.unwrap());
    session.pan(3.0, 0.0);
    let moved = session.camera().target - before.target;
    assert!(
        (moved.x - 3.0).abs() < 1e-3 && moved.y.abs() < 1e-3,
        "{moved}"
    );

    // Framing still frames; back to perspective shows about as much.
    session.focus_selected();
    let half = session.camera().ortho.unwrap();
    session.set_orthographic(false);
    let camera = session.camera();
    let seen = (camera.position - camera.target).length()
        * (camera.fov_y_degrees.to_radians() * 0.5).tan();
    assert!((seen - half).abs() < 1e-3, "{seen} vs {half}");
    session.orbit(10.0, -20.0);
    assert_eq!(session.camera().up, runity::glam::Vec3::Y);

    // Every side has a name an agent can say.
    for side in Side::ALL {
        assert_eq!(Side::from_name(side.name()), Some(side));
        session.look_from(side);
        session.render();
    }
}

#[test]
fn a_right_drag_looks_and_wasd_flies_without_touching_the_tools() {
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let Some((mut session, _)) = open("flythrough") else {
        return;
    };
    let mut input = Input::new();
    let tool = session.tool();
    let start = session.camera();
    let at = (50.0, 50.0);

    // Turning the head keeps the feet where they are.
    view_frame(&mut session, &mut input, at, &[E::MouseDown(M::Right)]);
    let did = view_frame(&mut session, &mut input, (80.0, 50.0), &[]);
    assert!(did.contains(&"look"), "{did:?}");
    assert_eq!(session.camera().position, start.position);
    assert_ne!(session.camera().target, start.target);

    // W flies where it looks, a second's worth over sixty frames; the
    // tool stays what it was.
    let looking = session.camera();
    let ahead = (looking.target - looking.position).normalize();
    view_frame(
        &mut session,
        &mut input,
        (80.0, 50.0),
        &[E::KeyDown(Key::W)],
    );
    for _ in 0..59 {
        view_frame(&mut session, &mut input, (80.0, 50.0), &[]);
    }
    let went = session.camera().position - looking.position;
    assert!((went.length() - session.fly_speed()).abs() < 0.01, "{went}");
    assert!(went.normalize().dot(ahead) > 0.999);
    assert_eq!(
        session.tool(),
        tool,
        "W flew rather than picking the move tool"
    );

    // The wheel while flying sets the speed rather than zooming.
    let speed = session.fly_speed();
    let camera = session.camera();
    input.begin_frame();
    input.handle(&E::MouseMoved { x: 80.0, y: 50.0 });
    input.handle(&E::KeyUp(Key::W));
    input.handle(&E::Scroll { x: 0.0, y: 1.0 });
    let did = session.scene_view(&input, 1.0 / 60.0).unwrap();
    assert!(did.contains(&"fly speed"), "{did:?}");
    assert!(session.fly_speed() > speed);
    assert_eq!(session.camera().position, camera.position);
    view_frame(
        &mut session,
        &mut input,
        (80.0, 50.0),
        &[E::MouseUp(M::Right)],
    );

    // From an axis view, looking around goes back to perspective.
    session.look_from(runity_editor::Side::Top);
    session.look(10.0, 0.0);
    assert!(!session.is_orthographic());
}

#[test]
fn ctrl_shift_puts_the_selection_on_whatever_the_cursor_is_over() {
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let scene = SCENE.replace(
        "        (\n            name: \"crate\",",
        "        (name: \"table\", model: \"builtin:cube\", transform: (position: (4.0, 0.5, 0.0), scale: (2.0, 1.0, 2.0))),\n        (\n            name: \"crate\",",
    );
    let Some((mut session, _)) = open_with("surface", &scene) else {
        return;
    };
    let (crate_id, table) = (id(&session, "crate"), id(&session, "table"));
    session.select(Some(crate_id)).unwrap();
    session.look_from(runity_editor::Side::Top);
    let (w, h) = session.size();
    let size = runity::glam::Vec2::new(w as f32, h as f32);
    let over = |session: &Session, p: runity::glam::Vec3| {
        let at = session.camera().screen_point(p, size).unwrap();
        (at.x as u32, at.y as u32)
    };

    // Onto the table's top, by the bottom of the crate, lid and all.
    let (x, y) = over(&session, runity::glam::Vec3::new(4.3, 1.0, 0.2));
    assert!(session.place_on_surface(x, y).unwrap());
    let (low, high) = session.world_bounds(crate_id).unwrap();
    assert!((low.y - 1.0).abs() < 1e-3, "on the top: {low}");
    let middle = (low + high) * 0.5;
    assert!(
        (middle.x - 4.3).abs() < 0.1 && (middle.z - 0.2).abs() < 0.1,
        "{middle}"
    );
    let (table_low, _) = session.world_bounds(table).unwrap();
    assert_eq!(table_low.y, 0.0, "the table did not move");

    session.undo().unwrap();
    assert!((session.world_bounds(crate_id).unwrap().0.y).abs() < 1e-3);

    // By hand: a move handle grabbed, then Ctrl Shift and over the table.
    session.focus_selected();
    let (_, r) = grab_right_of_centre(&mut session).expect("a handle");
    session.gizmo_end();
    let grab = ((w / 2 + r) as f32, (h / 2) as f32);
    let mut input = Input::new();
    view_frame(&mut session, &mut input, grab, &[E::MouseDown(M::Left)]);
    let (x, y) = over(&session, runity::glam::Vec3::new(4.0, 1.0, 0.0));
    let did = view_frame(
        &mut session,
        &mut input,
        (x as f32, y as f32),
        &[E::KeyDown(Key::LeftControl), E::KeyDown(Key::LeftShift)],
    );
    assert!(did.contains(&"place"), "{did:?}");
    view_frame(
        &mut session,
        &mut input,
        (x as f32, y as f32),
        &[
            E::MouseUp(M::Left),
            E::KeyUp(Key::LeftControl),
            E::KeyUp(Key::LeftShift),
        ],
    );
    assert!((session.world_bounds(crate_id).unwrap().0.y - 1.0).abs() < 1e-3);
    session.undo().unwrap();
    assert!(
        (session.world_bounds(crate_id).unwrap().0.y).abs() < 1e-3,
        "the whole gesture is one step"
    );

    // End drops what is selected onto what is beneath it.
    session
        .set_transform(
            crate_id,
            runity::Transform {
                position: runity::glam::Vec3::new(4.0, 5.0, 0.0),
                ..Default::default()
            },
        )
        .unwrap();
    let did = view_frame(
        &mut session,
        &mut input,
        (1.0, 1.0),
        &[E::KeyDown(Key::End)],
    );
    assert!(did.contains(&"drop to ground"), "{did:?}");
    assert!((session.world_bounds(crate_id).unwrap().0.y - 1.0).abs() < 1e-3);
}

#[test]
fn a_handle_moves_turns_and_stretches_everything_selected() {
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"barrel\", model: \"builtin:cube\", transform: (position: (-3.0, 0.5, 1.0))),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("multi-gizmo", &scene) else {
        return;
    };
    let (crate_id, barrel) = (id(&session, "crate"), id(&session, "barrel"));
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    session.add_to_selection(barrel).unwrap();
    let (w, h) = session.size();
    let crate_from = session.world_position(crate_id).unwrap();
    let barrel_from = session.world_position(barrel).unwrap();

    let (handle, r) = grab_right_of_centre(&mut session).expect("a handle");
    assert_eq!(handle, Handle::X);
    session.gizmo_drag(w / 2 + r + 15, h / 2).unwrap();
    session.gizmo_end();
    let went = session.world_position(crate_id).unwrap() - crate_from;
    assert!(went.x > 0.01 && went.y.abs() < 1e-4, "{went}");
    let barrel_went = session.world_position(barrel).unwrap() - barrel_from;
    assert!(
        (barrel_went - went).length() < 1e-4,
        "{barrel_went} vs {went}"
    );
    session.undo().unwrap();
    assert_eq!(
        session.world_position(barrel).unwrap(),
        barrel_from,
        "one step"
    );

    // Stretched by the same factor, each about itself.
    session.set_tool(runity::gizmo::Tool::Scale);
    let (_, r) = grab_right_of_centre(&mut session).expect("a scale handle");
    session.gizmo_drag(w / 2 + r + 15, h / 2).unwrap();
    session.gizmo_end();
    let crate_scale = session.transform(crate_id).unwrap().scale;
    let barrel_scale = session.transform(barrel).unwrap().scale;
    assert!(crate_scale.x > 1.01, "{crate_scale}");
    assert_eq!(crate_scale, barrel_scale);
    assert_eq!(session.world_position(barrel).unwrap(), barrel_from);
    session.undo().unwrap();

    // Turned by as much, each about its own pivot.
    session.set_tool(runity::gizmo::Tool::Rotate);
    let (_, r) = grab_right_of_centre(&mut session).expect("a ring");
    session.gizmo_drag(w / 2 + r, h / 2 + 12).unwrap();
    session.gizmo_end();
    let crate_turn = session.transform(crate_id).unwrap().rotation_deg;
    let barrel_turn = session.transform(barrel).unwrap().rotation_deg;
    assert!(crate_turn.length() > 0.1, "{crate_turn}");
    assert!((crate_turn - barrel_turn).length() < 1e-3, "{barrel_turn}");
    assert_eq!(session.world_position(barrel).unwrap(), barrel_from);
}

#[test]
fn what_is_selected_is_outlined_in_orange() {
    let Some((mut session, _)) = open("outline") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let orange = |session: &Session| {
        session
            .frame_pixels()
            .chunks(4)
            .filter(|p| p[0] > 200 && (60..180).contains(&p[1]) && p[2] < 60)
            .count()
    };
    session.render();
    assert!(orange(&session) > 20, "{}", orange(&session));
    session.set_hidden(&[crate_id], true).unwrap();
    session.render();
    assert_eq!(orange(&session), 0, "hidden, so not outlined either");
    session.set_hidden(&[crate_id], false).unwrap();
    session.select(None).unwrap();
    session.render();
    assert_eq!(orange(&session), 0);
}

#[test]
fn local_handles_slide_a_turned_wall_along_its_own_length() {
    use runity::glam::{Vec2, Vec3};
    use runity::input::{Input, InputEvent as E, Key};
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"wall\", model: \"builtin:cube\", transform: (position: (0.0, 1.0, 0.0), rotation_deg: (0.0, 90.0, 0.0), scale: (4.0, 2.0, 0.2))),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("local-handles", &scene) else {
        return;
    };
    let wall = id(&session, "wall");
    session.select(Some(wall)).unwrap();
    session.set_camera(Vec3::new(8.0, 4.0, 3.0), Vec3::new(0.0, 1.0, 0.0));
    let (w, h) = session.size();
    let size = Vec2::new(w as f32, h as f32);
    let origin = session.world_position(wall).unwrap();
    // The wall's own x is the world's −z.
    let along = Vec3::NEG_Z;
    let arm = session.camera().apparent_distance(origin) * 0.15;
    let camera = session.camera();
    let pixel = |p: Vec3| {
        let at = camera.screen_point(p, size).unwrap();
        (at.x as u32, at.y as u32)
    };

    let mut input = Input::new();
    let did = view_frame(&mut session, &mut input, (1.0, 1.0), &[E::KeyDown(Key::X)]);
    assert!(did.contains(&"space"), "{did:?}");
    assert_eq!(session.space(), runity_editor::Space::Local);

    let (x, y) = pixel(origin + along * arm * 0.7);
    assert_eq!(session.gizmo_begin(x, y).unwrap(), Some(Handle::X));
    let (x, y) = pixel(origin + along * arm * 1.5);
    session.gizmo_drag(x, y).unwrap();
    session.gizmo_end();
    let went = session.world_position(wall).unwrap() - origin;
    assert!(went.z < -0.05, "{went}");
    assert!(
        went.x.abs() < 1e-3 && went.y.abs() < 1e-3,
        "only along its length: {went}"
    );
}

#[test]
fn in_center_mode_the_selection_turns_about_its_middle() {
    use runity::glam::Vec3;
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"barrel\", model: \"builtin:cube\", transform: (position: (4.0, 0.5, 0.0))),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("center-pivot", &scene) else {
        return;
    };
    let (crate_id, barrel) = (id(&session, "crate"), id(&session, "barrel"));
    session.select(Some(crate_id)).unwrap();
    session.add_to_selection(barrel).unwrap();
    session.set_pivot(runity_editor::Pivot::Center);
    // The middle of the box around both: x halfway, y halfway up the lid.
    let middle = Vec3::new(2.0, 0.8, 0.0);
    session.set_camera(middle + Vec3::new(1.0, 3.0, 9.0), middle);
    session.set_tool(runity::gizmo::Tool::Rotate);
    let (w, h) = session.size();
    let crate_from = session.world_position(crate_id).unwrap();
    let barrel_from = session.world_position(barrel).unwrap();

    let (_, r) = grab_right_of_centre(&mut session).expect("a ring");
    session.gizmo_drag(w / 2 + r, h / 2 + 15).unwrap();
    session.gizmo_end();
    let crate_to = session.world_position(crate_id).unwrap();
    let barrel_to = session.world_position(barrel).unwrap();
    assert!(
        (crate_to - crate_from).length() > 0.05,
        "went round: {crate_to}"
    );
    for (from, to) in [(crate_from, crate_to), (barrel_from, barrel_to)] {
        let (before, after) = ((from - middle).length(), (to - middle).length());
        assert!((before - after).abs() < 1e-3, "{before} vs {after}");
    }
    assert_eq!(
        session.transform(crate_id).unwrap().rotation_deg,
        session.transform(barrel).unwrap().rotation_deg
    );
}

#[test]
fn the_inspector_edits_several_things_at_once_in_one_step() {
    let Some((mut session, _)) = open("multi-edit") else {
        return;
    };
    let (crate_id, lid, ground) = (
        id(&session, "crate"),
        id(&session, "lid"),
        id(&session, "ground"),
    );
    let fields = session.inspect_all(&[crate_id, lid]).unwrap();
    let value = |fields: &[runity_editor::panels::Field], name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value(&fields, "model"), "builtin:cube", "they agree");
    assert_eq!(value(&fields, "name"), runity_editor::panels::MIXED);
    assert_eq!(value(&fields, "position"), runity_editor::panels::MIXED);

    let steps = session.undo_steps().len();
    session
        .set_field_all(&[crate_id, lid, ground], "layer", "debris")
        .unwrap();
    assert_eq!(session.undo_steps().len(), steps + 1, "one step");
    let fields = session.inspect_all(&[crate_id, lid, ground]).unwrap();
    assert_eq!(value(&fields, "layer"), "debris");
    session.undo().unwrap();
    for id in [crate_id, lid, ground] {
        assert_eq!(value(&session.inspect(id).unwrap(), "layer"), "");
    }

    // Text that does not parse changes none of them.
    assert!(session
        .set_field_all(&[crate_id, lid], "scale", "(big)")
        .is_err());
    assert_eq!(session.undo_steps().len(), steps);
}

#[test]
fn a_hierarchy_drag_reorders_and_reparents_without_moving_anything_in_the_world() {
    let Some((mut session, _)) = open("hierarchy-drag") else {
        return;
    };
    let (ground, crate_id, lid) = (
        id(&session, "ground"),
        id(&session, "crate"),
        id(&session, "lid"),
    );
    let names =
        |s: &Session| -> Vec<String> { s.hierarchy().into_iter().map(|r| r.name).collect() };

    // Between two lines: the ground after the crate.
    assert!(session.move_in_hierarchy(ground, None, Some(1)).unwrap());
    assert_eq!(names(&session), ["crate", "lid", "ground"]);

    // Out of its parent to the top, staying where it is in the world.
    session
        .set_transform(
            crate_id,
            runity::Transform {
                position: runity::glam::Vec3::new(2.0, 0.5, 0.0),
                rotation_deg: runity::glam::Vec3::new(0.0, 90.0, 0.0),
                ..Default::default()
            },
        )
        .unwrap();
    let world = session.world_position(lid).unwrap();
    assert!(session.move_in_hierarchy(lid, None, Some(0)).unwrap());
    assert_eq!(names(&session), ["lid", "crate", "ground"]);
    assert!((session.world_position(lid).unwrap() - world).length() < 1e-4);
    assert!((session.transform(lid).unwrap().rotation_deg.y - 90.0).abs() < 1e-3);

    // And back under it, still where it is.
    assert!(session
        .move_in_hierarchy(lid, Some(crate_id), None)
        .unwrap());
    assert!((session.world_position(lid).unwrap() - world).length() < 1e-4);
    assert!(session.transform(lid).unwrap().rotation_deg.length() < 1e-3);

    // Never under itself.
    assert!(!session
        .move_in_hierarchy(crate_id, Some(lid), None)
        .unwrap());
    session.undo().unwrap();
    assert_eq!(names(&session), ["lid", "crate", "ground"]);
}

#[test]
fn move_to_view_and_align_with_view_place_things_from_the_camera() {
    use runity::glam::Vec3;
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"eye\", camera: (fov_deg: 50.0)),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("to-view", &scene) else {
        return;
    };
    let (crate_id, lid, eye) = (
        id(&session, "crate"),
        id(&session, "lid"),
        id(&session, "eye"),
    );
    let lid_above =
        session.world_position(lid).unwrap() - session.world_position(crate_id).unwrap();

    // The crate to where the view looks, its lid with it.
    session.set_camera(Vec3::new(9.0, 6.0, 9.0), Vec3::new(5.0, 0.0, -3.0));
    session.select(Some(crate_id)).unwrap();
    assert!(session.move_to_view().unwrap());
    assert!(
        (session.world_position(crate_id).unwrap() - Vec3::new(5.0, 0.0, -3.0)).length() < 1e-4
    );
    assert!(
        (session.world_position(lid).unwrap()
            - session.world_position(crate_id).unwrap()
            - lid_above)
            .length()
            < 1e-4
    );

    // The game's camera to where the view is, looking where it looks.
    session.select(Some(eye)).unwrap();
    assert!(session.align_with_view().unwrap());
    let game = session.game_camera().expect("the eye is a camera");
    let view = session.camera();
    assert!(
        (game.position - view.position).length() < 1e-3,
        "{}",
        game.position
    );
    let (a, b) = (
        (game.target - game.position).normalize(),
        (view.target - view.position).normalize(),
    );
    assert!(a.dot(b) > 0.9999, "{a} vs {b}");
    session.undo().unwrap();
    assert!(
        session.world_position(eye).unwrap().length() < 1e-4,
        "one step"
    );
}

#[test]
fn v_snaps_a_vertex_of_the_selection_onto_a_vertex_of_something_else() {
    use runity::glam::{Vec2, Vec3};
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let scene = r#"(
    entities: [
        (name: "a", model: "builtin:cube", transform: (position: (0.0, 0.5, 0.0))),
        (name: "b", model: "builtin:cube", transform: (position: (3.0, 0.5, 0.2))),
    ],
)"#;
    let Some((mut session, _)) = open_with("vertex-snap", scene) else {
        return;
    };
    let (a, b) = (id(&session, "a"), id(&session, "b"));
    session.select(Some(a)).unwrap();
    session.set_camera(Vec3::new(1.5, 6.0, 7.0), Vec3::new(1.5, 0.5, 0.0));
    let (w, h) = session.size();
    let camera = session.camera();
    let pixel = |p: Vec3| {
        let at = camera
            .screen_point(p, Vec2::new(w as f32, h as f32))
            .unwrap();
        (at.x, at.y)
    };
    // a's top right front corner onto b's top left front corner.
    let from = pixel(Vec3::new(0.5, 1.0, 0.5));
    let to = pixel(Vec3::new(2.5, 1.0, 0.7));
    let mut input = Input::new();
    let did = view_frame(
        &mut session,
        &mut input,
        from,
        &[E::KeyDown(Key::V), E::MouseDown(M::Left)],
    );
    assert!(did.contains(&"vertex grab"), "{did:?}");
    let did = view_frame(&mut session, &mut input, to, &[]);
    assert!(did.contains(&"vertex snap"), "{did:?}");
    view_frame(
        &mut session,
        &mut input,
        to,
        &[E::MouseUp(M::Left), E::KeyUp(Key::V)],
    );
    let moved = session.world_position(a).unwrap();
    assert!(
        (moved - Vec3::new(2.0, 0.5, 0.2)).length() < 1e-4,
        "{moved}"
    );
    assert_eq!(session.world_position(b).unwrap(), Vec3::new(3.0, 0.5, 0.2));
    session.undo().unwrap();
    assert_eq!(
        session.world_position(a).unwrap(),
        Vec3::new(0.0, 0.5, 0.0),
        "one step"
    );
}

#[test]
fn one_overridden_field_of_a_part_is_applied_or_reverted_on_its_own() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "field-overrides",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "fire two", prefab: "campfire", transform: (position: (4.0, 0.0, 0.0))),
        ])"#,
    );
    let prefab = root_of(&path).join("prefabs/campfire.prefab");
    std::fs::write(
        &prefab,
        "(id: \"00000000000000c1\", name: \"campfire\", model: \"builtin:cube\",\n    children: [(id: \"00000000000000c2\", name: \"ember\", model: \"builtin:sphere\", material: \"ember\")])\n",
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let (one, two): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
    let part: EntityId = "c2".parse().unwrap();
    let ember = one.within(part);
    session.set_field(ember, "material", "moss").unwrap();
    session.set_field(ember, "layer", "debris").unwrap();
    session.set_field(ember, "name", "coal").unwrap();
    let overridden = |s: &Session, name: &str| {
        s.inspect(ember)
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap()
            .overridden
    };

    // Revert the name alone.
    assert!(session.revert_field(ember, "name").unwrap());
    assert_eq!(session.entity_name(ember).as_deref(), Some("ember"));
    assert!(overridden(&session, "material") && overridden(&session, "layer"));
    assert!(
        !session.revert_field(ember, "name").unwrap(),
        "nothing left to revert"
    );

    // Apply the material alone: every instance has it; the layer stays ours.
    let steps = session.undo_steps().len();
    assert!(session.apply_field(ember, "material").unwrap());
    assert_eq!(session.undo_steps().len(), steps + 1, "one step");
    assert_eq!(
        session.material_name(two.within(part)).as_deref(),
        Some("moss")
    );
    assert!(!overridden(&session, "material"));
    assert!(overridden(&session, "layer"));
    let text = std::fs::read_to_string(&prefab).unwrap();
    assert!(
        text.contains("\"moss\"") && !text.contains("debris"),
        "{text}"
    );

    assert!(
        session.revert_field(one, "name").is_err(),
        "a scene line has no prefab to go back to"
    );
    assert!(session.revert_field(ember, "colour").is_err());
}

#[test]
fn a_locked_ground_is_drawn_but_neither_clicked_nor_boxed() {
    use runity::glam::Vec2;
    let Some((mut session, _)) = open("pickable") else {
        return;
    };
    let (ground, crate_id) = (id(&session, "ground"), id(&session, "crate"));
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let (w, h) = session.size();
    session.set_pickable(&[ground], false).unwrap();
    assert!(session
        .hierarchy()
        .iter()
        .any(|r| r.id == ground && r.locked && !r.hidden));
    let taken = session
        .select_in_rect(Vec2::ZERO, Vec2::new(w as f32, h as f32), false)
        .unwrap();
    assert!(
        taken.contains(&crate_id) && !taken.contains(&ground),
        "{taken:?}"
    );
    assert_ne!(
        session.pick(1, h - 2),
        Some(ground),
        "the ground's own pixel"
    );
    assert!(!session.can_undo(), "a view setting");
    session.set_pickable(&[ground], true).unwrap();
    assert_eq!(session.pick(1, h - 2), Some(ground));
}

#[test]
fn the_console_says_what_happened_once_with_a_count() {
    use runity::input::{Input, InputEvent as E, Key};
    use runity_editor::console::Level;
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"hut\", prefab: \"no_such_hut\"),\n    ],\n)",
    );
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file("console", &scene);
    let _ = session.open_scene(&path);
    let lines = session.console().to_vec();
    assert!(
        lines
            .iter()
            .any(|l| l.level == Level::Info && l.text.starts_with("opened")),
        "{lines:?}"
    );

    // A refused edit in the Scene view is said, and said again counts up.
    session.select(Some(id(&session, "crate"))).unwrap();
    session.play();
    let mut input = Input::new();
    for _ in 0..3 {
        let _ = view_frame_result(&mut session, &mut input, &[E::KeyDown(Key::Delete)]);
        view_frame_result(&mut session, &mut input, &[E::KeyUp(Key::Delete)]).unwrap();
    }
    session.stop();
    let errors: Vec<_> = session
        .console()
        .iter()
        .filter(|l| l.level == Level::Error)
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].count, 3);
    assert_eq!(session.console_counts().2, 1);
    session.clear_console();
    assert!(session.console().is_empty());
}

fn view_frame_result(
    session: &mut Session,
    input: &mut runity::input::Input,
    events: &[runity::input::InputEvent],
) -> Result<Vec<&'static str>, runity_editor::EditError> {
    input.begin_frame();
    for e in events {
        input.handle(e);
    }
    session.scene_view(input, 1.0 / 60.0)
}

#[test]
fn a_heap_of_cubes_becomes_a_group_without_anything_moving() {
    use runity::glam::Vec3;
    use runity::input::{Input, InputEvent as E, Key};
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"post\", model: \"builtin:cube\", transform: (position: (4.0, 0.5, 0.0))),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("group", &scene) else {
        return;
    };
    let (crate_id, post, lid) = (
        id(&session, "crate"),
        id(&session, "post"),
        id(&session, "lid"),
    );
    let before: Vec<Vec3> = [crate_id, post, lid]
        .iter()
        .map(|i| session.world_position(*i).unwrap())
        .collect();
    session.select(Some(crate_id)).unwrap();
    session.add_to_selection(post).unwrap();
    let steps = session.undo_steps().len();
    let group = session.group_selection("hut").unwrap();
    assert_eq!(session.undo_steps().len(), steps + 1, "one step");
    let names: Vec<(String, usize)> = session
        .hierarchy()
        .into_iter()
        .map(|r| (r.name, r.depth))
        .collect();
    assert_eq!(
        names,
        [
            ("ground".into(), 0),
            ("hut".into(), 0),
            ("crate".into(), 1),
            ("lid".into(), 2),
            ("post".into(), 1)
        ]
    );
    for (i, id) in [crate_id, post, lid].iter().enumerate() {
        assert!((session.world_position(*id).unwrap() - before[i]).length() < 1e-4);
    }
    let middle = session.world_position(group).unwrap();
    assert!(
        (middle - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-3,
        "on the ground in the middle: {middle}"
    );
    assert_eq!(session.selected(), Some(group));

    // Ctrl A takes every top line; Ctrl Shift N makes an empty where the view looks.
    let mut input = Input::new();
    let did = view_frame(
        &mut session,
        &mut input,
        (1.0, 1.0),
        &[E::KeyDown(Key::LeftControl), E::KeyDown(Key::A)],
    );
    assert!(did.contains(&"select all"), "{did:?}");
    assert_eq!(session.selection().len(), 2, "ground and the hut");
    view_frame(&mut session, &mut input, (1.0, 1.0), &[E::KeyUp(Key::A)]);
    let did = view_frame(
        &mut session,
        &mut input,
        (1.0, 1.0),
        &[E::KeyDown(Key::LeftShift), E::KeyDown(Key::N)],
    );
    assert!(did.contains(&"create empty"), "{did:?}");
    let empty = session.selected().unwrap();
    assert_eq!(
        session.world_position(empty).unwrap(),
        session.camera().target
    );
}

#[test]
fn ctrl_while_dragging_moves_in_quarter_metres() {
    use runity::input::{Input, InputEvent as E, Key, MouseButton as M};
    let Some((mut session, _)) = open("increment") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.focus_selected();
    let (w, h) = session.size();
    let (_, r) = grab_right_of_centre(&mut session).expect("a handle");
    session.gizmo_end();
    let grab = ((w / 2 + r) as f32, (h / 2) as f32);
    let mut input = Input::new();
    view_frame(
        &mut session,
        &mut input,
        grab,
        &[E::KeyDown(Key::LeftControl), E::MouseDown(M::Left)],
    );
    view_frame(&mut session, &mut input, (grab.0 + 23.0, grab.1), &[]);
    view_frame(
        &mut session,
        &mut input,
        (grab.0 + 23.0, grab.1),
        &[E::MouseUp(M::Left), E::KeyUp(Key::LeftControl)],
    );
    let x = session.transform(crate_id).unwrap().position.x;
    assert!(x > 0.0, "it moved: {x}");
    assert!(
        (x / 0.25 - (x / 0.25).round()).abs() < 1e-4,
        "in quarter metres: {x}"
    );
    assert_eq!(
        session.snap(),
        runity_editor::Snap::default(),
        "the grid setting is untouched"
    );
}

#[test]
fn the_view_shows_where_a_walker_can_go_and_keeps_up_with_edits() {
    let scene = r#"(
    entities: [
        (name: "ground", model: "builtin:plane", transform: (scale: (10.0, 1.0, 10.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5))),
        (name: "wall", model: "builtin:cube", transform: (position: (0.0, 1.0, 0.0), scale: (6.0, 2.0, 0.3)), body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
    ],
)"#;
    let Some((mut session, _)) = open_with("nav-view", scene) else {
        return;
    };
    let wall = id(&session, "wall");
    session.set_show_navigation(Some(runity::navigation::NavSettings::default()));
    assert_eq!(session.walkable_cells(), None, "baked when drawn");
    session.render();
    let with_wall = session.walkable_cells().expect("baked");
    assert!(with_wall > 100, "{with_wall}");

    session.delete(wall).unwrap();
    assert_eq!(session.walkable_cells(), None, "stale after an edit");
    session.render();
    assert!(
        session.walkable_cells().unwrap() > with_wall,
        "the wall's ground is walkable now"
    );
    session.set_show_navigation(None);
    session.render();
    assert_eq!(session.walkable_cells(), None);
}

#[test]
fn a_thumbnail_pictures_an_asset_alone_and_changes_nothing() {
    let Some((mut session, _)) = open("thumbnail") else {
        return;
    };
    let (count, camera) = (session.entity_count(), session.camera());
    let pixels = session.thumbnail("builtin:cone", 64).unwrap();
    assert_eq!(pixels.len(), 64 * 64 * 4);
    let at = |x: usize, y: usize| pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3].to_vec();
    assert_ne!(
        at(32, 36),
        at(1, 1),
        "the cone is in the middle, not the backdrop"
    );
    assert_eq!(at(1, 1), at(62, 1), "the corners are backdrop");
    assert!(session.thumbnail("no_such_thing", 64).is_err());
    assert_eq!(session.entity_count(), count);
    assert_eq!(session.camera().position, camera.position);
}

#[test]
fn an_asset_dropped_into_the_view_stands_where_it_was_let_go() {
    use runity::glam::{Vec2, Vec3};
    let scene = SCENE.replace(
        "        (name: \"ground\", model: \"builtin:plane\", transform: (scale: (20.0, 1.0, 20.0))),",
        "        (name: \"ground\", model: \"builtin:plane\", transform: (scale: (20.0, 1.0, 20.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5))),",
    );
    let Some((mut session, _)) = open_with("drop-asset", &scene) else {
        return;
    };
    session.set_camera(Vec3::new(0.0, 8.0, 10.0), Vec3::ZERO);
    let (w, h) = session.size();
    let camera = session.camera();
    let spot = Vec3::new(3.0, 0.0, 2.0);
    let at = camera
        .screen_point(spot, Vec2::new(w as f32, h as f32))
        .unwrap();
    let steps = session.undo_steps().len();
    let cone = session
        .drop_asset("builtin:cone", at.x as u32, at.y as u32)
        .unwrap();
    assert_eq!(session.undo_steps().len(), steps + 1, "one step");
    assert_eq!(session.selected(), Some(cone));
    assert_eq!(session.entity_name(cone).as_deref(), Some("cone"));
    let (low, high) = session.world_bounds(cone).unwrap();
    assert!(low.y.abs() < 0.06, "on the ground: {low}");
    let middle = (low + high) * 0.5;
    assert!(
        (middle.x - 3.0).abs() < 0.2 && (middle.z - 2.0).abs() < 0.2,
        "{middle}"
    );
    assert!(session.drop_asset("nothing_by_that_name", 10, 10).is_err());
}

#[test]
fn the_view_is_where_this_person_left_it_and_the_scene_file_does_not_know() {
    use runity::glam::Vec3;
    let Some((mut session, path)) = open("prefs") else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap();
    session.set_camera(Vec3::new(7.0, 3.0, -4.0), Vec3::new(1.0, 0.0, 1.0));
    session.set_snap(runity_editor::Snap {
        meters: 0.5,
        degrees: 0.0,
        scale: 0.0,
    });
    session.set_show_grid(false);
    let cave = session.new_scene("cave").unwrap();
    assert_ne!(session.camera().position, Vec3::new(7.0, 3.0, -4.0));

    // Back to the first: the view comes back, the file never changed.
    session.open_scene(&path).unwrap();
    assert_eq!(session.camera().position, Vec3::new(7.0, 3.0, -4.0));
    assert_eq!(session.snap().meters, 0.5);
    assert!(!session.show_grid(), "the grid stays off");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

    // A new session starts on the scene open last.
    session.open_scene(&cave).unwrap();
    drop(session);
    let project = runity::Project::find(&path).unwrap();
    assert_eq!(runity_editor::Session::last_scene(&project), Some(cave));
    let ignore = std::fs::read_to_string(project.root().join(".gitignore")).unwrap();
    assert!(ignore.contains("/.runity/"), "one person's, not the team's");
}

#[test]
fn the_inspector_knows_a_game_component_by_what_the_game_wrote_down() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Door {
        open_angle: f32,
        #[serde(default)]
        locked: bool,
    }
    let Some((mut session, path)) = open("shapes") else {
        return;
    };
    let crate_id = id(&session, "crate");
    // Without the file the Inspector edits RON as before.
    session
        .set_component(crate_id, "door", Some("(open_angel: 1.0)"))
        .unwrap();
    session.set_component(crate_id, "door", None).unwrap();

    // The game writes what its components look like...
    let mut components = runity::Components::new();
    components.register::<Door>("door");
    components
        .write_shapes(root_of(&path).join(runity::project::SHAPES))
        .unwrap();

    // ...and the editor starts one from it, labels it, and refuses a typo.
    assert_eq!(
        session.add_component(crate_id, "door").unwrap(),
        "(open_angle: 0.0, locked: false)"
    );
    let field = session
        .inspect(crate_id)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "components.door")
        .unwrap();
    assert_eq!(field.shape, "(open_angle: number, locked: bool)");
    let e = session
        .set_field(crate_id, "components.door", "(open_angel: 90.0)")
        .unwrap_err()
        .to_string();
    assert!(e.contains("did you mean `open_angle`?"), "{e}");
    session
        .set_field(crate_id, "components.door", "(open_angle: 90.0)")
        .unwrap();
    let e = session
        .add_component(crate_id, "dor")
        .unwrap_err()
        .to_string();
    assert!(e.contains("did you mean `door`?"), "{e}");
}

#[test]
fn play_in_the_game_saves_the_scene_and_names_it_to_the_game() {
    let Some((mut session, path)) = open("game-command") else {
        return;
    };
    session
        .set_field(id(&session, "crate"), "name", "box")
        .unwrap();
    let command = session.game_command().unwrap();
    assert!(
        std::fs::read_to_string(&path).unwrap().contains("\"box\""),
        "saved first"
    );
    assert_eq!(command.get_program(), "cargo");
    assert_eq!(command.get_args().collect::<Vec<_>>(), ["run"]);
    assert_eq!(command.get_current_dir(), Some(root_of(&path).as_path()));
    let scene = command
        .get_envs()
        .find(|(k, _)| *k == "RUNITY_SCENE")
        .and_then(|(_, v)| v)
        .unwrap();
    assert_eq!(scene, "scene");

    // The game watches the editor's document, not the saved file: a game
    // started with that command is given every edit, unsaved.
    let live = command
        .get_envs()
        .find(|(k, _)| *k == "RUNITY_SCENE_FILE")
        .and_then(|(_, v)| v)
        .map(PathBuf::from)
        .unwrap();
    assert!(live.starts_with(root_of(&path).join(".runity")), "{live:?}");
    let given = |live: &Path| -> runity::Scene {
        runity::ron::from_str(&std::fs::read_to_string(live).unwrap()).unwrap()
    };
    assert!(given(&live).find("box").is_some());
    // And the game can play it from there: the project is found above it.
    let (_, problems) = runity::LiveScene::open(&live).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let mut game = std::process::Command::new("sleep");
    game.arg("30").env("RUNITY_SCENE_FILE", &live);
    session.run_in_console(game).unwrap();
    session
        .set_field(id(&session, "box"), "name", "chest")
        .unwrap();
    session.poll_game();
    assert!(given(&live).find("chest").is_some(), "given without a save");
    assert!(
        std::fs::read_to_string(&path).unwrap().contains("\"box\""),
        "the scene file says what was saved"
    );
    // Undo is an edit too.
    session.undo().unwrap();
    session.poll_game();
    assert!(given(&live).find("box").is_some());
    session.stop_game();

    // And back: what the game says about its world shows in the Inspector,
    // beside what the document says, and cannot be edited there.
    let state = command
        .get_envs()
        .find(|(k, _)| *k == "RUNITY_STATE_FILE")
        .and_then(|(_, v)| v)
        .map(PathBuf::from)
        .unwrap();
    let mut game = std::process::Command::new("sleep");
    game.arg("30")
        .env("RUNITY_SCENE_FILE", &live)
        .env("RUNITY_STATE_FILE", &state);
    session.run_in_console(game).unwrap();
    let box_id = id(&session, "box");
    assert!(session.game_state().is_none(), "nothing said yet");
    runity::save::SaveGame {
        entities: vec![runity::save::Saved {
            id: box_id,
            transform: Transform {
                position: Vec3::new(0.0, 0.5, 3.0),
                ..Default::default()
            },
            components: vec![("door".into(), "(open: true)".into())],
            prefab: String::new(),
        }],
        gone: vec![],
    }
    .write(&state)
    .unwrap();
    let fields = session.inspect(box_id).unwrap();
    let value = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.clone())
    };
    assert_eq!(value("game.position").as_deref(), Some("(0.0,0.5,3.0)"));
    assert_eq!(
        value("game.components.door").as_deref(),
        Some("(open: true)")
    );
    assert_ne!(
        value("position"),
        value("game.position"),
        "the document is where it starts"
    );
    // The Scene view outlines it where the game has it.
    let cyan = |session: &mut Session| {
        session.render();
        session
            .frame_pixels()
            .chunks(4)
            .filter(|p| p[0] < 90 && p[1] > 180 && p[2] > 220)
            .count()
    };
    session.set_camera(Vec3::new(0.0, 6.0, 9.0), Vec3::new(0.0, 0.0, 2.0));
    let with = cyan(&mut session);
    assert!(with > 20, "{with} cyan pixels");
    let e = session
        .set_field(box_id, "game.position", "(0.0, 0.0, 0.0)")
        .unwrap_err()
        .to_string();
    assert!(e.contains("what the running game says"), "{e}");
    session.stop_game();
    assert!(session.game_state().is_none(), "not after it stopped");
    assert!(cyan(&mut session) < with / 4, "and no outline");
}

#[test]
fn a_selected_light_shows_how_far_it_reaches() {
    let scene = SCENE.replace(
        "    ],\n)",
        "        (name: \"lamp\", transform: (position: (0.0, 1.0, 0.0)), light: (range: 3.0)),\n    ],\n)",
    );
    let Some((mut session, _)) = open_with("light-range", &scene) else {
        return;
    };
    let lamp = id(&session, "lamp");
    session.set_camera(
        runity::glam::Vec3::new(0.0, 12.0, 0.1),
        runity::glam::Vec3::ZERO,
    );
    let orange = |s: &Session| {
        s.frame_pixels()
            .chunks(4)
            .filter(|p| p[0] > 200 && (60..180).contains(&p[1]) && p[2] < 60)
            .count()
    };
    session.render();
    let before = orange(&session);
    session.select(Some(lamp)).unwrap();
    session.render();
    assert!(
        orange(&session) > before + 50,
        "a ring of its range: {} vs {before}",
        orange(&session)
    );
    let fields = session.inspect(lamp).unwrap();
    let light = fields.iter().find(|f| f.name == "light").unwrap();
    assert!(light.value.contains("range:3.0"), "{}", light.value);
    session.set_field(lamp, "light", "None").unwrap();
    assert_eq!(
        session
            .inspect(lamp)
            .unwrap()
            .iter()
            .find(|f| f.name == "light")
            .unwrap()
            .value,
        "None"
    );
}

#[test]
fn an_asset_s_import_settings_are_edited_and_it_is_built_again() {
    let Some((mut session, path)) = open("import-settings") else {
        return;
    };
    let root = root_of(&path);
    std::fs::write(
        root.join("assets/wedge.obj"),
        "v -1.0 0.0 -1.0\nv  1.0 0.0 -1.0\nv  1.0 1.0  1.0\nf 1 3 2\n",
    )
    .unwrap();
    session.import(root.join("assets/wedge.obj")).unwrap();
    let wedge = session.add(None, "wedge").unwrap();
    let before = session.world_bounds(wedge).unwrap();
    assert_eq!(
        session.import_settings("assets/wedge.obj").unwrap().scale,
        1.0
    );

    session
        .set_import_setting("assets/wedge.obj", "scale", "2")
        .unwrap();
    assert_eq!(
        session.import_settings("assets/wedge.obj").unwrap().scale,
        2.0,
        "in the sidecar"
    );
    let after = session.world_bounds(wedge).unwrap();
    let (b, a) = (before.1 - before.0, after.1 - after.0);
    assert!(
        (a.x - 2.0 * b.x).abs() < 1e-3,
        "twice as big in the scene now: {b} → {a}"
    );

    let e = session
        .set_import_setting("assets/wedge.obj", "scael", "2")
        .unwrap_err()
        .to_string();
    assert!(e.contains("there are scale"), "{e}");
    assert!(session
        .set_import_setting("assets/wedge.obj", "scale", "-1")
        .is_err());
    assert!(session.import_settings("assets/nothing.obj").is_err());
}

#[test]
fn the_hierarchy_search_shows_only_matches_and_reset_puts_a_field_back() {
    let Some((mut session, _)) = open("hierarchy-search") else {
        return;
    };
    let (crate_id, lid) = (id(&session, "crate"), id(&session, "lid"));
    let rows = session.hierarchy_matching("lid").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["lid"]
    );
    assert_eq!(rows[0].depth, 0, "flat, as Unity's search shows it");
    let all = session.hierarchy_matching("  ").unwrap();
    assert_eq!(all.len(), 3);
    let cubes = session.hierarchy_matching("model:builtin:cube").unwrap();
    assert_eq!(cubes.len(), 2, "{cubes:?}");

    session.set_field(lid, "scale", "(2.0, 2.0, 2.0)").unwrap();
    session.reset_field(lid, "scale").unwrap();
    assert_eq!(
        session.transform(lid).unwrap().scale,
        runity::glam::Vec3::ONE
    );
    session.set_field(crate_id, "body", "Dynamic").unwrap();
    session.reset_field(crate_id, "body").unwrap();
    let body = session
        .inspect(crate_id)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "body")
        .unwrap();
    assert!(body.value.ends_with("None"), "{}", body.value);
    assert!(session.reset_field(crate_id, "name").is_err());
}

#[test]
fn a_box_collider_is_fitted_to_the_model() {
    let Some((mut session, path)) = open("fit-collider") else {
        return;
    };
    let root = root_of(&path);
    // A post two metres tall, its origin at its foot once imported.
    std::fs::write(
        root.join("assets/post.obj"),
        "v -0.5 0 -0.5\nv 0.5 0 -0.5\nv 0.5 2 0.5\nv -0.5 2 0.5\nf 1 2 3\nf 1 3 4\n",
    )
    .unwrap();
    session.import(root.join("assets/post.obj")).unwrap();
    let post = session.add(None, "post").unwrap();
    assert!(session.fit_collider(post).unwrap());
    let collider = session
        .inspect(post)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "collider")
        .unwrap()
        .value;
    assert!(
        collider.contains("half:(0.5,1.0,0.5)") && collider.contains("center:(0.0,1.0,0.0)"),
        "{collider}"
    );
    let empty = session.add(None, "").unwrap();
    assert!(!session.fit_collider(empty).unwrap(), "nothing to fit");
}

#[test]
fn a_lift_on_a_route_carries_what_stands_on_it() {
    let scene = r#"(
    entities: [
        (name: "lift", model: "builtin:cube", transform: (scale: (3.0, 0.2, 3.0)),
         body: Kinematic, collider: Box(half: (0.5, 0.5, 0.5)),
         route: (points: [(0.0, 0.0, 0.0), (0.0, 3.0, 0.0)], speed: 1.0, ends: Stop, smooth: false)),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.7, 0.0)),
         body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5))),
    ],
)"#;
    let Some((mut session, _)) = open_with("lift", scene) else {
        return;
    };
    let (lift, crate_id) = (id(&session, "lift"), id(&session, "crate"));
    // Selected, it shows where it goes.
    session.select(Some(lift)).unwrap();
    session.render();
    let route = session
        .inspect(lift)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "route")
        .unwrap();
    assert!(route.value.contains("Stop"), "{}", route.value);

    session.play();
    for _ in 0..240 {
        session.step(1.0 / 60.0);
    }
    let top = session.world_position(lift).unwrap();
    assert!(
        (top.y - 3.0).abs() < 0.05,
        "the lift reached the top: {top}"
    );
    let riding = session.world_position(crate_id).unwrap();
    assert!(riding.y > 3.3, "the crate rode up with it: {riding}");
    session.stop();
    assert_eq!(
        session.world_position(lift).unwrap().y,
        0.0,
        "back where it was"
    );
}

#[test]
fn an_instance_changes_its_part_s_light_and_the_others_keep_the_prefab_s() {
    let Some(mut session) = new_session() else {
        return;
    };
    let path = scene_file(
        "light-override",
        r#"(entities: [
            (id: "00000000000000a1", name: "fire one", prefab: "campfire"),
            (id: "00000000000000a2", name: "fire two", prefab: "campfire", transform: (position: (4.0, 0.0, 0.0))),
        ])"#,
    );
    std::fs::write(
        root_of(&path).join("prefabs/campfire.prefab"),
        "(id: \"00000000000000c1\", name: \"campfire\", model: \"builtin:cube\",\n    children: [(id: \"00000000000000c2\", name: \"ember\", model: \"builtin:sphere\", light: (intensity: 1.0, range: 3.0))])\n",
    )
    .unwrap();
    session.open_scene(&path).unwrap();
    let (one, two): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
    let part: EntityId = "c2".parse().unwrap();
    let light = |s: &Session, id: EntityId| {
        s.inspect(id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == "light")
            .unwrap()
    };

    session
        .set_field(one.within(part), "light", "(intensity: 4.0, range: 3.0)")
        .unwrap();
    let mine = light(&session, one.within(part));
    assert!(
        mine.overridden && mine.value.contains("intensity:4.0"),
        "{mine:?}"
    );
    let theirs = light(&session, two.within(part));
    assert!(
        !theirs.overridden && theirs.value.contains("intensity:1.0"),
        "{theirs:?}"
    );
    let saved = session.scene().get(one).unwrap().overrides.clone();
    assert!(saved.values().any(|o| o.light.is_some()));

    assert!(session.apply_field(one.within(part), "light").unwrap());
    assert!(
        light(&session, two.within(part))
            .value
            .contains("intensity:4.0"),
        "every fire now"
    );
}

#[test]
fn what_the_game_prints_comes_back_into_the_console() {
    use runity_editor::console::Level;
    let Some((mut session, _)) = open("game-output") else {
        return;
    };
    let mut command = std::process::Command::new("sh");
    command.args([
        "-c",
        "echo 'the door opened'; echo 'warning: slow frame' >&2; \
         echo \"thread 'main' panicked at src/main.rs:3:5:\" >&2; \
         echo '   0: game::main' >&2; echo '   1: std::rt::lang_start' >&2; exit 101",
    ]);
    session.run_in_console(command).unwrap();
    let started = std::time::Instant::now();
    let code = loop {
        if let Some(code) = session.poll_game() {
            break code;
        }
        assert!(started.elapsed().as_secs() < 10, "the game never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(code, 101);
    assert!(!session.game_running());
    let said = |level: Level, text: &str| {
        session
            .console()
            .iter()
            .any(|l| l.level == level && l.text.contains(text))
    };
    assert!(
        said(Level::Info, "the door opened"),
        "{:#?}",
        session.console()
    );
    assert!(said(Level::Warning, "slow frame"));
    assert!(said(Level::Error, "panicked at src/main.rs"));
    // The trace is part of the panic's entry, not lines of its own.
    let panic = session
        .console()
        .iter()
        .find(|l| l.text.contains("panicked"))
        .unwrap();
    assert!(panic.text.ends_with("1: std::rt::lang_start"), "{panic:?}");
    assert!(!session
        .console()
        .iter()
        .any(|l| l.text.starts_with("   0:")));
    assert!(said(Level::Error, "the game ended with"));

    // A game still running is stopped by the editor, and a second start
    // replaces the first.
    let mut forever = std::process::Command::new("sleep");
    forever.arg("30");
    session.run_in_console(forever).unwrap();
    assert!(session.game_running());
    assert!(session.stop_game());
    assert!(!session.game_running() && !session.stop_game());
}

fn poly(points: &[(f32, f32)], height: f32) -> runity_import::poly::PolySource {
    runity_import::poly::PolySource {
        points: points.to_vec(),
        height,
        ..Default::default()
    }
}

#[test]
fn a_poly_shape_is_an_l_shaped_floor_from_its_outline_and_changes_with_it() {
    let Some((mut session, path)) = open("poly") else {
        return;
    };
    let l = [
        (0.0, 0.0),
        (8.0, 0.0),
        (8.0, 6.0),
        (3.0, 6.0),
        (3.0, 10.0),
        (0.0, 10.0),
    ];
    let steps = session.undo_steps().len();
    let hall = session
        .poly_shape("hall", &poly(&l, 0.5), Vec3::new(20.0, 0.0, 0.0))
        .unwrap();
    assert_eq!(session.undo_steps().len(), steps + 1, "one step");
    assert_eq!(session.selected(), Some(hall));
    assert!(root_of(&path).join("assets/hall.rpoly").is_file());
    let (low, high) = session.world_bounds(hall).unwrap();
    assert!((low - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-3, "{low}");
    assert!(
        (high - Vec3::new(28.0, 0.5, 10.0)).length() < 1e-3,
        "{high}"
    );

    let line = session.scene().get(hall).unwrap().clone();
    assert_eq!(line.collider, runity::scene::Collider::Model);
    assert_eq!(line.body, runity::Body::Static);

    // A new outline: every placement changes; a bad one is refused.
    let mut source = session.poly("hall").unwrap();
    assert_eq!(source.points.len(), 6);
    source.height = 2.0;
    session.set_poly("hall", &source).unwrap();
    let (_, high) = session.world_bounds(hall).unwrap();
    assert!((high.y - 2.0).abs() < 1e-3, "{high}");
    source.points = vec![(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)];
    let e = session.set_poly("hall", &source).unwrap_err().to_string();
    assert!(e.contains("crosses itself"), "{e}");
    assert_eq!(session.poly("hall").unwrap().height, 2.0, "the file kept");

    // Solid in its own shape: a box over the L's missing corner falls
    // past it, one over the floor stands on it, two metres up.
    let dropped = |session: &mut Session, x: f32, z: f32| {
        session
            .add_entity(
                None,
                runity::EntityDesc {
                    name: format!("box {x}"),
                    model: "builtin:cube".into(),
                    body: runity::Body::Dynamic,
                    collider: runity::scene::Collider::Box {
                        half: Vec3::splat(0.25),
                        center: Vec3::ZERO,
                    },
                    transform: runity::scene::Transform {
                        position: Vec3::new(x, 4.0, z),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let (over_floor, over_gap) = (
        dropped(&mut session, 21.0, 2.0),
        dropped(&mut session, 26.0, 8.0),
    );
    session.play();
    for _ in 0..120 {
        session.step(1.0 / 60.0);
    }
    let y = |id| session.world_position(id).unwrap().y;
    assert!(
        (y(over_floor) - 2.25).abs() < 0.1,
        "on the hall: {}",
        y(over_floor)
    );
    assert!(y(over_gap) < 1.0, "past it: {}", y(over_gap));
    session.stop();

    // Push the wall along z = 0 out by two metres: the hall is longer
    // toward -z, by exactly that, whichever way the outline runs.
    session.set_poly("hall", &poly(&l, 2.0)).unwrap();
    session.push_poly_edge("hall", 0, 2.0).unwrap();
    let (low, high) = session.world_bounds(hall).unwrap();
    assert!(
        (low.z + 2.0).abs() < 1e-3 && (high.z - 10.0).abs() < 1e-3,
        "{low} {high}"
    );
    let mut reversed: Vec<(f32, f32)> = l.to_vec();
    reversed.reverse();
    session.set_poly("hall", &poly(&reversed, 2.0)).unwrap();
    // Reversed, the wall along z = 0 runs from point 4 to point 5.
    session.push_poly_edge("hall", 4, -1.0).unwrap();
    let (low, _) = session.world_bounds(hall).unwrap();
    assert!((low.z - 1.0).abs() < 1e-3, "pulled in: {low}");
    assert!(session.push_poly_edge("hall", 6, 1.0).is_err());

    let e = session
        .poly_shape("hall", &poly(&l, 1.0), Vec3::ZERO)
        .unwrap_err()
        .to_string();
    assert!(e.contains("already"), "{e}");
    let e = session
        .poly_shape("Big Hall", &poly(&l, 1.0), Vec3::ZERO)
        .unwrap_err()
        .to_string();
    assert!(e.contains("snake_case"), "{e}");
}

#[test]
fn snapping_the_selection_puts_a_greybox_dragged_by_eye_on_the_grid() {
    let Some((mut session, _)) = open("snap-all") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session
        .set_transform(
            crate_id,
            transform([1.37, 0.52, -2.26], [0.0, 47.0, 0.0], [1.0, 1.0, 1.0]),
        )
        .unwrap();
    session.select(Some(crate_id)).unwrap();
    let steps = session.undo_steps().len();
    assert_eq!(session.snap_selection().unwrap(), 1);
    let t = session.transform(crate_id).unwrap();
    assert_eq!(
        t.position,
        Vec3::new(1.0, 1.0, -2.0),
        "a metre with the grid off"
    );
    assert_eq!(t.rotation_deg.y, 47.0, "turns only with an angle step");
    assert_eq!(session.undo_steps().len(), steps + 1);
    session.set_snap(runity_editor::Snap {
        meters: 0.25,
        degrees: 15.0,
        scale: 0.0,
    });
    session
        .set_transform(
            crate_id,
            transform([1.37, 0.52, -2.26], [0.0, 47.0, 0.0], [1.0, 1.0, 1.0]),
        )
        .unwrap();
    session.snap_selection().unwrap();
    let t = session.transform(crate_id).unwrap();
    assert_eq!(t.position, Vec3::new(1.25, 0.5, -2.25));
    assert_eq!(t.rotation_deg.y, 45.0);
    assert_eq!(session.snap_selection().unwrap(), 0, "already on it");
}

#[test]
fn the_game_view_draws_through_the_game_camera_without_the_editors_marks() {
    let Some((mut session, _path)) = open("gameview") else {
        return;
    };
    let crate_id = id(&session, "crate");
    session.select(Some(crate_id)).unwrap();
    session.render();
    let scene_view = session.frame_pixels().to_vec();
    session.set_game_view(true);
    assert!(session.is_game_view());
    session.render();
    let game_view = session.frame_pixels().to_vec();
    assert_ne!(scene_view, game_view, "the gizmo and grid are gone");
    session.set_game_view(false);
    session.render();
    assert_eq!(session.frame_pixels(), &scene_view[..], "and come back");
}

#[test]
fn the_sun_and_the_fog_are_one_undo_step_each() {
    let Some((mut session, _path)) = open("environment") else {
        return;
    };
    let before = session.environment();
    session
        .set_environment("sun", "(hour: 18.5, intensity: 0.6)")
        .unwrap();
    let sun = &session.environment()[0].1;
    assert!(sun.contains("18.5"), "{sun}");
    assert!(session.set_environment("fog", "(nonsense").is_err());
    assert!(session.set_environment("rain", "()").is_err());
    session.undo().unwrap();
    assert_eq!(session.environment(), before);
}

#[test]
fn a_new_terrain_is_found_under_the_cursor_and_rises_where_it_is_stroked() {
    let Some((mut session, _path)) = open("terrain") else {
        return;
    };
    let terrain = session.new_terrain("meadow", 30.0).unwrap();
    assert_eq!(session.selected(), Some(terrain));
    session.set_camera(Vec3::new(0.0, 20.0, 20.0), Vec3::ZERO);
    let (w, h) = session.size();
    let at = session
        .point_under(w / 2, h / 2)
        .expect("the ground is under the middle");
    assert!(at.length() < 1.0, "{at:?}");
    session.sculpt(terrain, at, 4.0, 2.0, false).unwrap();
    let after = session.point_under(w / 2, h / 2).unwrap();
    assert!(after.y > 1.5, "raised: {after:?}");
    assert!(
        session.new_terrain("meadow", 30.0).is_err(),
        "the name is taken"
    );
}
