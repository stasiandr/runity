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
