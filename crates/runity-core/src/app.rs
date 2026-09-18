use crate::clock::WorldClock;
use crate::input::Input;
use crate::time::Time;
use crate::transform::Camera;
use crate::world::World;
use runity_math::Mat4;
use runity_physics::PhysicsWorld;
use runity_platform::{open_window, Event, Window, WindowConfig};
use runity_render::{
    debug, CameraView, Color, DrawStats, Framebuffer, Material, Mesh, PolygonMode, Renderer,
    Shader, ToneMap,
};
use std::io;

/// What the frame should show. One switch, applied by the main loop, so a
/// running game can be inspected without touching its rendering code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DebugView {
    /// The scene as the game drew it.
    #[default]
    Shaded,
    /// The same draw calls, but only triangle edges.
    Wireframe,
    /// The depth buffer, normalized over what the frame contains.
    Depth,
    /// How many times each pixel was written.
    Overdraw,
    /// Surface color straight out of the geometry pass, before any lighting.
    Albedo,
    /// World-space normals, `n * 0.5 + 0.5`.
    Normals,
    /// Roughness in green, metallic in blue.
    Material,
    /// Screen-space ambient occlusion on its own.
    Occlusion,
}

/// Everything a game is handed each frame: the world, timing, input, the
/// camera, and the framebuffer it draws into.
pub struct Engine {
    pub world: World,
    pub time: Time,
    /// Simulation time: whole ticks, speed, pause and the calendar.
    ///
    /// Separate from [`Engine::time`] on purpose. `time` is how fast this
    /// machine is drawing; `clock` is how fast the world is living, and only
    /// the second one may decide what happens in it.
    pub clock: WorldClock,
    /// Rigid bodies, stepped once per fixed update unless
    /// [`Engine::auto_step_physics`] is off.
    pub physics: PhysicsWorld,
    /// Whether the main loop steps [`Engine::physics`] itself.
    pub auto_step_physics: bool,
    pub input: Input,
    pub camera: Camera,
    pub framebuffer: Framebuffer,
    /// The deferred renderer: lights, environment and passes.
    pub renderer: Renderer,
    /// Background where the sky is switched off.
    pub clear_color: Color,
    /// Which debug view the main loop presents after `render`.
    pub debug_view: DebugView,
    /// Write count that counts as "fully red" in [`DebugView::Overdraw`].
    pub overdraw_saturation: u32,
    /// Curve used to turn the frame's linear HDR light into displayable pixels.
    pub tone_map: ToneMap,
    /// Linear multiplier applied before the tone curve — the camera's exposure.
    pub exposure: f32,
    running: bool,
    frame_stats: DrawStats,
}

impl Engine {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            world: World::new(),
            time: Time::new(),
            clock: WorldClock::default(),
            physics: PhysicsWorld::new(),
            auto_step_physics: true,
            input: Input::new(),
            camera: Camera::default(),
            framebuffer: Framebuffer::new(width, height),
            renderer: Renderer::default(),
            clear_color: Color::rgb(0.05, 0.06, 0.09),
            debug_view: DebugView::Shaded,
            overdraw_saturation: 4,
            tone_map: ToneMap::default(),
            exposure: 1.0,
            running: true,
            frame_stats: DrawStats::default(),
        }
    }

    /// Ask the main loop to stop after this frame.
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.framebuffer.aspect_ratio()
    }

    pub fn view_projection(&self) -> Mat4 {
        self.camera.view_projection(self.aspect_ratio())
    }

    /// The camera as the renderer needs it: matrices, position and range.
    pub fn camera_view(&self) -> CameraView {
        CameraView::new(
            self.camera.view(),
            self.camera.projection(self.aspect_ratio()),
            self.camera.position,
            self.camera.near,
            self.camera.far,
        )
    }

    /// Start the frame: clear, attach the G-buffer, tell the renderer where the
    /// camera is. The main loop does this before `Game::render`.
    pub fn begin_frame(&mut self) {
        let camera = self.camera_view();
        self.frame_stats = DrawStats::default();
        self.renderer
            .begin_frame(&mut self.framebuffer, camera, self.clear_color);
    }

    /// Light everything the geometry pass wrote, then fill in the sky. The main
    /// loop does this after `Game::render`, before `Game::overlay`.
    pub fn shade(&mut self) {
        self.renderer.shade(&mut self.framebuffer);
    }

    /// Draw a mesh into the G-buffer with a material — the usual way to draw.
    pub fn draw_pbr(&mut self, mesh: &Mesh, model: Mat4, material: &Material<'_>) -> DrawStats {
        let stats = self
            .renderer
            .draw(&mut self.framebuffer, mesh, model, material);
        self.accumulate(stats);
        stats
    }

    /// Draw a mesh with a shader you wrote yourself.
    pub fn draw<S: Shader>(&mut self, mesh: &Mesh, shader: &S) -> DrawStats {
        let stats = self
            .renderer
            .rasterizer
            .draw_mesh(&mut self.framebuffer, mesh, shader);
        self.accumulate(stats);
        stats
    }

    fn accumulate(&mut self, stats: DrawStats) {
        self.frame_stats.triangles_in += stats.triangles_in;
        self.frame_stats.triangles_rasterized += stats.triangles_rasterized;
        self.frame_stats.fragments_shaded += stats.fragments_shaded;
        self.frame_stats.fragments_written += stats.fragments_written;
    }

    /// What the previous frame cost.
    pub fn frame_stats(&self) -> DrawStats {
        self.frame_stats
    }

    /// Overlay a mesh's triangle edges, ignoring depth.
    ///
    /// For a wireframe that respects depth and runs the real shader, set
    /// [`DebugView::Wireframe`] instead.
    pub fn draw_wireframe(&mut self, mesh: &Mesh, model: Mat4, color: Color) {
        let mvp = self.view_projection() * model;
        debug::draw_wireframe(&mut self.framebuffer, mesh, mvp, color);
    }

    /// Overlay each vertex normal as a short segment.
    pub fn draw_normals(&mut self, mesh: &Mesh, model: Mat4, length: f32, color: Color) {
        let view_projection = self.view_projection();
        debug::draw_normals(
            &mut self.framebuffer,
            mesh,
            model,
            view_projection,
            length,
            color,
        );
    }

    /// Overlay the world axes at the origin.
    pub fn draw_axes(&mut self, length: f32) {
        let view_projection = self.view_projection();
        debug::draw_axes(&mut self.framebuffer, view_projection, length);
    }
}

/// The game side of the loop. Every method has a default, so a game implements
/// only what it needs.
pub trait Game {
    /// Called once before the first frame.
    fn start(&mut self, _engine: &mut Engine) -> io::Result<()> {
        Ok(())
    }
    /// Called for every platform event, before `update`.
    fn on_event(&mut self, _engine: &mut Engine, _event: &Event) {}
    /// Called once per frame with a variable delta.
    fn update(&mut self, _engine: &mut Engine) {}
    /// Called zero or more times per frame with `Time::fixed_delta`, after
    /// the physics step.
    fn fixed_update(&mut self, _engine: &mut Engine) {}

    /// Called once per world tick — the simulation's own clock, which runs at
    /// its own rate, can be paused and sped up, and does not care how fast
    /// the frame is.
    ///
    /// This is where a world that keeps developing while nobody watches does
    /// its developing. Render from the state it leaves behind, interpolating
    /// with [`WorldClock::interpolation`], rather than moving anything here
    /// per frame.
    fn world_tick(&mut self, _engine: &mut Engine) {}
    /// Called once per frame to draw the scene's geometry. Lighting happens
    /// after it returns.
    fn render(&mut self, _engine: &mut Engine) {}

    /// Called after the scene has been lit — for anything that should sit on
    /// top of the finished image: gizmos, wireframes, UI.
    fn overlay(&mut self, _engine: &mut Engine) {}
}

/// How the main loop should run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Stop after this many frames. `None` runs until the window closes.
    pub max_frames: Option<u64>,
    /// Use a fixed delta instead of the wall clock — deterministic runs for
    /// tests, recordings and CI.
    pub frame_delta: Option<f32>,
    /// Sleep to keep the frame rate near this value.
    pub target_fps: Option<f32>,
    /// Upper bound on fixed-update catch-up steps, so a slow frame cannot
    /// spiral into running fixed steps forever.
    pub max_fixed_steps: u32,
    /// Render at this multiple of the window size and average back down.
    ///
    /// The cheapest anti-aliasing there is, and on a software rasterizer the
    /// only one worth having: 2 costs four times the fragments and removes
    /// every jagged edge, including the ones inside reflections.
    pub supersample: u32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_frames: None,
            frame_delta: None,
            target_fps: Some(60.0),
            max_fixed_steps: 8,
            supersample: 1,
        }
    }
}

/// Builds the window and drives the loop.
pub struct App {
    pub window_config: WindowConfig,
    pub options: RunOptions,
}

impl App {
    pub fn new(window_config: WindowConfig) -> Self {
        Self {
            window_config,
            options: RunOptions::default(),
        }
    }

    pub fn with_max_frames(mut self, frames: u64) -> Self {
        self.options.max_frames = Some(frames);
        self
    }

    pub fn with_frame_delta(mut self, delta: f32) -> Self {
        self.options.frame_delta = Some(delta);
        self
    }

    pub fn with_target_fps(mut self, fps: Option<f32>) -> Self {
        self.options.target_fps = fps;
        self
    }

    /// Render at `factor` times the window resolution and average back down.
    pub fn with_supersampling(mut self, factor: u32) -> Self {
        self.options.supersample = factor.max(1);
        self
    }

    /// Open a window and run until the game quits or the window closes.
    ///
    /// Returns the final [`Engine`], so a caller can inspect — or save — the
    /// last frame.
    pub fn run(self, game: impl Game) -> io::Result<Engine> {
        let window = open_window(&self.window_config)?;
        self.run_with_window(window, game)
    }

    /// Same, but against a window you supply. Pass a
    /// [`runity_platform::HeadlessWindow`] to run without a display.
    pub fn run_with_window(
        self,
        mut window: Box<dyn Window>,
        mut game: impl Game,
    ) -> io::Result<Engine> {
        let (width, height) = window.size();
        let supersample = self.options.supersample.max(1) as usize;
        let mut engine = Engine::new(width as usize * supersample, height as usize * supersample);
        // Reused every frame: tone mapping is the only place the renderer
        // produces 8-bit pixels, and it should not allocate to do it.
        let mut resolved: Vec<u32> = Vec::with_capacity(engine.framebuffer.len());
        game.start(&mut engine)?;

        while engine.is_running() {
            let frame_started = std::time::Instant::now();

            // --- input ----------------------------------------------------
            engine.input.begin_frame();
            for event in window.poll_events()? {
                match event {
                    Event::CloseRequested => engine.quit(),
                    Event::Resized { width, height } if width > 0 && height > 0 => {
                        engine
                            .framebuffer
                            .resize(width as usize * supersample, height as usize * supersample);
                    }
                    _ => {}
                }
                engine.input.handle(&event);
                game.on_event(&mut engine, &event);
            }

            // --- simulation ----------------------------------------------
            match self.options.frame_delta {
                Some(delta) => engine.time.advance(delta),
                None => engine.time.tick(),
            }
            game.update(&mut engine);
            let mut steps = 0;
            while steps < self.options.max_fixed_steps && engine.time.next_fixed_step() {
                if engine.auto_step_physics {
                    let dt = engine.time.fixed_delta;
                    engine.physics.step(dt);
                }
                game.fixed_update(&mut engine);
                steps += 1;
            }

            // The world's own clock, which the frame rate does not govern.
            engine.clock.advance(engine.time.delta());
            while engine.clock.next_tick().is_some() {
                engine.world.advance_tick();
                game.world_tick(&mut engine);
            }

            // --- rendering ------------------------------------------------
            engine.renderer.rasterizer.polygon_mode = match engine.debug_view {
                DebugView::Wireframe => PolygonMode::Line,
                _ => PolygonMode::Fill,
            };
            let counting = engine.debug_view == DebugView::Overdraw;
            if counting != engine.framebuffer.overdraw().is_some() {
                engine.framebuffer.track_overdraw(counting);
            }

            engine.begin_frame();
            game.render(&mut engine); // geometry
            engine.shade(); // lighting, then the sky behind it
            game.overlay(&mut engine); // gizmos on top of the finished image

            // Debug views replace the frame's contents after the game has drawn
            // it, so a game needs no awareness of them. The buffer itself is
            // kept — it owns the depth, the attachments and the size.
            let view = match engine.debug_view {
                DebugView::Shaded | DebugView::Wireframe => None,
                DebugView::Depth => Some(debug::depth_view(&engine.framebuffer)),
                DebugView::Overdraw => Some(debug::overdraw_view(
                    &engine.framebuffer,
                    engine.overdraw_saturation,
                )),
                DebugView::Albedo => Some(debug::albedo_view(&engine.framebuffer)),
                DebugView::Normals => Some(debug::normal_view(&engine.framebuffer)),
                DebugView::Material => Some(debug::material_view(&engine.framebuffer)),
                DebugView::Occlusion => Some(runity_render::ssao::occlusion_view(
                    engine.renderer.occlusion(),
                )),
            };
            match view {
                Some(view) => {
                    engine
                        .framebuffer
                        .colors_mut()
                        .copy_from_slice(view.colors());
                    // A debug view is data, not light.
                    engine.framebuffer.tone_map = ToneMap::Raw;
                    engine.framebuffer.exposure = 1.0;
                }
                None => {
                    engine.framebuffer.tone_map = engine.tone_map;
                    engine.framebuffer.exposure = engine.exposure;
                }
            }

            // Supersampled frames are averaged down before tone mapping, which
            // is what makes the edges clean rather than merely grey.
            let presented = if supersample > 1 {
                engine.framebuffer.downsample(supersample)
            } else {
                engine.framebuffer.clone()
            };
            let (w, h) = (presented.width() as u32, presented.height() as u32);
            presented.resolve_into(&mut resolved);
            window.present(&resolved, w, h)?;

            if let Some(max) = self.options.max_frames {
                if engine.time.frame() >= max {
                    engine.quit();
                }
            }

            if let Some(fps) = self.options.target_fps.filter(|f| *f > 0.0) {
                let budget = std::time::Duration::from_secs_f32(1.0 / fps);
                if let Some(remaining) = budget.checked_sub(frame_started.elapsed()) {
                    std::thread::sleep(remaining);
                }
            }
        }
        Ok(engine)
    }
}
