//! A desktop window and the loop that drives it.
//!
//! This is a convenience, not the architecture. The engine works without it —
//! the editor, and later a `UIViewController` on iOS and an `Activity` on
//! Android, each own their window and drive their own loop, and the engine
//! draws for whichever one it is handed. What this module adds is the
//! shortest way to see something move.
//!
//! The loop's shape is the one a fixed-step simulation needs:
//!
//! ```text
//! tick the clock → drain input → step the world N times → draw once
//! ```
//!
//! `N` comes from the clock, not from the frame rate, so the world runs at
//! the same speed on every machine and a fast one simply draws it more often.
//!
//! Both game callbacks go through `subsecond::call`, so a rebuilt `step` or
//! `frame` takes effect in the running process. That is why there is no
//! scripting language here: the reason to embed one is to avoid waiting for
//! a compile, and hot-patching Rust avoids it without a second language, a
//! second set of types and a boundary between them.
//!
//! ```text
//! dx serve --hotpatch     # rebuilds and patches while the window stays open
//! ```
//!
//! With a render thread (on by default where there are threads), a frame
//! is drawn on a thread of its own while the main one runs the next
//! frame's fixed steps:
//!
//! ```text
//! main:    … frame N ─┬─ tick, steps for N+1 ──┬─ frame N+1 ─┬─ …
//! render:             └─ draw N, present N ────┘             └─ …
//! ```
//!
//! What a step sees is what had arrived by the last frame — a step's input
//! is a frame older than without it — and a step is handed no renderer
//! ([`StepContext`]): what is reproducible does not touch the GPU. `frame`
//! has the renderer to itself, as before: the thread that drew is done by
//! then. In the browser, in the editor's view, or with `render_thread:
//! false`, the same turns run one after the other.

use std::sync::Arc;

use scrap_core::web_time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
mod embedded;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::input::{Input, InputEvent, Key, MouseButton};
use crate::render::{Frame, Renderer};
use crate::surface::{Surface, SurfaceError};
use crate::time::{Time, TimeAsk, TimeSettings};

/// What the window is called and how big it starts.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub time: TimeSettings,
    /// Draw each frame on a thread of its own, beside the next frame's
    /// steps (see the module). On by default; `SCRAP_RENDER_THREAD=0`
    /// turns it off, as does a target with no threads.
    pub render_thread: bool,
    /// Sticks and buttons drawn on the screen and fed in as a pad's
    /// ([`crate::touch_pad`]), while the pointer is captured. The
    /// standard layout on iOS and Android and with `SCRAP_TOUCH_PAD=1`;
    /// none elsewhere. A game lays out its own or turns it off.
    pub touch_pad: Option<crate::touch_pad::TouchLayout>,
}

/// Whether the platform owns the screen and gives it to the game whole:
/// the browser's page, a phone's. A size asked for there means nothing.
const SCREEN_IS_GIVEN: bool = cfg!(any(target_arch = "wasm32", target_os = "ios", target_os = "android"));

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "scrap".into(),
            width: 1280,
            height: 720,
            time: TimeSettings::default(),
            render_thread: cfg!(not(target_arch = "wasm32"))
                && std::env::var("SCRAP_RENDER_THREAD").map_or(true, |v| v != "0"),
            touch_pad: match std::env::var("SCRAP_TOUCH_PAD").as_deref() {
                Ok("1") => true,
                Ok(_) => false,
                Err(_) => cfg!(any(target_os = "ios", target_os = "android")),
            }
            .then(crate::touch_pad::TouchLayout::standard),
        }
    }
}

pub use scrap_core::project::WINDOW_VAR;

/// `x,y,width,height`, or `None` when it is not four numbers.
pub fn placement(text: &str) -> Option<(i32, i32, u32, u32)> {
    let mut parts = text.split(',').map(str::trim);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let width = parts.next()?.parse().ok()?;
    let height = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((x, y, width, height))
}

/// What a game is handed each time it is called.
pub struct Context<'a> {
    pub time: &'a Time,
    pub input: &'a Input,
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    /// What draws the overlay: where a game gives its own font
    /// ([`crate::ui_render::UiRenderer::use_font`]).
    pub overlay: &'a mut crate::ui_render::UiRenderer,
    /// The drawable size in physical pixels, which is not the window's size
    /// on a HiDPI display.
    pub size: (u32, u32),
    /// Whether the pointer is captured: hidden, pinned, reporting only how
    /// far it moved ([`Context::capture_cursor`]).
    pub cursor_captured: bool,
    /// What the loop itself spent, on the CPU: the steps, the frame, the
    /// drawing and the wait for the screen — beside the game's own
    /// profiler, for F3.
    pub loop_times: &'a scrap_core::perf::Profiler,
    /// Lines the editor's Console sent for the game's own console since
    /// the last frame ([`scrap_core::embed::ToGame::Command`]): none in a
    /// window of its own.
    pub commands: &'a [String],
    quit: bool,
    capture: Option<bool>,
    asked: TimeAsk,
}

/// What a fixed step is handed: the clock and the input, no renderer — a
/// step is what has to be reproducible, and with a render thread it runs
/// while the last frame is drawn. Nor can it capture the pointer: that is
/// the frame's to ask ([`Context::capture_cursor`]).
pub struct StepContext<'a> {
    pub time: &'a Time,
    pub input: &'a Input,
    pub size: (u32, u32),
    quit: bool,
    asked: TimeAsk,
}

impl StepContext<'_> {
    /// Ask the loop to stop after this frame.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Ask the clock for slow motion or a hit-stop ([`TimeAsk`]) — what the
    /// world's systems asked, from `scrap::time::sync`. Done before the
    /// next frame.
    pub fn ask_time(&mut self, asked: TimeAsk) {
        self.asked = self.asked.and(asked);
    }
}

impl Context<'_> {
    /// Ask the loop to stop after this frame.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Capture the pointer — hidden and held in the window, the mouse then
    /// turning the view by how far it moves (`Input::mouse_motion`), as a
    /// first-person game wants — or let it go, for a menu. Done after this
    /// call to the game returns.
    pub fn capture_cursor(&mut self, on: bool) {
        self.capture = Some(on);
    }

    /// Ask the clock for slow motion or a hit-stop ([`TimeAsk`]) — what the
    /// world's systems asked, from `scrap::time::sync`. Done before the
    /// next frame.
    pub fn ask_time(&mut self, asked: TimeAsk) {
        self.asked = self.asked.and(asked);
    }

    pub fn aspect(&self) -> f32 {
        self.size.0 as f32 / self.size.1.max(1) as f32
    }
}

/// What the shell calls.
pub trait Game {
    /// Once, after the device and the window exist. Meshes get uploaded here.
    fn start(&mut self, _ctx: &mut Context) {}

    /// Once per fixed simulation step, possibly several times per frame, and
    /// possibly not at all. Everything that has to be reproducible goes here.
    fn step(&mut self, _ctx: &mut StepContext) {}

    /// Once per frame. Camera and animation belong here, where the delta is
    /// the real one and motion stays smooth.
    fn frame(&mut self, ctx: &mut Context) -> Frame;

    /// After a hot patch, before anything else runs: rebuild whatever holds
    /// values of the game's own types, which the patch may have laid out
    /// differently — `LiveScene::reinstance` does it for a world
    /// that started from a scene. Called through `subsecond::call`, so it is
    /// the new code. Nothing by default, which is right for a game that
    /// keeps no state of its own types.
    ///
    /// The `Game` value itself is not rebuilt: a patch that changes its own
    /// fields needs a restart. Keep state in the world, where this can
    /// carry it across.
    fn patched(&mut self, _ctx: &mut Context) {}

    /// What to draw over the scene. Empty by default.
    fn overlay(&mut self) -> &crate::ui::Ui {
        // A shared empty list, so a game with no overlay allocates nothing
        // and the shell still has something to hand the renderer.
        static EMPTY: std::sync::OnceLock<crate::ui::Ui> = std::sync::OnceLock::new();
        EMPTY.get_or_init(crate::ui::Ui::new)
    }
}

/// Set when a hot patch has been applied, until the loop has told the game.
static PATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A game callback, through `subsecond::call` so a rebuilt one takes
/// effect in the running process. The browser has no hot patches: there it
/// is the call itself.
fn hot<R>(f: impl FnMut() -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    return subsecond::call(f);
    #[cfg(target_arch = "wasm32")]
    {
        let mut f = f;
        f()
    }
}

/// Open a window and run until the game or the user says otherwise.
///
/// In the browser the "window" is the page's `<canvas id="scrap">` (one is
/// made when the page has none), the loop is the browser's own — this
/// returns at once and the game runs on `requestAnimationFrame` — and the
/// device comes up asynchronously, so `Game::start` is called a moment
/// later, when it has.
///
/// Started by the editor's Play (`SCRAP_EMBED` set) there is no window:
/// the game draws into the editor's view instead ([`scrap_core::embed`]).
pub fn run<G: Game + 'static>(config: WindowConfig, game: G) -> anyhow::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(address) = std::env::var(scrap_core::embed::EMBED_VAR) {
        return embedded::run(&address, config, game);
    }
    // Told on the loop's thread at the next turn, not in the handler: the
    // handler runs wherever the patch arrived, mid-frame for all it knows.
    #[cfg(not(target_arch = "wasm32"))]
    subsecond::register_handler(Arc::new(|| {
        PATCHED.store(true, std::sync::atomic::Ordering::Release)
    }));
    let event_loop = EventLoop::<Running>::with_user_event().build()?;
    // Poll rather than Wait: a game draws continuously, and waiting for an
    // event means the world only advances when the mouse moves.
    event_loop.set_control_flow(ControlFlow::Poll);
    let time = Time::new(config.time);
    let render_thread = config.render_thread && cfg!(not(target_arch = "wasm32"));
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut shell = Shell {
        config,
        game,
        state: None,
        proxy: Some(event_loop.create_proxy()),
        time,
        input: Input::new(),
        render_thread,
        stepped_ahead: false,
        last_steps: Duration::ZERO,
        loop_times: scrap_core::perf::Profiler::new(600),
        say_times: std::env::var_os("SCRAP_LOOP_TIMES").map(|_| Instant::now()),
        pads: match gilrs::Gilrs::new() {
            Ok(pads) => Some(pads),
            Err(e) => {
                eprintln!("no gamepads: {e}");
                None
            }
        },
        captured: false,
        touch_pad: None,
    };
    shell.touch_pad = shell.config.touch_pad.clone().map(crate::touch_pad::TouchPad::new);
    #[cfg(target_os = "ios")]
    crate::ios::register();
    #[cfg(not(target_arch = "wasm32"))]
    event_loop.run_app(&mut shell)?;
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(shell);
    }
    Ok(())
}

/// Everything that only exists once there is a window.
pub struct Running {
    window: Arc<Window>,
    gpu: Arc<Gpu>,
    /// What draws: here between frames, with the render thread while it
    /// draws one.
    drawing: Option<Box<Drawing>>,
    #[cfg(not(target_arch = "wasm32"))]
    render: Option<RenderThread>,
}

/// What a frame is drawn with: handed to the render thread and back.
struct Drawing {
    surface: Surface,
    renderer: Renderer,
    overlay: crate::ui_render::UiRenderer,
}

/// A frame to draw, and what drew it back.
#[cfg(not(target_arch = "wasm32"))]
struct Job {
    drawing: Box<Drawing>,
    frame: Frame,
    ui: crate::ui::Ui,
}

#[cfg(not(target_arch = "wasm32"))]
struct Done {
    drawing: Box<Drawing>,
    drawn: Drawn,
    times: Vec<(&'static str, Duration)>,
}

/// The render thread: made the first time a frame is drawn on it, kept
/// until the window goes. What draws is sent to it with the frame and
/// comes back with the frame shown — one thread has it at a time. None in
/// the browser, which gives a page one thread.
#[cfg(not(target_arch = "wasm32"))]
struct RenderThread {
    jobs: std::sync::mpsc::Sender<Job>,
    done: std::sync::mpsc::Receiver<Done>,
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderThread {
    fn new(gpu: Arc<Gpu>) -> Self {
        let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
        let (outbox, done) = std::sync::mpsc::channel::<Done>();
        std::thread::Builder::new()
            .name("scrap-render".into())
            .spawn(move || {
                // Ends when the window's side hangs up.
                for mut job in inbox {
                    let mut times = Vec::with_capacity(3);
                    let d = &mut *job.drawing;
                    let drawn = draw_frame(
                        &gpu,
                        &d.surface,
                        &mut d.renderer,
                        &mut d.overlay,
                        &job.frame,
                        &job.ui,
                        &mut times,
                    );
                    if outbox
                        .send(Done {
                            drawing: job.drawing,
                            drawn,
                            times,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("a thread to draw on");
        Self { jobs, done }
    }
}

struct Shell<G: Game> {
    config: WindowConfig,
    game: G,
    state: Option<Running>,
    /// Where a device made asynchronously (the browser's) is handed back
    /// to the loop; taken when the window is made.
    proxy: Option<EventLoopProxy<Running>>,
    time: Time,
    input: Input,
    /// Draw on a render thread when the steps are worth it (never in the
    /// browser).
    render_thread: bool,
    /// The next frame's steps were run while the last one was drawn.
    stepped_ahead: bool,
    /// What the last frame's steps took: steps cheaper than the handover
    /// to the render thread are not worth drawing beside.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    last_steps: Duration,
    loop_times: scrap_core::perf::Profiler,
    /// `SCRAP_LOOP_TIMES`: the loop's times to stderr every few seconds,
    /// and when it last said them.
    say_times: Option<Instant>,
    /// Gamepads. `None` where the platform has no way to ask — the game
    /// still runs, on keyboard and mouse.
    pads: Option<gilrs::Gilrs>,
    /// The pointer is captured ([`Context::capture_cursor`]).
    captured: bool,
    /// The pad on the screen, where there is one.
    touch_pad: Option<crate::touch_pad::TouchPad>,
}

impl<G: Game> Shell<G> {
    fn context<'a>(
        state: &'a mut Running,
        time: &'a Time,
        input: &'a Input,
        captured: bool,
        loop_times: &'a scrap_core::perf::Profiler,
    ) -> Context<'a> {
        let drawing = state.drawing.as_deref_mut().expect("back from the render thread");
        Context {
            time,
            input,
            gpu: &state.gpu,
            size: (drawing.surface.width(), drawing.surface.height()),
            renderer: &mut drawing.renderer,
            overlay: &mut drawing.overlay,
            cursor_captured: captured,
            loop_times,
            commands: &[],
            quit: false,
            capture: None,
            asked: TimeAsk::default(),
        }
    }
}

/// Advance the clock to now: `Instant` on the desktop, the page's
/// `performance.now()` in the browser, where `std`'s panics.
fn tick(time: &mut Time) {
    #[cfg(not(target_arch = "wasm32"))]
    time.tick();
    #[cfg(target_arch = "wasm32")]
    time.tick_at(
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now() / 1000.0),
    );
}

/// Tick the clock and run the steps it owes, each through [`hot`], so a
/// rebuilt `step` takes effect in the running process. It costs one
/// indirection through a jump table and buys not restarting to see a rule
/// change — which is the entire reason gameplay is not behind a scripting
/// language here. Whether the game asked to quit.
fn run_steps<G: Game>(game: &mut G, time: &mut Time, input: &Input, size: (u32, u32)) -> bool {
    tick(time);
    let mut quit = false;
    while time.next_step().is_some() {
        let mut ctx = StepContext {
            time,
            input,
            size,
            quit: false,
            asked: TimeAsk::default(),
        };
        hot(|| game.step(&mut ctx));
        quit |= ctx.quit;
        let asked = ctx.asked;
        time.ask(asked);
    }
    quit
}

/// Steps at least this long get the drawing on the render thread beside
/// them: handing a frame over and back costs tens of microseconds.
#[cfg(not(target_arch = "wasm32"))]
const THREAD_FROM: Duration = Duration::from_micros(100);

/// How drawing a frame went.
enum Drawn {
    Shown,
    /// The swapchain needs making again (a drag, a minimise).
    Outdated,
    Failed(SurfaceError),
}

/// Draw `frame` and the overlay into the next swapchain image and show it,
/// timing the CPU's part: acquiring (the wait for the screen), drawing,
/// presenting.
fn draw_frame(
    gpu: &Gpu,
    surface: &Surface,
    renderer: &mut Renderer,
    overlay: &mut crate::ui_render::UiRenderer,
    frame: &Frame,
    ui: &crate::ui::Ui,
    times: &mut Vec<(&'static str, Duration)>,
) -> Drawn {
    let start = Instant::now();
    // One acquired frame for both passes: the scene, then the overlay
    // over it, then a single present. Acquiring twice would show an empty
    // frame on top of a full one.
    let acquired = match surface.begin_frame() {
        Ok(acquired) => acquired,
        Err(SurfaceError::Outdated) => return Drawn::Outdated,
        Err(e) => return Drawn::Failed(e),
    };
    let acquired_at = Instant::now();
    times.push(("acquire", acquired_at - start));
    renderer.draw_ui_pictures(gpu, overlay, frame);
    renderer.render_to_frame(gpu, &acquired, frame);
    overlay.render_to_frame(gpu, &acquired, ui);
    let drawn_at = Instant::now();
    times.push(("render", drawn_at - acquired_at));
    acquired.present(gpu);
    times.push(("present", drawn_at.elapsed()));
    Drawn::Shown
}

/// Capture the window's pointer or let it go: locked where the platform
/// can, else confined; hidden while captured.
fn set_captured(window: &Window, on: bool) {
    use winit::window::CursorGrabMode;
    if on {
        let _ = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
    } else {
        let _ = window.set_cursor_grab(CursorGrabMode::None);
    }
    window.set_cursor_visible(!on);
}

impl<G: Game> Shell<G> {
    /// One turn of the loop: advance the clock, run whatever simulation
    /// steps are owed, draw once — with a render thread, the next frame's
    /// steps beside the drawing.
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // The window's size as it is now: a resize that came while the
        // device was still coming up (in the browser it comes up after the
        // canvas is laid out) was told to nobody.
        #[cfg(not(any(target_arch = "wasm32", target_os = "ios")))]
        let (width, height) = state.window.inner_size().into();
        // On iOS winit's inner size is the safe area, the screen less the
        // notch and the home bar; the view it draws in is the whole screen.
        #[cfg(target_os = "ios")]
        let (width, height) = state.window.outer_size().into();
        // In the browser the canvas's pixels are the page's to keep up:
        // its laid-out size times the device's pixel ratio, the space
        // winit gives pointer and touch positions in.
        #[cfg(target_arch = "wasm32")]
        let (width, height) = canvas_pixels(&state.window);
        let size = {
            let drawing = state.drawing.as_mut().expect("back from the render thread");
            drawing.surface.resize(&state.gpu, width, height);
            (drawing.surface.width(), drawing.surface.height())
        };

        let mut quit = false;
        // The pointer the game asked for, captured or free, applied once the
        // frame is done.
        let mut wanted: Option<bool> = None;
        // Before this frame's steps — though with a render thread the steps
        // run beside the last frame's drawing were the old code's.
        if PATCHED.swap(false, std::sync::atomic::Ordering::AcqRel) {
            let mut ctx = Self::context(state, &self.time, &self.input, self.captured, &self.loop_times);
            let game = &mut self.game;
            hot(|| game.patched(&mut ctx));
            quit |= ctx.quit;
            wanted = ctx.capture.or(wanted);
            let asked = ctx.asked;
            self.time.ask(asked);
        }
        if !self.stepped_ahead {
            let start = Instant::now();
            quit |= run_steps(&mut self.game, &mut self.time, &self.input, size);
            self.last_steps = start.elapsed();
            self.loop_times.record("steps", self.last_steps);
        }
        self.stepped_ahead = false;

        let start = Instant::now();
        let mut ctx = Self::context(state, &self.time, &self.input, self.captured, &self.loop_times);
        let game = &mut self.game;
        let frame = hot(|| game.frame(&mut ctx));
        quit |= ctx.quit;
        wanted = ctx.capture.or(wanted);
        let asked = ctx.asked;
        self.time.ask(asked);
        self.loop_times.record("frame", start.elapsed());
        if let Some(on) = wanted.filter(|on| *on != self.captured) {
            set_captured(&state.window, on);
            self.captured = on;
            if let (false, Some(pad)) = (on, self.touch_pad.as_mut()) {
                for event in pad.release_all() {
                    self.input.handle(&event);
                }
            }
        }
        // The pad on the screen over the game's overlay, while playing.
        let padded = match self.touch_pad.as_mut() {
            Some(pad) if self.captured => {
                pad.resize(
                    glam::Vec2::new(size.0 as f32, size.1 as f32),
                    state.window.scale_factor() as f32,
                );
                let mut ui = self.game.overlay().clone();
                pad.draw(&mut ui);
                Some(ui)
            }
            _ => None,
        };

        let mut times = Vec::with_capacity(3);
        #[cfg(not(target_arch = "wasm32"))]
        let threaded = self.render_thread && !quit && self.last_steps >= THREAD_FROM;
        #[cfg(target_arch = "wasm32")]
        let threaded = false;
        let drawn = if threaded {
            #[cfg(not(target_arch = "wasm32"))]
            {
                // The overlay is the game's, which the steps are about to
                // change: the drawing gets its own copy.
                let ui = padded.unwrap_or_else(|| self.game.overlay().clone());
                let thread = state
                    .render
                    .get_or_insert_with(|| RenderThread::new(state.gpu.clone()));
                let drawing = state.drawing.take().expect("back from the render thread");
                thread
                    .jobs
                    .send(Job { drawing, frame, ui })
                    .expect("the render thread is gone");
                let start = Instant::now();
                quit |= run_steps(&mut self.game, &mut self.time, &self.input, size);
                let took = start.elapsed();
                let done = thread.done.recv().expect("the render thread panicked");
                state.drawing = Some(done.drawing);
                times = done.times;
                self.stepped_ahead = true;
                self.last_steps = took;
                self.loop_times.record("steps", took);
                done.drawn
            }
            #[cfg(target_arch = "wasm32")]
            unreachable!("no threads in the browser")
        } else {
            let d = state.drawing.as_deref_mut().expect("back from the render thread");
            draw_frame(
                &state.gpu,
                &d.surface,
                &mut d.renderer,
                &mut d.overlay,
                &frame,
                padded.as_ref().unwrap_or_else(|| self.game.overlay()),
                &mut times,
            )
        };
        for (name, took) in times {
            self.loop_times.record(name, took);
        }
        match drawn {
            Drawn::Shown => {}
            // Routine: the window is being dragged or is minimised. Rebuild
            // the swapchain and let the next frame have it.
            Drawn::Outdated => {
                if let Some(d) = state.drawing.as_ref() {
                    d.surface.reconfigure(&state.gpu);
                }
            }
            Drawn::Failed(e) => {
                eprintln!("{e}");
                quit = true;
            }
        }

        // Events consumed, so a press does not survive into the next frame.
        // After drawing rather than before, because the frame that reads a
        // press is the one it arrived in — and, with a render thread, after
        // the next frame's steps, which read it too.
        self.input.begin_frame();

        if self
            .say_times
            .is_some_and(|said| said.elapsed().as_secs_f32() > 5.0)
        {
            eprintln!(
                "loop, render thread {}:",
                if self.render_thread { "on" } else { "off" }
            );
            for line in self.loop_times.lines() {
                eprintln!("  {line}");
            }
            self.say_times = Some(Instant::now());
        }

        if quit {
            event_loop.exit();
        }
    }
}

impl<G: Game> Shell<G> {
    /// The device and the window are there: the game starts.
    fn begin(&mut self, mut state: Running) {
        let mut ctx = Self::context(&mut state, &self.time, &self.input, self.captured, &self.loop_times);
        self.game.start(&mut ctx);
        if let Some(on) = ctx.capture {
            set_captured(&state.window, on);
            self.captured = on;
        }
        self.state = Some(state);
    }
}

/// The canvas the game draws on: the page's `#scrap`, or a new one over
/// the whole page.
#[cfg(target_arch = "wasm32")]
fn canvas() -> Option<web_sys::HtmlCanvasElement> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()?.document()?;
    if let Some(found) = document.get_element_by_id("scrap") {
        return found.dyn_into().ok();
    }
    let canvas: web_sys::HtmlCanvasElement =
        document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_id("scrap");
    let _ = canvas.set_attribute(
        "style",
        "position:fixed;inset:0;width:100vw;height:100vh;touch-action:none",
    );
    document.body()?.append_child(&canvas).ok()?;
    Some(canvas)
}

/// The canvas's drawing buffer made its laid-out size in device pixels,
/// and that size.
#[cfg(target_arch = "wasm32")]
fn canvas_pixels(window: &Window) -> (u32, u32) {
    use winit::platform::web::WindowExtWebSys;
    let Some(canvas) = window.canvas() else {
        return window.inner_size().into();
    };
    let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    let width = (canvas.client_width() as f64 * ratio).round().max(1.0) as u32;
    let height = (canvas.client_height() as f64 * ratio).round().max(1.0) as u32;
    if canvas.width() != width {
        canvas.set_width(width);
    }
    if canvas.height() != height {
        canvas.set_height(height);
    }
    (width, height)
}

/// Say on the page what went wrong, where a player can read it: the
/// browser has no terminal. The page's `scrapFailed(why)`, if it has one.
#[cfg(target_arch = "wasm32")]
pub fn fail(why: &str) {
    web_sys::console::error_1(&why.into());
    if let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::get(&window, &"scrapFailed".into())
            .ok()
            .and_then(|f| wasm_bindgen::JsCast::dyn_into::<js_sys::Function>(f).ok())
            .map(|f| f.call1(&window, &why.into()));
    }
}

impl<G: Game> ApplicationHandler<Running> for Shell<G> {
    /// A device made asynchronously — the browser's — has come up.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, state: Running) {
        self.begin(state);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Called again after the window is destroyed and recreated, which on
        // Android happens whenever the app comes back to the foreground.
        if self.state.is_some() || self.proxy.is_none() {
            return;
        }
        let mut attributes = Window::default_attributes().with_title(self.config.title.clone());
        // In the browser the page's layout sizes the canvas, on a phone the
        // screen is the window; a size here would pin it in pixels.
        if !SCREEN_IS_GIVEN {
            attributes = attributes.with_inner_size(winit::dpi::LogicalSize::new(
                self.config.width,
                self.config.height,
            ));
        }
        if let Some((x, y, width, height)) = std::env::var(WINDOW_VAR)
            .ok()
            .as_deref()
            .and_then(placement)
        {
            attributes = attributes
                .with_position(winit::dpi::LogicalPosition::new(x, y))
                .with_inner_size(winit::dpi::LogicalSize::new(width, height));
        }
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes = attributes
                .with_canvas(canvas())
                .with_prevent_default(true)
                .with_focusable(true);
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("could not open a window: {e}");
                event_loop.exit();
                return;
            }
        };

        #[cfg(target_os = "ios")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(RawWindowHandle::UiKit(h)) = window.window_handle().map(|h| h.as_raw()) {
                crate::ios::window_made(h.ui_view.as_ptr());
            }
        }
        let proxy = self.proxy.take().expect("the window is made once");
        // The browser gives its device only asynchronously: made there and
        // handed back to the loop as an event.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = event_loop;
            wasm_bindgen_futures::spawn_local(async move {
                let gpu = match Gpu::headless(false).await {
                    Ok(gpu) => gpu,
                    Err(e) => return fail(&format!("WebGPU: {e}")),
                };
                match running(gpu, window) {
                    Ok(state) => {
                        let _ = proxy.send_event(state);
                    }
                    Err(e) => fail(&e),
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = proxy;
            let gpu = match Gpu::headless_blocking(false) {
                Ok(gpu) => gpu,
                Err(e) => {
                    eprintln!("{e}");
                    event_loop.exit();
                    return;
                }
            };
            match running(gpu, window) {
                Ok(state) => self.begin(state),
                Err(e) => {
                    eprintln!("{e}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(d) = state.drawing.as_mut() {
                    d.surface.resize(&state.gpu, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            // Captured, the pointer's position means nothing: only how far
            // it moves counts, and that comes as device motion.
            WindowEvent::CursorMoved { .. } if self.captured => {}
            WindowEvent::Touch(touch) if self.touch_pad.is_some() => {
                let pad = self.touch_pad.as_mut().expect("just checked");
                let phase = match touch.phase {
                    winit::event::TouchPhase::Started => crate::input::TouchPhase::Started,
                    winit::event::TouchPhase::Moved => crate::input::TouchPhase::Moved,
                    winit::event::TouchPhase::Ended => crate::input::TouchPhase::Ended,
                    winit::event::TouchPhase::Cancelled => crate::input::TouchPhase::Cancelled,
                };
                let (x, y) = (touch.location.x as f32, touch.location.y as f32);
                for event in pad.touch(touch.id, phase, x, y, self.captured) {
                    self.input.handle(&event);
                }
            }
            other => {
                for event in translate(&other) {
                    self.input.handle(&event);
                }
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        // Raw motion, for looking around with the pointer captured.
        if let (true, winit::event::DeviceEvent::MouseMotion { delta }) = (self.captured, event) {
            self.input.handle(&InputEvent::MouseMotion {
                dx: delta.0 as f32,
                dy: delta.1 as f32,
            });
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        for event in crate::web::drain() {
            self.input.handle(&event);
        }
        if let Some(pads) = self.pads.as_mut() {
            while let Some(event) = pads.next_event() {
                if let Some(event) = translate_pad(event.event) {
                    self.input.handle(&event);
                }
            }
        }
        // Asking for a redraw here rather than drawing here is what keeps the
        // frame on the compositor's schedule instead of ahead of it.
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

/// The surface and the renderers for a window, on a device.
fn running(gpu: Gpu, window: Arc<Window>) -> Result<Running, String> {
    let surface = Surface::from_window(&gpu, window.clone()).map_err(|e| e.to_string())?;
    let renderer = Renderer::for_surface(&gpu, &surface);
    let overlay = crate::ui_render::UiRenderer::for_surface(&gpu, &surface);
    Ok(Running {
        window,
        gpu: Arc::new(gpu),
        drawing: Some(Box::new(Drawing {
            surface,
            renderer,
            overlay,
        })),
        #[cfg(not(target_arch = "wasm32"))]
        render: None,
    })
}

/// A gilrs button as ours, by where it is on the pad.
fn pad_button(b: gilrs::Button) -> Option<crate::input::PadButton> {
    use crate::input::PadButton;
    use gilrs::Button;
    Some(match b {
        Button::South => PadButton::South,
        Button::East => PadButton::East,
        Button::West => PadButton::West,
        Button::North => PadButton::North,
        // gilrs calls the bumpers triggers and the triggers "2".
        Button::LeftTrigger => PadButton::LeftBumper,
        Button::RightTrigger => PadButton::RightBumper,
        Button::LeftTrigger2 => PadButton::LeftTrigger,
        Button::RightTrigger2 => PadButton::RightTrigger,
        Button::Select => PadButton::Select,
        Button::Start => PadButton::Start,
        Button::LeftThumb => PadButton::LeftStick,
        Button::RightThumb => PadButton::RightStick,
        Button::DPadUp => PadButton::DPadUp,
        Button::DPadDown => PadButton::DPadDown,
        Button::DPadLeft => PadButton::DPadLeft,
        Button::DPadRight => PadButton::DPadRight,
        _ => return None,
    })
}

/// A gilrs stick axis as ours; gilrs already has up and right positive.
fn pad_stick(a: gilrs::Axis) -> Option<crate::input::PadAxis> {
    use crate::input::PadAxis;
    use gilrs::Axis;
    Some(match a {
        Axis::LeftStickX => PadAxis::LeftX,
        Axis::LeftStickY => PadAxis::LeftY,
        Axis::RightStickX => PadAxis::RightX,
        Axis::RightStickY => PadAxis::RightY,
        _ => return None,
    })
}

/// Turn one gilrs event into ours: triggers as both a value and a button.
fn translate_pad(event: gilrs::EventType) -> Option<InputEvent> {
    use crate::input::PadAxis;
    use gilrs::{Button, EventType};
    match event {
        EventType::ButtonPressed(b, _) => pad_button(b).map(InputEvent::PadDown),
        EventType::ButtonReleased(b, _) => pad_button(b).map(InputEvent::PadUp),
        EventType::ButtonChanged(Button::LeftTrigger2, value, _) => Some(InputEvent::PadMoved {
            axis: PadAxis::LeftTrigger,
            value,
        }),
        EventType::ButtonChanged(Button::RightTrigger2, value, _) => Some(InputEvent::PadMoved {
            axis: PadAxis::RightTrigger,
            value,
        }),
        EventType::AxisChanged(axis, value, _) => {
            pad_stick(axis).map(|axis| InputEvent::PadMoved { axis, value })
        }
        _ => None,
    }
}

/// Turn one winit event into ours. A list, because a single winit event can
/// carry both a key and the text it produced.
pub fn translate(event: &WindowEvent) -> Vec<InputEvent> {
    match event {
        WindowEvent::KeyboardInput { event, .. } => {
            let mut out = Vec::new();
            if let PhysicalKey::Code(code) = event.physical_key {
                let key = key_from(code);
                out.push(match event.state {
                    ElementState::Pressed => InputEvent::KeyDown(key),
                    ElementState::Released => InputEvent::KeyUp(key),
                });
            }
            if event.state.is_pressed() {
                if let Some(text) = &event.text {
                    out.push(InputEvent::Text(text.to_string()));
                }
            }
            out
        }
        WindowEvent::Touch(touch) => vec![InputEvent::Touch {
            id: touch.id,
            phase: match touch.phase {
                winit::event::TouchPhase::Started => crate::input::TouchPhase::Started,
                winit::event::TouchPhase::Moved => crate::input::TouchPhase::Moved,
                winit::event::TouchPhase::Ended => crate::input::TouchPhase::Ended,
                winit::event::TouchPhase::Cancelled => crate::input::TouchPhase::Cancelled,
            },
            x: touch.location.x as f32,
            y: touch.location.y as f32,
        }],
        WindowEvent::CursorMoved { position, .. } => vec![InputEvent::MouseMoved {
            x: position.x as f32,
            y: position.y as f32,
        }],
        WindowEvent::MouseInput { state, button, .. } => {
            let button = match button {
                winit::event::MouseButton::Left => MouseButton::Left,
                winit::event::MouseButton::Right => MouseButton::Right,
                winit::event::MouseButton::Middle => MouseButton::Middle,
                winit::event::MouseButton::Other(n) => MouseButton::Other(*n),
                _ => MouseButton::Other(u16::MAX),
            };
            vec![match state {
                ElementState::Pressed => InputEvent::MouseDown(button),
                ElementState::Released => InputEvent::MouseUp(button),
            }]
        }
        WindowEvent::MouseWheel { delta, .. } => {
            let (x, y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                // A trackpad reports pixels; dividing by a typical line
                // height keeps one scroll notch and one swipe comparable.
                MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 16.0, p.y as f32 / 16.0),
            };
            vec![InputEvent::Scroll { x, y }]
        }
        WindowEvent::Focused(false) => vec![InputEvent::FocusLost],
        _ => Vec::new(),
    }
}

fn key_from(code: KeyCode) -> Key {
    use KeyCode as C;
    match code {
        C::KeyA => Key::A,
        C::KeyB => Key::B,
        C::KeyC => Key::C,
        C::KeyD => Key::D,
        C::KeyE => Key::E,
        C::KeyF => Key::F,
        C::KeyG => Key::G,
        C::KeyH => Key::H,
        C::KeyI => Key::I,
        C::KeyJ => Key::J,
        C::KeyK => Key::K,
        C::KeyL => Key::L,
        C::KeyM => Key::M,
        C::KeyN => Key::N,
        C::KeyO => Key::O,
        C::KeyP => Key::P,
        C::KeyQ => Key::Q,
        C::KeyR => Key::R,
        C::KeyS => Key::S,
        C::KeyT => Key::T,
        C::KeyU => Key::U,
        C::KeyV => Key::V,
        C::KeyW => Key::W,
        C::KeyX => Key::X,
        C::KeyY => Key::Y,
        C::KeyZ => Key::Z,
        C::Digit0 => Key::Digit0,
        C::Digit1 => Key::Digit1,
        C::Digit2 => Key::Digit2,
        C::Digit3 => Key::Digit3,
        C::Digit4 => Key::Digit4,
        C::Digit5 => Key::Digit5,
        C::Digit6 => Key::Digit6,
        C::Digit7 => Key::Digit7,
        C::Digit8 => Key::Digit8,
        C::Digit9 => Key::Digit9,
        C::Escape => Key::Escape,
        C::Space => Key::Space,
        C::Enter => Key::Enter,
        C::Tab => Key::Tab,
        C::Backspace => Key::Backspace,
        C::Delete => Key::Delete,
        C::Insert => Key::Insert,
        C::Home => Key::Home,
        C::End => Key::End,
        C::PageUp => Key::PageUp,
        C::PageDown => Key::PageDown,
        C::ArrowLeft => Key::Left,
        C::ArrowRight => Key::Right,
        C::ArrowUp => Key::Up,
        C::ArrowDown => Key::Down,
        C::ShiftLeft => Key::LeftShift,
        C::ShiftRight => Key::RightShift,
        C::ControlLeft => Key::LeftControl,
        C::ControlRight => Key::RightControl,
        C::AltLeft => Key::LeftAlt,
        C::AltRight => Key::RightAlt,
        C::SuperLeft => Key::LeftSuper,
        C::SuperRight => Key::RightSuper,
        C::F1 => Key::F1,
        C::F2 => Key::F2,
        C::F3 => Key::F3,
        C::F4 => Key::F4,
        C::F5 => Key::F5,
        C::F6 => Key::F6,
        C::F7 => Key::F7,
        C::F8 => Key::F8,
        C::F9 => Key::F9,
        C::F10 => Key::F10,
        C::F11 => Key::F11,
        C::F12 => Key::F12,
        C::Comma => Key::Comma,
        C::Period => Key::Period,
        C::Slash => Key::Slash,
        C::Semicolon => Key::Semicolon,
        C::Quote => Key::Quote,
        C::Minus => Key::Minus,
        C::Equal => Key::Equal,
        C::BracketLeft => Key::BracketLeft,
        C::BracketRight => Key::BracketRight,
        C::Backslash => Key::Backslash,
        C::Backquote => Key::Backquote,
        // Keeping the platform's own value means a rebindable game does not
        // lose keys this enum has no name for.
        other => Key::Other(other as u32),
    }
}

#[cfg(test)]
mod pad_tests {
    use super::*;
    use crate::input::{PadAxis, PadButton};
    use gilrs::{Axis, Button, EventType};

    #[test]
    fn a_pad_is_read_by_position() {
        assert_eq!(pad_button(Button::South), Some(PadButton::South));
        assert_eq!(pad_button(Button::LeftTrigger), Some(PadButton::LeftBumper));
        assert_eq!(
            pad_button(Button::RightTrigger2),
            Some(PadButton::RightTrigger)
        );
        assert_eq!(pad_stick(Axis::LeftStickY), Some(PadAxis::LeftY));
        assert_eq!(pad_stick(Axis::LeftZ), None);
        assert_eq!(translate_pad(EventType::Connected), None);
    }
}

#[cfg(test)]
mod placement_tests {
    use super::placement;

    #[test]
    fn a_window_is_placed_by_four_numbers() {
        assert_eq!(placement("40, 60,640,360"), Some((40, 60, 640, 360)));
        assert_eq!(placement("-700,0,640,360"), Some((-700, 0, 640, 360)));
        assert_eq!(placement("40,60,640"), None);
        assert_eq!(placement("40,60,640,360,1"), None);
        assert_eq!(placement("left,top,640,360"), None);
    }
}
