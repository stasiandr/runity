//! Golden-image test for [`UnlitShader`]'s alpha cutoff: a fragment below the
//! threshold must not be written at all, so whatever is behind it shows
//! through untouched.

use runity_math::{Mat4, Vec2, Vec3};
use runity_render::golden::Tolerance;
use runity_render::{golden, Color, Framebuffer, Mesh, Rasterizer, UnlitShader, Vertex};

/// An NDC quad spanning `[x0, x1]` horizontally and the full vertical range, at
/// a fixed color — no camera needed, since `UnlitShader::new` is given the
/// identity matrix and vertex positions are placed directly in clip space.
fn ndc_quad(x0: f32, x1: f32, color: Color) -> Mesh {
    let v = |x, y| Vertex::new(Vec3::new(x, y, 0.5), Vec3::Z, Vec2::ZERO).with_color(color);
    Mesh::new(
        vec![v(x0, -1.0), v(x1, -1.0), v(x1, 1.0), v(x0, 1.0)],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn render_alpha_cutoff_demo() -> Framebuffer {
    let mut fb = Framebuffer::new(64, 48);
    let background = Color::rgb(0.05, 0.08, 0.35);
    fb.clear(background);

    let shader = UnlitShader::new(Mat4::IDENTITY).with_alpha_cutoff(0.5);
    // Left half: opaque red, drawn normally.
    let opaque = ndc_quad(-1.0, -0.05, Color::rgba(0.9, 0.15, 0.1, 1.0));
    // Right half: same red, but below the cutoff — must vanish entirely.
    let transparent = ndc_quad(0.05, 1.0, Color::rgba(0.9, 0.15, 0.1, 0.1));

    let raster = Rasterizer::new();
    raster.draw_mesh(&mut fb, &opaque, &shader);
    raster.draw_mesh(&mut fb, &transparent, &shader);
    fb
}

/// The framebuffer quantizes to 8 bits per channel, so a barycentric-weighted
/// sum of identical corner colors can land one step off the input.
fn assert_color_close(actual: Color, expected: Color, msg: &str) {
    let close = |a: f32, b: f32| (a - b).abs() <= 1.0 / 255.0 + f32::EPSILON;
    assert!(
        close(actual.r, expected.r) && close(actual.g, expected.g) && close(actual.b, expected.b),
        "{msg}: expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn alpha_cutoff_lets_the_background_show_through() {
    let fb = render_alpha_cutoff_demo();

    // Left half kept: solid red over the background.
    assert_color_close(
        fb.get_pixel(10, 24),
        Color::rgba(0.9, 0.15, 0.1, 1.0),
        "a fragment above the cutoff must be drawn as-is",
    );
    // Right half discarded: the clear color survives untouched.
    assert_color_close(
        fb.get_pixel(54, 24),
        Color::rgb(0.05, 0.08, 0.35),
        "a fragment below the cutoff must not be written",
    );

    golden::assert_matches("tests/golden/alpha-cutoff.png", &fb, Tolerance::default());
}

#[test]
fn rendering_the_alpha_cutoff_demo_twice_gives_identical_pixels() {
    assert_eq!(
        render_alpha_cutoff_demo().pixels(),
        render_alpha_cutoff_demo().pixels()
    );
}
