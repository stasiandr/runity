//! Card #49 in one picture: four lean-tos around the camp's one hearth, each
//! with its own dragged-in log — no shared unfinished wall in sight.
//!
//! ```text
//! cargo run --release --example demo
//! ```
//!
//! `04-player.md`'s "Первые десять минут" used to say the four settlers the
//! player wakes up next to are dragging logs to an unfinished communal wall.
//! `12-minds.md` corrects that: at Camp, before any settlement-level stake
//! exists, a shalash is personal business — each settler raises their own in
//! an evening, with no shared decision behind it (a stake only shows up once
//! a deficit gets bad enough for the settlement's own planner to notice, and
//! that takes a full day). This headless render is that corrected scene and
//! nothing else: the hearth the player wakes up next to, and four separate
//! lean-tos, each getting its own log. Written to `$STUDIO_ARTIFACTS/demo.png`
//! (or the working directory, when that is unset).

use runity::prelude::*;
use std::f32::consts::TAU;
use std::path::PathBuf;

/// A ridge tent, base on the ground, apex a line along Z — the shape a
/// shalash reads as at a glance. Six vertices, six triangles: this scene is
/// flat-lit, nothing here samples a texture.
fn shalash(width: f32, depth: f32, height: f32) -> Mesh {
    let (w, d, h) = (width * 0.5, depth * 0.5, height);
    let v = |x: f32, y: f32, z: f32| Vertex::new(Vec3::new(x, y, z), Vec3::Y, Vec2::ZERO);
    let (a, b, c, dd) = (v(-w, 0.0, -d), v(w, 0.0, -d), v(w, 0.0, d), v(-w, 0.0, d));
    let (rf, rb) = (v(0.0, h, -d), v(0.0, h, d));
    let mut mesh = Mesh::new(
        vec![a, b, c, dd, rf, rb],
        vec![
            0, 4, 1, // front gable
            3, 2, 5, // back gable
            0, 5, 4, 0, 3, 5, // left roof slope
            1, 5, 2, 1, 4, 5, // right roof slope
        ],
    );
    mesh.recompute_normals();
    mesh
}

fn main() -> std::io::Result<()> {
    let frame = headless::render(960, 540, |engine| {
        engine.clear_color = Color::rgb(0.05, 0.05, 0.07);
        engine.light = DirectionalLight {
            direction: Vec3::new(0.4, -0.75, 0.5).normalized(),
            color: Color::rgb(1.0, 0.93, 0.82),
            intensity: 1.2,
        };
        engine.camera.position = Vec3::new(0.0, 4.6, 7.5);
        engine.camera.target = Vec3::new(0.0, 0.4, 0.0);
        // The shalash below is authored by hand, not by winding discipline —
        // both sides show so a stray triangle never reads as a hole.
        engine.rasterizer.cull = CullMode::None;

        // A camp-firelight fill: brighter than the engine's default ambient,
        // so the lean-tos read as lit huddled shapes, not black silhouettes.
        let lit = |engine: &Engine, model: Mat4, color: Color| {
            let mut shader = engine.lit_shader(model);
            shader.base_color = color;
            shader.ambient = Color::rgb(0.22, 0.20, 0.22);
            shader.specular_strength = 0.0;
            shader
        };

        let ground = Mesh::plane(12.0, 1);
        let shader = lit(engine, Mat4::IDENTITY, Color::rgb(0.20, 0.16, 0.10));
        engine.draw(&ground, &shader);

        // The hearth the player wakes up next to: three crossed logs and the
        // coals under them. It is part of how the game starts, not a stake
        // — see 12-minds.md, часть 3.
        let ember = Mesh::sphere(0.22, 12, 8);
        let model = Mat4::from_translation(Vec3::new(0.0, 0.08, 0.0));
        let shader = lit(engine, model, Color::rgb(0.95, 0.45, 0.12));
        engine.draw(&ember, &shader);

        let log = Mesh::cube(1.0);
        for angle in [0.0_f32, TAU / 3.0, 2.0 * TAU / 3.0] {
            let model = Mat4::from_rotation_y(angle)
                * Mat4::from_translation(Vec3::new(0.0, 0.07, 0.0))
                * Mat4::from_scale(Vec3::new(0.9, 0.09, 0.14));
            let shader = lit(engine, model, Color::rgb(0.30, 0.18, 0.10));
            engine.draw(&log, &shader);
        }

        // Four settlers, four shalashes, four logs — each dragged to its own
        // lean-to, none of them to a shared wall.
        let tent = shalash(1.6, 1.8, 1.3);
        for i in 0..4 {
            let angle = i as f32 / 4.0 * TAU + 0.4;
            let outward = Vec3::new(angle.cos(), 0.0, angle.sin());
            let position = outward * 3.2;

            let tent_model = Mat4::from_translation(position) * Mat4::from_rotation_y(-angle);
            let shader = lit(engine, tent_model, Color::rgb(0.50, 0.40, 0.26));
            engine.draw(&tent, &shader);

            let log_model = Mat4::from_translation(position - outward * 1.15 + Vec3::Y * 0.07)
                * Mat4::from_rotation_y(-angle + 0.3)
                * Mat4::from_scale(Vec3::new(0.8, 0.11, 0.14));
            let shader = lit(engine, log_model, Color::rgb(0.58, 0.40, 0.20));
            engine.draw(&log, &shader);
        }
    });

    let out = artifacts_dir();
    std::fs::create_dir_all(&out)?;
    let path = out.join("demo.png");
    save_png(&path, &frame)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Where the studio looks for what this run produced.
fn artifacts_dir() -> PathBuf {
    std::env::var_os("STUDIO_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
