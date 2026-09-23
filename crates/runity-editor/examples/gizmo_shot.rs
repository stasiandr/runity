//! `gizmo_shot` — a scene with the gizmo on the selection, rendered
//! headlessly to a PNG. What the editor's viewport would show, without a
//! window.

use runity::glam::Vec3;
use runity_editor::Session;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("runity-gizmo-shot");
    std::fs::create_dir_all(&dir)?;
    let scene = dir.join("scene.ron");
    std::fs::write(
        &scene,
        r#"(
    fog: (color: (0.62, 0.68, 0.74), start: 30.0, end: 120.0),
    entities: [
        (name: "ground", model: "builtin:plane", transform: (scale: (40.0, 1.0, 40.0)), material: "grass"),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.5, 0.0)), material: "earth"),
        (name: "rock", model: "builtin:sphere", transform: (position: (1.8, 0.4, 0.6), scale: (0.8, 0.8, 0.8)), material: "stone"),
    ],
)"#,
    )?;

    let mut session = Session::offscreen(640, 400)?;
    session.open_scene(&scene)?;
    session.set_camera(Vec3::new(2.6, 2.2, 4.2), Vec3::new(0.0, 0.5, 0.0));
    session.select(Some(1))?;
    session.render();

    let (width, height) = session.size();
    let out = dir.join("gizmo.png");
    image::save_buffer(
        &out,
        session.frame_pixels(),
        width,
        height,
        image::ColorType::Rgba8,
    )?;
    println!("wrote {}", out.display());
    Ok(())
}
