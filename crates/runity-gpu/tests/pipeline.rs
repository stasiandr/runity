//! The conventions the two renderers have to share, one at a time.
//!
//! These run before any shader is worth comparing: which way a front face
//! winds, which way Y runs, whether the depth test keeps the near surface,
//! whether a `Vertex` and a uniform block mean the same thing on both sides.
//! Each one fails on a single fact, so a broken assumption is named rather
//! than diagnosed from a lit scene that looks slightly wrong.
//!
//! Without a Metal device they all skip, loudly.

use runity_gpu::diff::{self, Canvas};
use runity_gpu::{Gpu, GpuShader, PulseUniforms, Target, ENGINE_SOURCE};
use runity_math::{Mat4, Vec2, Vec3, Vec4};
use runity_render::golden::Tolerance;
use runity_render::{
    Color, CullMode, Framebuffer, Mesh, Shader, UnlitShader, Vertex, VertexOutput,
};

/// Open a device, or explain and skip. `RUNITY_REQUIRE_GPU` turns the skip
/// into a failure, which is what a machine with a GPU should be doing.
fn device(test: &str) -> Option<Gpu> {
    match Gpu::new() {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            let required = std::env::var(diff::REQUIRE_ENV)
                .map(|v| !matches!(v.as_str(), "" | "0" | "false"))
                .unwrap_or(false);
            assert!(
                !required,
                "{} is set, but {test} could not run: {error}",
                diff::REQUIRE_ENV
            );
            eprintln!(
                "\n=== SKIPPED: {test} — no GPU ===\n    {error}\n    set {}=1 to make this \
                 a failure\n",
                diff::REQUIRE_ENV
            );
            None
        }
    }
}

/// One triangle in clip space, wound the way `Mesh` documents front faces.
fn counter_clockwise_triangle() -> Mesh {
    // Counter-clockwise in NDC with +Y up: bottom-left, bottom-right, top.
    Mesh::new(
        vec![
            Vertex::new(Vec3::new(-0.9, -0.9, 0.5), Vec3::Z, Vec2::ZERO)
                .with_color(Color::rgb(1.0, 0.0, 0.0)),
            Vertex::new(Vec3::new(0.9, -0.9, 0.5), Vec3::Z, Vec2::ZERO)
                .with_color(Color::rgb(1.0, 0.0, 0.0)),
            Vertex::new(Vec3::new(0.0, 0.9, 0.5), Vec3::Z, Vec2::ZERO)
                .with_color(Color::rgb(1.0, 0.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
}

/// The same triangle, wound the other way.
fn clockwise_triangle() -> Mesh {
    let mut mesh = counter_clockwise_triangle();
    mesh.indices = vec![0, 2, 1];
    mesh
}

/// Render one mesh with back-face culling on and count the lit pixels.
fn lit_pixels_with_backface_culling(gpu: &mut Gpu, mesh: &Mesh) -> usize {
    let mut frame = Framebuffer::new(32, 32);
    let clear = Color::BLACK;
    assert!(gpu.begin_frame(
        Target::Offscreen {
            width: 32,
            height: 32
        },
        clear
    ));
    let shader = UnlitShader::new(Mat4::IDENTITY);
    gpu.draw(
        mesh,
        &shader,
        CullMode::Back,
        runity_render::Blend::Replace,
        true,
        true,
    );
    gpu.end_frame(Some(&mut frame));
    frame
        .pixels()
        .iter()
        .filter(|p| **p != clear.to_argb8())
        .count()
}

#[test]
fn a_counter_clockwise_triangle_is_the_one_that_survives_back_face_culling() {
    let Some(mut gpu) = device("winding") else {
        return;
    };
    let front = lit_pixels_with_backface_culling(&mut gpu, &counter_clockwise_triangle());
    let back = lit_pixels_with_backface_culling(&mut gpu, &clockwise_triangle());
    assert!(
        front > 200,
        "the front-facing triangle should cover most of a 32x32 target, got {front}"
    );
    assert_eq!(
        back, 0,
        "the back-facing one should be culled entirely, got {back} pixels"
    );
}

#[test]
fn y_is_not_flipped_between_the_rasterizer_and_metal() {
    let Some(mut gpu) = device("orientation") else {
        return;
    };
    // A triangle filling only the top half of clip space.
    let mesh = Mesh::new(
        vec![
            Vertex::new(Vec3::new(-1.0, 0.1, 0.5), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE),
            Vertex::new(Vec3::new(1.0, 0.1, 0.5), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE),
            Vertex::new(Vec3::new(0.0, 1.0, 0.5), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE),
        ],
        vec![0, 1, 2],
    );
    let mut frame = Framebuffer::new(32, 32);
    assert!(gpu.begin_frame(
        Target::Offscreen {
            width: 32,
            height: 32
        },
        Color::BLACK
    ));
    let shader = UnlitShader::new(Mat4::IDENTITY);
    gpu.draw(
        &mesh,
        &shader,
        CullMode::None,
        runity_render::Blend::Replace,
        true,
        true,
    );
    gpu.end_frame(Some(&mut frame));

    let lit_in_row = |y: usize| {
        (0..32)
            .filter(|x| frame.get_pixel(*x, y).to_argb8() != Color::BLACK.to_argb8())
            .count()
    };
    // Clip +Y is up, and a framebuffer's first row is the top: the triangle
    // has to land in the early rows, exactly as `to_screen` puts it.
    assert!(lit_in_row(4) > 0, "clip +Y must reach the first rows");
    assert_eq!(lit_in_row(28), 0, "and nothing near the bottom");
}

/// The uniform block, read straight back out.
///
/// `echo_uniform_fragment` writes uniform word `y * 4 + x` into the pixel at
/// `(x, y)`, so a 4-wide target is the block laid out as a picture. A field at
/// the wrong offset fails on its own pixel.
struct EchoUniform {
    values: [f32; 24],
}

impl Shader for EchoUniform {
    type Varying = Color;
    fn vertex(&self, v: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: Vec4::new(v.position.x, v.position.y, v.position.z, 1.0),
            varying: v.color,
        }
    }
    fn fragment(&self, color: &Color) -> Option<Color> {
        // There is no CPU twin of an echo: it exists to read GPU memory back.
        Some(*color)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EchoBlock([f32; 24]);

impl GpuShader for EchoUniform {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = "echo_vertex";
    const FRAGMENT: &'static str = "echo_uniform_fragment";
    type Uniforms = EchoBlock;
    fn uniforms(&self) -> EchoBlock {
        EchoBlock(self.values)
    }
}

/// A full-screen quad whose positions are already clip-space.
fn clip_quad() -> Mesh {
    let corner = |x: f32, y: f32| {
        Vertex::new(Vec3::new(x, y, 0.5), Vec3::Z, Vec2::ZERO).with_color(Color::WHITE)
    };
    Mesh::new(
        vec![
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
            corner(1.0, -1.0),
            corner(-1.0, -1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

#[test]
fn every_word_of_a_uniform_block_reaches_the_shader_at_the_offset_it_was_written_at() {
    let Some(mut gpu) = device("echo: uniforms") else {
        return;
    };
    runity_gpu::assert_uniform_layout::<EchoBlock>();

    // Whole 8-bit steps, so the round trip through a BGRA8Unorm target is
    // exact and a mismatch is a layout fault rather than a rounding one.
    let values: [f32; 24] = std::array::from_fn(|i| (i as f32 * 10.0 + 3.0) / 255.0);
    let mut frame = Framebuffer::new(4, 6);
    assert!(gpu.begin_frame(
        Target::Offscreen {
            width: 4,
            height: 6
        },
        Color::BLACK
    ));
    gpu.draw(
        &clip_quad(),
        &EchoUniform { values },
        CullMode::None,
        runity_render::Blend::Replace,
        false,
        false,
    );
    gpu.end_frame(Some(&mut frame));

    for (index, expected) in values.iter().enumerate() {
        let (x, y) = (index % 4, index / 4);
        let got = frame.get_pixel(x, y).r;
        assert!(
            (got - expected).abs() <= 1.5 / 255.0,
            "uniform word {index} (byte offset {}) came back as {got}, not {expected}",
            index * 4
        );
    }
}

/// The vertex struct, read straight back out.
///
/// `echo_vertex` copies vertex 0's `normal`, `uv` and `color` into flat
/// varyings, and `echo_vertex_fragment` writes word `x` of those nine floats
/// into column `x`. A `Vertex` whose members sit at different offsets in MSL
/// than in Rust fails on the first field that moved.
struct EchoVertex;

impl Shader for EchoVertex {
    type Varying = Color;
    fn vertex(&self, v: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: Vec4::new(v.position.x, v.position.y, v.position.z, 1.0),
            varying: v.color,
        }
    }
    fn fragment(&self, color: &Color) -> Option<Color> {
        Some(*color)
    }
}

impl GpuShader for EchoVertex {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = "echo_vertex";
    const FRAGMENT: &'static str = "echo_vertex_fragment";
    type Uniforms = PulseUniforms;
    fn uniforms(&self) -> PulseUniforms {
        PulseUniforms::new(Mat4::IDENTITY, 1.0)
    }
}

#[test]
fn every_field_of_a_vertex_reaches_the_shader_at_the_offset_it_was_written_at() {
    let Some(mut gpu) = device("echo: vertices") else {
        return;
    };
    // Nine probe values in [0, 1], each a whole 8-bit step and all different.
    let probe: [f32; 9] = std::array::from_fn(|i| (i as f32 * 25.0 + 5.0) / 255.0);
    let probe_vertex = Vertex {
        // The probe mesh carries clip-space positions: no matrix is involved,
        // so a failure here is about layout and nothing else.
        position: Vec3::new(-1.0, 1.0, 0.5),
        normal: Vec3::new(probe[0], probe[1], probe[2]),
        uv: Vec2::new(probe[3], probe[4]),
        color: Color::rgba(probe[5], probe[6], probe[7], probe[8]),
    };
    let filler = |x: f32, y: f32| Vertex {
        position: Vec3::new(x, y, 0.5),
        ..Vertex::default()
    };
    // One triangle, so the provoking vertex of the flat varyings is vertex 0.
    let mesh = Mesh::new(
        vec![probe_vertex, filler(3.0, 1.0), filler(-1.0, -3.0)],
        vec![0, 1, 2],
    );

    let mut frame = Framebuffer::new(9, 1);
    assert!(gpu.begin_frame(
        Target::Offscreen {
            width: 9,
            height: 1
        },
        Color::BLACK
    ));
    gpu.draw(
        &mesh,
        &EchoVertex,
        CullMode::None,
        runity_render::Blend::Replace,
        false,
        false,
    );
    gpu.end_frame(Some(&mut frame));

    const FIELDS: [&str; 9] = [
        "normal.x", "normal.y", "normal.z", "uv.x", "uv.y", "color.r", "color.g", "color.b",
        "color.a",
    ];
    for (index, expected) in probe.iter().enumerate() {
        let got = frame.get_pixel(index, 0).r;
        assert!(
            (got - expected).abs() <= 1.5 / 255.0,
            "Vertex::{} (byte offset {}) came back as {got}, not {expected}",
            FIELDS[index],
            12 + index * 4
        );
    }
}

#[test]
fn the_depth_test_keeps_the_nearer_surface_on_both_renderers() {
    // Two overlapping quads at different depths, drawn far-then-near and
    // near-then-far: only a working depth test gives the same answer twice.
    diff::assert_agrees("depth-order", 64, 64, Tolerance::new(2, 0.0), |canvas| {
        canvas.set_cull(CullMode::None);
        for (z, color) in [(0.8_f32, Color::RED), (0.3, Color::GREEN)] {
            let mesh = quad_at(z, color);
            let shader = UnlitShader::new(Mat4::IDENTITY);
            canvas.draw(&mesh, &shader);
        }
        // Now the other way round: the near one is already in the buffer.
        let mesh = quad_at(0.8, Color::BLUE);
        let shader = UnlitShader::new(Mat4::IDENTITY);
        canvas.draw(&mesh, &shader);
    });
}

fn quad_at(z: f32, color: Color) -> Mesh {
    let corner =
        |x: f32, y: f32| Vertex::new(Vec3::new(x, y, z), Vec3::Z, Vec2::ZERO).with_color(color);
    Mesh::new(
        vec![
            corner(-0.8, 0.8),
            corner(0.8, 0.8),
            corner(0.8, -0.8),
            corner(-0.8, -0.8),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

#[test]
fn back_face_culling_agrees_between_the_renderers() {
    diff::assert_agrees(
        "culling",
        64,
        64,
        Tolerance::new(2, 0.0),
        |canvas: &mut Canvas| {
            // A cube: half its faces are facing away, and which half is exactly
            // what the winding convention decides.
            canvas.state.cull = CullMode::Back;
            let mvp = Mat4::perspective(1.0, 1.0, 0.1, 100.0)
                * Mat4::look_at(Vec3::new(2.0, 1.5, 3.0), Vec3::ZERO, Vec3::Y);
            let shader = UnlitShader::new(mvp);
            let mut cube = Mesh::cube(1.6);
            cube.set_color(Color::rgb(0.9, 0.5, 0.2));
            canvas.draw(&cube, &shader);
        },
    );
}
