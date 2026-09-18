//! SmallWorld: six scenes in one window — the whole engine, page by page.
//!
//! ```text
//! cargo run --release --example smallworld
//! RUNITY_HEADLESS=1 cargo run --release --example smallworld   # 01-lit-scene.png ... 06-world.png
//! ```
//!
//! Keyboard only; this list and the README are the only places the keys are
//! written down.
//!
//! | Key | What it does |
//! | --- | --- |
//! | `Tab` / `Shift+Tab` | next / previous scene, wrapping around all six |
//! | arrows or `WASD` | orbit the current scene's camera |
//! | `Q` / `E` | zoom out / in |
//! | `Space` | the current scene's action (see below) |
//! | `Backspace` | undo it — in *World*, drop the last ten entities |
//! | `1` `2` `3` `4` | shaded, wireframe, depth buffer, overdraw |
//! | `N` | overlay vertex normals and the world axes |
//! | `Escape`, or the close button | quit |
//!
//! What `Space` does, scene by scene: *Lit scene* pauses the animation,
//! *Triangle* freezes the pulse, *Meshes* stops the turntable, *Textures*
//! swaps the texture for the one that went through this engine's PNG codec,
//! *Depth & blending* steps through the blend and depth-buffer settings, and
//! *World* spawns ten more entities.
//!
//! The camera, and everything else a scene owns, belongs to that scene: the
//! five scenes that are not on screen are frozen exactly where they were left.
//! The debug view is the one thing they share, so it survives `Tab`.
//!
//! Environment variables:
//!
//! * `RUNITY_HEADLESS=1` — no window: run every scene for 60 frames at a fixed
//!   1/60 s step and save its last frame as `01-lit-scene.png` … `06-world.png`
//!   in the current directory.
//! * `RUNITY_SCENE=1..6` — open straight into that scene (windowed).
//! * `RUNITY_DEBUG_VIEW=shaded|wireframe|depth|overdraw` — start in that view;
//!   it applies to every scene.
//! * `RUNITY_FRAMES=<dir>` — headless only: also keep all 60 frames of every
//!   scene as PNGs in `<dir>`.

use runity::prelude::*;
use std::f32::consts::{PI, TAU};
use std::io;

/// The window never changes shape, so the headless PNGs are the same pixels.
const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
/// Frames each scene is given in a headless run, and the step between them.
const HEADLESS_FRAMES: u64 = 60;
const HEADLESS_DELTA: f32 = 1.0 / 60.0;

// ---------------------------------------------------------------------------
// Which scene is on screen
// ---------------------------------------------------------------------------

/// The six pages of the showcase, in the order `Tab` walks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneId {
    Lit,
    Triangle,
    Meshes,
    Textures,
    DepthAndBlending,
    World,
}

const SCENE_COUNT: usize = 6;

impl SceneId {
    const ALL: [SceneId; SCENE_COUNT] = [
        SceneId::Lit,
        SceneId::Triangle,
        SceneId::Meshes,
        SceneId::Textures,
        SceneId::DepthAndBlending,
        SceneId::World,
    ];

    /// What the title bar calls it.
    fn name(self) -> &'static str {
        match self {
            SceneId::Lit => "Lit scene",
            SceneId::Triangle => "Triangle",
            SceneId::Meshes => "Meshes",
            SceneId::Textures => "Textures",
            SceneId::DepthAndBlending => "Depth & blending",
            SceneId::World => "World",
        }
    }

    /// The headless screenshot's file name, numbered so the six sort in order.
    fn file_stem(self) -> &'static str {
        match self {
            SceneId::Lit => "01-lit-scene",
            SceneId::Triangle => "02-triangle",
            SceneId::Meshes => "03-meshes",
            SceneId::Textures => "04-textures",
            SceneId::DepthAndBlending => "05-depth-blending",
            SceneId::World => "06-world",
        }
    }

    /// Zero-based position in [`SceneId::ALL`]; the title shows it plus one.
    fn index(self) -> usize {
        match self {
            SceneId::Lit => 0,
            SceneId::Triangle => 1,
            SceneId::Meshes => 2,
            SceneId::Textures => 3,
            SceneId::DepthAndBlending => 4,
            SceneId::World => 5,
        }
    }

    fn from_index(index: usize) -> Self {
        SceneId::ALL[index % SCENE_COUNT]
    }

    fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    fn previous(self) -> Self {
        Self::from_index(self.index() + SCENE_COUNT - 1)
    }
}

/// `SmallWorld — 3/6 Meshes — 60 fps`.
///
/// Rebuilt every frame and handed to [`Engine::set_title`], which only forwards
/// it when it actually changed — and the frame rate is averaged over half a
/// second, so that is roughly twice a second.
fn window_title(scene: SceneId, fps: f32) -> String {
    format!(
        "SmallWorld — {}/{} {} — {:.0} fps",
        scene.index() + 1,
        SCENE_COUNT,
        scene.name(),
        fps
    )
}

// ---------------------------------------------------------------------------
// A camera each scene owns
// ---------------------------------------------------------------------------

/// Where a scene's camera sits. Every scene keeps one, so walking away with
/// `Tab` and coming back finds the view exactly where it was left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    target: Vec3,
}

impl Orbit {
    fn new(yaw: f32, pitch: f32, distance: f32) -> Self {
        Self {
            yaw,
            pitch,
            distance,
            target: Vec3::ZERO,
        }
    }

    fn looking_at(mut self, target: Vec3) -> Self {
        self.target = target;
        self
    }

    /// Arrows or WASD to swing around, Q/E to pull back or lean in.
    fn steer(&mut self, input: &Input, dt: f32) {
        self.yaw += (input.axis(Key::Left, Key::Right) + input.axis(Key::A, Key::D)) * dt * 1.5;
        self.pitch = (self.pitch
            + (input.axis(Key::Down, Key::Up) + input.axis(Key::S, Key::W)) * dt * 1.2)
            .clamp(-1.2, 1.4);
        self.distance = (self.distance + input.axis(Key::E, Key::Q) * dt * 6.0).clamp(1.5, 40.0);
    }

    fn apply(&self, camera: &mut Camera) {
        camera.target = self.target;
        camera.orbit(self.yaw, self.pitch, self.distance);
    }
}

// ---------------------------------------------------------------------------
// What a scene is
// ---------------------------------------------------------------------------

/// One page of the showcase.
///
/// A scene owns its camera and all of its state, and the application only ever
/// drives the one on screen — which is the whole trick behind "the other five
/// are frozen".
trait Scene {
    /// Background and lighting this scene wants. Called before the frame is
    /// cleared, so a scene that follows another one's environment still gets
    /// its own on the very first frame it is shown.
    fn environment(&self, engine: &mut Engine);

    fn orbit_mut(&mut self) -> &mut Orbit;

    /// Per-frame simulation, with the frame's delta.
    fn update(&mut self, _engine: &mut Engine, _dt: f32) {}

    /// Fixed-step simulation, at `engine.time.fixed_delta`.
    fn fixed_update(&mut self, _engine: &mut Engine) {}

    fn render(&mut self, engine: &mut Engine);

    /// Space.
    fn action(&mut self, _engine: &mut Engine) {}

    /// Backspace: the reverse of `action`, where there is one.
    fn undo(&mut self, _engine: &mut Engine) {}

    /// Extra lines drawn when the overlay key (N) is on, over the world axes
    /// the application draws for every scene.
    fn overlay(&mut self, _engine: &mut Engine) {}
}

fn build_scene(id: SceneId, engine: &mut Engine) -> Box<dyn Scene> {
    match id {
        SceneId::Lit => Box::new(LitScene::new()),
        SceneId::Triangle => Box::new(TriangleScene::new()),
        SceneId::Meshes => Box::new(MeshesScene::new()),
        SceneId::Textures => Box::new(TexturesScene::new()),
        SceneId::DepthAndBlending => Box::new(DepthScene::new()),
        SceneId::World => Box::new(WorldScene::new(engine)),
    }
}

// ---------------------------------------------------------------------------
// 1. Lit scene
// ---------------------------------------------------------------------------

/// The flagship: a textured crate on a checkerboard floor, two spheres orbiting
/// it, one directional light and the specular highlights it throws.
struct LitScene {
    orbit: Orbit,
    crate_mesh: Mesh,
    floor: Mesh,
    sphere: Mesh,
    crate_texture: Texture,
    floor_texture: Texture,
    angle: f32,
    animating: bool,
}

impl LitScene {
    fn new() -> Self {
        let crate_texture = Texture::from_fn(64, 64, |x, y| {
            // Planks, generated rather than loaded: this scene needs no asset.
            let plank = (y / 16) % 2;
            let grain = ((x * 7 + y * 3) % 32) as f32 / 32.0;
            let shade = 0.55 + grain * 0.25;
            if x % 16 == 0 || y % 16 == 0 {
                Color::rgb(0.25, 0.16, 0.10)
            } else if plank == 0 {
                Color::rgb(0.72 * shade, 0.45 * shade, 0.22 * shade)
            } else {
                Color::rgb(0.62 * shade, 0.38 * shade, 0.18 * shade)
            }
        });

        let mut floor_texture = Texture::checker(
            128,
            16,
            Color::rgb(0.20, 0.22, 0.26),
            Color::rgb(0.32, 0.34, 0.40),
        );
        floor_texture.wrap = Wrap::Repeat;

        Self {
            orbit: Orbit::new(0.6, 0.45, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0)),
            crate_mesh: Mesh::cube(1.4),
            floor: Mesh::plane(14.0, 1),
            sphere: Mesh::sphere(0.55, 28, 18),
            crate_texture,
            floor_texture,
            angle: 0.0,
            animating: true,
        }
    }

    fn crate_model(&self) -> Mat4 {
        Mat4::from_rotation_y(self.angle) * Mat4::from_rotation_x(self.angle * 0.6)
    }
}

impl Scene for LitScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.04, 0.05, 0.08);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.5, -0.85, -0.35).normalized(),
            color: Color::rgb(1.0, 0.96, 0.88),
            intensity: 1.15,
        };
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    fn update(&mut self, _engine: &mut Engine, dt: f32) {
        if self.animating {
            self.angle += dt * 0.8;
        }
    }

    fn action(&mut self, _engine: &mut Engine) {
        self.animating = !self.animating;
    }

    fn render(&mut self, engine: &mut Engine) {
        let mut shader = engine.lit_shader(Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0)));
        shader.texture = Some(&self.floor_texture);
        shader.specular_strength = 0.05;
        engine.draw(&self.floor, &shader);

        let mut shader = engine.lit_shader(self.crate_model());
        shader.texture = Some(&self.crate_texture);
        engine.draw(&self.crate_mesh, &shader);

        for (i, tint) in [Color::rgb(0.9, 0.3, 0.35), Color::rgb(0.35, 0.65, 0.95)]
            .into_iter()
            .enumerate()
        {
            let phase = self.angle * 1.6 + i as f32 * PI;
            let position = Vec3::new(
                phase.cos() * 2.3,
                0.35 + (phase * 2.0).sin() * 0.45,
                phase.sin() * 2.3,
            );
            let mut shader = engine.lit_shader(Mat4::from_translation(position));
            shader.base_color = tint;
            shader.specular_strength = 0.6;
            shader.shininess = 64.0;
            engine.draw(&self.sphere, &shader);
        }
    }

    fn overlay(&mut self, engine: &mut Engine) {
        engine.draw_normals(
            &self.crate_mesh,
            self.crate_model(),
            0.35,
            Color::rgb(0.2, 1.0, 0.4),
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Triangle
// ---------------------------------------------------------------------------

/// A shader is a pair of functions: `vertex` hands the rasterizer a clip-space
/// position and whatever the fragment stage needs, and the rasterizer
/// interpolates that with perspective correction.
struct PulseShader {
    mvp: Mat4,
    pulse: f32,
}

impl Shader for PulseShader {
    /// The interpolant: the vertex color, and nothing else.
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

/// One triangle, one hand-written shader: color interpolated across the face
/// and a pulse applied per fragment.
struct TriangleScene {
    orbit: Orbit,
    triangle: Mesh,
    angle: f32,
    phase: f32,
    pulsing: bool,
}

impl TriangleScene {
    fn new() -> Self {
        let vertices = vec![
            Vertex::new(Vec3::new(-1.2, -0.9, 0.0), Vec3::Z, Vec2::new(0.0, 1.0))
                .with_color(Color::rgb(0.95, 0.25, 0.30)),
            Vertex::new(Vec3::new(1.2, -0.9, 0.0), Vec3::Z, Vec2::new(1.0, 1.0))
                .with_color(Color::rgb(0.25, 0.85, 0.40)),
            Vertex::new(Vec3::new(0.0, 1.2, 0.0), Vec3::Z, Vec2::new(0.5, 0.0))
                .with_color(Color::rgb(0.30, 0.45, 0.98)),
        ];
        Self {
            orbit: Orbit::new(0.0, 0.15, 3.0),
            triangle: Mesh::new(vertices, vec![0, 1, 2]),
            angle: 0.0,
            phase: 0.0,
            pulsing: true,
        }
    }

    /// Brightness the fragment stage multiplies in — a slow pulse, so it is
    /// obvious that the shader really runs every frame.
    fn pulse(&self) -> f32 {
        0.75 + 0.25 * (self.phase * 2.0).sin()
    }
}

impl Scene for TriangleScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.06, 0.06, 0.10);
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    fn update(&mut self, _engine: &mut Engine, dt: f32) {
        self.angle += dt * 0.5;
        if self.pulsing {
            self.phase += dt;
        }
    }

    fn action(&mut self, _engine: &mut Engine) {
        self.pulsing = !self.pulsing;
    }

    fn render(&mut self, engine: &mut Engine) {
        // A triangle has no back, so let both sides show.
        engine.rasterizer.cull = CullMode::None;
        let shader = PulseShader {
            mvp: engine.view_projection() * Mat4::from_rotation_y(self.angle),
            pulse: self.pulse(),
        };
        engine.draw(&self.triangle, &shader);
    }
}

// ---------------------------------------------------------------------------
// 3. Meshes
// ---------------------------------------------------------------------------

/// The mesh shelf: the three procedural primitives next to the Newell teapot,
/// parsed out of `examples/assets/teapot.obj`.
///
/// The teapot is lit but untextured — the file carries no `vt` lines, so there
/// are no UVs to sample with, and its normals are the ones `Mesh::from_obj`
/// derived from the faces.
struct MeshesScene {
    orbit: Orbit,
    cube: Mesh,
    plane: Mesh,
    sphere: Mesh,
    teapot: Mesh,
    angle: f32,
    turning: bool,
}

impl MeshesScene {
    fn new() -> Self {
        let teapot = Mesh::from_obj(include_str!("assets/teapot.obj"))
            .expect("assets/teapot.obj ships with this example and parses");
        Self {
            orbit: Orbit::new(0.3, 0.3, 8.0).looking_at(Vec3::new(0.0, 0.1, 0.0)),
            cube: Mesh::cube(1.3),
            plane: Mesh::plane(1.8, 4),
            sphere: Mesh::sphere(0.8, 28, 18),
            teapot,
            angle: 0.0,
            turning: true,
        }
    }

    /// Where each mesh stands on the shelf, and what it is made of.
    fn shelf(&self) -> [(&Mesh, Mat4, Color); 4] {
        let turntable = Mat4::from_rotation_y(self.angle);
        let place = |x: f32, y: f32, scale: f32| {
            Mat4::from_translation(Vec3::new(x, y, 0.0))
                * turntable
                * Mat4::from_scale(Vec3::splat(scale))
        };
        [
            (
                &self.cube,
                place(-3.9, 0.0, 1.0),
                Color::rgb(0.85, 0.55, 0.25),
            ),
            (
                &self.plane,
                place(-1.3, 0.0, 1.0),
                Color::rgb(0.45, 0.70, 0.55),
            ),
            (
                &self.sphere,
                place(1.3, 0.0, 1.0),
                Color::rgb(0.35, 0.65, 0.95),
            ),
            (
                &self.teapot,
                place(3.9, -0.6, 0.38),
                Color::rgb(0.80, 0.78, 0.74),
            ),
        ]
    }
}

impl Scene for MeshesScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.05, 0.06, 0.09);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.35, -0.8, -0.5).normalized(),
            color: Color::WHITE,
            intensity: 1.05,
        };
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    fn update(&mut self, _engine: &mut Engine, dt: f32) {
        if self.turning {
            self.angle += dt * 0.6;
        }
    }

    fn action(&mut self, _engine: &mut Engine) {
        self.turning = !self.turning;
    }

    fn render(&mut self, engine: &mut Engine) {
        // The plane is one-sided; from below it would otherwise vanish.
        engine.rasterizer.cull = CullMode::None;
        for (mesh, model, color) in self.shelf() {
            let mut shader = engine.lit_shader(model);
            shader.base_color = color;
            shader.specular_strength = 0.35;
            engine.draw(mesh, &shader);
        }
    }

    fn overlay(&mut self, engine: &mut Engine) {
        let model = self.shelf()[2].1;
        engine.draw_normals(&self.sphere, model, 0.3, Color::rgb(0.2, 1.0, 0.4));
    }
}

// ---------------------------------------------------------------------------
// 4. Textures
// ---------------------------------------------------------------------------

/// Every [`Filter`] against every [`Wrap`]: six quads, the same texture, the
/// same UVs running from -0.5 to 1.5 so what happens outside the unit square is
/// on screen.
///
/// The texture itself makes a round trip through this engine's own PNG codec
/// before it is ever sampled, and `Space` swaps between the original and the
/// decoded copy — they should be impossible to tell apart.
struct TexturesScene {
    orbit: Orbit,
    quad: Mesh,
    source: Texture,
    decoded: Texture,
    variants: Vec<Texture>,
    show_decoded: bool,
}

impl TexturesScene {
    const FILTERS: [Filter; 2] = [Filter::Nearest, Filter::Bilinear];
    const WRAPS: [Wrap; 3] = [Wrap::Repeat, Wrap::Clamp, Wrap::Mirror];

    fn new() -> Self {
        let source = Self::source_texture();
        let decoded = png_round_trip(&source);
        let mut scene = Self {
            orbit: Orbit::new(0.0, 0.0, 5.6),
            quad: unit_quad(1.5, -0.5, 1.5),
            source,
            decoded,
            variants: Vec::new(),
            show_decoded: true,
        };
        scene.rebuild_variants();
        scene
    }

    /// Something with hard edges and a diagonal, so nearest and bilinear cannot
    /// look the same. Every channel is a whole 8-bit step, which is what lets
    /// the PNG round trip below be exact.
    fn source_texture() -> Texture {
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

    /// One texture per cell of the grid: `Filter` and `Wrap` live on the
    /// texture, so each combination needs its own copy.
    fn rebuild_variants(&mut self) {
        let base = if self.show_decoded {
            &self.decoded
        } else {
            &self.source
        };
        self.variants.clear();
        for filter in Self::FILTERS {
            for wrap in Self::WRAPS {
                let mut texture = base.clone();
                texture.filter = filter;
                texture.wrap = wrap;
                self.variants.push(texture);
            }
        }
    }

    /// Where the cell for `index` sits: filters in rows, wraps in columns.
    fn cell_model(index: usize) -> Mat4 {
        let row = index / Self::WRAPS.len();
        let column = index % Self::WRAPS.len();
        Mat4::from_translation(Vec3::new(
            (column as f32 - 1.0) * 1.8,
            0.95 - row as f32 * 1.9,
            0.0,
        ))
    }
}

impl Scene for TexturesScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.07, 0.07, 0.09);
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    fn action(&mut self, _engine: &mut Engine) {
        self.show_decoded = !self.show_decoded;
        self.rebuild_variants();
        println!(
            "textures: showing the {} texture",
            if self.show_decoded {
                "PNG round-tripped"
            } else {
                "original"
            }
        );
    }

    fn render(&mut self, engine: &mut Engine) {
        // Flat quads facing the camera: no lighting to get in the way of the
        // thing being compared.
        engine.rasterizer.cull = CullMode::None;
        let view_projection = engine.view_projection();
        for (index, texture) in self.variants.iter().enumerate() {
            let mut shader = UnlitShader::new(view_projection * Self::cell_model(index));
            shader.texture = Some(texture);
            engine.draw(&self.quad, &shader);
        }
    }
}

/// Encode a texture as a PNG and decode it straight back — the codec in
/// `runity-render`, both directions, inside the frame.
fn png_round_trip(texture: &Texture) -> Texture {
    let pixels: Vec<u32> = (0..texture.height())
        .flat_map(|y| (0..texture.width()).map(move |x| (x, y)))
        .map(|(x, y)| {
            let u = (x as f32 + 0.5) / texture.width() as f32;
            let v = (y as f32 + 0.5) / texture.height() as f32;
            texture.sample(u, v).to_argb8()
        })
        .collect();
    let encoded = encode_png(texture.width(), texture.height(), &pixels);
    let image = decode_png(&encoded).expect("our own encoder produces a PNG we can read");
    let texels = image
        .pixels
        .iter()
        .map(|p| Color::from_argb8(*p))
        .collect::<Vec<_>>();
    let mut decoded = Texture::new(image.width, image.height, texels);
    decoded.filter = texture.filter;
    decoded.wrap = texture.wrap;
    decoded
}

/// A quad in the XY plane, with UVs that deliberately run outside `[0, 1]`.
fn unit_quad(size: f32, uv_min: f32, uv_max: f32) -> Mesh {
    let h = size * 0.5;
    let corner = |x: f32, y: f32, u: f32, v: f32| {
        Vertex::new(Vec3::new(x, y, 0.0), Vec3::Z, Vec2::new(u, v))
    };
    Mesh::new(
        vec![
            corner(-h, -h, uv_min, uv_max),
            corner(h, -h, uv_max, uv_max),
            corner(h, h, uv_max, uv_min),
            corner(-h, h, uv_min, uv_min),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

// ---------------------------------------------------------------------------
// 5. Depth & blending
// ---------------------------------------------------------------------------

/// Three quads through one another, and the three fixed-function settings that
/// decide what that looks like. `Space` steps through them.
struct DepthScene {
    orbit: Orbit,
    quad: Mesh,
    mode: usize,
    angle: f32,
}

/// What `Space` cycles: how fragments are combined, and whether the depth
/// buffer is read and written at all.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DepthMode {
    name: &'static str,
    blend: Blend,
    depth_test: bool,
    depth_write: bool,
    alpha: f32,
}

const DEPTH_MODES: [DepthMode; 3] = [
    DepthMode {
        name: "opaque, depth test and write",
        blend: Blend::Replace,
        depth_test: true,
        depth_write: true,
        alpha: 1.0,
    },
    DepthMode {
        name: "alpha blend, depth test, no depth write",
        blend: Blend::Alpha,
        depth_test: true,
        depth_write: false,
        alpha: 0.55,
    },
    DepthMode {
        name: "alpha blend, no depth test",
        blend: Blend::Alpha,
        depth_test: false,
        depth_write: false,
        alpha: 0.55,
    },
];

impl DepthScene {
    fn new() -> Self {
        Self {
            orbit: Orbit::new(0.7, 0.35, 5.0),
            quad: unit_quad(2.6, 0.0, 1.0),
            mode: 0,
            angle: 0.0,
        }
    }

    fn mode(&self) -> DepthMode {
        DEPTH_MODES[self.mode % DEPTH_MODES.len()]
    }

    /// Three quads sharing the origin, each turned a third of the way around,
    /// so whichever setting is on they intersect.
    fn quads(&self) -> [(Mat4, Color); 3] {
        let alpha = self.mode().alpha;
        let tint = |r, g, b| Color::rgba(r, g, b, alpha);
        [
            (
                Mat4::from_rotation_y(self.angle) * Mat4::from_rotation_x(0.35),
                tint(0.95, 0.35, 0.35),
            ),
            (
                Mat4::from_rotation_y(self.angle + TAU / 3.0) * Mat4::from_rotation_x(0.35),
                tint(0.35, 0.90, 0.45),
            ),
            (
                Mat4::from_rotation_y(self.angle + 2.0 * TAU / 3.0) * Mat4::from_rotation_x(0.35),
                tint(0.40, 0.55, 0.98),
            ),
        ]
    }
}

impl Scene for DepthScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.08, 0.08, 0.11);
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    fn update(&mut self, _engine: &mut Engine, dt: f32) {
        self.angle += dt * 0.35;
    }

    fn action(&mut self, _engine: &mut Engine) {
        self.mode = (self.mode + 1) % DEPTH_MODES.len();
        println!("depth & blending: {}", self.mode().name);
    }

    fn render(&mut self, engine: &mut Engine) {
        let mode = self.mode();
        engine.rasterizer.cull = CullMode::None;
        engine.rasterizer.blend = mode.blend;
        engine.rasterizer.depth_test = mode.depth_test;
        engine.rasterizer.depth_write = mode.depth_write;

        let view_projection = engine.view_projection();
        for (model, color) in self.quads() {
            let mut shader = UnlitShader::new(view_projection * model);
            shader.tint = color;
            engine.draw(&self.quad, &shader);
        }
    }
}

// ---------------------------------------------------------------------------
// 6. World
// ---------------------------------------------------------------------------

/// Component: a slot on the ring, and the color the entity was born with.
#[derive(Debug, Clone, Copy)]
struct Ringed {
    /// Angle around the ring, before the ring's own rotation is added.
    slot: f32,
    /// How much faster than the ring the cube itself turns.
    spin: f32,
    tint: Color,
}

/// Entities on a ring, ten at a time: `Space` spawns another ten, `Backspace`
/// despawns the last ten.
///
/// The color is picked from the entity's generation, so a slot that comes back
/// from the free list is visibly not the entity that used to live there.
struct WorldScene {
    orbit: Orbit,
    mesh: Mesh,
    /// Spawn order, which is what makes "the last ten" well defined.
    spawned: Vec<Entity>,
    rotation: f32,
}

/// How many entities a batch is.
const BATCH: usize = 10;

impl WorldScene {
    fn new(engine: &mut Engine) -> Self {
        let mut scene = Self {
            orbit: Orbit::new(0.4, 0.55, 9.0),
            mesh: Mesh::cube(0.62),
            spawned: Vec::new(),
            rotation: 0.0,
        };
        scene.spawn_batch(engine);
        scene
    }

    fn radius(&self) -> f32 {
        2.4 + self.spawned.len() as f32 * 0.05
    }

    fn spawn_batch(&mut self, engine: &mut Engine) {
        for _ in 0..BATCH {
            let entity = engine.world.spawn();
            let tint = generation_tint(entity.generation());
            engine.world.insert(entity, Transform::IDENTITY);
            engine.world.insert(
                entity,
                Ringed {
                    slot: 0.0,
                    spin: 1.0 + (self.spawned.len() % 3) as f32 * 0.6,
                    tint,
                },
            );
            self.spawned.push(entity);
        }
        self.relayout(engine);
    }

    fn despawn_batch(&mut self, engine: &mut Engine) {
        for entity in self
            .spawned
            .split_off(self.spawned.len().saturating_sub(BATCH))
        {
            engine.world.despawn(entity);
        }
        self.relayout(engine);
    }

    /// Spread whatever is alive evenly around the ring.
    fn relayout(&mut self, engine: &mut Engine) {
        let count = self.spawned.len().max(1) as f32;
        let radius = self.radius();
        for (index, entity) in self.spawned.iter().enumerate() {
            if let Some(ring) = engine.world.get_mut::<Ringed>(*entity) {
                ring.slot = index as f32 / count * TAU;
            }
        }
        self.orbit.distance = (radius * 2.2).clamp(5.5, 20.0);
        self.place(engine);
    }

    /// Write every entity's ring position into its [`Transform`].
    fn place(&self, engine: &mut Engine) {
        let radius = self.radius();
        for entity in engine.world.entities_with::<Ringed>() {
            let Some(ring) = engine.world.get::<Ringed>(entity).copied() else {
                continue;
            };
            let angle = ring.slot + self.rotation;
            let position = Vec3::new(
                angle.cos() * radius,
                (angle * 3.0).sin() * 0.4,
                angle.sin() * radius,
            );
            if let Some(transform) = engine.world.get_mut::<Transform>(entity) {
                transform.position = position;
                transform.rotation = Quat::from_axis_angle(Vec3::Y, angle * ring.spin);
            }
        }
    }

    fn report(&self, engine: &Engine) {
        let newest = self
            .spawned
            .last()
            .map(|e| e.generation())
            .unwrap_or_default();
        println!(
            "world: {} entities, newest slot is on generation {newest}",
            engine.world.entity_count()
        );
    }
}

/// Six colors, indexed by generation: reusing a slot moves its generation on,
/// so the replacement lands in a different color than what it replaced.
fn generation_tint(generation: u32) -> Color {
    const PALETTE: [Color; 6] = [
        Color::rgb(0.95, 0.45, 0.35),
        Color::rgb(0.45, 0.85, 0.50),
        Color::rgb(0.40, 0.60, 0.98),
        Color::rgb(0.95, 0.80, 0.35),
        Color::rgb(0.75, 0.45, 0.95),
        Color::rgb(0.40, 0.88, 0.88),
    ];
    PALETTE[generation as usize % PALETTE.len()]
}

impl Scene for WorldScene {
    fn environment(&self, engine: &mut Engine) {
        engine.clear_color = Color::rgb(0.04, 0.05, 0.07);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.3, -0.9, -0.4).normalized(),
            color: Color::WHITE,
            intensity: 1.1,
        };
    }

    fn orbit_mut(&mut self) -> &mut Orbit {
        &mut self.orbit
    }

    /// The ring turns on the fixed step, not the frame: the same number of
    /// ticks always produces the same picture, however fast the machine is.
    fn fixed_update(&mut self, engine: &mut Engine) {
        self.rotation += engine.time.fixed_delta * 0.35;
        self.place(engine);
    }

    fn action(&mut self, engine: &mut Engine) {
        self.spawn_batch(engine);
        self.report(engine);
    }

    fn undo(&mut self, engine: &mut Engine) {
        self.despawn_batch(engine);
        self.report(engine);
    }

    fn render(&mut self, engine: &mut Engine) {
        // Collected first: drawing needs the engine, and iterating borrows it.
        let instances: Vec<(Mat4, Color)> = engine
            .world
            .iter::<Transform>()
            .filter_map(|(entity, transform)| {
                let ring = engine.world.get::<Ringed>(entity)?;
                Some((transform.matrix(), ring.tint))
            })
            .collect();
        for (model, tint) in instances {
            let mut shader = engine.lit_shader(model);
            shader.base_color = tint;
            shader.specular_strength = 0.4;
            engine.draw(&self.mesh, &shader);
        }
    }
}

// ---------------------------------------------------------------------------
// The application
// ---------------------------------------------------------------------------

/// The six scenes, the one on screen, and the keys that move between them.
struct SmallWorld {
    current: SceneId,
    /// Built the first time a scene is visited, and kept from then on.
    scenes: [Option<Box<dyn Scene>>; SCENE_COUNT],
    debug_view: DebugView,
    overlays: bool,
}

impl SmallWorld {
    fn new(start: SceneId, debug_view: DebugView) -> Self {
        Self {
            current: start,
            scenes: std::array::from_fn(|_| None),
            debug_view,
            overlays: false,
        }
    }

    /// The scene on screen, built if this is the first visit.
    fn active(&mut self, engine: &mut Engine) -> &mut dyn Scene {
        let index = self.current.index();
        self.scenes[index]
            .get_or_insert_with(|| build_scene(SceneId::ALL[index], engine))
            .as_mut()
    }
}

impl Game for SmallWorld {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        // One debug view for all six scenes, which is why `Tab` cannot lose it.
        engine.debug_view = self.debug_view;
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }
        if engine.input.key_pressed(Key::Tab) {
            let backwards =
                engine.input.key_down(Key::LeftShift) || engine.input.key_down(Key::RightShift);
            self.current = if backwards {
                self.current.previous()
            } else {
                self.current.next()
            };
        }
        for (key, view) in [
            (Key::Num1, DebugView::Shaded),
            (Key::Num2, DebugView::Wireframe),
            (Key::Num3, DebugView::Depth),
            (Key::Num4, DebugView::Overdraw),
        ] {
            if engine.input.key_pressed(key) {
                engine.debug_view = view;
            }
        }
        if engine.input.key_pressed(Key::N) {
            self.overlays = !self.overlays;
        }

        let dt = engine.time.delta();
        let action = engine.input.key_pressed(Key::Space);
        let undo = engine.input.key_pressed(Key::Backspace);

        // Only the scene on screen is touched; the other five stand still.
        let scene = self.active(engine);
        scene.environment(engine);
        if action {
            scene.action(engine);
        }
        if undo {
            scene.undo(engine);
        }
        scene.orbit_mut().steer(&engine.input, dt);
        scene.orbit_mut().apply(&mut engine.camera);
        scene.update(engine, dt);

        engine.set_title(window_title(self.current, engine.time.average_fps()));
    }

    fn fixed_update(&mut self, engine: &mut Engine) {
        self.active(engine).fixed_update(engine);
    }

    fn render(&mut self, engine: &mut Engine) {
        // Every scene starts from the same fixed-function state; the one that
        // changes it (Depth & blending) says so itself, every frame.
        engine.rasterizer.cull = CullMode::Back;
        engine.rasterizer.depth_test = true;
        engine.rasterizer.depth_write = true;
        engine.rasterizer.blend = Blend::Replace;

        let overlays = self.overlays;
        let scene = self.active(engine);
        scene.render(engine);
        if overlays {
            scene.overlay(engine);
            engine.draw_axes(1.5);
        }
    }
}

// ---------------------------------------------------------------------------
// Starting up
// ---------------------------------------------------------------------------

/// `RUNITY_SCENE` counts from 1, like the number in the title.
fn scene_from_env(value: Option<&str>) -> SceneId {
    match value.and_then(|v| v.trim().parse::<usize>().ok()) {
        Some(n) if (1..=SCENE_COUNT).contains(&n) => SceneId::from_index(n - 1),
        _ => SceneId::Lit,
    }
}

fn debug_view_from_env(value: Option<&str>) -> DebugView {
    match value.unwrap_or_default().trim() {
        "wireframe" => DebugView::Wireframe,
        "depth" => DebugView::Depth,
        "overdraw" => DebugView::Overdraw,
        _ => DebugView::Shaded,
    }
}

/// Render every scene with no display and write the six screenshots.
fn run_headless(debug_view: DebugView) -> io::Result<()> {
    let frames_directory = std::env::var_os("RUNITY_FRAMES");
    for id in SceneId::ALL {
        let game = SmallWorld::new(id, debug_view);
        let frame = match &frames_directory {
            Some(directory) => {
                let frames =
                    headless::record(game, WIDTH, HEIGHT, HEADLESS_FRAMES, HEADLESS_DELTA)?;
                headless::save_frames(directory, &format!("{}-", id.file_stem()), &frames)?;
                frames.last().cloned().expect("60 frames were recorded")
            }
            None => {
                headless::run(game, WIDTH, HEIGHT, HEADLESS_FRAMES, HEADLESS_DELTA)?.framebuffer
            }
        };
        let path = format!("{}.png", id.file_stem());
        save_png(&path, &frame)?;
        println!(
            "wrote {path} ({}x{}) — {} after {HEADLESS_FRAMES} frames",
            frame.width(),
            frame.height(),
            id.name()
        );
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let debug_view = debug_view_from_env(std::env::var("RUNITY_DEBUG_VIEW").ok().as_deref());

    if std::env::var("RUNITY_HEADLESS").is_ok() {
        return run_headless(debug_view);
    }

    let start = scene_from_env(std::env::var("RUNITY_SCENE").ok().as_deref());
    let mut config = WindowConfig::new("SmallWorld", WIDTH, HEIGHT);
    // Six scenes, one size: nothing here has a reason to be resized, and a
    // fixed framebuffer is what makes the window and the PNGs the same pixels.
    config.resizable = false;
    App::new(config).run(SmallWorld::new(start, debug_view))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_render::golden::Tolerance;

    /// One frame with `keys` freshly pressed, then one with them released:
    /// `Input` only reports a press on the transition, so a test that wants to
    /// tap the same key twice has to let go in between.
    fn tap(app: &mut SmallWorld, engine: &mut Engine, keys: &[Key]) {
        engine.input.begin_frame();
        for key in keys {
            engine.input.handle(&Event::KeyDown(*key));
        }
        app.update(engine);
        engine.input.begin_frame();
        for key in keys {
            engine.input.handle(&Event::KeyUp(*key));
        }
        app.update(engine);
    }

    /// Hold `keys` down for one frame worth of `dt`.
    fn hold(app: &mut SmallWorld, engine: &mut Engine, keys: &[Key], dt: f32) {
        engine.input.begin_frame();
        for key in keys {
            engine.input.handle(&Event::KeyDown(*key));
        }
        engine.time.advance(dt);
        app.update(engine);
    }

    /// Let go, without spending a frame on it.
    fn release(engine: &mut Engine, keys: &[Key]) {
        for key in keys {
            engine.input.handle(&Event::KeyUp(*key));
        }
    }

    fn started(start: SceneId) -> (SmallWorld, Engine) {
        let mut engine = Engine::new(64, 48);
        let mut app = SmallWorld::new(start, DebugView::Shaded);
        app.start(&mut engine).expect("start never fails");
        (app, engine)
    }

    fn orbit_of(app: &mut SmallWorld, id: SceneId) -> Orbit {
        *app.scenes[id.index()]
            .as_mut()
            .expect("the scene has been visited")
            .orbit_mut()
    }

    #[test]
    fn scenes_cycle_in_both_directions() {
        for (index, id) in SceneId::ALL.into_iter().enumerate() {
            assert_eq!(id.index(), index);
            assert_eq!(SceneId::from_index(index), id);
        }
        assert_eq!(SceneId::Lit.next(), SceneId::Triangle);
        assert_eq!(SceneId::World.next(), SceneId::Lit, "forwards wraps around");
        assert_eq!(SceneId::Lit.previous(), SceneId::World, "and so does back");
        assert_eq!(SceneId::Triangle.previous(), SceneId::Lit);
    }

    #[test]
    fn the_screenshot_names_sort_into_scene_order() {
        let stems: Vec<&str> = SceneId::ALL.iter().map(|id| id.file_stem()).collect();
        let mut sorted = stems.clone();
        sorted.sort_unstable();
        assert_eq!(stems, sorted);
        assert_eq!(stems.first(), Some(&"01-lit-scene"));
        assert_eq!(stems.last(), Some(&"06-world"));
    }

    #[test]
    fn the_title_says_which_scene_and_how_fast() {
        assert_eq!(
            window_title(SceneId::Lit, 59.8),
            "SmallWorld — 1/6 Lit scene — 60 fps"
        );
        assert_eq!(
            window_title(SceneId::DepthAndBlending, 30.2),
            "SmallWorld — 5/6 Depth & blending — 30 fps"
        );
    }

    #[test]
    fn tab_walks_the_scenes_and_shift_tab_walks_back() {
        let (mut app, mut engine) = started(SceneId::Lit);
        for expected in [
            SceneId::Triangle,
            SceneId::Meshes,
            SceneId::Textures,
            SceneId::DepthAndBlending,
            SceneId::World,
            SceneId::Lit,
        ] {
            tap(&mut app, &mut engine, &[Key::Tab]);
            assert_eq!(app.current, expected);
        }
        for expected in [SceneId::World, SceneId::DepthAndBlending] {
            tap(&mut app, &mut engine, &[Key::LeftShift, Key::Tab]);
            assert_eq!(app.current, expected);
        }
    }

    #[test]
    fn a_scene_is_built_on_its_first_visit_and_not_before() {
        let (mut app, mut engine) = started(SceneId::Lit);
        assert!(app.scenes.iter().all(|s| s.is_none()));

        app.update(&mut engine);
        assert!(app.scenes[SceneId::Lit.index()].is_some());
        assert_eq!(
            app.scenes.iter().filter(|s| s.is_some()).count(),
            1,
            "only the scene on screen has been built"
        );

        tap(&mut app, &mut engine, &[Key::Tab]);
        assert!(app.scenes[SceneId::Triangle.index()].is_some());
        assert_eq!(app.scenes.iter().filter(|s| s.is_some()).count(), 2);
    }

    #[test]
    fn each_scene_keeps_its_own_camera_and_freezes_when_it_is_not_on_screen() {
        let (mut app, mut engine) = started(SceneId::Lit);
        for _ in 0..10 {
            hold(&mut app, &mut engine, &[Key::Right], 1.0 / 60.0);
        }
        release(&mut engine, &[Key::Right]);
        let lit = orbit_of(&mut app, SceneId::Lit);
        assert!(lit.yaw > 0.6, "holding Right swung the camera: {lit:?}");

        tap(&mut app, &mut engine, &[Key::Tab]);
        assert_eq!(app.current, SceneId::Triangle);
        for _ in 0..10 {
            hold(&mut app, &mut engine, &[Key::Right], 1.0 / 60.0);
        }
        release(&mut engine, &[Key::Right]);

        let triangle = orbit_of(&mut app, SceneId::Triangle);
        assert!(triangle.yaw > 0.0, "the scene on screen still steers");
        assert_eq!(
            orbit_of(&mut app, SceneId::Lit),
            lit,
            "the scene off screen did not move"
        );

        // Coming back finds the camera where it was, so nothing was rebuilt.
        tap(&mut app, &mut engine, &[Key::LeftShift, Key::Tab]);
        assert_eq!(app.current, SceneId::Lit);
        assert_eq!(orbit_of(&mut app, SceneId::Lit), lit);
    }

    #[test]
    fn the_debug_view_is_shared_and_survives_a_scene_switch() {
        let (mut app, mut engine) = started(SceneId::Lit);
        tap(&mut app, &mut engine, &[Key::Num3]);
        assert_eq!(engine.debug_view, DebugView::Depth);

        tap(&mut app, &mut engine, &[Key::Tab]);
        tap(&mut app, &mut engine, &[Key::Tab]);
        assert_eq!(app.current, SceneId::Meshes);
        assert_eq!(
            engine.debug_view,
            DebugView::Depth,
            "the debug view belongs to the application, not to a scene"
        );

        tap(&mut app, &mut engine, &[Key::Num1]);
        assert_eq!(engine.debug_view, DebugView::Shaded);
    }

    #[test]
    fn escape_stops_the_loop() {
        let (mut app, mut engine) = started(SceneId::Lit);
        assert!(engine.is_running());
        engine.input.begin_frame();
        engine.input.handle(&Event::KeyDown(Key::Escape));
        app.update(&mut engine);
        assert!(!engine.is_running());
    }

    #[test]
    fn the_environment_variables_pick_a_scene_and_a_debug_view() {
        assert_eq!(scene_from_env(None), SceneId::Lit);
        assert_eq!(scene_from_env(Some("1")), SceneId::Lit);
        assert_eq!(scene_from_env(Some(" 6 ")), SceneId::World);
        assert_eq!(scene_from_env(Some("0")), SceneId::Lit, "1-based, not 0");
        assert_eq!(scene_from_env(Some("7")), SceneId::Lit);
        assert_eq!(scene_from_env(Some("nonsense")), SceneId::Lit);

        assert_eq!(debug_view_from_env(None), DebugView::Shaded);
        assert_eq!(debug_view_from_env(Some("wireframe")), DebugView::Wireframe);
        assert_eq!(debug_view_from_env(Some("depth")), DebugView::Depth);
        assert_eq!(debug_view_from_env(Some("overdraw")), DebugView::Overdraw);
        assert_eq!(debug_view_from_env(Some("nonsense")), DebugView::Shaded);
    }

    #[test]
    fn the_debug_view_from_the_environment_reaches_the_engine() {
        let mut engine = Engine::new(16, 16);
        let mut app = SmallWorld::new(SceneId::Triangle, DebugView::Overdraw);
        app.start(&mut engine).unwrap();
        assert_eq!(engine.debug_view, DebugView::Overdraw);
    }

    #[test]
    fn space_spawns_ten_entities_and_backspace_takes_the_last_ten_back() {
        let mut engine = Engine::new(32, 32);
        let mut scene = WorldScene::new(&mut engine);
        assert_eq!(
            engine.world.entity_count(),
            BATCH,
            "the ring starts with 10"
        );

        scene.action(&mut engine);
        assert_eq!(engine.world.entity_count(), 2 * BATCH);
        assert_eq!(scene.spawned.len(), 2 * BATCH);

        scene.undo(&mut engine);
        assert_eq!(engine.world.entity_count(), BATCH);
        assert!(
            scene.spawned.iter().all(|e| engine.world.is_alive(*e)),
            "the ten that are left are the ten that were spawned first"
        );

        // Every entity on the ring is placed and drawable.
        assert_eq!(engine.world.iter::<Transform>().count(), BATCH);
        assert!(engine
            .world
            .iter::<Transform>()
            .all(|(_, t)| t.position.length() > 1.0));
    }

    #[test]
    fn a_reused_slot_comes_back_on_a_later_generation() {
        let mut engine = Engine::new(32, 32);
        let mut scene = WorldScene::new(&mut engine);
        scene.action(&mut engine);
        let before = scene.spawned[BATCH..].to_vec();

        scene.undo(&mut engine);
        assert!(before.iter().all(|e| !engine.world.is_alive(*e)));
        scene.action(&mut engine);

        let after = &scene.spawned[BATCH..];
        assert_eq!(engine.world.entity_count(), 2 * BATCH);

        let mut old_slots: Vec<u32> = before.iter().map(|e| e.index()).collect();
        let mut new_slots: Vec<u32> = after.iter().map(|e| e.index()).collect();
        old_slots.sort_unstable();
        new_slots.sort_unstable();
        assert_eq!(old_slots, new_slots, "the same slots came back");
        assert!(
            after.iter().all(|e| e.generation() > 0),
            "but on a later generation"
        );
        assert!(
            before
                .iter()
                .all(|e| engine.world.get::<Ringed>(*e).is_none()),
            "and the old handles reach nothing"
        );
    }

    #[test]
    fn the_ring_turns_on_the_fixed_step_and_nowhere_else() {
        let mut engine = Engine::new(32, 32);
        let mut scene = WorldScene::new(&mut engine);
        let entity = scene.spawned[0];
        let start = engine.world.get::<Transform>(entity).unwrap().position;

        scene.update(&mut engine, 1.0 / 60.0);
        assert_eq!(
            engine.world.get::<Transform>(entity).unwrap().position,
            start,
            "a variable-delta frame does not move the ring"
        );

        for _ in 0..30 {
            scene.fixed_update(&mut engine);
        }
        let moved = engine.world.get::<Transform>(entity).unwrap().position;
        assert!((moved - start).length() > 0.1, "{start:?} -> {moved:?}");
    }

    #[test]
    fn the_generation_palette_changes_colour_when_a_slot_is_reused() {
        assert_ne!(generation_tint(0), generation_tint(1));
        assert_eq!(
            generation_tint(0),
            generation_tint(6),
            "six colours, then it repeats"
        );
    }

    #[test]
    fn the_texture_on_screen_survived_the_png_round_trip_unchanged() {
        let source = TexturesScene::source_texture();
        let decoded = png_round_trip(&source);
        assert_eq!(
            (decoded.width(), decoded.height()),
            (source.width(), source.height())
        );
        for y in 0..source.height() {
            for x in 0..source.width() {
                let u = (x as f32 + 0.5) / source.width() as f32;
                let v = (y as f32 + 0.5) / source.height() as f32;
                let (a, b) = (source.sample(u, v), decoded.sample(u, v));
                let delta = (a.r - b.r)
                    .abs()
                    .max((a.g - b.g).abs())
                    .max((a.b - b.b).abs());
                assert!(delta <= 1.0 / 255.0, "texel {x},{y}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn the_textures_scene_shows_every_filter_against_every_wrap() {
        let scene = TexturesScene::new();
        assert_eq!(scene.variants.len(), 6);
        let combinations: Vec<(Filter, Wrap)> =
            scene.variants.iter().map(|t| (t.filter, t.wrap)).collect();
        for filter in TexturesScene::FILTERS {
            for wrap in TexturesScene::WRAPS {
                assert!(
                    combinations.contains(&(filter, wrap)),
                    "{filter:?} x {wrap:?} is missing"
                );
            }
        }
        // The cells do not overlap: six quads, six places.
        let mut centers: Vec<(i32, i32)> = (0..6)
            .map(|i| {
                let p = TexturesScene::cell_model(i)
                    .transform_point(Vec3::ZERO)
                    .xyz();
                ((p.x * 100.0) as i32, (p.y * 100.0) as i32)
            })
            .collect();
        centers.sort_unstable();
        centers.dedup();
        assert_eq!(centers.len(), 6);
    }

    #[test]
    fn space_steps_the_depth_scene_through_every_setting() {
        let mut engine = Engine::new(16, 16);
        let mut scene = DepthScene::new();
        assert_eq!(scene.mode().blend, Blend::Replace);
        assert!(scene.mode().depth_write);

        scene.action(&mut engine);
        assert_eq!(scene.mode().blend, Blend::Alpha);
        assert!(scene.mode().depth_test && !scene.mode().depth_write);

        scene.action(&mut engine);
        assert!(!scene.mode().depth_test);

        scene.action(&mut engine);
        assert_eq!(scene.mode(), DEPTH_MODES[0], "and around again");
    }

    #[test]
    fn the_meshes_scene_puts_the_teapot_next_to_the_primitives() {
        let scene = MeshesScene::new();
        assert!(scene.teapot.triangle_count() > 1000, "that is the teapot");
        let shelf = scene.shelf();
        assert_eq!(shelf.len(), 4);
        let mut x: Vec<i32> = shelf
            .iter()
            .map(|(_, model, _)| (model.transform_point(Vec3::ZERO).x * 10.0) as i32)
            .collect();
        let sorted = {
            let mut copy = x.clone();
            copy.sort_unstable();
            copy
        };
        x.dedup();
        assert_eq!(x.len(), 4, "the four meshes stand apart");
        assert_eq!(x, sorted, "left to right: cube, plane, sphere, teapot");
        assert!(
            shelf.iter().all(|(mesh, _, _)| mesh.triangle_count() > 0),
            "every shelf slot has geometry"
        );
    }

    /// Run one scene with no window, exactly as `RUNITY_HEADLESS=1` does.
    fn headless_scene(id: SceneId, width: u32, height: u32, frames: u64) -> Engine {
        headless::run(
            SmallWorld::new(id, DebugView::Shaded),
            width,
            height,
            frames,
            HEADLESS_DELTA,
        )
        .expect("a headless run needs no display")
    }

    #[test]
    fn every_scene_draws_something_and_draws_it_the_same_way_twice() {
        for id in SceneId::ALL {
            let first = headless_scene(id, 96, 72, 8);
            let second = headless_scene(id, 96, 72, 8);
            assert!(
                first.frame_stats().triangles_rasterized > 0,
                "{} drew nothing",
                id.name()
            );
            assert_eq!(
                first.framebuffer.pixels(),
                second.framebuffer.pixels(),
                "{} is not deterministic",
                id.name()
            );
            assert_eq!(
                first.title(),
                window_title(id, first.time.average_fps()),
                "the title names the scene that is on screen"
            );
        }
    }

    #[test]
    fn the_triangle_scene_still_looks_the_same() {
        // The reference sits with the other goldens; regenerate it deliberately
        // with `RUNITY_UPDATE_GOLDEN=1 cargo test`.
        let frame = headless_scene(SceneId::Triangle, 192, 108, 30).framebuffer;
        golden::assert_matches(
            "tests/golden/smallworld-triangle.png",
            &frame,
            // Looser than the renderer's own golden: a rotating edge lands a
            // pixel either side of itself when `sin` differs in its last bit.
            Tolerance::new(8, 0.04),
        );
    }

    #[test]
    fn the_textures_scene_still_looks_the_same() {
        let frame = headless_scene(SceneId::Textures, 192, 108, 2).framebuffer;
        golden::assert_matches(
            "tests/golden/smallworld-textures.png",
            &frame,
            Tolerance::new(8, 0.04),
        );
    }
}
