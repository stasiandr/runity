//! The C boundary, exercised the way a host would use it.
//!
//! These call through the `extern "C"` functions rather than around them,
//! because the interesting failures live exactly there: a length reported
//! wrong, a buffer written past, a null that was assumed not to be one.

use std::ffi::{c_char, CString};

use runity_ffi::*;

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

fn scene_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("runity-ffi-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    std::fs::write(&path, SCENE).unwrap();
    path
}

fn c(path: &std::path::Path) -> CString {
    CString::new(path.to_string_lossy().as_bytes()).unwrap()
}

struct Handle(*mut Editor);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { runity_editor_free(self.0) };
    }
}

fn open(name: &str) -> Option<(Handle, std::path::PathBuf)> {
    let editor = unsafe { runity_editor_create_offscreen(192, 128) };
    if editor.is_null() {
        let mut message = vec![0u8; 256];
        unsafe { runity_last_error(message.as_mut_ptr() as *mut c_char, 256) };
        eprintln!("skipping: {}", String::from_utf8_lossy(&message));
        return None;
    }
    let path = scene_file(name);
    assert!(unsafe { runity_editor_open_scene(editor, c(&path).as_ptr()) });
    Some((Handle(editor), path))
}

#[test]
fn a_scene_opens_and_lists_its_entities_including_children() {
    let Some((editor, _)) = open("list") else {
        return;
    };
    assert_eq!(
        unsafe { runity_editor_entity_count(editor.0) },
        3,
        "ground, crate, lid"
    );

    let mut buffer = vec![0u8; 64];
    let needed =
        unsafe { runity_editor_entity_name(editor.0, 1, buffer.as_mut_ptr() as *mut c_char, 64) };
    assert_eq!(needed, 5);
    assert_eq!(
        std::ffi::CStr::from_bytes_until_nul(&buffer)
            .unwrap()
            .to_str()
            .unwrap(),
        "crate"
    );
}

#[test]
fn a_name_longer_than_the_buffer_truncates_and_says_how_long_it_was() {
    // The usual C shape: ask with a small buffer, get told the real length,
    // ask again. A caller that trusts a length it was never given is the
    // bug this prevents.
    let Some((editor, _)) = open("truncate") else {
        return;
    };
    let mut small = vec![0xAAu8; 4];
    let needed =
        unsafe { runity_editor_entity_name(editor.0, 0, small.as_mut_ptr() as *mut c_char, 4) };
    assert_eq!(needed, 6, "\"ground\" is six characters");
    assert_eq!(small[3], 0, "and what was written is terminated");
    assert_eq!(&small[..3], b"gro");
}

#[test]
fn a_transform_round_trips_through_nine_floats_and_reaches_the_file() {
    let Some((editor, path)) = open("transform") else {
        return;
    };
    let values: [f32; 9] = [1.0, 2.0, 3.0, 0.0, 45.0, 0.0, 2.0, 2.0, 2.0];
    assert!(unsafe { runity_editor_set_transform(editor.0, 1, values.as_ptr()) });

    let mut read = [0.0f32; 9];
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, read.as_mut_ptr()) });
    assert_eq!(read, values);

    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
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
fn clicking_on_a_thing_finds_it_and_clicking_on_sky_does_not() {
    let Some((editor, _)) = open("pick") else {
        return;
    };
    let eye = [0.0f32, 1.0, 5.0];
    let target = [0.0f32, 0.5, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), target.as_ptr()) });

    let middle = unsafe {
        runity_editor_pick(
            editor.0,
            runity_editor_width(editor.0) / 2,
            runity_editor_height(editor.0) / 2,
        )
    };
    assert!(middle >= 0, "something is under the middle of the view");

    // Straight up, where there is neither crate nor ground.
    let sky = unsafe { runity_editor_pick(editor.0, runity_editor_width(editor.0) / 2, 0) };
    assert_eq!(sky, -1, "nothing is up there");
}

#[test]
fn rendering_produces_a_frame_the_host_can_read() {
    let Some((editor, _)) = open("render") else {
        return;
    };
    assert!(unsafe { runity_editor_render(editor.0) });

    let needed = unsafe { runity_editor_frame_pixels(editor.0, std::ptr::null_mut(), 0) };
    assert_eq!(needed, 192 * 128 * 4);

    let mut pixels = vec![0u8; needed as usize];
    let written = unsafe { runity_editor_frame_pixels(editor.0, pixels.as_mut_ptr(), needed) };
    assert_eq!(written, needed);
    assert!(pixels.iter().any(|b| *b != 0), "something was drawn");
}

#[test]
fn every_call_survives_a_null_editor() {
    // An editor's UI outlives its document. A boundary that segfaults when a
    // panel repaints after a close gets wrapped in defensive code on the
    // other side, and that code is where the real bugs then live.
    let null: *mut Editor = std::ptr::null_mut();
    unsafe {
        assert_eq!(runity_editor_entity_count(null), 0);
        assert_eq!(runity_editor_width(null), 0);
        assert_eq!(runity_editor_pick(null, 10, 10), -1);
        assert!(!runity_editor_render(null));
        assert!(!runity_editor_open_scene(null, std::ptr::null()));
        assert!(!runity_editor_save_scene(null, std::ptr::null()));
        assert!(!runity_editor_get_transform(null, 0, std::ptr::null_mut()));
        assert_eq!(
            runity_editor_entity_name(null, 0, std::ptr::null_mut(), 0),
            0
        );
        assert!(!runity_editor_get_material(null, 0, std::ptr::null_mut()));
        assert!(!runity_editor_set_material(null, 0, std::ptr::null()));
        assert!(!runity_editor_set_material_name(null, 0, std::ptr::null()));
        assert_eq!(
            runity_editor_material_name(null, 0, std::ptr::null_mut(), 0),
            0
        );
        assert_eq!(runity_editor_palette_count(null), 0);
        assert_eq!(runity_editor_prefab_count(null), 0);
        assert!(!runity_editor_set_tool(null, 1));
        assert!(!runity_editor_play(null));
        assert!(!runity_editor_set_snap(null, 0.25, 15.0, 0.1));
        assert!(!runity_editor_snap(null, std::ptr::null_mut()));
        assert!(!runity_editor_focus_selected(null));
        assert!(!runity_editor_import(null, std::ptr::null()));
        assert_eq!(runity_editor_reload_assets(null), 0);
        assert!(!runity_editor_save_material(null, 0, std::ptr::null()));
        assert_eq!(runity_editor_step(null, 0.016), 0);
        assert!(!runity_editor_stop(null));
        assert!(!runity_editor_is_playing(null));
        assert!(!runity_editor_world_position(null, 0, std::ptr::null_mut()));
        assert_eq!(runity_editor_tool(null), 0);
        assert_eq!(runity_editor_add_instance(null, -1, std::ptr::null()), -1);
        assert!(!runity_editor_make_prefab(null, 0, std::ptr::null()));
        assert!(!runity_editor_set_prefabs(null, std::ptr::null()));
        assert!(!runity_editor_get_camera(
            null,
            std::ptr::null_mut(),
            std::ptr::null_mut()
        ));
        assert!(!runity_editor_capture_camera(null));
        assert!(!runity_editor_palette_color(null, 0, std::ptr::null_mut()));
        runity_editor_free(null);
    }
}

#[test]
fn the_gizmo_moves_the_thing_it_is_on_and_writes_it_to_the_scene() {
    let Some((editor, path)) = open("gizmo") else {
        return;
    };
    let eye = [0.0f32, 1.0, 6.0];
    let target = [0.0f32, 0.5, 0.0];
    unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), target.as_ptr()) };

    // Nothing selected: no handles to find.
    assert_eq!(unsafe { runity_editor_selected(editor.0) }, -1);
    assert_eq!(unsafe { runity_editor_gizmo_hover(editor.0, 96, 64) }, -1);

    assert!(unsafe { runity_editor_select(editor.0, 1) }, "the crate");
    assert_eq!(unsafe { runity_editor_selected(editor.0) }, 1);

    // Sweep the middle band for an arm rather than guessing a pixel: where
    // the handles land depends on the projection, and a test that hard-codes
    // that is testing arithmetic it does not own.
    let width = unsafe { runity_editor_width(editor.0) };
    let height = unsafe { runity_editor_height(editor.0) };
    let found = (0..width)
        .map(|x| {
            (x, unsafe {
                runity_editor_gizmo_begin(editor.0, x, height / 2)
            })
        })
        .find(|(_, arm)| *arm >= 0);
    let (grab_x, arm) = found.expect("an arm crosses the middle of the view");
    assert!(arm >= 0);

    let mut before = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, before.as_mut_ptr()) };

    // Grabbing and not moving must not move anything.
    assert!(unsafe { runity_editor_gizmo_drag(editor.0, grab_x, height / 2) });
    let mut still = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, still.as_mut_ptr()) };
    assert_eq!(still, before, "a grab on its own is not a move");

    // Now drag sideways.
    assert!(unsafe { runity_editor_gizmo_drag(editor.0, grab_x + 30, height / 2) });
    unsafe { runity_editor_gizmo_end(editor.0) };

    let mut after = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, after.as_mut_ptr()) };
    assert_ne!(after[..3], before[..3], "the crate moved");
    assert_eq!(after[3..], before[3..], "and only its position changed");

    // And it survives a save and reopen, which is the point of the whole
    // boundary: a drag has to end up in a file.
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    let reopened = runity::Scene::load(&path).unwrap();
    let moved = reopened.find("crate").unwrap().transform.position;
    assert!((moved.x - after[0]).abs() < 1e-4 && (moved.y - after[1]).abs() < 1e-4);
}

#[test]
fn dragging_a_child_keeps_it_in_its_parent() {
    // The gizmo sits at a world position and the file holds a local one.
    // Writing the world position straight back yanks a child out of its
    // parent by however far the parent is offset.
    let Some((editor, _)) = open("gizmo-child") else {
        return;
    };
    let eye = [0.0f32, 1.5, 6.0];
    let target = [0.0f32, 1.0, 0.0];
    unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), target.as_ptr()) };
    assert!(unsafe { runity_editor_select(editor.0, 2) }, "the lid");

    let height = unsafe { runity_editor_height(editor.0) };
    let width = unsafe { runity_editor_width(editor.0) };
    let Some((grab_x, _)) = (0..width)
        .map(|x| {
            (x, unsafe {
                runity_editor_gizmo_begin(editor.0, x, height / 2)
            })
        })
        .find(|(_, arm)| *arm >= 0)
    else {
        return;
    };

    let mut before = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 2, before.as_mut_ptr()) };
    assert!(unsafe { runity_editor_gizmo_drag(editor.0, grab_x, height / 2) });
    let mut after = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 2, after.as_mut_ptr()) };

    // The crate sits at y = 0.5 and the lid at y = 0.6 above it. A drag
    // that does not undo the parent would rewrite the lid's local y as 1.1.
    assert!(
        (after[1] - before[1]).abs() < 1e-3,
        "the lid's local position should be unchanged by a grab, \
         was {} and is now {}",
        before[1],
        after[1]
    );
}

#[test]
fn resizing_an_offscreen_editor_changes_what_it_reports_and_renders() {
    let Some((editor, _)) = open("resize") else {
        return;
    };
    assert_eq!(unsafe { runity_editor_width(editor.0) }, 192);

    assert!(unsafe { runity_editor_resize(editor.0, 64, 48) });
    assert_eq!(unsafe { runity_editor_width(editor.0) }, 64);
    assert_eq!(unsafe { runity_editor_height(editor.0) }, 48);

    // And the frame that comes out is the new size. An offscreen target
    // cannot be resized in place, so reporting the new size while still
    // rendering the old one is the failure this catches.
    assert!(unsafe { runity_editor_render(editor.0) });
    let needed = unsafe { runity_editor_frame_pixels(editor.0, std::ptr::null_mut(), 0) };
    assert_eq!(needed, 64 * 48 * 4);
}

#[test]
fn asking_for_a_layer_surface_without_a_layer_fails_rather_than_crashing() {
    // A host that passes a view it has not made yet is a normal bug on the
    // far side. It should get null and a message.
    let editor = unsafe { runity_editor_create_for_layer(std::ptr::null_mut(), 100, 100) };
    assert!(editor.is_null());

    let mut message = vec![0u8; 128];
    let length = unsafe { runity_last_error(message.as_mut_ptr() as *mut c_char, 128) };
    assert!(length > 0, "and a reason, not silence");
}

#[test]
fn editing_and_undoing_goes_through_the_boundary_intact() {
    let Some((editor, _)) = open("edit") else {
        return;
    };
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);
    assert!(!unsafe { runity_editor_can_undo(editor.0) }, "nothing yet");

    let model = CString::new("builtin:sphere").unwrap();
    let added = unsafe { runity_editor_add(editor.0, -1, model.as_ptr()) };
    assert_eq!(added, 3, "appended at the top level");
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 4);
    assert!(unsafe { runity_editor_can_undo(editor.0) });

    // Duplicating the crate copies its child too.
    let copy = unsafe { runity_editor_duplicate(editor.0, 1) };
    assert!(copy > 0);
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 6);

    assert!(unsafe { runity_editor_undo(editor.0) });
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 4);
    assert!(unsafe { runity_editor_undo(editor.0) });
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);
    assert!(
        !unsafe { runity_editor_undo(editor.0) },
        "back to the start"
    );

    assert!(unsafe { runity_editor_can_redo(editor.0) });
    assert!(unsafe { runity_editor_redo(editor.0) });
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 4);
}

#[test]
fn a_whole_drag_undoes_in_one_step() {
    // The thing an editor gets wrong: sixty mutations a second becoming
    // sixty undo steps, so undo appears not to work at all.
    let Some((editor, _)) = open("drag-undo") else {
        return;
    };
    let eye = [0.0f32, 1.0, 6.0];
    let target = [0.0f32, 0.5, 0.0];
    unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), target.as_ptr()) };
    unsafe { runity_editor_select(editor.0, 1) };

    let mut before = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, before.as_mut_ptr()) };

    let height = unsafe { runity_editor_height(editor.0) };
    let width = unsafe { runity_editor_width(editor.0) };
    let Some((grab_x, _)) = (0..width)
        .map(|x| {
            (x, unsafe {
                runity_editor_gizmo_begin(editor.0, x, height / 2)
            })
        })
        .find(|(_, arm)| *arm >= 0)
    else {
        return;
    };
    for step in 1..=40 {
        unsafe { runity_editor_gizmo_drag(editor.0, grab_x + step, height / 2) };
    }
    unsafe { runity_editor_gizmo_end(editor.0) };

    let mut moved = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, moved.as_mut_ptr()) };
    assert_ne!(moved[..3], before[..3], "it moved");

    assert!(unsafe { runity_editor_undo(editor.0) });
    let mut back = [0.0f32; 9];
    unsafe { runity_editor_get_transform(editor.0, 1, back.as_mut_ptr()) };
    assert_eq!(back, before, "one undo restores the whole gesture");
    assert!(!unsafe { runity_editor_can_undo(editor.0) }, "and only one");
}

#[test]
fn reparenting_refuses_to_make_a_loop_across_the_boundary() {
    let Some((editor, _)) = open("reparent") else {
        return;
    };
    // The lid is the crate's child; putting the crate under the lid would
    // make a cycle, and a cycle in a scene tree is unrecoverable.
    assert!(!unsafe { runity_editor_reparent(editor.0, 1, 2) });
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);

    // Moving the lid to the top level is fine.
    assert!(unsafe { runity_editor_reparent(editor.0, 2, -1) });
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);
}

#[test]
fn deleting_clears_a_selection_that_would_otherwise_point_at_a_stranger() {
    let Some((editor, _)) = open("delete") else {
        return;
    };
    unsafe { runity_editor_select(editor.0, 1) };
    assert_eq!(unsafe { runity_editor_selected(editor.0) }, 1);

    assert!(unsafe { runity_editor_delete(editor.0, 1) });
    assert_eq!(
        unsafe { runity_editor_selected(editor.0) },
        -1,
        "an index into a list that changed shape points at whatever slid \
         into the gap"
    );
    assert_eq!(
        unsafe { runity_editor_entity_count(editor.0) },
        1,
        "the lid went too"
    );
}

/// Read one entity's material through the boundary.
fn material_of(editor: *mut Editor, index: u32) -> [f32; 4] {
    let mut four = [0.0f32; 4];
    assert!(unsafe { runity_editor_get_material(editor, index, four.as_mut_ptr()) });
    four
}

/// The name it points at, or "" when it carries its own colour.
fn material_name_of(editor: *mut Editor, index: u32) -> String {
    let mut buffer = vec![0u8; 128];
    unsafe { runity_editor_material_name(editor, index, buffer.as_mut_ptr() as *mut c_char, 128) };
    std::ffi::CStr::from_bytes_until_nul(&buffer)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_colour_can_be_tuned_on_one_object_and_taken_back() {
    // The thing a person actually wants an editor for: nudge a colour, look
    // at it, undo it. A scene file cannot do this, which is why the editor
    // exists at all.
    let Some((editor, path)) = open("material") else {
        return;
    };
    let before = material_of(editor.0, 1);
    assert_eq!(before, [0.8, 0.8, 0.8, 0.0], "the default grey, lit");

    let wanted = [0.31, 0.12, 0.05, 0.0];
    assert!(unsafe { runity_editor_set_material(editor.0, 1, wanted.as_ptr()) });
    assert_eq!(material_of(editor.0, 1), wanted);
    assert_eq!(
        material_name_of(editor.0, 1),
        "",
        "a tuned colour belongs to the object, not to a name"
    );

    // It reaches the file, and it comes back.
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    let reopened = runity::Scene::load(&path).unwrap();
    assert_eq!(
        reopened.find("crate").unwrap().material().base_color[0],
        0.31
    );

    assert!(unsafe { runity_editor_undo(editor.0) });
    assert_eq!(
        material_of(editor.0, 1),
        before,
        "one step, all the way back"
    );
}

#[test]
fn an_entity_can_be_pointed_at_the_palette_and_reports_what_it_became() {
    let Some((editor, path)) = open("palette") else {
        return;
    };
    let stone = CString::new("stone").unwrap();
    assert!(unsafe { runity_editor_set_material_name(editor.0, 1, stone.as_ptr()) });
    assert_eq!(material_name_of(editor.0, 1), "stone");

    // The colour reported is the colour stone is, not the word: an
    // inspector's swatch has to match what is on screen.
    let reported = material_of(editor.0, 1);
    let builtin = runity::material::builtin::STONE;
    assert_eq!(
        [reported[0], reported[1], reported[2]],
        builtin.base_color,
        "the name should resolve to a colour"
    );

    // And the file keeps the name rather than the colour, which is the whole
    // point of a palette.
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("\"stone\""));

    // An empty name is refused: clearing a link means giving a colour.
    let empty = CString::new("").unwrap();
    assert!(!unsafe { runity_editor_set_material_name(editor.0, 1, empty.as_ptr()) });
    assert_eq!(material_name_of(editor.0, 1), "stone", "nothing changed");
}

#[test]
fn the_palette_offers_the_builtins_and_the_librarys_own() {
    let Some((editor, _)) = open("palette-list") else {
        return;
    };

    // With no library it is the builtins, each with a colour to draw a
    // swatch with.
    let count = unsafe { runity_editor_palette_count(editor.0) };
    assert_eq!(count as usize, runity::material::builtin::NAMES.len());
    let mut names = Vec::new();
    for i in 0..count {
        let mut buffer = vec![0u8; 64];
        unsafe {
            runity_editor_palette_name(editor.0, i, buffer.as_mut_ptr() as *mut c_char, 64);
        }
        names.push(
            std::ffi::CStr::from_bytes_until_nul(&buffer)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
        let mut colour = [0.0f32; 4];
        assert!(unsafe { runity_editor_palette_color(editor.0, i, colour.as_mut_ptr()) });
        assert!(colour[..3].iter().all(|c| (0.0..=1.0).contains(c)));
    }
    assert!(names.contains(&"ember".to_string()));

    // Nothing past the end, rather than whatever is next in memory.
    assert!(!unsafe { runity_editor_palette_color(editor.0, count, [0.0f32; 4].as_mut_ptr()) });

    // Point it at a library holding two materials: one new, one shadowing a
    // builtin. The palette should gain one entry, not two, and `stone`
    // should now be the project's.
    let library = std::env::temp_dir().join("runity-ffi-palette-library");
    let _ = std::fs::remove_dir_all(&library);
    std::fs::create_dir_all(&library).unwrap();
    write_material(&library, "moss", runity::Material::new(0.1, 0.3, 0.05));
    write_material(&library, "stone", runity::Material::new(0.9, 0.0, 0.0));
    assert!(unsafe { runity_editor_set_library(editor.0, c(&library).as_ptr()) });

    let after = unsafe { runity_editor_palette_count(editor.0) };
    assert_eq!(
        after,
        count + 1,
        "one new name, and `stone` listed once rather than twice"
    );

    let stone = CString::new("stone").unwrap();
    assert!(unsafe { runity_editor_set_material_name(editor.0, 1, stone.as_ptr()) });
    let reported = material_of(editor.0, 1);
    assert!(
        reported[0] > 0.8 && reported[1] < 0.01,
        "the project's stone, not the engine's: {reported:?}"
    );

    let forced = CString::new("builtin:stone").unwrap();
    assert!(unsafe { runity_editor_set_material_name(editor.0, 1, forced.as_ptr()) });
    assert_eq!(
        material_of(editor.0, 1)[..3],
        runity::material::builtin::STONE.base_color,
        "and the engine's is still reachable"
    );
}

/// Write a material asset straight into a library directory.
///
/// No importer here on purpose: the editor's boundary should not depend on
/// `runity-import`, and a test that reached for it would hide the day it
/// started to.
fn write_material(directory: &std::path::Path, name: &str, material: runity::Material) {
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
fn the_colour_conversions_are_the_engines_own_and_round_trip() {
    // A host that carries its own curve gets it slightly wrong — usually as
    // powf(2.2) — and the colour picked is not the colour drawn.
    for step in 0..=10 {
        let c = step as f32 / 10.0;
        let back = runity_linear_to_srgb(runity_srgb_to_linear(c));
        assert!((back - c).abs() < 1e-4, "{c} came back as {back}");
    }
    // Mid grey in sRGB is not half in linear, which is the whole reason this
    // is not a multiplication.
    assert!((runity_srgb_to_linear(0.5) - 0.2140).abs() < 0.001);
}

#[test]
fn opening_a_scene_looks_where_the_scene_says_and_keeping_a_view_is_a_decision() {
    let Some((editor, path)) = open("camera") else {
        return;
    };

    // The test scene carries no view, so it opens at the format's default
    // rather than at whatever the last document left behind.
    let (mut eye, mut target) = ([0.0f32; 3], [0.0f32; 3]);
    assert!(unsafe { runity_editor_get_camera(editor.0, eye.as_mut_ptr(), target.as_mut_ptr()) });
    let default = runity::View::default();
    assert_eq!(eye, default.position.to_array());

    // Flying around is not an edit: nothing is dirtied and nothing is saved.
    let moved = [4.0f32, 9.0, 1.0];
    let at = [0.0f32, 1.0, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, moved.as_ptr(), at.as_ptr()) });
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    assert_eq!(
        runity::Scene::load(&path).unwrap().view,
        default,
        "looking around should not have changed the file"
    );

    // Keeping it is. And it survives the round trip.
    assert!(unsafe { runity_editor_capture_camera(editor.0) });
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    let kept = runity::Scene::load(&path).unwrap().view;
    assert_eq!(kept.position.to_array(), moved);
    assert_eq!(kept.target.to_array(), at);

    // One undoable step, like anything else typed into an inspector.
    assert!(unsafe { runity_editor_undo(editor.0) });
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    assert_eq!(runity::Scene::load(&path).unwrap().view, default);

    // And reopening a scene with a view in it points the camera there.
    let framed = path.parent().unwrap().join("framed.ron");
    std::fs::write(
        &framed,
        r#"(view: (position: (1.0, 2.0, 3.0), target: (0.0, 0.0, 0.0), fov_deg: 40.0), entities: [])"#,
    )
    .unwrap();
    assert!(unsafe { runity_editor_open_scene(editor.0, c(&framed).as_ptr()) });
    assert!(unsafe { runity_editor_get_camera(editor.0, eye.as_mut_ptr(), target.as_mut_ptr()) });
    assert_eq!(eye, [1.0, 2.0, 3.0]);
    assert_eq!(target, [0.0, 0.0, 0.0]);
}

/// Read a string out of one of the buffer-and-capacity calls.
fn text_from(call: impl FnOnce(*mut c_char, u32) -> u32) -> String {
    let mut buffer = vec![0u8; 128];
    call(buffer.as_mut_ptr() as *mut c_char, 128);
    std::ffi::CStr::from_bytes_until_nul(&buffer)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_thing_arranged_once_becomes_a_thing_placed_many_times() {
    let Some((editor, path)) = open("prefab") else {
        return;
    };
    // The crate has a lid, so it is a two-entity arrangement: exactly the
    // kind of thing that should stop being copied by hand.
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);

    let name = CString::new("crate").unwrap();
    assert!(unsafe { runity_editor_make_prefab(editor.0, 1, name.as_ptr()) });

    // The scene now holds one row where there were two, and it says what it
    // is an instance of.
    assert_eq!(
        unsafe { runity_editor_entity_count(editor.0) },
        2,
        "the lid lives in the prefab file now"
    );
    assert_eq!(
        text_from(|b, c| unsafe { runity_editor_entity_prefab(editor.0, 1, b, c) }),
        "crate"
    );
    let written = path.parent().unwrap().join("prefabs/crate.prefab");
    assert!(
        written.exists(),
        "{} should have been written",
        written.display()
    );

    // Place a second one. Both are one row each, and both draw the whole
    // thing — which is what `entity_count` not moving and the pick below
    // between them prove.
    let placed = unsafe { runity_editor_add_instance(editor.0, -1, name.as_ptr()) };
    assert!(placed >= 0, "placing an instance");
    assert_eq!(unsafe { runity_editor_entity_count(editor.0) }, 3);
    assert_eq!(
        text_from(|b, c| unsafe { runity_editor_entity_prefab(editor.0, placed as u32, b, c) }),
        "crate"
    );

    // A prefab nothing answers to is refused rather than left as an entity
    // with no model and no explanation.
    let missing = CString::new("not_a_prefab").unwrap();
    assert_eq!(
        unsafe { runity_editor_add_instance(editor.0, -1, missing.as_ptr()) },
        -1
    );

    // It is listed for a host to offer.
    assert_eq!(unsafe { runity_editor_prefab_count(editor.0) }, 1);
    assert_eq!(
        text_from(|b, c| unsafe { runity_editor_prefab_name(editor.0, 0, b, c) }),
        "crate"
    );

    // And the scene reopens: the reference is what was saved, and opening
    // finds the prefabs beside it without being told where they are.
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("prefab"),
        "the scene keeps the reference:\n{text}"
    );
    // `name: "lid"`, not "lid" — which also appears inside `collider`, and
    // spent a run insisting this had failed when it had not.
    assert!(
        !text.contains(r#"name: "lid""#),
        "and not a copy of what is in it"
    );
    assert!(unsafe { runity_editor_open_scene(editor.0, c(&path).as_ptr()) });
    assert_eq!(unsafe { runity_editor_prefab_count(editor.0) }, 1);
}

#[test]
fn clicking_something_a_prefab_brought_selects_the_instance() {
    // The rule that makes an instance one thing: a click on the lid is a
    // click on the crate, because the crate is what the document can move.
    let Some((editor, _)) = open("prefab-pick") else {
        return;
    };
    let eye = [0.0f32, 1.2, 6.0];
    let at = [0.0f32, 0.9, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), at.as_ptr()) });

    // The lid sits above the crate, so a ray through the upper middle finds
    // the lid first. Before it is a prefab, that is its own entity.
    let (x, y) = (96u32, 52u32);
    let before = unsafe { runity_editor_pick(editor.0, x, y) };
    assert_eq!(before, 2, "the lid, which has its own row");

    let name = CString::new("crate").unwrap();
    assert!(unsafe { runity_editor_make_prefab(editor.0, 1, name.as_ptr()) });
    let after = unsafe { runity_editor_pick(editor.0, x, y) };
    assert_eq!(
        after, 1,
        "the same pixel now finds the instance, not a part of it"
    );
}

#[test]
fn the_rotate_tool_turns_the_thing_it_is_on_and_undoes_in_one_step() {
    let Some((editor, path)) = open("rotate") else {
        return;
    };
    // Looking down at the crate from above, so the flat Y ring is the one
    // facing the camera and the one a ray can cross.
    let eye = [0.0f32, 8.0, 0.01];
    let at = [0.0f32, 0.5, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), at.as_ptr()) });
    assert!(unsafe { runity_editor_select(editor.0, 1) });

    assert!(unsafe { runity_editor_set_tool(editor.0, 1) });
    assert_eq!(unsafe { runity_editor_tool(editor.0) }, 1);
    assert!(
        !unsafe { runity_editor_set_tool(editor.0, 7) },
        "a tool nobody has is refused rather than treated as move"
    );
    assert_eq!(unsafe { runity_editor_tool(editor.0) }, 1, "and unchanged");

    // Grab the Y ring where it crosses the screen, then drag a quarter of
    // the way around it. The exact pixels do not matter; what matters is
    // that the rotation changes and nothing else does.
    let width = unsafe { runity_editor_width(editor.0) };
    let height = unsafe { runity_editor_height(editor.0) };
    let (cx, cy) = (width / 2, height / 2);
    let mut grabbed = -1;
    let mut radius = 0;
    for r in 4..(width / 2) {
        let handle = unsafe { runity_editor_gizmo_begin(editor.0, cx + r, cy) };
        if handle >= 0 {
            grabbed = handle;
            radius = r;
            break;
        }
    }
    assert_eq!(grabbed, 1, "the Y ring, seen from above");

    assert!(unsafe { runity_editor_gizmo_drag(editor.0, cx, cy + radius) });
    unsafe { runity_editor_gizmo_end(editor.0) };

    let mut nine = [0.0f32; 9];
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    let yaw = nine[4];
    assert!(
        (yaw.abs() - 90.0).abs() < 5.0,
        "a quarter turn around the ring should be about ninety degrees, got {yaw}"
    );
    assert_eq!(
        [nine[0], nine[1], nine[2]],
        [0.0, 0.5, 0.0],
        "it did not move"
    );
    assert_eq!(
        [nine[6], nine[7], nine[8]],
        [1.0, 1.0, 1.0],
        "and did not grow"
    );

    // The whole gesture is one step, like a move drag.
    assert!(unsafe { runity_editor_undo(editor.0) });
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    assert!(
        nine[4].abs() < 1e-3,
        "back to where it started: {}",
        nine[4]
    );

    // And what is written is the local rotation, which is what reopens.
    assert!(unsafe { runity_editor_redo(editor.0) });
    assert!(unsafe { runity_editor_save_scene(editor.0, std::ptr::null()) });
    let reopened = runity::Scene::load(&path).unwrap();
    let turned = reopened.find("crate").unwrap().transform.rotation_deg.y;
    assert!((turned.abs() - 90.0).abs() < 5.0, "got {turned}");
}

#[test]
fn the_scale_tool_stretches_one_axis_and_leaves_the_rest() {
    let Some((editor, _)) = open("scale") else {
        return;
    };
    let eye = [0.0f32, 0.5, 6.0];
    let at = [0.0f32, 0.5, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), at.as_ptr()) });
    assert!(unsafe { runity_editor_select(editor.0, 1) });
    assert!(unsafe { runity_editor_set_tool(editor.0, 2) });

    let width = unsafe { runity_editor_width(editor.0) };
    let height = unsafe { runity_editor_height(editor.0) };
    let (cx, cy) = (width / 2, height / 2);
    let mut grabbed = -1;
    let mut radius = 0;
    for r in 4..(width / 2) {
        let handle = unsafe { runity_editor_gizmo_begin(editor.0, cx + r, cy) };
        if handle >= 0 {
            grabbed = handle;
            radius = r;
            break;
        }
    }
    assert_eq!(grabbed, 0, "the X arm");

    // Drag outward: the same arm, twice as far from the middle.
    assert!(unsafe { runity_editor_gizmo_drag(editor.0, cx + radius * 2, cy) });
    unsafe { runity_editor_gizmo_end(editor.0) };

    let mut nine = [0.0f32; 9];
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    assert!(nine[6] > 1.2, "x should have grown, got {}", nine[6]);
    assert_eq!((nine[7], nine[8]), (1.0, 1.0), "y and z are untouched");
    assert_eq!(
        [nine[0], nine[1], nine[2]],
        [0.0, 0.5, 0.0],
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

fn falling_scene(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("runity-ffi-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    std::fs::write(&path, FALLING).unwrap();
    path
}

/// Where an entity actually is in the world, which during play is not what
/// the document says.
fn drawn_height(editor: *mut Editor) -> f32 {
    let mut three = [0.0f32; 3];
    assert!(unsafe { runity_editor_world_position(editor, 1, three.as_mut_ptr()) });
    three[1]
}

#[test]
fn pressing_play_drops_the_crate_and_stopping_puts_it_back() {
    let editor = unsafe { runity_editor_create_offscreen(192, 128) };
    if editor.is_null() {
        return;
    }
    let editor = Handle(editor);
    let path = falling_scene("play");
    assert!(unsafe { runity_editor_open_scene(editor.0, c(&path).as_ptr()) });
    assert!(!unsafe { runity_editor_is_playing(editor.0) });

    let before = drawn_height(editor.0);
    assert!(
        (before - 4.0).abs() < 1e-3,
        "it starts up in the air: {before}"
    );

    assert!(unsafe { runity_editor_play(editor.0) });
    assert!(unsafe { runity_editor_is_playing(editor.0) });

    // Two seconds of frames. The count of fixed steps is what the engine
    // decides, not what the host asks for: sixty a second at the default
    // rate.
    let mut steps = 0;
    for _ in 0..120 {
        steps += unsafe { runity_editor_step(editor.0, 1.0 / 60.0) };
    }
    assert!(
        (100..=140).contains(&steps),
        "about two seconds of fixed steps, got {steps}"
    );

    let landed = drawn_height(editor.0);
    assert!(
        (landed - 0.55).abs() < 0.2,
        "the crate should be resting on the floor, got {landed}"
    );

    // The document never moved: play is a preview, not an edit.
    let mut nine = [0.0f32; 9];
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    assert!(
        (nine[1] - 4.0).abs() < 1e-3,
        "the file still says 4: {}",
        nine[1]
    );
    assert!(
        !unsafe { runity_editor_can_undo(editor.0) },
        "and nothing to undo"
    );

    // Editing while playing is refused rather than thrown away later.
    let values: [f32; 9] = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    assert!(!unsafe { runity_editor_set_transform(editor.0, 1, values.as_ptr()) });
    let mut message = vec![0u8; 256];
    unsafe { runity_last_error(message.as_mut_ptr() as *mut c_char, 256) };
    let message = String::from_utf8_lossy(&message);
    assert!(message.contains("playing"), "it should say why: {message}");

    assert!(unsafe { runity_editor_stop(editor.0) });
    assert!(!unsafe { runity_editor_is_playing(editor.0) });
    let back = drawn_height(editor.0);
    assert!((back - 4.0).abs() < 1e-3, "back up in the air: {back}");
    assert_eq!(
        unsafe { runity_editor_step(editor.0, 0.1) },
        0,
        "and stopped"
    );

    // Editing works again, and stopping did not cost a step of real history.
    assert!(unsafe { runity_editor_set_transform(editor.0, 1, values.as_ptr()) });
    assert!(unsafe { runity_editor_can_undo(editor.0) });
    assert!(unsafe { runity_editor_undo(editor.0) });
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    assert!((nine[1] - 4.0).abs() < 1e-3);
}

#[test]
fn a_drag_with_snapping_on_lands_on_the_grid() {
    let Some((editor, _)) = open("snap") else {
        return;
    };
    assert!(unsafe { runity_editor_select(editor.0, 1) });
    assert!(unsafe { runity_editor_set_snap(editor.0, 0.5, 15.0, 0.25) });
    let mut three = [0.0f32; 3];
    assert!(unsafe { runity_editor_snap(editor.0, three.as_mut_ptr()) });
    assert_eq!(three, [0.5, 15.0, 0.25]);

    // Grab the X arm and drag it somewhere off-grid.
    let eye = [0.0f32, 1.0, 6.0];
    let at = [0.0f32, 0.5, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, eye.as_ptr(), at.as_ptr()) });
    let width = unsafe { runity_editor_width(editor.0) };
    let height = unsafe { runity_editor_height(editor.0) };
    let (cx, cy) = (width / 2, height / 2);
    let mut radius = 0;
    for r in 4..(width / 2) {
        if unsafe { runity_editor_gizmo_begin(editor.0, cx + r, cy) } == 0 {
            radius = r;
            break;
        }
    }
    assert!(radius > 0, "the X arm is somewhere to the right of centre");
    assert!(unsafe { runity_editor_gizmo_drag(editor.0, cx + radius + 37, cy) });
    unsafe { runity_editor_gizmo_end(editor.0) };

    let mut nine = [0.0f32; 9];
    assert!(unsafe { runity_editor_get_transform(editor.0, 1, nine.as_mut_ptr()) });
    assert!(nine[0] != 0.0, "it should have moved at all: {nine:?}");
    assert!(
        (nine[0] / 0.5 - (nine[0] / 0.5).round()).abs() < 1e-4,
        "x should be a multiple of half a metre, got {}",
        nine[0]
    );
    // And the other two are still exactly where they were: snapping must
    // not drag an untouched axis onto the grid behind your back.
    assert_eq!((nine[1], nine[2]), (0.5, 0.0));
}

#[test]
fn focusing_frames_the_selection_whatever_size_it_is() {
    let Some((editor, _)) = open("focus") else {
        return;
    };
    // Start looking somewhere else entirely.
    let away = [40.0f32, 30.0, 40.0];
    let at = [40.0f32, 0.0, 0.0];
    assert!(unsafe { runity_editor_set_camera(editor.0, away.as_ptr(), at.as_ptr()) });
    assert!(
        !unsafe { runity_editor_focus_selected(editor.0) },
        "nothing selected, nothing to focus"
    );

    assert!(unsafe { runity_editor_select(editor.0, 1) });
    assert!(unsafe { runity_editor_focus_selected(editor.0) });

    let (mut eye, mut target) = ([0.0f32; 3], [0.0f32; 3]);
    assert!(unsafe { runity_editor_get_camera(editor.0, eye.as_mut_ptr(), target.as_mut_ptr()) });
    let mut three = [0.0f32; 3];
    assert!(unsafe { runity_editor_world_position(editor.0, 1, three.as_mut_ptr()) });
    assert_eq!(target, three, "it looks at the thing");

    let distance = ((eye[0] - target[0]).powi(2)
        + (eye[1] - target[1]).powi(2)
        + (eye[2] - target[2]).powi(2))
    .sqrt();
    assert!(
        (1.0..12.0).contains(&distance),
        "a metre-wide crate should be framed from a few metres, got {distance}"
    );

    // The big one is framed from further away, which is the whole point of
    // sizing it from what is selected.
    let nine: [f32; 9] = [0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
    assert!(unsafe { runity_editor_set_transform(editor.0, 1, nine.as_ptr()) });
    assert!(unsafe { runity_editor_focus_selected(editor.0) });
    assert!(unsafe { runity_editor_get_camera(editor.0, eye.as_mut_ptr(), target.as_mut_ptr()) });
    let bigger = ((eye[0] - target[0]).powi(2)
        + (eye[1] - target[1]).powi(2)
        + (eye[2] - target[2]).powi(2))
    .sqrt();
    assert!(
        bigger > distance * 3.0,
        "ten times the size should be much further back: {distance} then {bigger}"
    );
}

#[test]
fn a_tuned_colour_becomes_a_material_every_scene_can_name() {
    let Some((editor, path)) = open("save-material") else {
        return;
    };
    let library = path.parent().unwrap().join("library");
    std::fs::create_dir_all(&library).unwrap();
    assert!(unsafe { runity_editor_set_library(editor.0, c(&library).as_ptr()) });

    // Tune a colour on one object, the way a slider would.
    let wanted = [0.31f32, 0.12, 0.05, 0.0];
    assert!(unsafe { runity_editor_set_material(editor.0, 1, wanted.as_ptr()) });

    let name = CString::new("clay").unwrap();
    assert!(unsafe { runity_editor_save_material(editor.0, 1, name.as_ptr()) });

    // What came out is a file a person could have written.
    let source = path.parent().unwrap().join("materials/clay.rmat");
    let text = std::fs::read_to_string(&source).expect("the .rmat source");
    assert!(
        text.contains("color: \"#"),
        "sRGB hex, not a dump of floats: {text}"
    );
    assert!(library.join("clay.rasset").exists(), "and it was imported");

    // The entity now names it, and still draws the colour — to within the
    // eight bits a hex code has.
    assert_eq!(material_name_of(editor.0, 1), "clay");
    let drawn = material_of(editor.0, 1);
    for axis in 0..3 {
        assert!(
            (drawn[axis] - wanted[axis]).abs() < 0.005,
            "channel {axis}: {} against {}",
            drawn[axis],
            wanted[axis]
        );
    }

    // And it is in the palette, so the next object can be given the same
    // colour by name rather than by eye.
    let count = unsafe { runity_editor_palette_count(editor.0) };
    let mut names = Vec::new();
    for i in 0..count {
        names.push(text_from(|b, c| unsafe {
            runity_editor_palette_name(editor.0, i, b, c)
        }));
    }
    assert!(names.contains(&"clay".to_string()), "{names:?}");

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
    assert_eq!(unsafe { runity_editor_reload_assets(editor.0) }, 1);
    let after = material_of(editor.0, 1);
    assert!(
        after[2] > after[0],
        "warm clay should have become cool: {after:?}"
    );
}

#[test]
fn a_model_dropped_on_the_editor_becomes_something_a_scene_can_use() {
    let Some((editor, path)) = open("import") else {
        return;
    };
    let library = path.parent().unwrap().join("library");
    std::fs::create_dir_all(&library).unwrap();
    assert!(unsafe { runity_editor_set_library(editor.0, c(&library).as_ptr()) });

    // A triangle is enough: what is being tested is the path, not the
    // parser, which has its own tests next door.
    let source = path.parent().unwrap().join("wedge.obj");
    std::fs::write(
        &source,
        "v -1.0 0.0 -1.0\nv  1.0 0.0 -1.0\nv  1.0 0.0  1.0\nf 1 3 2\n",
    )
    .unwrap();
    assert!(unsafe { runity_editor_import(editor.0, c(&source).as_ptr()) });
    assert!(library.join("wedge.rasset").exists());

    // And a scene can use it straight away, by the name the file had.
    let model = CString::new("wedge").unwrap();
    let added = unsafe { runity_editor_add(editor.0, -1, model.as_ptr()) };
    assert!(added >= 0);
    let mut three = [0.0f32; 3];
    assert!(unsafe { runity_editor_world_position(editor.0, added as u32, three.as_mut_ptr()) });

    // Importing without a library says so rather than writing somewhere
    // surprising.
    let Some((fresh, _)) = open("import-nowhere") else {
        return;
    };
    assert!(!unsafe { runity_editor_import(fresh.0, c(&source).as_ptr()) });
    let mut message = vec![0u8; 256];
    unsafe { runity_last_error(message.as_mut_ptr() as *mut c_char, 256) };
    assert!(String::from_utf8_lossy(&message).contains("library"));
}
