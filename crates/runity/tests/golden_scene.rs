//! Golden-image regression test: the workflow this engine is built around.
//!
//! The reference lives in `tests/golden/`. When a change to the renderer is
//! intended, rerun with `RUNITY_UPDATE_GOLDEN=1 cargo test` and commit the new
//! image; the diff in review then shows exactly what changed on screen.

use runity::prelude::*;
use runity_render::golden::Tolerance;

/// A small scene with nothing random in it, so the only thing that can move the
/// pixels is a change in the renderer.
///
/// A platform's `libm` is not bit-identical to another's, and the shading path
/// is full of `powf`, so the comparison is deliberately tolerant: this exists
/// to catch our own regressions, not the C library's rounding.
fn reference_scene() -> Framebuffer {
    headless::render(128, 96, |engine| {
        engine.camera.position = Vec3::new(2.5, 2.0, 3.5);
        engine.camera.target = Vec3::new(0.0, 0.3, 0.0);
        engine.renderer.set_sky(Sky::new(SkyParams::default()));

        let floor_texture = Texture::checker(
            64,
            8,
            Color::rgb(0.05, 0.055, 0.06),
            Color::rgb(0.2, 0.21, 0.23),
        );
        let floor = Material {
            roughness: 0.6,
            base_color_texture: Some(&floor_texture),
            ..Material::default()
        };
        engine.draw_pbr(
            &Mesh::plane(8.0, 1),
            Mat4::from_translation(Vec3::new(0.0, -0.7, 0.0)),
            &floor,
        );

        engine.draw_pbr(
            &Mesh::cube(1.2),
            Mat4::from_rotation_y(0.6),
            &Material::dielectric(Color::rgb(0.5, 0.25, 0.08), 0.55),
        );

        engine.draw_pbr(
            &Mesh::sphere(0.55, 24, 16),
            Mat4::from_translation(Vec3::new(-1.4, 0.0, 0.8)),
            &Material::metal(Color::rgb(0.9, 0.75, 0.4), 0.2),
        );
    })
}

#[test]
fn the_reference_scene_still_looks_the_same() {
    let frame = reference_scene();
    // A pixel may differ by a few steps, and up to 2% of them may differ at
    // all: that absorbs one-ULP differences in `sin`/`tan` between platforms
    // without hiding a real change.
    golden::assert_matches("tests/golden/scene.png", &frame, Tolerance::new(4, 0.02));
}

#[test]
fn rendering_the_same_scene_twice_gives_identical_pixels() {
    // Headless rendering must be deterministic, or golden images are worthless.
    assert_eq!(reference_scene().colors(), reference_scene().colors());
}

#[test]
fn the_diff_report_describes_a_real_change() {
    let frame = reference_scene();
    // Round-trip through the encoder rather than reading the reference file, so
    // this test does not race the one that may be creating it.
    let reference =
        decode_png(&encode_png(frame.width(), frame.height(), &frame.resolve())).expect("decodes");

    let clean = golden::compare(&frame, &reference, Tolerance::new(4, 0.02)).expect("same size");
    assert!(clean.is_within(Tolerance::new(4, 0.02)), "{clean}");

    // Now break one pixel and check the report points at it.
    let mut broken = frame.clone();
    broken.set_pixel(40, 30, Color::rgb(1.0, 0.0, 1.0));
    let report = golden::compare(&broken, &reference, Tolerance::EXACT).expect("same size");
    assert!(report.differing_pixels >= 1);
    assert!(report.max_channel_delta > 4);
    assert_eq!(report.first_difference, Some((40, 30)));
}

#[test]
fn debug_views_describe_the_same_frame() {
    let frame = reference_scene();

    // Depth: the scene fills part of the frame, so some pixels are untouched.
    let depth = debug::depth_view(&frame);
    let lit = depth
        .colors()
        .iter()
        .filter(|c| **c != Color::BLACK)
        .count();
    assert!(lit > 1000 && lit < depth.len(), "{lit} pixels have depth");

    // Wireframe: same geometry, far fewer fragments.
    let mut wire = Framebuffer::new(128, 96);
    wire.clear(Color::BLACK);
    let mut raster = Rasterizer::new();
    raster.polygon_mode = PolygonMode::Line;
    let view_projection = Camera {
        position: Vec3::new(2.5, 2.0, 3.5),
        target: Vec3::new(0.0, 0.3, 0.0),
        ..Camera::default()
    }
    .view_projection(128.0 / 96.0);
    let filled = {
        let mut fb = Framebuffer::new(128, 96);
        fb.clear(Color::BLACK);
        Rasterizer::new()
            .draw_mesh(
                &mut fb,
                &Mesh::cube(1.2),
                &UnlitShader::new(view_projection),
            )
            .fragments_written
    };
    let edges = raster
        .draw_mesh(
            &mut wire,
            &Mesh::cube(1.2),
            &UnlitShader::new(view_projection),
        )
        .fragments_written;
    assert!(
        edges > 0 && edges * 2 < filled,
        "wireframe {edges} vs filled {filled}"
    );
}
