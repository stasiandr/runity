//! `gizmo_shot` — a scene with the gizmo on the selection, rendered
//! headlessly to PNGs: each tool, a handle under the pointer, a turn in
//! progress with its pie and angle, a stretch, and the selection's
//! outline. What the editor's viewport would show, without a window.
//!
//! ```text
//! cargo run --release -p runity-editor --example gizmo_shot [out dir]
//! ```

use runity::gizmo::{Grip, Handle, Tool};
use runity::glam::Vec3;
use runity_editor::Session;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("runity-gizmo-shot"));
    std::fs::create_dir_all(&dir)?;
    let scene = dir.join("scene.ron");
    std::fs::write(
        &scene,
        r#"(
    fog: (color: (0.62, 0.68, 0.74), start: 30.0, end: 120.0),
    entities: [
        (name: "ground", model: "builtin:plane", transform: (scale: (40.0, 1.0, 40.0)), material: "grass"),
        (name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.5, 0.0)), material: "earth", children: [
            (name: "lid", model: "builtin:cube", transform: (position: (0.0, 0.6, 0.0), scale: (0.8, 0.2, 0.8)), material: "stone"),
        ]),
        (name: "rock", model: "builtin:sphere", transform: (position: (1.8, 0.4, 0.6), scale: (0.8, 0.8, 0.8)), material: "stone"),
        (name: "post", model: "builtin:cube", transform: (position: (-0.6, 0.9, 1.4), scale: (0.3, 1.8, 0.3)), material: "earth"),
    ],
)"#,
    )?;

    // Twice a point, as on a Retina screen.
    let mut session = Session::offscreen(1280, 800)?;
    session.set_ui_scale(2.0);
    session.open_scene(&scene)?;
    session.set_camera(Vec3::new(2.6, 2.2, 4.2), Vec3::new(0.0, 0.5, 0.0));
    session.select(session.find("crate"))?;

    let shoot = |session: &mut Session, name: &str| -> Result<(), Box<dyn std::error::Error>> {
        session.render();
        let (width, height) = session.size();
        let out = dir.join(format!("{name}.png"));
        image::save_buffer(
            &out,
            session.frame_pixels(),
            width,
            height,
            image::ColorType::Rgba8,
        )?;
        println!("wrote {}", out.display());
        Ok(())
    };

    for (tool, name) in [
        (Tool::Move, "move"),
        (Tool::Rotate, "rotate"),
        (Tool::Scale, "scale"),
        (Tool::Transform, "transform"),
        (Tool::Rect, "rect"),
    ] {
        session.set_tool(tool);
        session.set_gizmo_hover(None);
        shoot(&mut session, name)?;
    }

    // A handle under the pointer: yellow.
    session.set_tool(Tool::Move);
    if let Some(at) = find(&session, Grip::new(Tool::Move, Handle::X)) {
        session.set_gizmo_hover(Some(at));
        shoot(&mut session, "move_hover")?;
    }
    session.set_gizmo_hover(None);

    // A turn in progress: the Y ring grabbed and dragged part way round.
    session.set_tool(Tool::Rotate);
    if let Some((x, y)) = find(&session, Grip::new(Tool::Rotate, Handle::Y)) {
        session.gizmo_begin(x, y)?;
        session.gizmo_drag(x.saturating_sub(260), y.saturating_sub(40))?;
        shoot(&mut session, "rotate_drag")?;
        session.gizmo_end();
        session.undo()?;
    }

    // A stretch in progress: the X arm pulled out.
    session.set_tool(Tool::Scale);
    if let Some((x, y)) = find(&session, Grip::new(Tool::Scale, Handle::X)) {
        session.gizmo_begin(x, y)?;
        session.gizmo_drag(x + 60, y + 20)?;
        shoot(&mut session, "scale_drag")?;
        session.gizmo_end();
        session.undo()?;
    }

    // The outline: the crate orange, its lid blue, the post in front of
    // part of it.
    session.set_tool(Tool::Move);
    session.set_camera(Vec3::new(-2.2, 1.6, 4.6), Vec3::new(0.0, 0.6, 0.0));
    shoot(&mut session, "outline")?;
    Ok(())
}

/// A pixel where a handle is grabbed, found by looking, not assumed.
fn find(session: &Session, grip: Grip) -> Option<(u32, u32)> {
    let (w, h) = session.size();
    (0..h)
        .step_by(4)
        .flat_map(|y| (0..w).step_by(4).map(move |x| (x, y)))
        .filter(|&(x, y)| session.gizmo_grip_at(x, y) == Some(grip))
        // The last found, top to bottom: the far end of an arm pointing
        // down, the near side of a ring.
        .last()
}
