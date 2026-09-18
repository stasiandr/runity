use crate::input::Input;
use crate::time::Time;
use crate::transform::Camera;
use crate::world::World;
use runity_math::Mat4;
use runity_platform::{open_window, Event, Window, WindowConfig};
use runity_render::{
    BasicShader, Color, DirectionalLight, DrawStats, Framebuffer, Mesh, Rasterizer, Shader,
};
use std::io;

/// Everything a game is handed each frame: the world, timing, input, the
/// camera, and the framebuffer it draws into.
pub struct Engine {
    pub world: World,
    pub time: Time,
    pub input: Input,
    pub camera: Camera,
    pub framebuffer: Framebuffer,
    pub rasterizer: Rasterizer,
    pub light: DirectionalLight,
    pub ambient: Color,
    pub clear_color: Color,
    running: bool,
    frame_stats: DrawStats,
}

impl Engine {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            world: World::new(),
            time: Time::new(),
            input: Input::new(),
            camera: Camera::default(),
            framebuffer: Framebuffer::new(width, height),
            rasterizer: Rasterizer::new(),
            light: DirectionalLight::default(),
            ambient: Color::rgb(0.12, 0.13, 0.16),
            clear_color: Color::rgb(0.05, 0.06, 0.09),
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

    /// A [`BasicShader`] already wired to this frame's camera and light.
    pub fn lit_shader(&self, model: Mat4) -> BasicShader<'static> {
        BasicShader::new(model, self.view_projection())
            .with_light(self.light)
            .with_camera_position(self.camera.position)
    }

    /// Draw a mesh, accumulating this frame's statistics.
    pub fn draw<S: Shader>(&mut self, mesh: &Mesh, shader: &S) -> DrawStats {
        let stats = self
            .rasterizer
            .draw_mesh(&mut self.framebuffer, mesh, shader);
        self.frame_stats.triangles_in += stats.triangles_in;
        self.frame_stats.triangles_rasterized += stats.triangles_rasterized;
        self.frame_stats.fragments_shaded += stats.fragments_shaded;
        self.frame_stats.fragments_written += stats.fragments_written;
        stats
    }

    /// What the previous frame cost.
    pub fn frame_stats(&self) -> DrawStats {
        self.frame_stats
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
    /// Called zero or more times per frame with `Time::fixed_delta`.
    fn fixed_update(&mut self, _engine: &mut Engine) {}
    /// Called once per frame, after the framebuffer has been cleared.
    fn render(&mut self, _engine: &mut Engine) {}
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
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_frames: None,
            frame_delta: None,
            target_fps: Some(60.0),
            max_fixed_steps: 8,
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
        let mut engine = Engine::new(width as usize, height as usize);
        game.start(&mut engine)?;

        while engine.is_running() {
            let frame_started = std::time::Instant::now();

            // --- input ----------------------------------------------------
            engine.input.begin_frame();
            for event in window.poll_events()? {
                match event {
                    Event::CloseRequested => engine.quit(),
                    Event::Resized { width, height } if width > 0 && height > 0 => {
                        engine.framebuffer.resize(width as usize, height as usize);
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
                game.fixed_update(&mut engine);
                steps += 1;
            }

            // --- rendering ------------------------------------------------
            engine.frame_stats = DrawStats::default();
            let clear = engine.clear_color;
            engine.framebuffer.clear(clear);
            game.render(&mut engine);

            let (w, h) = (
                engine.framebuffer.width() as u32,
                engine.framebuffer.height() as u32,
            );
            window.present(engine.framebuffer.pixels(), w, h)?;

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
