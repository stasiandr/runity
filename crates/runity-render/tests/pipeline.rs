//! End-to-end tests for the rasterizer: fill rule, depth, culling, clipping and
//! perspective correction.

use runity_math::{Mat4, Vec2, Vec3, Vec4};
use runity_render::{
    Color, CullMode, Framebuffer, Mesh, Rasterizer, Shader, UnlitShader, Vertex, VertexOutput,
};

/// Feeds vertex positions straight through as NDC, so a test can place
/// geometry on exact pixels without going through a camera.
struct Ndc;

impl Shader for Ndc {
    type Varying = Color;

    fn vertex(&self, v: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: Vec4::new(v.position.x, v.position.y, v.position.z, 1.0),
            varying: v.color,
        }
    }

    fn fragment(&self, c: &Color) -> Option<Color> {
        Some(*c)
    }
}

fn ndc_vertex(x: f32, y: f32, z: f32, color: Color) -> Vertex {
    Vertex::new(Vec3::new(x, y, z), Vec3::Z, Vec2::ZERO).with_color(color)
}

/// Two triangles covering the whole viewport, wound counter-clockwise.
fn fullscreen_quad(z: f32, color: Color) -> Mesh {
    let v = vec![
        ndc_vertex(-1.0, -1.0, z, color),
        ndc_vertex(1.0, -1.0, z, color),
        ndc_vertex(1.0, 1.0, z, color),
        ndc_vertex(-1.0, 1.0, z, color),
    ];
    Mesh::new(v, vec![0, 1, 2, 0, 2, 3])
}

#[test]
fn fullscreen_quad_covers_every_pixel_exactly_once() {
    let mut fb = Framebuffer::new(64, 48);
    fb.clear(Color::BLACK);
    let stats = Rasterizer::new().draw_mesh(&mut fb, &fullscreen_quad(0.5, Color::RED), &Ndc);

    assert_eq!(stats.triangles_rasterized, 2);
    // The top-left fill rule must hand the shared diagonal to exactly one
    // triangle: no gaps, no pixel shaded twice.
    assert_eq!(stats.fragments_written, 64 * 48);
    assert_eq!(stats.fragments_shaded, 64 * 48);
    assert!(fb.pixels().iter().all(|p| *p == Color::RED.to_argb8()));
}

#[test]
fn nearer_geometry_wins_regardless_of_draw_order() {
    let near = fullscreen_quad(0.2, Color::BLUE);
    let far = fullscreen_quad(0.8, Color::RED);
    let raster = Rasterizer::new();

    for order in [[&near, &far], [&far, &near]] {
        let mut fb = Framebuffer::new(16, 16);
        fb.clear(Color::BLACK);
        for mesh in order {
            raster.draw_mesh(&mut fb, mesh, &Ndc);
        }
        assert!(
            fb.pixels().iter().all(|p| *p == Color::BLUE.to_argb8()),
            "the near quad must survive the depth test in both orders"
        );
    }
}

#[test]
fn depth_test_can_be_disabled() {
    let mut raster = Rasterizer::new();
    raster.depth_test = false;
    let mut fb = Framebuffer::new(8, 8);
    fb.clear(Color::BLACK);
    raster.draw_mesh(&mut fb, &fullscreen_quad(0.2, Color::BLUE), &Ndc);
    raster.draw_mesh(&mut fb, &fullscreen_quad(0.8, Color::RED), &Ndc);
    assert!(
        fb.pixels().iter().all(|p| *p == Color::RED.to_argb8()),
        "painter's order wins"
    );
}

#[test]
fn backfaces_are_culled_and_can_be_kept() {
    let mut cw = fullscreen_quad(0.5, Color::RED);
    cw.indices = vec![0, 2, 1, 0, 3, 2]; // reversed winding

    let mut fb = Framebuffer::new(16, 16);
    fb.clear(Color::BLACK);
    let stats = Rasterizer::new().draw_mesh(&mut fb, &cw, &Ndc);
    assert_eq!(stats.triangles_rasterized, 0);
    assert_eq!(stats.fragments_written, 0);

    let mut raster = Rasterizer::new();
    raster.cull = CullMode::None;
    let stats = raster.draw_mesh(&mut fb, &cw, &Ndc);
    assert_eq!(stats.fragments_written, 16 * 16);
}

#[test]
fn geometry_behind_the_near_plane_is_clipped_away() {
    let raster = Rasterizer::new();
    let mut fb = Framebuffer::new(32, 32);
    fb.clear(Color::BLACK);

    // Entirely behind the near plane (z < 0 in clip space).
    let behind = Mesh::new(
        vec![
            ndc_vertex(-1.0, -1.0, -0.5, Color::RED),
            ndc_vertex(1.0, -1.0, -0.5, Color::RED),
            ndc_vertex(0.0, 1.0, -0.5, Color::RED),
        ],
        vec![0, 1, 2],
    );
    let stats = raster.draw_mesh(&mut fb, &behind, &Ndc);
    assert_eq!(stats.triangles_rasterized, 0);
    assert_eq!(stats.fragments_written, 0);

    // Straddling it: the clipper must keep the visible half and nothing else.
    let straddling = Mesh::new(
        vec![
            ndc_vertex(-1.0, -1.0, -0.5, Color::RED),
            ndc_vertex(1.0, -1.0, 0.5, Color::RED),
            ndc_vertex(0.0, 1.0, 0.5, Color::RED),
        ],
        vec![0, 1, 2],
    );
    let stats = raster.draw_mesh(&mut fb, &straddling, &Ndc);
    assert!(
        stats.triangles_rasterized >= 1,
        "the clipper produced no geometry"
    );
    assert!(stats.fragments_written > 0);
    assert!(
        stats.fragments_written < 32 * 32,
        "the clipped half must not cover the whole screen"
    );
}

#[test]
fn a_triangle_outside_the_viewport_is_skipped() {
    let mut fb = Framebuffer::new(16, 16);
    fb.clear(Color::BLACK);
    let offscreen = Mesh::new(
        vec![
            ndc_vertex(3.0, 3.0, 0.5, Color::RED),
            ndc_vertex(5.0, 3.0, 0.5, Color::RED),
            ndc_vertex(4.0, 5.0, 0.5, Color::RED),
        ],
        vec![0, 1, 2],
    );
    let stats = Rasterizer::new().draw_mesh(&mut fb, &offscreen, &Ndc);
    assert_eq!(stats.fragments_written, 0);
    assert!(fb.pixels().iter().all(|p| *p == Color::BLACK.to_argb8()));
}

/// The real test of the interpolator: render a ground plane in perspective and
/// compare the interpolated UVs against UVs derived independently, by
/// intersecting the camera ray for that pixel with the plane.
#[test]
fn interpolation_is_perspective_correct() {
    const W: usize = 128;
    const H: usize = 96;

    // Ground quad on y = 0, spanning [-1, 1] in x and z, uv = position remapped.
    let corner = |x: f32, z: f32| {
        Vertex::new(
            Vec3::new(x, 0.0, z),
            Vec3::Y,
            Vec2::new((x + 1.0) * 0.5, (z + 1.0) * 0.5),
        )
    };
    // Counter-clockwise seen from +Y.
    let mesh = Mesh::new(
        vec![
            corner(-1.0, -1.0),
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
            corner(1.0, -1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );

    let eye = Vec3::new(0.0, 1.2, 2.2);
    let view = Mat4::look_at(eye, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective(60f32.to_radians(), W as f32 / H as f32, 0.05, 50.0);
    let view_proj = proj * view;

    struct Uv(Mat4);
    impl Shader for Uv {
        type Varying = Vec2;
        fn vertex(&self, v: &Vertex) -> VertexOutput<Vec2> {
            VertexOutput {
                clip_position: self.0.transform_point(v.position),
                varying: v.uv,
            }
        }
        fn fragment(&self, uv: &Vec2) -> Option<Color> {
            Some(Color::rgb(uv.x, uv.y, 0.0))
        }
    }

    let mut fb = Framebuffer::new(W, H);
    fb.clear(Color::BLACK);
    let stats = Rasterizer::new().draw_mesh(&mut fb, &mesh, &Uv(view_proj));
    assert!(
        stats.fragments_written > 1000,
        "the plane should cover a good chunk of the screen"
    );

    let inv = view_proj.inverse().expect("view-projection is invertible");
    let mut checked = 0;
    for py in 0..H {
        for px in 0..W {
            let pixel = fb.get_pixel(px, py);
            if pixel == Color::BLACK {
                continue; // background
            }

            // Unproject the pixel center into a world-space ray.
            let ndc_x = (px as f32 + 0.5) / W as f32 * 2.0 - 1.0;
            let ndc_y = 1.0 - (py as f32 + 0.5) / H as f32 * 2.0;
            let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
            let dir = (far.perspective_divide() - eye).normalized();
            assert!(dir.y < 0.0, "a lit pixel must look downward at the ground");

            // Intersect with the y = 0 plane and turn the hit into UVs.
            let t = -eye.y / dir.y;
            let hit = eye + dir * t;
            let expected = Vec2::new((hit.x + 1.0) * 0.5, (hit.z + 1.0) * 0.5);

            // 8-bit output plus the half-pixel of slack at silhouette edges.
            assert!(
                (pixel.r - expected.x).abs() < 0.02 && (pixel.g - expected.y).abs() < 0.02,
                "pixel ({px},{py}): got {pixel:?}, ray hit expects {expected:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 1000, "only checked {checked} pixels");
}

#[test]
fn alpha_blending_mixes_with_the_destination() {
    let mut raster = Rasterizer::new();
    raster.blend = runity_render::Blend::Alpha;
    raster.depth_test = false;

    let mut fb = Framebuffer::new(8, 8);
    fb.clear(Color::BLACK);
    let mut quad = fullscreen_quad(0.5, Color::rgba(1.0, 1.0, 1.0, 0.5));
    quad.set_color(Color::rgba(1.0, 1.0, 1.0, 0.5));
    raster.draw_mesh(&mut fb, &quad, &Ndc);

    let p = fb.get_pixel(4, 4);
    assert!(
        (p.r - 0.5).abs() < 0.01,
        "half-transparent white over black is grey, got {p:?}"
    );
}

#[test]
fn unlit_shader_draws_a_cube_without_panicking() {
    let mut fb = Framebuffer::new(64, 64);
    fb.clear(Color::BLACK);
    let view = Mat4::look_at(Vec3::new(2.0, 2.0, 3.0), Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective(50f32.to_radians(), 1.0, 0.1, 100.0);
    let stats =
        Rasterizer::new().draw_mesh(&mut fb, &Mesh::cube(1.0), &UnlitShader::new(proj * view));
    assert_eq!(stats.triangles_in, 12);
    // Exactly three faces of a cube can be visible from a corner.
    assert_eq!(stats.triangles_rasterized, 6);
    assert!(stats.fragments_written > 0);
}
