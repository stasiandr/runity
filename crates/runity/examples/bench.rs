//! What each pass costs, on this machine, for a scene like the showcase.
//!
//! ```text
//! cargo run --release --example bench
//! ```
//!
//! The numbers in the documentation come from here. Re-run it after any change
//! that is supposed to be faster — or any change that is not.
use runity::prelude::*;
use std::time::Instant;

fn scene(engine: &mut Engine) {
    engine.renderer.set_sky(Sky::new(SkyParams::golden_hour()));
    engine.renderer.settings.post = PostSettings::cinematic();
    engine.camera.position = Vec3::new(4.6, 2.8, 6.4);
    engine.camera.target = Vec3::new(0.0, 0.55, 0.0);
    let floor = Texture::checker(
        256,
        32,
        Color::rgb(0.05, 0.05, 0.06),
        Color::rgb(0.12, 0.13, 0.14),
    );
    engine.draw_pbr(
        &Mesh::plane(60.0, 1),
        Mat4::IDENTITY,
        &Material {
            roughness: 0.22,
            base_color_texture: Some(&floor),
            uv_scale: Vec2::splat(6.0),
            ..Material::default()
        },
    );
    for i in 0..7 {
        let t = i as f32 / 6.0;
        let x = (t - 0.5) * 5.0;
        engine.draw_pbr(
            &Mesh::sphere(0.42, 40, 28),
            Mat4::from_translation(Vec3::new(x, 0.42, -1.9)),
            &Material::metal(Color::rgb(0.94, 0.78, 0.42), 0.04 + t * 0.7),
        );
        engine.draw_pbr(
            &Mesh::sphere(0.42, 40, 28),
            Mat4::from_translation(Vec3::new(x, 0.42, -0.55)),
            &Material::dielectric(Color::rgb(0.55, 0.1, 0.09), 0.04 + t * 0.7),
        );
    }
    engine.draw_pbr(
        &Mesh::torus(0.65, 0.22, 48, 24),
        Mat4::from_translation(Vec3::new(2.3, 0.72, 1.6)),
        &Material::metal(Color::rgb(0.95, 0.96, 0.98), 0.08),
    );
}

/// A scene setup: turns some passes off, then draws.
type Variant = (&'static str, fn(&mut Engine));

fn main() {
    // Which pass costs what, at 640x360.
    let variants: [Variant; 5] = [
        ("everything", |e| scene(e)),
        ("no ssao", |e| {
            e.renderer.settings.ssao.enabled = false;
            scene(e)
        }),
        ("no ssr", |e| {
            e.renderer.settings.ssr.enabled = false;
            scene(e)
        }),
        ("no shadows", |e| {
            e.renderer.settings.shadows.enabled = false;
            scene(e)
        }),
        ("geometry + lighting only", |e| {
            e.renderer.settings.ssao.enabled = false;
            e.renderer.settings.ssr.enabled = false;
            e.renderer.settings.bloom.enabled = false;
            e.renderer.settings.shadows.enabled = false;
            e.renderer.settings.post = PostSettings::default();
            scene(e)
        }),
    ];
    for (name, setup) in variants {
        let _ = headless::render_at(640, 360, 1, setup);
        let start = Instant::now();
        for _ in 0..3 {
            let _ = headless::render_at(640, 360, 1, setup);
        }
        println!("{name}: {:.3} s/frame", start.elapsed().as_secs_f32() / 3.0);
    }

    for (w, h, ss) in [(640usize, 360usize, 1usize), (640, 360, 2), (960, 540, 1)] {
        // warm up, then measure three frames
        let _ = headless::render_at(w, h, ss, scene);
        let start = Instant::now();
        const RUNS: u32 = 3;
        for _ in 0..RUNS {
            let _ = headless::render_at(w, h, ss, scene);
        }
        let per_frame = start.elapsed().as_secs_f32() / RUNS as f32;
        println!(
            "{w}x{h} ss={ss}: {:.3} s/frame ({:.1} fps)",
            per_frame,
            1.0 / per_frame
        );
    }
}
