//! Every shader the engine ships, drawn twice.
//!
//! One full-screen quad, one shader, two renderers, two pixels per channel of
//! slack. A quad rather than a scene on purpose: there is no geometry here to
//! blame, so anything that differs is the shader arithmetic.
//!
//! Specular highlights are off throughout (`specular_strength = 0`), exactly
//! as in `golden_scene.rs`: `pow` is the one place where two implementations
//! of the same formula are entitled to disagree in the last bits, and a
//! highlight magnifies that into visible pixels. What the highlight *is* is
//! tested separately, with room to breathe.

use runity_gpu::diff::{self, Canvas};
use runity_gpu::{GpuShader, PulseUniforms, ENGINE_SOURCE, PULSE_FRAGMENT, PULSE_VERTEX};
use runity_math::{Mat4, Vec2, Vec3};
use runity_render::golden::Tolerance;
use runity_render::{
    BasicShader, Blend, Color, CullMode, DirectionalLight, Filter, Mesh, Shader, Texture,
    UnlitShader, Vertex, VertexOutput, Wrap,
};

/// Two pixels per channel, and not one pixel over budget: a shader that agrees
/// only on average is not a shader that agrees.
const SHADER_TOLERANCE: Tolerance = Tolerance {
    channel_delta: 2,
    differing_fraction: 0.0,
};

/// A quad across the whole viewport with UVs running outside the unit square,
/// so wrapping is on screen and not just in the sampler state.
fn screen_quad(uv_min: f32, uv_max: f32) -> Mesh {
    let corner = |x: f32, y: f32, u: f32, v: f32| {
        Vertex::new(Vec3::new(x, y, 0.5), Vec3::Z, Vec2::new(u, v)).with_color(Color::WHITE)
    };
    Mesh::new(
        vec![
            corner(-1.0, 1.0, uv_min, uv_min),
            corner(1.0, 1.0, uv_max, uv_min),
            corner(1.0, -1.0, uv_max, uv_max),
            corner(-1.0, -1.0, uv_min, uv_max),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// The same quad, with a different colour at each corner, so an interpolant
/// that is wrong anywhere is wrong somewhere visible.
fn coloured_quad() -> Mesh {
    let corner = |x: f32, y: f32, c: Color| {
        Vertex::new(Vec3::new(x, y, 0.5), Vec3::Z, Vec2::new(0.0, 0.0)).with_color(c)
    };
    Mesh::new(
        vec![
            corner(-1.0, 1.0, Color::rgba(0.95, 0.25, 0.30, 1.0)),
            corner(1.0, 1.0, Color::rgba(0.25, 0.85, 0.40, 1.0)),
            corner(1.0, -1.0, Color::rgba(0.30, 0.45, 0.98, 1.0)),
            corner(-1.0, -1.0, Color::rgba(0.90, 0.85, 0.20, 1.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// Hard edges and a diagonal, so nearest and bilinear cannot look the same —
/// the same texture the showcase's *Textures* page uses.
fn probe_texture() -> Texture {
    Texture::from_fn(16, 16, |x, y| {
        let step = |v: usize| v as f32 / 255.0;
        if x == y || x + y == 15 {
            Color::rgb(step(240), step(220), step(90))
        } else if (x / 4 + y / 4) % 2 == 0 {
            Color::rgb(step(40), step(60), step(110))
        } else {
            Color::rgb(step(200), step(70), step(90))
        }
    })
}

// ---------------------------------------------------------------------------
// The four shaders
// ---------------------------------------------------------------------------

#[test]
fn the_basic_shader_lights_a_surface_the_same_way_on_both_renderers() {
    diff::assert_agrees(
        "basic-lit",
        128,
        128,
        SHADER_TOLERANCE,
        |canvas: &mut Canvas| {
            canvas.set_cull(CullMode::None);
            let mut shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
                .with_base_color(Color::rgb(0.8, 0.6, 0.35))
                .with_camera_position(Vec3::new(0.0, 0.0, 3.0))
                .with_light(DirectionalLight {
                    direction: Vec3::new(-0.5, -0.85, -0.35).normalized(),
                    color: Color::rgb(1.0, 0.96, 0.88),
                    intensity: 1.15,
                });
            // `pow` is the one operation two implementations may round apart.
            shader.specular_strength = 0.0;
            canvas.draw(&sloped_quad(), &shader);
        },
    );
}

#[test]
fn the_basic_shader_modulates_a_texture_the_same_way() {
    let texture = probe_texture();
    diff::assert_agrees("basic-textured", 128, 128, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        let mut shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
            .with_base_color(Color::rgb(0.9, 0.9, 1.0))
            .with_texture(&texture);
        shader.specular_strength = 0.0;
        canvas.draw(&screen_quad(0.0, 1.0), &shader);
    });
}

#[test]
fn the_basic_shaders_specular_highlight_lands_in_the_same_place() {
    // A highlight is `pow(dot, shininess)`, which is where two renderers are
    // entitled to round apart — so this one gets the geometric budget rather
    // than the shader budget, and it is the highlight's *position* that is
    // being asserted.
    diff::assert_agrees(
        "basic-specular",
        128,
        128,
        Tolerance::new(8, 0.02),
        |canvas| {
            canvas.set_cull(CullMode::None);
            let mut shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
                .with_base_color(Color::rgb(0.7, 0.7, 0.75))
                .with_camera_position(Vec3::new(0.0, 0.0, 2.5));
            shader.specular_strength = 0.8;
            shader.shininess = 24.0;
            canvas.draw(&sloped_quad(), &shader);
        },
    );
}

#[test]
fn the_unlit_shader_tints_and_interpolates_the_same_way() {
    diff::assert_agrees("unlit", 128, 128, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        let mut shader = UnlitShader::new(Mat4::IDENTITY);
        shader.tint = Color::rgba(0.9, 0.8, 0.6, 1.0);
        canvas.draw(&coloured_quad(), &shader);
    });
}

#[test]
fn the_unlit_shader_samples_a_texture_the_same_way() {
    let texture = probe_texture();
    diff::assert_agrees("unlit-textured", 128, 128, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        let mut shader = UnlitShader::new(Mat4::IDENTITY);
        shader.texture = Some(&texture);
        canvas.draw(&screen_quad(0.0, 1.0), &shader);
    });
}

/// The showcase's `PulseShader`, written out here as the tests' own copy.
///
/// The Rust half is duplicated on purpose: the point of a differential test is
/// that two independently written programs agree, and importing one of them
/// would test a little less than that.
struct PulseShader {
    mvp: Mat4,
    pulse: f32,
}

impl Shader for PulseShader {
    type Varying = Color;
    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: self.mvp.transform_point(vertex.position),
            varying: vertex.color,
        }
    }
    fn fragment(&self, color: &Color) -> Option<Color> {
        Some(color.scale_rgb(self.pulse))
    }
}

impl GpuShader for PulseShader {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = PULSE_VERTEX;
    const FRAGMENT: &'static str = PULSE_FRAGMENT;
    type Uniforms = PulseUniforms;
    fn uniforms(&self) -> PulseUniforms {
        PulseUniforms::new(self.mvp, self.pulse)
    }
}

/// `hello_triangle`'s `Gradient`: the same shader, reached a different way —
/// the pulse is computed from a time rather than handed in.
struct Gradient {
    mvp: Mat4,
    time: f32,
}

impl Gradient {
    fn pulse(&self) -> f32 {
        0.75 + 0.25 * (self.time * 2.0).sin()
    }
}

impl Shader for Gradient {
    type Varying = Color;
    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: self.mvp.transform_point(vertex.position),
            varying: vertex.color,
        }
    }
    fn fragment(&self, color: &Color) -> Option<Color> {
        Some(color.scale_rgb(self.pulse()))
    }
}

impl GpuShader for Gradient {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = PULSE_VERTEX;
    const FRAGMENT: &'static str = PULSE_FRAGMENT;
    type Uniforms = PulseUniforms;
    fn uniforms(&self) -> PulseUniforms {
        PulseUniforms::new(self.mvp, self.pulse())
    }
}

#[test]
fn the_pulse_shader_dims_the_vertex_colour_the_same_way() {
    diff::assert_agrees("pulse", 128, 128, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        canvas.draw(
            &coloured_quad(),
            &PulseShader {
                mvp: Mat4::IDENTITY,
                pulse: 0.62,
            },
        );
    });
}

#[test]
fn the_gradient_shader_from_hello_triangle_agrees_too() {
    diff::assert_agrees("gradient", 128, 128, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        canvas.draw(
            &coloured_quad(),
            &Gradient {
                mvp: Mat4::IDENTITY,
                time: 1.37,
            },
        );
    });
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

#[test]
fn every_filter_against_every_wrap_samples_the_same_on_both_renderers() {
    for filter in [Filter::Nearest, Filter::Bilinear] {
        for wrap in [Wrap::Repeat, Wrap::Clamp, Wrap::Mirror] {
            let mut texture = probe_texture();
            texture.filter = filter;
            texture.wrap = wrap;
            let name = format!("sampler-{filter:?}-{wrap:?}").to_lowercase();
            diff::assert_agrees(&name, 96, 96, SHADER_TOLERANCE, |canvas| {
                canvas.set_cull(CullMode::None);
                let mut shader = UnlitShader::new(Mat4::IDENTITY);
                shader.texture = Some(&texture);
                // UVs from -0.5 to 1.5, so what happens outside the unit
                // square is most of the picture.
                canvas.draw(&screen_quad(-0.5, 1.5), &shader);
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Blending
// ---------------------------------------------------------------------------

#[test]
fn alpha_blending_composites_the_same_way_on_both_renderers() {
    diff::assert_agrees("alpha-blend", 96, 96, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        canvas.set_blending(Blend::Alpha, true, false);
        // Three translucent quads, offset so every overlap depth occurs.
        for (offset, color) in [
            (-0.3_f32, Color::rgba(0.95, 0.35, 0.35, 0.55)),
            (0.0, Color::rgba(0.35, 0.90, 0.45, 0.55)),
            (0.3, Color::rgba(0.40, 0.55, 0.98, 0.55)),
        ] {
            let mut shader =
                UnlitShader::new(Mat4::from_translation(Vec3::new(offset, offset, 0.0)));
            shader.tint = color;
            canvas.draw(&half_quad(), &shader);
        }
    });
}

#[test]
fn turning_the_depth_write_off_changes_both_renderers_the_same_way() {
    // The middle setting of the showcase's *Depth & blending* page: blended,
    // depth-tested, but not depth-written, which is the one combination where
    // a wrong depth-stencil state still draws something plausible.
    diff::assert_agrees("blend-no-depth-write", 96, 96, SHADER_TOLERANCE, |canvas| {
        canvas.set_cull(CullMode::None);
        canvas.set_blending(Blend::Replace, true, true);
        let mut opaque = UnlitShader::new(Mat4::IDENTITY);
        opaque.tint = Color::rgb(0.2, 0.2, 0.25);
        canvas.draw(&quad_at_depth(0.5), &opaque);

        canvas.set_blending(Blend::Alpha, true, false);
        for z in [0.3_f32, 0.7] {
            let mut shader = UnlitShader::new(Mat4::IDENTITY);
            shader.tint = Color::rgba(0.9, 0.4, 0.2, 0.5);
            canvas.draw(&quad_at_depth(z), &shader);
        }
    });
}

// ---------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------

/// A quad tilted away from the viewer, so the lighting has a gradient across
/// it instead of one flat value.
fn sloped_quad() -> Mesh {
    let mut mesh = screen_quad(0.0, 1.0);
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        let t = index as f32 / 3.0;
        vertex.normal = Vec3::new(t - 0.5, 0.35, 1.0).normalized();
    }
    mesh
}

/// A quad covering the middle of the viewport.
fn half_quad() -> Mesh {
    let corner = |x: f32, y: f32| {
        Vertex::new(Vec3::new(x, y, 0.5), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE)
    };
    Mesh::new(
        vec![
            corner(-0.5, 0.5),
            corner(0.5, 0.5),
            corner(0.5, -0.5),
            corner(-0.5, -0.5),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn quad_at_depth(z: f32) -> Mesh {
    let corner = |x: f32, y: f32| {
        Vertex::new(Vec3::new(x, y, z), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE)
    };
    Mesh::new(
        vec![
            corner(-0.7, 0.7),
            corner(0.7, 0.7),
            corner(0.7, -0.7),
            corner(-0.7, -0.7),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}
