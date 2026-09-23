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
    assert_eq!(session.entity_count(), before, "one step takes all of it back");

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
