//! Golden-image regression test: the workflow this engine is built around.
//!
//! The reference lives in `tests/golden/`. When a change to the renderer is
//! intended, rerun with `RUNITY_UPDATE_GOLDEN=1 cargo test` and commit the new
//! image; the diff in review then shows exactly what changed on screen.

use runity::prelude::*;
use runity_render::golden::Tolerance;

/// A deliberately boring scene: no specular term and no random input, so the
/// only thing that can move the pixels is a change in the renderer.
///
/// `powf` and friends are avoided on purpose — a platform's `libm` is not
/// bit-identical to another's, and the point of the reference is to catch our
/// own regressions, not the C library's rounding.
fn reference_scene() -> Framebuffer {
    headless::render(128, 96, |engine| {
        engine.clear_color = Color::rgb(0.04, 0.05, 0.08);
        engine.camera.position = Vec3::new(2.5, 2.0, 3.5);
        engine.camera.target = Vec3::new(0.0, 0.3, 0.0);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.5, -0.85, -0.35).normalized(),
            color: Color::WHITE,
            intensity: 1.1,
        };

        let floor_texture = Texture::checker(
            64,
            8,
            Color::rgb(0.20, 0.22, 0.26),
            Color::rgb(0.32, 0.34, 0.40),
        );
        let mut shader = engine.lit_shader(Mat4::from_translation(Vec3::new(0.0, -0.7, 0.0)));
        shader.texture = Some(&floor_texture);
        shader.specular_strength = 0.0;
        engine.draw(&Mesh::plane(8.0, 1), &shader);

        let model = Mat4::from_rotation_y(0.6);
        let mut shader = engine.lit_shader(model);
        shader.base_color = Color::rgb(0.85, 0.55, 0.25);
        shader.specular_strength = 0.0;
        engine.draw(&Mesh::cube(1.2), &shader);

        let mut shader = engine.lit_shader(Mat4::from_translation(Vec3::new(-1.4, 0.0, 0.8)));
        shader.base_color = Color::rgb(0.35, 0.65, 0.95);
        shader.specular_strength = 0.0;
        engine.draw(&Mesh::sphere(0.55, 24, 16), &shader);
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
    assert_eq!(reference_scene().pixels(), reference_scene().pixels());
}

#[test]
fn the_diff_report_describes_a_real_change() {
    let frame = reference_scene();
    // Round-trip through the encoder rather than reading the reference file, so
    // this test does not race the one that may be creating it.
    let reference =
        decode_png(&encode_png(frame.width(), frame.height(), frame.pixels())).expect("decodes");

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
        .pixels()
        .iter()
        .filter(|p| **p != Color::BLACK.to_argb8())
        .count();
    assert!(
        lit > 1000 && lit < depth.pixels().len(),
        "{lit} pixels have depth"
    );

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
