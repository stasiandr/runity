//! Real meshes, drawn twice.
//!
//! The shader tests use one flat quad so that nothing but arithmetic can
//! differ. These are the other half: a cube, a sphere, the Newell teapot and
//! three intersecting translucent planes, all under perspective, where a
//! triangle's edge lands on a pixel boundary and two rasterizers are entitled
//! to disagree about which side of it a pixel is on.
//!
//! Hence the looser budget — eight levels per channel and two per cent of the
//! pixels. That is an edge either side of itself, and not a surface with the
//! wrong colour on it: the shader tests have already ruled that out.

use runity_gpu::diff::{self, Canvas};
use runity_math::{Mat4, Vec3};
use runity_render::golden::Tolerance;
use runity_render::{BasicShader, Blend, Color, CullMode, DirectionalLight, Mesh, UnlitShader};
use std::f32::consts::TAU;

/// Edges land a pixel either side of themselves; surfaces do not.
const GEOMETRY_TOLERANCE: Tolerance = Tolerance {
    channel_delta: 8,
    differing_fraction: 0.02,
};

const WIDTH: usize = 160;
const HEIGHT: usize = 120;

fn camera(eye: Vec3, target: Vec3) -> Mat4 {
    Mat4::perspective(0.9, WIDTH as f32 / HEIGHT as f32, 0.1, 100.0)
        * Mat4::look_at(eye, target, Vec3::Y)
}

/// A light with no highlight: `pow` is the one operation the two renderers may
/// round apart, and the shader tests already measure it on its own.
fn lit(model: Mat4, view_projection: Mat4, eye: Vec3, color: Color) -> BasicShader<'static> {
    let mut shader = BasicShader::new(model, view_projection)
        .with_base_color(color)
        .with_camera_position(eye)
        .with_light(DirectionalLight {
            direction: Vec3::new(-0.35, -0.8, -0.5).normalized(),
            color: Color::WHITE,
            intensity: 1.05,
        });
    shader.specular_strength = 0.0;
    shader
}

#[test]
fn a_lit_cube_looks_the_same_on_both_renderers() {
    diff::assert_agrees("cube", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas: &mut Canvas| {
        let eye = Vec3::new(2.4, 1.8, 3.2);
        let vp = camera(eye, Vec3::ZERO);
        let model = Mat4::from_rotation_y(0.7) * Mat4::from_rotation_x(0.35);
        canvas.draw(
            &Mesh::cube(1.6),
            &lit(model, vp, eye, Color::rgb(0.85, 0.55, 0.25)),
        );
    });
}

#[test]
fn a_lit_sphere_looks_the_same_on_both_renderers() {
    diff::assert_agrees("sphere", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas| {
        let eye = Vec3::new(0.0, 1.2, 3.4);
        let vp = camera(eye, Vec3::ZERO);
        canvas.draw(
            &Mesh::sphere(1.1, 28, 18),
            &lit(Mat4::IDENTITY, vp, eye, Color::rgb(0.35, 0.65, 0.95)),
        );
    });
}

/// The teapot, straight out of the showcase's own asset.
///
/// Six thousand triangles is where the near-plane clip, the winding and the
/// depth test all get exercised at once, and a single misplaced triangle still
/// shows up against a two per cent budget.
#[test]
fn the_teapot_looks_the_same_on_both_renderers() {
    let teapot = Mesh::from_obj(include_str!(
        "../../runity/examples/assets/teapot.obj"
    ))
    .expect("the showcase's teapot parses");
    assert!(teapot.triangle_count() > 1000, "that is the teapot");

    diff::assert_agrees("teapot", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas| {
        let eye = Vec3::new(0.0, 2.2, 5.0);
        let vp = camera(eye, Vec3::new(0.0, 1.0, 0.0));
        let model = Mat4::from_rotation_y(0.9);
        canvas.draw(&teapot, &lit(model, vp, eye, Color::rgb(0.80, 0.78, 0.74)));
    });
}

/// Three translucent planes through one another — the showcase's *Depth &
/// blending* page, which is where the blend state, the depth test and the
/// depth write all have to be right at the same time.
#[test]
fn intersecting_translucent_planes_composite_the_same_way() {
    diff::assert_agrees("translucent-planes", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas| {
        let eye = Vec3::new(2.6, 1.6, 3.4);
        let vp = camera(eye, Vec3::ZERO);
        canvas.set_cull(CullMode::None);
        canvas.set_blending(Blend::Alpha, true, false);
        let quad = Mesh::plane(2.6, 1);
        for (turn, color) in [
            (0.0_f32, Color::rgba(0.95, 0.35, 0.35, 0.55)),
            (TAU / 3.0, Color::rgba(0.35, 0.90, 0.45, 0.55)),
            (2.0 * TAU / 3.0, Color::rgba(0.40, 0.55, 0.98, 0.55)),
        ] {
            let model = Mat4::from_rotation_y(turn) * Mat4::from_rotation_x(0.35);
            let mut shader = UnlitShader::new(vp * model);
            shader.tint = color;
            canvas.draw(&quad, &shader);
        }
    });
}

/// Geometry that crosses the near plane. The rasterizer clips against `z >= 0`
/// by hand; Metal does it in hardware, and the two have to end up with the
/// same silhouette.
#[test]
fn geometry_crossing_the_near_plane_is_clipped_the_same_way() {
    diff::assert_agrees("near-plane", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas| {
        let eye = Vec3::new(0.0, 0.0, 0.6);
        let vp = camera(eye, Vec3::ZERO);
        canvas.set_cull(CullMode::None);
        // A big cube with the camera inside it: every face straddles the near
        // plane somewhere.
        canvas.draw(
            &Mesh::cube(3.0),
            &lit(Mat4::IDENTITY, vp, eye, Color::rgb(0.6, 0.7, 0.5)),
        );
    });
}

/// A scene, not a mesh: a floor, a crate and two spheres, the way the
/// showcase's first page draws them.
#[test]
fn a_whole_scene_looks_the_same_on_both_renderers() {
    let mut floor_texture =
        runity_render::Texture::checker(64, 8, Color::rgb(0.20, 0.22, 0.26), Color::rgb(0.32, 0.34, 0.40));
    floor_texture.wrap = runity_render::Wrap::Repeat;

    diff::assert_agrees("scene", WIDTH, HEIGHT, GEOMETRY_TOLERANCE, |canvas| {
        let eye = Vec3::new(3.4, 2.6, 4.2);
        let vp = camera(eye, Vec3::new(0.0, 0.5, 0.0));

        let mut floor = lit(
            Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0)),
            vp,
            eye,
            Color::WHITE,
        );
        floor.texture = Some(&floor_texture);
        canvas.draw(&Mesh::plane(10.0, 1), &floor);

        let model = Mat4::from_rotation_y(0.6) * Mat4::from_rotation_x(0.3);
        canvas.draw(&Mesh::cube(1.4), &lit(model, vp, eye, Color::rgb(0.8, 0.5, 0.25)));

        for (x, tint) in [
            (-2.0_f32, Color::rgb(0.9, 0.3, 0.35)),
            (2.0, Color::rgb(0.35, 0.65, 0.95)),
        ] {
            let model = Mat4::from_translation(Vec3::new(x, 0.35, 0.0));
            canvas.draw(&Mesh::sphere(0.55, 28, 18), &lit(model, vp, eye, tint));
        }
    });
}
