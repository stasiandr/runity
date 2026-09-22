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
