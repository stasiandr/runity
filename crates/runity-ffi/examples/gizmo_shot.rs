use runity_ffi::*;
use std::ffi::CString;

fn main() {
    let dir = std::env::temp_dir().join("runity-gizmo-shot");
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.ron");
    std::fs::write(&scene, r#"(
    fog: (color: (0.62, 0.68, 0.74), start: 30.0, end: 120.0),
    entities: [
        (name: "ground", model: "builtin:plane", transform: (scale: (40.0, 1.0, 40.0)), material: "grass"),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.5, 0.0)), material: "earth"),
        (name: "rock", model: "builtin:sphere", transform: (position: (1.8, 0.4, 0.6), scale: (0.8, 0.8, 0.8)), material: "stone"),
    ],
)"#).unwrap();

    unsafe {
        let editor = runity_editor_create_offscreen(640, 400);
        assert!(!editor.is_null());
        let path = CString::new(scene.to_string_lossy().as_bytes()).unwrap();
        assert!(runity_editor_open_scene(editor, path.as_ptr()));
        let eye = [2.6f32, 2.2, 4.2];
        let target = [0.0f32, 0.5, 0.0];
        runity_editor_set_camera(editor, eye.as_ptr(), target.as_ptr());
        assert!(runity_editor_select(editor, 1));
        assert!(runity_editor_render(editor));

        let needed = runity_editor_frame_pixels(editor, std::ptr::null_mut(), 0);
        let mut pixels = vec![0u8; needed as usize];
        runity_editor_frame_pixels(editor, pixels.as_mut_ptr(), needed);
        image::save_buffer(
            dir.join("gizmo.png"),
            &pixels,
            640,
            400,
            image::ColorType::Rgba8,
        )
        .unwrap();
        println!("wrote {}", dir.join("gizmo.png").display());
        runity_editor_free(editor);
    }
}
