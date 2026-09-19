use crate::input::Input;
use crate::time::Time;
use crate::transform::Camera;
use crate::world::World;
use runity_gpu::{Gpu, GpuError, GpuShader, Target};
use runity_math::Mat4;
use runity_platform::{open_window, Event, NativeSurface, Window, WindowConfig};
use runity_render::{
    debug, BasicShader, Color, DirectionalLight, DrawStats, Framebuffer, Mesh, PolygonMode,
    Rasterizer, Shader,
};
use std::fmt;
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

/// Which renderer the main loop drives.
///
/// The software rasterizer is the reference — it is where the pipeline is
/// written down, it needs no display, and it is what the golden images are
/// made of. The GPU is how a game actually runs. They draw the same frames,
/// and `runity-gpu`'s differential tests are what says so.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    #[default]
    Cpu,
    Gpu,
}

impl Renderer {
    /// Decide which renderer to build.
    ///
    /// The default is the useful one rather than the uniform one: a window
    /// means a game, so it gets the GPU; no window means a test, a recording
    /// or a build machine, so it gets the rasterizer. `RUNITY_RENDERER`
    /// outranks a program's own choice, because it is the escape hatch — it
    /// has to work on a binary you cannot rebuild.
    pub fn resolve(env: Option<&str>, chosen: Option<Renderer>, has_surface: bool) -> Renderer {
        match env.map(str::trim) {
            Some("cpu") => return Renderer::Cpu,
            Some("gpu") => return Renderer::Gpu,
            _ => {}
        }
        chosen.unwrap_or(if has_surface {
            Renderer::Gpu
        } else {
            Renderer::Cpu
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Renderer::Cpu => "cpu",
            Renderer::Gpu => "gpu",
        }
    }
}

/// A debug view this renderer cannot show.
///
/// Two of the four are CPU-only by construction: [`DebugView::Wireframe`] is
/// the rasterizer running with a different fill rule, and
/// [`DebugView::Overdraw`] is a counter it keeps per written fragment. A GPU
/// has neither, and faking them would mean a debug view that lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedView {
    pub view: DebugView,
    pub renderer: Renderer,
}

impl fmt::Display for UnsupportedView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.view {
            DebugView::Wireframe => {
                "wireframe is a fill rule inside the rasterizer, and the GPU has its own"
            }
            DebugView::Overdraw => {
                "overdraw counts writes per pixel, which the hardware does not report"
            }
            _ => "this view is not available here",
        };
        write!(
            f,
            "{:?} needs the CPU renderer: {reason} — press 1, or restart with \
             RUNITY_RENDERER=cpu",
            self.view
        )
    }
}

impl std::error::Error for UnsupportedView {}

impl DebugView {
    /// Whether `renderer` can show this view.
    pub fn is_supported_by(self, renderer: Renderer) -> bool {
        match renderer {
            Renderer::Cpu => true,
            // Depth survives: the depth texture is downloaded and normalized
            // by the same `debug::depth_view` the rasterizer uses, a frame
            // behind. The other two do not, and say so.
            Renderer::Gpu => matches!(self, DebugView::Shaded | DebugView::Depth),
        }
    }
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
    /// Which debug view the main loop presents after `render`. Private
    /// because not every renderer can show every view — see
    /// [`Engine::set_debug_view`].
    debug_view: DebugView,
    /// Write count that counts as "fully red" in [`DebugView::Overdraw`].
    pub overdraw_saturation: u32,
    renderer: Renderer,
    /// The Metal device, while the GPU renderer is the one running. `draw`
    /// routes through it, and the main loop opens and closes a frame around
    /// `Game::render`.
    gpu: Option<Gpu>,
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
            renderer: Renderer::Cpu,
            gpu: None,
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

    /// Which renderer this frame is going through.
    pub fn renderer(&self) -> Renderer {
        self.renderer
    }

    /// The Metal device, while the GPU renderer is running.
    pub fn gpu(&self) -> Option<&Gpu> {
        self.gpu.as_ref()
    }

    /// The Metal device, to change how it presents.
    ///
    /// This is how the benchmark turns vsync off and pins the backing scale to
    /// 1 before its first frame. A game has no reason to reach in here: the
    /// defaults — present in step with the display, at whatever scale the
    /// display has — are the ones a game wants.
    pub fn gpu_mut(&mut self) -> Option<&mut Gpu> {
        self.gpu.as_mut()
    }

    /// Which debug view the main loop will present after `render`.
    pub fn debug_view(&self) -> DebugView {
        self.debug_view
    }

    /// Ask for a debug view, and find out whether this renderer has it.
    ///
    /// The refusal is a value rather than a panic or a silent no-op: an
    /// application that offers the views on keys wants to say *why* nothing
    /// happened, and the text to say it with is in the error.
    pub fn set_debug_view(&mut self, view: DebugView) -> Result<(), UnsupportedView> {
        if !view.is_supported_by(self.renderer) {
            return Err(UnsupportedView {
                view,
                renderer: self.renderer,
            });
        }
        self.debug_view = view;
        Ok(())
    }

    /// Draw a mesh, accumulating this frame's statistics.
    ///
    /// The bound is [`GpuShader`] rather than [`Shader`] because this is the
    /// draw call that works under either renderer, and it can only promise
    /// that of a shader that exists on both sides. A shader with no MSL twin
    /// goes through [`Engine::draw_cpu`].
    pub fn draw<S: GpuShader>(&mut self, mesh: &Mesh, shader: &S) -> DrawStats {
        let stats = match &mut self.gpu {
            Some(gpu) => {
                let state = self.rasterizer;
                gpu.draw(
                    mesh,
                    shader,
                    state.cull,
                    state.blend,
                    state.depth_test,
                    state.depth_write,
                )
            }
            None => self
                .rasterizer
                .draw_mesh(&mut self.framebuffer, mesh, shader),
        };
        self.frame_stats.triangles_in += stats.triangles_in;
        self.frame_stats.triangles_rasterized += stats.triangles_rasterized;
        self.frame_stats.fragments_shaded += stats.fragments_shaded;
        self.frame_stats.fragments_written += stats.fragments_written;
        stats
    }

    /// Draw a mesh with a shader that only exists on the CPU.
    ///
    /// Useful for a one-off, a tool, or a shader being worked on before its
    /// twin is written. Under [`Renderer::Gpu`] it panics rather than silently
    /// drawing nothing, because a missing shader that quietly renders an empty
    /// frame is the worst of the available outcomes.
    pub fn draw_cpu<S: Shader>(&mut self, mesh: &Mesh, shader: &S) -> DrawStats {
        assert!(
            self.gpu.is_none(),
            "this shader has no MSL twin, so it cannot run under the GPU renderer. \
             Implement `runity_gpu::GpuShader` for it and draw it with `Engine::draw`, \
             or start with RUNITY_RENDERER=cpu."
        );
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
        if self.gpu.is_some() {
            let (width, height) = self.overlay_size();
            let segments = runity_gpu::lines::wireframe_segments(mesh, mvp, width, height, color);
            self.draw_segments(&segments);
            return;
        }
        debug::draw_wireframe(&mut self.framebuffer, mesh, mvp, color);
    }

    /// Overlay each vertex normal as a short segment.
    pub fn draw_normals(&mut self, mesh: &Mesh, model: Mat4, length: f32, color: Color) {
        let view_projection = self.view_projection();
        if self.gpu.is_some() {
            let (width, height) = self.overlay_size();
            let segments = runity_gpu::lines::normal_segments(
                mesh,
                model,
                view_projection,
                length,
                width,
                height,
                color,
            );
            self.draw_segments(&segments);
            return;
        }
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
        if self.gpu.is_some() {
            let (width, height) = self.overlay_size();
            let segments = runity_gpu::lines::axis_segments(view_projection, length, width, height);
            self.draw_segments(&segments);
            return;
        }
        debug::draw_axes(&mut self.framebuffer, view_projection, length);
    }

    /// The pixel grid the overlay projections work in: the GPU's drawable when
    /// there is one, so a Retina window gets segments at backing resolution.
    fn overlay_size(&self) -> (f32, f32) {
        match self.gpu.as_ref().and_then(|g| g.frame_size()) {
            Some((w, h)) => (w as f32, h as f32),
            None => (
                self.framebuffer.width() as f32,
                self.framebuffer.height() as f32,
            ),
        }
    }

    /// How wide an overlay line is, in backing pixels.
    ///
    /// One logical pixel: on a 2x drawable that is two backing pixels, which
    /// is what makes a GPU overlay look like the rasterizer's at the size it
    /// is actually shown. It is the one thing the two renderers do not agree
    /// on pixel for pixel, and the only one.
    fn overlay_line_width(&self) -> f32 {
        let (drawable_width, _) = self.overlay_size();
        (drawable_width / self.framebuffer.width().max(1) as f32).max(1.0)
    }

    fn draw_segments(&mut self, segments: &[runity_gpu::lines::Segment]) {
        let width = self.overlay_line_width();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.draw_lines(segments, width);
        }
    }

    // -----------------------------------------------------------------------
    // The GPU frame, opened and closed by the main loop around `Game::render`
    // -----------------------------------------------------------------------

    fn begin_gpu_frame(&mut self, target: Target) -> bool {
        let clear = self.clear_color;
        let wants_depth = self.debug_view == DebugView::Depth;
        match self.gpu.as_mut() {
            Some(gpu) => {
                gpu.set_depth_readback(wants_depth);
                gpu.begin_frame(target, clear)
            }
            None => false,
        }
    }

    /// The depth view, on the GPU: the depth texture came back a lap of the
    /// ring ago, `debug::depth_view` normalizes it exactly as it does the
    /// rasterizer's, and the picture goes on screen as a full-screen quad.
    /// A frame of latency, and no stall.
    fn present_gpu_debug_view(&mut self) {
        if self.debug_view != DebugView::Depth {
            return;
        }
        let Some(view) = self
            .gpu
            .as_ref()
            .and_then(|gpu| gpu.depth_snapshot())
            .map(debug::depth_view)
        else {
            return;
        };
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.draw_fullscreen_image(&view);
        }
    }

    /// Close the frame. `download` is true for the offscreen path, where the
    /// frame has to come back into `framebuffer` before anything can look at
    /// it.
    fn end_gpu_frame(&mut self, download: bool) {
        let framebuffer = &mut self.framebuffer;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.end_frame(download.then_some(framebuffer));
            let stats = gpu.frame_stats();
            self.frame_stats.triangles_in = stats.triangles_in;
            self.frame_stats.triangles_rasterized = stats.triangles_rasterized;
        }
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
    /// Which renderer to build. `None` lets [`Renderer::resolve`] decide, and
    /// `RUNITY_RENDERER` outranks both.
    pub renderer: Option<Renderer>,
}

impl App {
    pub fn new(window_config: WindowConfig) -> Self {
        Self {
            window_config,
            options: RunOptions::default(),
            renderer: None,
        }
    }

    /// Pick a renderer explicitly. `RUNITY_RENDERER` still overrides it.
    pub fn with_renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = Some(renderer);
        self
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

        let surface = window.native_surface();
        let requested = Renderer::resolve(
            std::env::var(runity_gpu::RENDERER_ENV).ok().as_deref(),
            self.renderer,
            surface.is_some(),
        );
        if requested == Renderer::Gpu {
            // A GPU that will not start is fatal on purpose. The alternative
            // is to fall back quietly, and a game that silently runs on the
            // software rasterizer at four frames a second has told nobody
            // anything. The message always ends with the way out.
            match start_gpu(surface) {
                Ok(gpu) => {
                    engine.renderer = Renderer::Gpu;
                    engine.gpu = Some(gpu);
                }
                Err(error) => runity_gpu::fail(&error),
            }
        }

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
            match engine.renderer {
                Renderer::Cpu => {
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

                    // Debug views replace the frame after the game has drawn
                    // it, so a game needs no awareness of them.
                    match engine.debug_view {
                        DebugView::Shaded | DebugView::Wireframe => {}
                        DebugView::Depth => {
                            engine.framebuffer = debug::depth_view(&engine.framebuffer)
                        }
                        DebugView::Overdraw => {
                            let saturation = engine.overdraw_saturation;
                            engine.framebuffer =
                                debug::overdraw_view(&engine.framebuffer, saturation);
                        }
                    }
                }
                Renderer::Gpu => {
                    // The framebuffer is not cleared and not presented here:
                    // on the window path the clear is the render pass's load
                    // action and the drawable goes straight to the display.
                    let target = match gpu_target(&engine) {
                        Some(target) => target,
                        None => break,
                    };
                    if engine.begin_gpu_frame(target) {
                        game.render(&mut engine);
                        engine.present_gpu_debug_view();
                        engine.end_gpu_frame(!matches!(target, Target::Window));
                    }
                }
            }

            if let Some(title) = engine.pending_title.take() {
                window.set_title(&title)?;
            }

            if engine.renderer == Renderer::Cpu {
                let (w, h) = (
                    engine.framebuffer.width() as u32,
                    engine.framebuffer.height() as u32,
                );
                window.present(engine.framebuffer.pixels(), w, h)?;
            }

            if let Some(max) = self.options.max_frames {
                if engine.time.frame() >= max {
                    engine.quit();
                }
            }

            let paced_by_vsync = engine.gpu.as_ref().is_some_and(|gpu| gpu.paces_frames());
            if let Some(budget) = frame_budget(self.options.target_fps, paced_by_vsync) {
                if let Some(remaining) = budget.checked_sub(frame_started.elapsed()) {
                    std::thread::sleep(remaining);
                }
            }
        }
        Ok(engine)
    }
}

/// How long a frame is allowed to take, or `None` for no sleeping at all.
///
/// Presenting in step with the display is already a frame limiter, and a
/// better one: `nextDrawable` blocks until the display is ready, so the loop
/// runs at the refresh rate. A `thread::sleep` on top of that is a second
/// limiter fighting the first — the sleep keeps ending just after the display
/// was ready, one vsync is missed, and the loop settles *below* the refresh
/// rate instead of on it. Measured on a 120 Hz panel: 57 fps with both, 120
/// with vsync alone. So when the GPU paces the frame, the target is its
/// business and the loop does not sleep.
fn frame_budget(target_fps: Option<f32>, paced_by_vsync: bool) -> Option<std::time::Duration> {
    if paced_by_vsync {
        return None;
    }
    target_fps
        .filter(|fps| *fps > 0.0)
        .map(|fps| std::time::Duration::from_secs_f32(1.0 / fps))
}

/// Open the device and, when there is a view, put a `CAMetalLayer` on it.
fn start_gpu(surface: Option<NativeSurface>) -> Result<Gpu, GpuError> {
    let mut gpu = Gpu::new()?;
    if let Some(NativeSurface::AppKitView(view)) = surface {
        gpu.attach(view)?;
    }
    Ok(gpu)
}

/// Where this frame goes: the window's drawable, or a texture the size of the
/// engine's framebuffer.
///
/// `None` means the window has a drawable of zero size — minimized, usually —
/// and there is nothing to draw into this frame.
fn gpu_target(engine: &Engine) -> Option<Target> {
    match engine.gpu.as_ref().and_then(|gpu| gpu.drawable_size()) {
        Some((0, _)) | Some((_, 0)) => None,
        Some(_) => Some(Target::Window),
        None => Some(Target::Offscreen {
            width: engine.framebuffer.width(),
            height: engine.framebuffer.height(),
        }),
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
    fn a_window_gets_the_gpu_and_a_headless_run_gets_the_rasterizer() {
        // The default is the useful one: a window means a game.
        assert_eq!(Renderer::resolve(None, None, true), Renderer::Gpu);
        assert_eq!(Renderer::resolve(None, None, false), Renderer::Cpu);
    }

    #[test]
    fn the_environment_outranks_what_the_program_asked_for() {
        // RUNITY_RENDERER is the escape hatch, so it has to work on a binary
        // you cannot rebuild — including one that hard-coded the other answer.
        assert_eq!(
            Renderer::resolve(Some("cpu"), Some(Renderer::Gpu), true),
            Renderer::Cpu
        );
        assert_eq!(
            Renderer::resolve(Some("gpu"), Some(Renderer::Cpu), false),
            Renderer::Gpu
        );
        assert_eq!(Renderer::resolve(Some(" cpu "), None, true), Renderer::Cpu);
        // Anything else is not an override, it is a typo: fall through.
        assert_eq!(Renderer::resolve(Some("metal"), None, false), Renderer::Cpu);
        assert_eq!(
            Renderer::resolve(Some(""), Some(Renderer::Gpu), false),
            Renderer::Gpu
        );
    }

    #[test]
    fn a_program_that_asks_for_a_renderer_gets_it() {
        assert_eq!(
            Renderer::resolve(None, Some(Renderer::Cpu), true),
            Renderer::Cpu
        );
        assert_eq!(
            Renderer::resolve(None, Some(Renderer::Gpu), false),
            Renderer::Gpu
        );
        let app = App::new(WindowConfig::default()).with_renderer(Renderer::Cpu);
        assert_eq!(app.renderer, Some(Renderer::Cpu));
    }

    #[test]
    fn vsync_is_the_only_frame_limiter_when_it_is_pacing_the_frame() {
        // Two limiters are worse than one: the sleep keeps landing just after
        // the display was ready, and the loop settles below the refresh rate.
        assert_eq!(frame_budget(Some(60.0), true), None);
        assert_eq!(frame_budget(None, true), None);
    }

    #[test]
    fn the_frame_target_still_paces_every_path_vsync_does_not() {
        let sixtieth = std::time::Duration::from_secs_f32(1.0 / 60.0);
        assert_eq!(frame_budget(Some(60.0), false), Some(sixtieth));
        // The benchmark asks for no target at all, and must not get one.
        assert_eq!(frame_budget(None, false), None);
        // A nonsense target is not a zero-length sleep, it is no sleep.
        assert_eq!(frame_budget(Some(0.0), false), None);
        assert_eq!(frame_budget(Some(-1.0), false), None);
    }

    #[test]
    fn the_rasterizer_can_show_every_debug_view() {
        for view in [
            DebugView::Shaded,
            DebugView::Wireframe,
            DebugView::Depth,
            DebugView::Overdraw,
        ] {
            assert!(view.is_supported_by(Renderer::Cpu), "{view:?}");
        }
    }

    #[test]
    fn the_gpu_refuses_the_two_views_that_live_inside_the_rasterizer() {
        assert!(DebugView::Shaded.is_supported_by(Renderer::Gpu));
        assert!(
            DebugView::Depth.is_supported_by(Renderer::Gpu),
            "the depth texture comes back and `depth_view` normalizes it"
        );
        assert!(!DebugView::Wireframe.is_supported_by(Renderer::Gpu));
        assert!(!DebugView::Overdraw.is_supported_by(Renderer::Gpu));
    }

    #[test]
    fn a_refused_view_leaves_the_old_one_alone_and_says_why() {
        let mut engine = Engine::new(8, 8);
        engine.renderer = Renderer::Gpu;
        engine
            .set_debug_view(DebugView::Depth)
            .expect("depth works");
        assert_eq!(engine.debug_view(), DebugView::Depth);

        let refused = engine
            .set_debug_view(DebugView::Overdraw)
            .expect_err("overdraw does not");
        assert_eq!(refused.view, DebugView::Overdraw);
        assert_eq!(
            engine.debug_view(),
            DebugView::Depth,
            "the view that was showing is still showing"
        );

        let text = refused.to_string();
        assert!(text.contains("Overdraw"), "{text}");
        assert!(text.contains("RUNITY_RENDERER=cpu"), "{text}");
    }

    #[test]
    fn a_headless_engine_targets_a_texture_the_size_of_its_framebuffer() {
        // No GPU is open here, so there is no drawable: the offscreen target
        // is the one that gets picked, at the framebuffer's own size.
        let engine = Engine::new(40, 30);
        assert_eq!(
            gpu_target(&engine),
            Some(Target::Offscreen {
                width: 40,
                height: 30
            })
        );
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
}
