use crate::input::Input;
use crate::time::Time;
use crate::transform::Camera;
use crate::world::World;
use runity_math::{Mat4, Vec3};
use runity_platform::{open_window, Event, Window, WindowConfig};
use runity_render::{
    debug, BasicShader, Color, DirectionalLight, DrawStats, Framebuffer, Mesh, PolygonMode,
    Rasterizer, Shader, TextStyle,
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
}

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
    /// Which debug view the main loop presents after `render`.
    pub debug_view: DebugView,
    /// Write count that counts as "fully red" in [`DebugView::Overdraw`].
    pub overdraw_saturation: u32,
    running: bool,
    frame_stats: DrawStats,
    title: String,
    /// A title the loop has not handed to the window yet.
    pending_title: Option<String>,
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
            debug_view: DebugView::Shaded,
            overdraw_saturation: 4,
            running: true,
            frame_stats: DrawStats::default(),
            title: String::new(),
            pending_title: None,
        }
    }

    /// Ask the main loop to stop after this frame.
    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Retitle the window. The main loop applies it before presenting.
    ///
    /// Setting the title it already has costs nothing, so a game can call this
    /// every frame with a string it rebuilds every frame — only a real change
    /// reaches the window.
    pub fn set_title(&mut self, title: impl Into<String>) {
        let title = title.into();
        if title != self.title {
            self.title = title.clone();
            self.pending_title = Some(title);
        }
    }

    /// The title the window currently carries.
    pub fn title(&self) -> &str {
        &self.title
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

    /// Draw screen-space text with the top-left corner of its first line at
    /// `(x, y)` in framebuffer pixels.
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, style: &TextStyle) {
        style.draw(&mut self.framebuffer, text, x, y);
    }

    /// Draw a label anchored to a point in world space, at a constant size —
    /// no perspective scaling, the way a debug caption should read the same
    /// whether it is tagging something near the camera or far from it.
    ///
    /// Projects `world_pos` with the same [`debug::project`] the debug overlays
    /// use, so a label lines up with `draw_axes` or a wireframe drawn the same
    /// frame. Draws nothing and returns `None` for a point at or behind the
    /// near plane, exactly where `project` itself gives up.
    pub fn draw_text_at(
        &mut self,
        world_pos: Vec3,
        text: &str,
        style: &TextStyle,
    ) -> Option<(i32, i32)> {
        let clip = self.view_projection().transform_point(world_pos);
        let (width, height) = (
            self.framebuffer.width() as f32,
            self.framebuffer.height() as f32,
        );
        let (x, y) = debug::project(clip, width, height)?;
        let (x, y) = (x.round() as i32, y.round() as i32);
        style.draw(&mut self.framebuffer, text, x, y);
        Some((x, y))
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
        // The window already wears the configured title, so a game that asks
        // for that same title does not make the loop set it again.
        engine.title = self.window_config.title.clone();
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
            engine.rasterizer.polygon_mode = match engine.debug_view {
                DebugView::Wireframe => PolygonMode::Line,
                _ => PolygonMode::Fill,
            };
            let counting = engine.debug_view == DebugView::Overdraw;
            if counting != engine.framebuffer.overdraw().is_some() {
                engine.framebuffer.track_overdraw(counting);
            }

            let clear = engine.clear_color;
            engine.framebuffer.clear(clear);
            game.render(&mut engine);

            // Debug views replace the frame after the game has drawn it, so a
            // game needs no awareness of them.
            match engine.debug_view {
                DebugView::Shaded | DebugView::Wireframe => {}
                DebugView::Depth => engine.framebuffer = debug::depth_view(&engine.framebuffer),
                DebugView::Overdraw => {
                    let saturation = engine.overdraw_saturation;
                    engine.framebuffer = debug::overdraw_view(&engine.framebuffer, saturation);
                }
            }

            if let Some(title) = engine.pending_title.take() {
                window.set_title(&title)?;
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use runity_platform::HeadlessWindow;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct TitleChanger {
        title_set: bool,
    }

    impl Game for TitleChanger {
        fn update(&mut self, engine: &mut Engine) {
            if !self.title_set {
                engine.set_title("new title");
                self.title_set = true;
            }
        }
    }

    struct TrackingWindow {
        inner: HeadlessWindow,
        titles_set: Rc<RefCell<Vec<String>>>,
    }

    impl TrackingWindow {
        fn new(config: &WindowConfig, titles: Rc<RefCell<Vec<String>>>) -> Self {
            Self {
                inner: HeadlessWindow::new(config),
                titles_set: titles,
            }
        }
    }

    impl Window for TrackingWindow {
        fn size(&self) -> (u32, u32) {
            self.inner.size()
        }

        fn poll_events(&mut self) -> io::Result<Vec<Event>> {
            self.inner.poll_events()
        }

        fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()> {
            self.inner.present(pixels, width, height)
        }

        fn set_title(&mut self, title: &str) -> io::Result<()> {
            self.titles_set.borrow_mut().push(title.to_string());
            self.inner.set_title(title)
        }

        fn backend_name(&self) -> &'static str {
            self.inner.backend_name()
        }
    }

    #[test]
    fn set_title_calls_window_set_title_when_title_changes() -> io::Result<()> {
        let config = WindowConfig::new("initial title", 100, 100);
        let titles_set = Rc::new(RefCell::new(Vec::new()));
        let window = Box::new(TrackingWindow::new(&config, Rc::clone(&titles_set)));
        let app = App::new(config).with_max_frames(2);
        app.run_with_window(window, TitleChanger { title_set: false })?;
        assert_eq!(titles_set.borrow().as_slice(), &["new title".to_string()]);
        Ok(())
    }

    #[test]
    fn engine_set_title_stores_the_pending_title() {
        let mut engine = Engine::new(100, 100);
        assert!(engine.pending_title.is_none());
        engine.set_title("test title");
        assert_eq!(engine.pending_title, Some("test title".to_string()));
    }

    #[test]
    fn a_title_reaches_the_loop_once_per_change() {
        let mut engine = Engine::new(4, 4);
        engine.set_title("first");
        assert_eq!(engine.title(), "first");
        assert_eq!(engine.pending_title.take(), Some("first".to_string()));

        // The loop has applied it; asking for the same text again is a no-op,
        // which is what lets a game rebuild its title string every frame.
        engine.set_title("first");
        assert_eq!(engine.pending_title, None);

        engine.set_title("second");
        assert_eq!(engine.pending_title.as_deref(), Some("second"));
    }

    #[test]
    fn draw_text_lands_at_the_top_left_anchor() {
        let font = runity_render::Font::embedded();
        let style = TextStyle::new(&font).size(24.0).color(Color::WHITE);
        let mut engine = Engine::new(64, 64);
        engine.framebuffer.clear(Color::BLACK);
        engine.draw_text("W", 4, 4, &style);
        let lit = engine
            .framebuffer
            .pixels()
            .iter()
            .filter(|p| **p != Color::BLACK.to_argb8())
            .count();
        assert!(lit > 0, "draw_text must have put something on the frame");
    }

    #[test]
    fn draw_text_at_projects_through_the_camera_and_draws() {
        let font = runity_render::Font::embedded();
        let style = TextStyle::new(&font).size(16.0).color(Color::WHITE);
        let mut engine = Engine::new(200, 150);
        engine.framebuffer.clear(Color::BLACK);

        // In front of the default camera, which sits at (0, 1.5, 4) looking
        // at the origin.
        let drawn = engine.draw_text_at(Vec3::ZERO, "here", &style);
        assert!(
            drawn.is_some(),
            "a point in front of the camera must project"
        );
        let lit = engine
            .framebuffer
            .pixels()
            .iter()
            .filter(|p| **p != Color::BLACK.to_argb8())
            .count();
        assert!(lit > 0);
    }

    #[test]
    fn draw_text_at_draws_nothing_for_a_point_behind_the_near_plane() {
        let font = runity_render::Font::embedded();
        let style = TextStyle::new(&font).size(16.0).color(Color::WHITE);
        let mut engine = Engine::new(64, 64);
        engine.framebuffer.clear(Color::BLACK);

        // Further along the camera-to-target axis than the camera itself,
        // i.e. behind it.
        let behind = engine.camera.position + (engine.camera.position - engine.camera.target) * 4.0;
        let drawn = engine.draw_text_at(behind, "behind", &style);
        assert_eq!(drawn, None);
        assert!(engine
            .framebuffer
            .pixels()
            .iter()
            .all(|p| *p == Color::BLACK.to_argb8()));
    }
}
