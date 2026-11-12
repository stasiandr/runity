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
mod frame_debugger;
/// A running game driven from outside (`SCRAP_CONTROL`).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub mod control;
/// Programs of the control's commands, in Rhai (`script`).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub mod script;
/// A window the caller's own loop draws into.
#[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
mod pumped;
#[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
pub use pumped::Pumped;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::touch_pad::TouchMode;
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
    /// ([`crate::touch_pad`]): a first-person game's while the pointer is
    /// captured ([`TouchLayout::standard`](crate::touch_pad::TouchLayout::standard)),
    /// or a whole gamepad on a slot of its own while touch is the device
    /// in use ([`TouchLayout::gamepad`](crate::touch_pad::TouchLayout::gamepad);
    /// `SCRAP_TOUCH_PAD=pad`). The standard layout on iOS and Android and
    /// with `SCRAP_TOUCH_PAD=1`; none elsewhere. A game lays out its own or
    /// turns it off.
    pub touch_pad: Option<crate::touch_pad::TouchLayout>,
    /// On a screen the platform gives whole (a phone's): the game's picture
    /// kept to this aspect (width over height, `16.0 / 9.0`), as large as
    /// fits inside the safe area and centred in it, black round it — out
    /// of a notch's way and the shape a game laid out for a window
    /// expects. The game is told that rectangle's size and draws as in a
    /// window of it; its views and overlay are moved there, and the touch
    /// pad has the whole screen. None: the whole screen is the game's.
    pub letterbox: Option<f32>,
    /// Where the control socket listens (`control`): a running game
    /// driven from outside its window — shots, input, pause and step, the
    /// game's own commands. `SCRAP_CONTROL=1` (or a path) by default;
    /// none, the default without it. Never in a browser or on a phone.
    pub control: Option<std::path::PathBuf>,
}

/// The control socket `SCRAP_CONTROL` asks for, where there can be one.
fn control_from_env() -> Option<std::path::PathBuf> {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    return std::env::var(control::CONTROL_VAR).ok().filter(|v| v != "0").map(|v| control::socket_for(&v));
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    None
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
                Ok("1") => Some(crate::touch_pad::TouchLayout::standard()),
                Ok("pad") => Some(crate::touch_pad::TouchLayout::gamepad()),
                Ok(_) => None,
                Err(_) => cfg!(any(target_os = "ios", target_os = "android")).then(crate::touch_pad::TouchLayout::standard),
            },
            control: control_from_env(),
            letterbox: None,
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
    /// Whether the window fills its screen now ([`Context::set_fullscreen`]).
    pub fullscreen: bool,
    /// What of the drawable a notch, rounded corners or a home bar cover:
    /// insets in pixels from its edges (left, top, right, bottom). Zero
    /// where the whole window is the game's (a desktop's, a page's).
    pub safe_area: glam::Vec4,
    /// Input the game asked to be played ([`Context::inject`]).
    injected: Vec<(u32, InputEvent)>,
    quit: bool,
    capture: Option<bool>,
    fullscreen_asked: Option<bool>,
    keyboard_asked: Option<bool>,
    touch_pad_asked: Option<bool>,
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

    /// Fill the screen with the window (borderless, the screen's own
    /// mode: no mode switch, so it is quick and other windows stay
    /// reachable) or give it back its frame. Done after this call to the
    /// game returns; nothing where the platform owns the screen (a page,
    /// a phone). `SCRAP_FULLSCREEN=1` starts a window filling it.
    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen_asked = Some(on);
    }

    /// Bring the platform's keyboard onto the screen while the game wants
    /// text typed (a name, a room's code), or put it away: on a phone,
    /// whose keyboard is only on the glass. What is typed comes in as a
    /// keyboard's would — `InputEvent::Text`, Backspace, Enter as "\n".
    /// Nothing where a keyboard is always at hand.
    pub fn show_keyboard(&mut self, on: bool) {
        self.keyboard_asked = Some(on);
    }

    /// The pad on the glass ([`WindowConfig::touch_pad`]) in use or not —
    /// a player's setting. Off, it is hidden and lets go of its slot, and a
    /// touch no longer brings it back (touches reach the game as they would
    /// without one); on again, it is there as at the start. Nothing without
    /// a touch pad.
    pub fn use_touch_pad(&mut self, on: bool) {
        self.touch_pad_asked = Some(on);
    }

    /// Play `event` as if a device sent it, `after` frames from now (0:
    /// before the next frame's steps): what a control command that acts
    /// like a player (`Game::control`) presses and lets go with. Only
    /// with the control socket on; nothing otherwise.
    pub fn inject(&mut self, after: u32, event: InputEvent) {
        self.injected.push((after, event));
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

    /// The app went to the background (`false`: a phone's home gesture, a
    /// call, the screen locked) or came back (`true`). Between the two
    /// nothing is stepped or drawn and the clock does not run; a game
    /// pauses its play, saves, quiets its sound, tells an online partner.
    /// Only where the platform sends the app away (iOS, Android).
    fn active(&mut self, _active: bool) {}

    /// A request from the control socket (`SCRAP_CONTROL`, module
    /// `control`) that the shell does not answer itself: the game's own —
    /// its state, its cheats, a player's move. Called during the frame,
    /// after [`Game::frame`]; [`control::Answer::Pending`] is asked again
    /// next frame. Unknown by default.
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    fn control(&mut self, _ctx: &mut Context, _cmd: &str, _args: &control::Value) -> control::Answer {
        control::Answer::Unknown
    }

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
/// `SCRAP_STARTUP=1`: when each step of starting happened, in seconds of
/// the wall clock — to set against when the process was launched — and
/// every slow frame of the first few hundred; `quit` also ends the game at
/// its 300th frame.
static STARTUP: std::sync::LazyLock<Option<String>> = std::sync::LazyLock::new(|| std::env::var("SCRAP_STARTUP").ok());

fn startup(what: &str) {
    if STARTUP.is_some() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        eprintln!("startup {:.3} {what}", now.as_secs_f64());
    }
}

/// A frame drawn, for [`startup`]: the first, and those slow of the first
/// three hundred; whether the game is to end now.
fn startup_frame() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    if STARTUP.is_none() {
        return false;
    }
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    let mut last = LAST.lock().unwrap();
    let took = last.map(|t| t.elapsed());
    *last = Some(Instant::now());
    if n == 1 || (n <= 300 && took.is_some_and(|t| t > Duration::from_millis(50))) {
        startup(&format!("frame {n} ({:.0} ms since the last)", took.unwrap_or_default().as_secs_f32() * 1e3));
    }
    n >= 300 && STARTUP.as_deref() == Some("quit")
}

pub fn run<G: Game + 'static>(config: WindowConfig, game: G) -> anyhow::Result<()> {
    startup("the shell runs");
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
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut builder = EventLoop::<Running>::with_user_event();
    // On Android the loop is the Activity's: made on the app the game was
    // handed (`android::start`).
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = crate::android::app().ok_or_else(|| anyhow::anyhow!("scrap::shell::android::start was not called"))?;
        builder.with_android_app(app);
    }
    let event_loop = builder.build()?;
    // Poll rather than Wait: a game draws continuously, and waiting for an
    // event means the world only advances when the mouse moves. In the
    // browser the redraw asked for each turn is the page's animation frame,
    // which keeps the game going by itself; Poll there schedules a task
    // after task between frames — a busy loop that took the page's main
    // thread from everything else on it (its sound's buffers among them).
    event_loop.set_control_flow(if cfg!(target_arch = "wasm32") { ControlFlow::Wait } else { ControlFlow::Poll });
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
        script: scrap_core::input::InputScript::from_env().map(|s| (s, None)),
        #[cfg(not(target_arch = "wasm32"))]
        live_input: scrap_core::input::LiveInput::from_env(),
        render_thread,
        stepped_ahead: false,
        last_steps: Duration::ZERO,
        loop_times: scrap_core::perf::Profiler::new(600),
        say_times: std::env::var_os("SCRAP_LOOP_TIMES").map(|_| Instant::now()),
        pads: match gilrs::Gilrs::new() {
            Ok(pads) => Some((pads, scrap_core::input::PadSlots::new())),
            Err(e) => {
                eprintln!("no gamepads: {e}");
                None
            }
        },
        page_slots: scrap_core::input::PadSlots::new(),
        captured: false,
        wants_capture: false,
        focused: true,
        touch_pad: None,
        touch_pad_off: false,
        touch_pad_wanted: None,
        keyboard_shown: false,
        #[cfg(any(target_os = "android", target_os = "ios"))]
        suspended: false,
        debugger: Default::default(),
        frozen: None,
        #[cfg(not(target_arch = "wasm32"))]
        frame_log: FrameLog::from_env(),
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        desk: None,
        recent: Default::default(),
        scale_before_pause: 1.0,
        forced_steps: 0,
        last_began: None,
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
    /// [`WindowConfig::letterbox`]: the game's rectangle of the screen, in
    /// pixels (x, y, width, height), this frame.
    content: Option<(u32, u32, u32, u32)>,
}

/// The largest rectangle of `aspect` inside `screen` less the `safe`
/// insets, centred in what the insets leave: x, y, width, height.
pub fn letterbox(screen: (u32, u32), safe: glam::Vec4, aspect: f32) -> (u32, u32, u32, u32) {
    let (x0, y0) = (safe.x.max(0.0), safe.y.max(0.0));
    let w = (screen.0 as f32 - x0 - safe.z.max(0.0)).max(1.0);
    let h = (screen.1 as f32 - y0 - safe.w.max(0.0)).max(1.0);
    let (cw, ch) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    let (cw, ch) = (cw.floor().max(1.0), ch.floor().max(1.0));
    ((x0 + (w - cw) * 0.5).round() as u32, (y0 + (h - ch) * 0.5).round() as u32, cw as u32, ch as u32)
}

/// A frame made for a picture of `content`'s size, drawn into that
/// rectangle of the screen: its views moved there (one view of its camera
/// when it had none).
fn letterboxed(mut frame: Frame, content: (u32, u32, u32, u32)) -> Frame {
    use crate::render::{FrameView, ViewRect};
    let (ox, oy, w, h) = content;
    if frame.views.is_empty() {
        frame.views.push(FrameView { camera: frame.camera, rect: ViewRect::Share { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }, draws: Vec::new() });
    }
    for view in &mut frame.views {
        let (x, y, vw, vh) = view.rect.pixels((w, h));
        view.rect = ViewRect::Pixels { x: x + ox, y: y + oy, width: vw, height: vh };
    }
    frame
}

/// A pointer's or a finger's place in the game's part of the screen, `at`
/// being where that part begins ([`WindowConfig::letterbox`]).
fn within(event: InputEvent, at: (f32, f32)) -> InputEvent {
    match event {
        InputEvent::MouseMoved { x, y } => InputEvent::MouseMoved { x: x - at.0, y: y - at.1 },
        InputEvent::Touch { id, phase, x, y } => InputEvent::Touch { id, phase, x: x - at.0, y: y - at.1 },
        other => other,
    }
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
    /// Input played from a file (`SCRAP_INPUT_SCRIPT`), and when it began:
    /// the first frame drawn, the game loaded.
    script: Option<(scrap_core::input::InputScript, Option<Instant>)>,
    /// Input read from a file as it is written (`SCRAP_INPUT_LIVE`).
    #[cfg(not(target_arch = "wasm32"))]
    live_input: Option<scrap_core::input::LiveInput>,
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
    /// Gamepads, and the player slot each is in. `None` where the platform
    /// has no way to ask — the game still runs, on keyboard and mouse.
    pads: Option<(gilrs::Gilrs, scrap_core::input::PadSlots)>,
    /// The page's pad's slot (or the glass's, [`TouchMode::Pad`](crate::touch_pad::TouchMode))
    /// where there are no `pads` to share theirs.
    page_slots: scrap_core::input::PadSlots,
    /// The pointer is captured ([`Context::capture_cursor`]).
    captured: bool,
    /// What the game last asked of the pointer: captured only while the
    /// window has the focus, as Unity's cursor lock — another window (a
    /// second player's, the editor) must be reachable with the mouse.
    wants_capture: bool,
    focused: bool,
    /// The pad on the screen, where there is one.
    touch_pad: Option<crate::touch_pad::TouchPad>,
    /// The player switched the pad on the glass off ([`Context::use_touch_pad`]).
    touch_pad_off: bool,
    /// Asked last frame, applied as the next one begins.
    touch_pad_wanted: Option<bool>,
    /// The platform's keyboard is on the screen ([`Context::show_keyboard`]):
    /// its keys are typing, not a keyboard taking over from the glass's pad.
    keyboard_shown: bool,
    /// On a phone, between the app going to the background and coming
    /// back: nothing is stepped or drawn (Android's window is gone; iOS
    /// lets no GPU work from the background).
    #[cfg(any(target_os = "android", target_os = "ios"))]
    suspended: bool,
    /// The Frame Debugger's window (F9), and the frame it takes apart.
    debugger: frame_debugger::Panel,
    frozen: Option<Frame>,
    #[cfg(not(target_arch = "wasm32"))]
    frame_log: Option<FrameLog>,
    /// The control socket and what it asked for (`control`).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    desk: Option<control::Desk>,
    /// The last frames: interval ms, steps, GPU ms — `perf` on the socket.
    recent: std::collections::VecDeque<(f32, u32, f32)>,
    /// The time scale before a pause from the socket.
    scale_before_pause: f32,
    /// Steps run by the socket's `step`, while paused.
    forced_steps: u64,
    /// When the last turn began (the frames' intervals).
    last_began: Option<Instant>,
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
            size: state.content.map_or((drawing.surface.width(), drawing.surface.height()), |c| (c.2, c.3)),
            renderer: &mut drawing.renderer,
            overlay: &mut drawing.overlay,
            cursor_captured: captured,
            loop_times,
            commands: &[],
            fullscreen: !SCREEN_IS_GIVEN && state.window.fullscreen().is_some(),
            safe_area: safe_area(&state.window),
            injected: Vec::new(),
            quit: false,
            capture: None,
            fullscreen_asked: None,
            keyboard_asked: None,
            touch_pad_asked: None,
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
    use std::sync::atomic::Ordering;
    let turn = TURN.fetch_add(1, Ordering::Relaxed);
    let game_wants = SHOTS_WANTED.load(Ordering::Relaxed) > 0;
    let desk_wants = DESK_WANTED.load(Ordering::Relaxed) > 0;
    if game_wants || desk_wants {
        if let Some(rgba) = acquired.read_rgba(gpu) {
            let shot = WindowShot { width: acquired.width, height: acquired.height, rgba, turn };
            if desk_wants {
                DESK_WANTED.fetch_sub(1, Ordering::Relaxed);
                if let Ok(mut shots) = DESK_SHOTS.lock() {
                    shots.push(shot.clone());
                }
            }
            if game_wants {
                SHOTS_WANTED.fetch_sub(1, Ordering::Relaxed);
                if let Ok(mut shots) = SHOTS.lock() {
                    shots.push(shot);
                }
            }
        } else {
            // Nothing to read here: nobody waits for ever.
            SHOTS_WANTED.store(0, Ordering::Relaxed);
            DESK_WANTED.store(0, Ordering::Relaxed);
        }
    }
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
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if self.suspended {
            return;
        }
        // F9: the Frame Debugger opens on this frame, or lets the game go
        // on (shell/frame_debugger.rs).
        if self.input.pressed(frame_debugger::KEY) {
            self.toggle_debugger();
        }
        self.apply_touch_pad_wanted();
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

        if self.debugger.open {
            self.draw_debugged(size);
            return;
        }
        // The whole screen, and the game's part of it (`letterbox`): what
        // the game is told and draws for from here on is its part.
        let screen = size;
        state.content = self.config.letterbox.map(|aspect| letterbox(screen, safe_area(&state.window), aspect));
        let content = state.content;
        let size = content.map_or(screen, |c| (c.2, c.3));

        let mut quit = false;
        let turn_began = Instant::now();
        LOOP_TURN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let steps_before = self.time.step();
        let mut steps_ms = 0.0f32;
        // A script's input due by now, as if the keys were pressed.
        if let Some((script, began)) = self.script.as_mut() {
            let seconds = began.get_or_insert_with(Instant::now).elapsed().as_secs_f32();
            script.play(seconds, &mut self.input);
            quit |= script.over(seconds);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(live) = self.live_input.as_mut() {
            live.play(&mut self.input);
        }
        // The control socket: what came, the input due, the shell's own
        // commands answered; the game's are asked after its frame.
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        {
            if let Some(desk) = self.desk.as_mut() {
                quit |= desk_before(desk, &mut self.input, &mut self.time, &mut self.scale_before_pause, self.forced_steps, &self.recent, &self.config.title, size);
            }
        }
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
        // Paused from the socket: only the steps it asked for.
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        if let Some(desk) = self.desk.as_mut().filter(|d| d.paused && d.steps_owed > 0) {
            let owed = std::mem::take(&mut desk.steps_owed);
            for _ in 0..owed {
                let mut ctx = StepContext { time: &self.time, input: &self.input, size, quit: false, asked: TimeAsk::default() };
                let game = &mut self.game;
                hot(|| game.step(&mut ctx));
                quit |= ctx.quit;
                self.forced_steps += 1;
            }
        }
        if !self.stepped_ahead {
            let start = Instant::now();
            quit |= run_steps(&mut self.game, &mut self.time, &self.input, size);
            self.last_steps = start.elapsed();
            steps_ms += self.last_steps.as_secs_f32() * 1e3;
            self.loop_times.record("steps", self.last_steps);
        }
        self.stepped_ahead = false;

        let start = Instant::now();
        let mut ctx = Self::context(state, &self.time, &self.input, self.captured, &self.loop_times);
        let game = &mut self.game;
        let frame = hot(|| game.frame(&mut ctx));
        let frame = match content {
            Some(content) => letterboxed(frame, content),
            None => frame,
        };
        quit |= ctx.quit;
        wanted = ctx.capture.or(wanted);
        let fullscreen = ctx.fullscreen_asked;
        let keyboard = ctx.keyboard_asked;
        let touch_pad_asked = ctx.touch_pad_asked;
        let mut injected = std::mem::take(&mut ctx.injected);
        let asked = ctx.asked;
        self.time.ask(asked);
        let frame_ms = start.elapsed().as_secs_f32() * 1e3;
        self.loop_times.record("frame", start.elapsed());
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        {
            // The game's own commands, with the frame's context.
            let mut ctx = Self::context(state, &self.time, &self.input, self.captured, &self.loop_times);
            if let Some(desk) = self.desk.as_mut() {
                let game = &mut self.game;
                desk_game(desk, game, &mut ctx);
            }
            injected.append(&mut ctx.injected);
            quit |= ctx.quit;
            if let Some(desk) = self.desk.as_mut() {
                let turn = LOOP_TURN.load(std::sync::atomic::Ordering::Relaxed);
                desk.due.extend(injected.drain(..).map(|(after, e)| (turn + 1 + after as u64, e)));
            }
        }
        drop(injected);
        if let Some(on) = wanted {
            self.wants_capture = on;
        }
        if let (Some(on), false) = (fullscreen, SCREEN_IS_GIVEN) {
            if on != state.window.fullscreen().is_some() {
                state.window.set_fullscreen(on.then_some(winit::window::Fullscreen::Borderless(None)));
            }
        }
        // A phone's keyboard on the glass (UIKit's first responder).
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if let Some(on) = keyboard {
            state.window.set_ime_allowed(on);
            self.keyboard_shown = on;
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        let _ = keyboard;
        // The glass's pad switched off (or on) by the player: applied as
        // the next frame begins.
        if let Some(on) = touch_pad_asked {
            self.touch_pad_wanted = Some(on);
        }
        if let Some(on) = wanted.map(|on| on && self.focused).filter(|on| *on != self.captured) {
            set_captured(&state.window, on);
            self.captured = on;
            if let Some(pad) = self.touch_pad.as_mut().filter(|p| !on && p.mode() == TouchMode::Look) {
                for event in pad.release_all() {
                    self.input.handle(&event);
                }
            }
        }
        // The pad on the screen over the game's overlay: while playing
        // (`Look`), or while touch is the device in use (`Pad`).
        // The game's overlay moved into its part of a letterboxed screen.
        let pad_on = !self.touch_pad_off && self.touch_pad.as_ref().is_some_and(|pad| (pad.mode() == TouchMode::Look && self.captured) || (pad.mode() == TouchMode::Pad && pad.shown()));
        let padded = if pad_on || state.content.is_some() {
            let mut ui = self.game.overlay().clone();
            if let Some((x, y, _, _)) = state.content {
                ui.shift(x as f32, y as f32);
            }
            if let Some(pad) = self.touch_pad.as_mut().filter(|_| pad_on) {
                pad.resize(glam::Vec2::new(screen.0 as f32, screen.1 as f32), state.window.scale_factor() as f32);
                pad.set_safe_area(safe_area(&state.window));
                pad.draw(&mut ui);
            }
            Some(ui)
        } else {
            None
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
                steps_ms += took.as_secs_f32() * 1e3;
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
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(log) = self.frame_log.as_mut() {
            use std::io::Write;
            let ms = |name: &str| times.iter().find(|(n, _)| *n == name).map_or(0.0, |(_, t)| t.as_secs_f32() * 1e3);
            let interval = log.last.map_or(0.0, |l| (turn_began - l).as_secs_f32() * 1e3);
            log.last = Some(turn_began);
            let (gpu_ms, stats) = state.drawing.as_ref().map_or((0.0, Default::default()), |d| {
                (d.renderer.gpu_times().iter().map(|(_, t)| *t).sum::<f32>(), d.renderer.stats())
            });
            let note = NOTE.lock().map(|n| n.replace(',', ";")).unwrap_or_default();
            let _ = writeln!(
                log.out,
                "{},{:.3},{:.2},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{},{},{:.2},{},{},{},{},{},{}",
                TURN.load(std::sync::atomic::Ordering::Relaxed),
                (turn_began - log.began).as_secs_f32(),
                interval,
                self.time.step() - steps_before,
                steps_ms,
                frame_ms,
                ms("acquire"),
                ms("render"),
                ms("present"),
                size.0,
                size.1,
                state.window.scale_factor(),
                gpu_ms,
                stats.submitted,
                stats.drawn,
                stats.triangles,
                stats.batches,
                stats.shadow_casters,
                note,
            );
            // Each line out at once: a lagging game is often killed, not quit.
            let _ = log.out.flush();
        }
        {
            let interval = self.last_began.map_or(0.0, |l| (turn_began - l).as_secs_f32() * 1e3);
            self.last_began = Some(turn_began);
            let gpu_ms = state.drawing.as_ref().map_or(0.0, |d| d.renderer.gpu_times().iter().map(|(_, t)| *t).sum::<f32>());
            if self.recent.len() >= 600 {
                self.recent.pop_front();
            }
            self.recent.push_back((interval, (self.time.step() - steps_before) as u32, gpu_ms));
        }
        for (name, took) in times {
            self.loop_times.record(name, took);
        }
        match drawn {
            Drawn::Shown => {
                quit |= startup_frame();
                if STARTUP.is_some() {
                    use std::sync::atomic::{AtomicU8, Ordering};
                    // 0 nothing yet, 1 building, 2 all in once.
                    static BUILT: AtomicU8 = AtomicU8::new(0);
                    let building = state.drawing.as_ref().map_or(0, |d| d.renderer.pipelines_building());
                    match (BUILT.load(Ordering::Relaxed), building) {
                        (0, n) if n > 0 => BUILT.store(1, Ordering::Relaxed),
                        (1, 0) => {
                            BUILT.store(2, Ordering::Relaxed);
                            startup("every pipeline the frames asked for is in");
                        }
                        _ => {}
                    }
                }
            }
            // Routine: the window is being dragged or is minimised. Rebuild
            // the swapchain and let the next frame have it.
            Drawn::Outdated => {
                {
                    use std::sync::atomic::{AtomicU32, Ordering};
                    static OUTDATED: AtomicU32 = AtomicU32::new(0);
                    let n = OUTDATED.fetch_add(1, Ordering::Relaxed);
                    if n.is_power_of_two() || n == 0 {
                        startup(&format!("frame not shown: the surface is outdated ({} times; the window {size:?}, occluded {:?}, visible {:?})", n + 1, state.window.is_minimized(), state.window.is_visible()));
                    }
                }
                if let Some(d) = state.drawing.as_ref() {
                    d.surface.reconfigure(&state.gpu);
                }
                // No picture to wait for (a window minimised, hidden, the
                // screen asleep, a fullscreen switch under way): without a
                // pause the loop turned a few thousand times a second on a
                // core of its own until the screen took frames again.
                #[cfg(not(target_arch = "wasm32"))]
                std::thread::sleep(Duration::from_millis(8));
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

/// The control socket at the start of a turn: requests taken in, the
/// input due played, shots written, the shell's own commands answered.
/// Whether one asked to quit.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
#[allow(clippy::too_many_arguments)]
fn desk_before(desk: &mut control::Desk, input: &mut Input, time: &mut Time, scale_before_pause: &mut f32, forced_steps: u64, recent: &std::collections::VecDeque<(f32, u32, f32)>, title: &str, size: (u32, u32)) -> bool {
    use control::{Until, Value};
    use std::sync::atomic::Ordering;
    let turn = LOOP_TURN.load(Ordering::Relaxed);
    // Shots count the frames drawn, the rest the loop's turns.
    let drawn = TURN.load(Ordering::Relaxed);
    desk.take_new(turn);
    for event in desk.due_by(turn) {
        input.handle(&event);
    }
    // Shots the window kept, written for the jobs that asked.
    for shot in DESK_SHOTS.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default() {
        let Some(job) = desk.shots.iter_mut().find(|j| j.left > 0 && j.from_turn <= shot.turn) else { continue };
        job.left -= 1;
        let index = job.armed_count - job.left - 1;
        let written = control::write_shot(&shot, job, index);
        let id = job.request;
        if let Some(r) = desk.waiting.iter_mut().find(|r| r.turn == id && r.cmd == "shot") {
            if let Some(Until::Shots { files, .. }) = r.until.as_mut() {
                files.push(written.unwrap_or_else(|e| Value::String(e)));
            }
        }
    }
    desk.shots.retain(|j| j.left > 0);
    for job in desk.shots.iter_mut().filter(|j| !j.armed && j.from_turn <= drawn) {
        job.armed = true;
        DESK_WANTED.fetch_add(job.left, Ordering::Relaxed);
    }
    let mut quit = false;
    let steps_now = forced_steps;
    for mut r in std::mem::take(&mut desk.waiting) {
        let timeout = r.timeout();
        if r.began.elapsed().as_secs_f32() > timeout {
            let what = r.cmd.clone();
            r.answer(Err(format!("{what}: timed out after {timeout:.0} s")));
            continue;
        }
        match r.until.as_mut() {
            Some(Until::Turn(t)) => {
                if turn >= *t {
                    r.answer(Ok(control::json::json!({ "turn": turn })));
                } else {
                    desk.waiting.push(r);
                }
                continue;
            }
            Some(Until::Step(n)) => {
                if steps_now >= *n {
                    r.answer(Ok(control::json::json!({ "steps": steps_now, "turn": turn })));
                } else {
                    desk.waiting.push(r);
                }
                continue;
            }
            Some(Until::Shots { want, files }) => {
                if files.is_empty() && turn > r.turn + 240 && drawn == r.drawn_then {
                    r.answer(Err("no frame was shown for 240 turns: the window is hidden, minimised, or the screen is asleep or locked".into()));
                    continue;
                }
                if files.len() >= *want {
                    let files = std::mem::take(files);
                    r.answer(Ok(Value::Array(files)));
                } else {
                    desk.waiting.push(r);
                }
                continue;
            }
            Some(Until::Game) => {
                desk.waiting.push(r);
                continue;
            }
            Some(Until::Script) => {
                match desk.script.as_ref().map(|s| s.done.try_recv()) {
                    Some(Ok(result)) => {
                        desk.script = None;
                        r.answer(Ok(result));
                    }
                    Some(Err(std::sync::mpsc::TryRecvError::Empty)) => desk.waiting.push(r),
                    _ => {
                        desk.script = None;
                        r.answer(Err("the script's thread ended without a word".into()));
                    }
                }
                continue;
            }
            None => {}
        }
        let arg = |name: &str| r.args.get(name).and_then(Value::as_f64);
        match r.cmd.as_str() {
            "hello" => {
                let answer = control::json::json!({
                    "game": title, "size": [size.0, size.1], "paused": desk.paused,
                    "turn": turn, "steps": time.step(), "fixed_delta": time.settings().fixed_delta,
                    "pid": std::process::id(),
                });
                r.answer(Ok(answer));
            }
            "pause" => {
                if !desk.paused {
                    desk.paused = true;
                    *scale_before_pause = time.settings().scale;
                    time.set_scale(0.0);
                }
                r.answer(Ok(control::json::json!({ "paused": true, "turn": turn })));
            }
            "resume" => {
                if desk.paused {
                    desk.paused = false;
                    time.set_scale(*scale_before_pause);
                }
                r.answer(Ok(control::json::json!({ "paused": false, "turn": turn })));
            }
            "step" => {
                if !desk.paused {
                    desk.paused = true;
                    *scale_before_pause = time.settings().scale;
                    time.set_scale(0.0);
                }
                let n = arg("steps").unwrap_or(1.0).max(0.0) as u32;
                desk.steps_owed += n;
                r.until = Some(Until::Step(steps_now + desk.steps_owed as u64));
                desk.waiting.push(r);
            }
            "frames" => {
                let n = arg("frames").unwrap_or(1.0).max(0.0) as u64;
                r.until = Some(Until::Turn(turn + n));
                desk.waiting.push(r);
            }
            "time_scale" => {
                let scale = arg("scale").unwrap_or(1.0).max(0.0) as f32;
                if desk.paused {
                    *scale_before_pause = scale;
                } else {
                    time.set_scale(scale);
                }
                r.answer(Ok(control::json::json!({ "scale": scale })));
            }
            "input" => match control::events_of(&r.args) {
                Ok(events) => {
                    let n = events.len();
                    for (after, event) in events {
                        if after == 0 {
                            input.handle(&event);
                        } else {
                            desk.due.push((turn + after, event));
                        }
                    }
                    r.answer(Ok(control::json::json!({ "events": n, "turn": turn })));
                }
                Err(e) => r.answer(Err(e)),
            },
            "shot" => {
                let frames = arg("frames").unwrap_or(1.0).clamp(1.0, 16.0) as u32;
                let after = arg("after").unwrap_or(0.0).max(0.0) as u64;
                let path = r.args.get("path").and_then(Value::as_str).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("scrap-shot.png"));
                let crop = match r.args.get("half").and_then(Value::as_str) {
                    Some("p1" | "left" | "1") => Some((0, 0, size.0 / 2, size.1)),
                    Some("p2" | "right" | "2") => Some((size.0 / 2, 0, size.0 - size.0 / 2, size.1)),
                    _ => r.args.get("crop").and_then(Value::as_array).filter(|c| c.len() == 4).map(|c| {
                        let n = |i: usize| c[i].as_u64().unwrap_or(0) as u32;
                        (n(0), n(1), n(2), n(3))
                    }),
                };
                let width = arg("width").map(|w| w as u32);
                desk.shots.push(control::ShotJob { from_turn: drawn + after, left: frames, armed_count: frames, armed: false, path, crop, width, request: turn });
                r.until = Some(Until::Shots { want: frames as usize, files: Vec::new() });
                r.drawn_then = drawn;
                desk.waiting.push(r);
            }
            "perf" => {
                let n = arg("frames").unwrap_or(240.0).max(1.0) as usize;
                let recent: Vec<&(f32, u32, f32)> = recent.iter().rev().take(n).collect();
                let mut intervals: Vec<f32> = recent.iter().map(|r| r.0).filter(|v| *v > 0.0).collect();
                intervals.sort_by(f32::total_cmp);
                let avg = if intervals.is_empty() { 0.0 } else { intervals.iter().sum::<f32>() / intervals.len() as f32 };
                let pick = |q: f32| intervals.get(((intervals.len() as f32 * q) as usize).min(intervals.len().saturating_sub(1))).copied().unwrap_or(0.0);
                let steps = recent.iter().map(|r| r.1 as f32).sum::<f32>() / recent.len().max(1) as f32;
                let gpu = recent.iter().map(|r| r.2).sum::<f32>() / recent.len().max(1) as f32;
                let answer = control::json::json!({
                    "frames": intervals.len(), "fps": if avg > 0.0 { 1000.0 / avg } else { 0.0 },
                    "avg_ms": avg, "p50_ms": pick(0.5), "p99_ms": pick(0.99), "max_ms": intervals.last().copied().unwrap_or(0.0),
                    "steps_per_frame": steps, "gpu_ms": gpu, "size": [size.0, size.1],
                });
                r.answer(Ok(answer));
            }
            "script" => {
                if desk.script.is_some() {
                    r.answer(Err("busy: a script is under way (script_cancel stops it)".into()));
                    continue;
                }
                let source = match (r.args.get("source").and_then(Value::as_str), r.args.get("path").and_then(Value::as_str)) {
                    (Some(s), _) => Ok(s.to_string()),
                    (None, Some(p)) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")),
                    _ => Err("script wants source=… or path=…".to_string()),
                };
                match source {
                    Ok(source) => {
                        if r.args.get("timeout").is_none() {
                            if let Value::Object(o) = &mut r.args {
                                o.insert("timeout".into(), Value::from(600));
                            }
                        }
                        let library = match r.args.get("lib").and_then(Value::as_str).map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))) {
                            Some(Err(e)) => {
                                r.answer(Err(e));
                                continue;
                            }
                            Some(Ok(l)) => Some(l),
                            None => None,
                        };
                        desk.script = Some(script::start(source, library, desk.to_loop.clone()));
                        r.until = Some(Until::Script);
                        desk.waiting.push(r);
                    }
                    Err(e) => r.answer(Err(e)),
                }
            }
            "script_cancel" => {
                let running = desk.script.as_ref().map(|s| s.cancel.store(true, Ordering::Relaxed)).is_some();
                r.answer(Ok(control::json::json!({ "cancelled": running })));
            }
            "quit" => {
                r.answer(Ok(control::json::json!({ "quit": true })));
                quit = true;
            }
            // The game's: asked after its frame.
            _ => {
                r.until = Some(Until::Game);
                desk.waiting.push(r);
            }
        }
    }
    quit
}

/// The game's commands waiting on the desk, asked with the frame's
/// context; answered, failed, or kept for the next frame.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn desk_game<G: Game>(desk: &mut control::Desk, game: &mut G, ctx: &mut Context) {
    use control::{Answer, Until};
    for r in std::mem::take(&mut desk.waiting) {
        if !matches!(r.until, Some(Until::Game)) {
            desk.waiting.push(r);
            continue;
        }
        let (cmd, args) = (r.cmd.clone(), r.args.clone());
        match hot(|| game.control(ctx, &cmd, &args)) {
            Answer::Done(v) => r.answer(Ok(v)),
            Answer::Failed(e) => r.answer(Err(e)),
            Answer::Pending => desk.waiting.push(r),
            Answer::Unknown => r.answer(Err(format!("unknown command `{cmd}`"))),
        }
    }
}

impl<G: Game> Shell<G> {
    /// The Frame Debugger opened — the game stopped where it is, the
    /// pointer let go, the passes timed — or closed, and all of it undone.
    fn toggle_debugger(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
        let open = !self.debugger.open;
        self.debugger.open = open;
        self.frozen = None;
        if let Some(d) = state.drawing.as_deref_mut() {
            if open {
                d.renderer.profile_gpu(true);
            } else {
                d.renderer.stop_debugging();
                d.renderer.profile_gpu(std::env::var_os("SCRAP_GPU_TIMES").is_some());
            }
        }
        if open {
            self.debugger.was_captured = self.captured;
            if self.captured {
                set_captured(&state.window, false);
                self.captured = false;
            }
        } else if self.debugger.was_captured && self.focused {
            set_captured(&state.window, true);
            self.captured = true;
        }
    }

    /// A turn with the Frame Debugger open: no steps, the game's last
    /// frame drawn again stopped at the event picked, its picture and the
    /// window over it.
    fn draw_debugged(&mut self, size: (u32, u32)) {
        let Some(state) = self.state.as_mut() else { return };
        if self.frozen.is_none() {
            let mut ctx = Self::context(state, &self.time, &self.input, self.captured, &self.loop_times);
            let game = &mut self.game;
            self.frozen = Some(hot(|| game.frame(&mut ctx)));
        }
        let Some(frame) = self.frozen.as_ref() else { return };
        let gpu = &state.gpu;
        let d = state.drawing.as_deref_mut().expect("back from the render thread");
        self.debugger.update(&self.input, d.renderer.frame_capture(), size);
        d.renderer.debug_frame(self.debugger.stop());
        match d.surface.begin_frame() {
            Ok(acquired) => {
                d.renderer.draw_ui_pictures(gpu, &mut d.overlay, frame);
                d.renderer.render_to_frame(gpu, &acquired, frame);
                d.renderer.show_debug_picture(gpu, &acquired.view, frame_debugger::Panel::picture_area(size));
                let times = d.renderer.gpu_times();
                self.debugger.draw(d.renderer.frame_capture(), &self.input, size, &times);
                d.overlay.render_to_frame(gpu, &acquired, &self.debugger.ui);
                acquired.present(gpu);
            }
            Err(SurfaceError::Outdated) => d.surface.reconfigure(gpu),
            Err(e) => eprintln!("{e}"),
        }
        self.input.begin_frame();
    }

    /// The device and the window are there: the game starts.
    /// What the glass's pad said, as a pad of its own: its slot taken or
    /// left, its sticks and buttons on that slot.
    fn glass(&mut self, said: Vec<PagePad>) {
        let slots = match self.pads.as_mut() {
            Some((_, slots)) => slots,
            None => &mut self.page_slots,
        };
        let events: Vec<InputEvent> = said.into_iter().flat_map(|s| page_pad_events(slots, s)).collect();
        for event in events {
            self.input.handle(&event);
        }
    }

    /// The player's [`Context::use_touch_pad`], as a frame begins.
    fn apply_touch_pad_wanted(&mut self) {
        let Some(on) = self.touch_pad_wanted.take().filter(|on| *on == self.touch_pad_off) else { return };
        self.touch_pad_off = !on;
        if on {
            if let Some(pad) = self.touch_pad.as_mut().filter(|p| p.mode() == TouchMode::Pad && !p.shown()) {
                pad.show(true);
                self.glass(vec![PagePad::Shown(true)]);
            }
        } else {
            self.hide_glass();
            if let Some(pad) = self.touch_pad.as_mut().filter(|p| p.mode() == TouchMode::Look) {
                for event in pad.release_all() {
                    self.input.handle(&event);
                }
            }
        }
    }

    /// Another device is used: the glass's pad lets go and leaves its slot.
    fn hide_glass(&mut self) {
        let Some(pad) = self.touch_pad.as_mut().filter(|p| p.mode() == TouchMode::Pad && p.shown()) else {
            return;
        };
        let mut said: Vec<PagePad> = pad.show(false).into_iter().map(PagePad::Event).collect();
        said.push(PagePad::Shown(false));
        self.glass(said);
    }

    fn begin(&mut self, mut state: Running) {
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        if let Some(socket) = self.config.control.clone() {
            match control::Desk::open(&socket, &self.config.title) {
                Ok(desk) => self.desk = Some(desk),
                Err(e) => eprintln!("control {}: {e}", socket.display()),
            }
        }
        // The glass's pad is a pad plugged in from the start.
        if self.touch_pad.as_ref().is_some_and(|p| p.mode() == TouchMode::Pad && p.shown()) {
            self.glass(vec![PagePad::Shown(true)]);
        }
        let mut ctx = Self::context(&mut state, &self.time, &self.input, self.captured, &self.loop_times);
        self.game.start(&mut ctx);
        startup("the game started");
        if let Some(on) = ctx.capture {
            set_captured(&state.window, on);
            self.captured = on;
        }
        self.state = Some(state);
    }
}

/// The tallest the picture is drawn in a browser, in pixels (0: as tall as
/// the canvas is on the screen, times the device's pixel ratio). The page
/// stretches it over the canvas: a game's render scale on the web, where a
/// laptop's 2x screen would otherwise ask for four times the pixels of its
/// window. Nothing on the desktop, whose window is its own size.
static CANVAS_CAP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_canvas_height_cap(pixels: u32) {
    CANVAS_CAP.store(pixels, std::sync::atomic::Ordering::Relaxed);
}

/// A word on the frames from now on in `SCRAP_FRAME_LOG`'s last column —
/// what the game is showing (`title`, `fight 85%`), so a slow frame is
/// read against where it happened. Nothing else reads it.
pub fn note_frame(text: &str) {
    if let Ok(mut note) = NOTE.lock() {
        if note.as_str() != text {
            note.clear();
            note.push_str(text);
        }
    }
}

static NOTE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// A frame of the window as it was shown: the scene and the overlay,
/// RGBA8 (sRGB bytes), the drawable's size in pixels.
#[derive(Debug, Clone)]
pub struct WindowShot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The loop's turn it was drawn on (consecutive shots differ by one).
    pub turn: u64,
}

/// Keep the next `frames` frames the window shows ([`take_shots`]): each
/// one waits on the GPU, so this is for a tool, a check or a player's
/// screenshot key, not every frame. Nothing where the platform's swapchain
/// cannot be read.
pub fn capture_frames(frames: u32) {
    SHOTS_WANTED.fetch_add(frames, std::sync::atomic::Ordering::Relaxed);
}

/// The frames kept since the last call ([`capture_frames`]).
pub fn take_shots() -> Vec<WindowShot> {
    SHOTS.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default()
}

static SHOTS_WANTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The control socket's shots, apart from the game's own.
static DESK_WANTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DESK_SHOTS: std::sync::Mutex<Vec<WindowShot>> = std::sync::Mutex::new(Vec::new());
static SHOTS: std::sync::Mutex<Vec<WindowShot>> = std::sync::Mutex::new(Vec::new());
static TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The loop's turns, drawn or not (a window the screen takes no frames
/// from still turns): what the control socket's waits and answers count.
static LOOP_TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `SCRAP_FRAME_LOG=path`: a line per frame, CSV — when, how long since
/// the last, the fixed steps run and what they took, the game's frame,
/// the wait for the screen, the drawing, the present, the drawable's
/// pixels and the display's scale, the GPU's time (with
/// `SCRAP_GPU_TIMES`), what the renderer drew, and the game's note
/// ([`note_frame`]). What a "the game lags" is measured with in the real
/// window, where the offscreen numbers miss the screen.
#[cfg(not(target_arch = "wasm32"))]
struct FrameLog {
    out: std::io::BufWriter<std::fs::File>,
    began: Instant,
    last: Option<Instant>,
}

#[cfg(not(target_arch = "wasm32"))]
impl FrameLog {
    fn from_env() -> Option<Self> {
        let path = std::env::var_os("SCRAP_FRAME_LOG")?;
        let file = match std::fs::File::create(&path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("SCRAP_FRAME_LOG {}: {e}", std::path::Path::new(&path).display());
                return None;
            }
        };
        let mut out = std::io::BufWriter::new(file);
        use std::io::Write;
        let _ = writeln!(out, "turn,t,interval_ms,steps,steps_ms,frame_ms,acquire_ms,render_ms,present_ms,width,height,scale,gpu_ms,submitted,drawn,triangles,batches,shadow_casters,note");
        Some(Self { out, began: Instant::now(), last: None })
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
    let mut ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    // A game's cap on the picture's height (`set_canvas_height_cap`): the
    // page stretches a smaller picture over the canvas, the cheapest
    // render scale there is.
    let cap = CANVAS_CAP.load(std::sync::atomic::Ordering::Relaxed);
    let tall = canvas.client_height() as f64 * ratio;
    if cap > 0 && tall > cap as f64 {
        ratio *= cap as f64 / tall;
    }
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
        // iOS: back in the foreground. The view and its layer stayed (UIKit
        // keeps them), so the surface is the same; the clock starts again
        // from here rather than counting the time away.
        #[cfg(target_os = "ios")]
        if std::mem::take(&mut self.suspended) {
            self.time.resume_clock();
            self.game.active(true);
            return;
        }
        // Called again after the window is destroyed and recreated, which on
        // Android happens whenever the app comes back to the foreground.
        // Android's is a new window under the same `Window`: the surface
        // is made again on it, the device and the game kept.
        #[cfg(target_os = "android")]
        if let (true, Some(state)) = (std::mem::take(&mut self.suspended), self.state.as_mut()) {
            if let Some(d) = state.drawing.as_mut() {
                match Surface::from_window(&state.gpu, state.window.clone()) {
                    Ok(surface) => d.surface = surface,
                    Err(e) => eprintln!("no surface on the window come back: {e}"),
                }
            }
            self.time.resume_clock();
            self.game.active(true);
            return;
        }
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
        if !SCREEN_IS_GIVEN && std::env::var("SCRAP_FULLSCREEN").is_ok_and(|v| v == "1") {
            attributes = attributes.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
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
        // A phone's game owns the screen: no status bar, the home bar
        // fading until touched, and a swipe from an edge goes to the game
        // first (a second swipe to the system), as a thumb on a stick
        // near the edge would otherwise leave the app.
        #[cfg(target_os = "ios")]
        {
            use winit::platform::ios::{ScreenEdge, WindowAttributesExtIOS};
            attributes = attributes
                .with_prefers_status_bar_hidden(true)
                .with_prefers_home_indicator_hidden(true)
                .with_preferred_screen_edges_deferring_system_gestures(ScreenEdge::ALL);
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
            Ok(window) => {
                startup("window");
                Arc::new(window)
            }
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
            startup("device");
            match running(gpu, window) {
                Ok(state) => {
                    startup("renderer");
                    self.begin(state)
                }
                Err(e) => {
                    eprintln!("{e}");
                    event_loop.exit();
                }
            }
        }
    }

    /// Android's Activity went to the background and took its window with
    /// it: nothing is drawn until [`Self::resumed`] brings one back.
    #[cfg(target_os = "android")]
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.suspended = self.state.is_some();
        if self.suspended {
            self.game.active(false);
        }
    }

    /// iOS: the app is going to the background (or a call, Control
    /// Centre, the lock came over it). No GPU work may be sent from the
    /// background, so nothing is drawn — nor stepped — until
    /// [`Self::resumed`]; fingers on the glass are let go.
    #[cfg(target_os = "ios")]
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if self.state.is_none() || self.suspended {
            return;
        }
        self.suspended = true;
        if let Some(pad) = self.touch_pad.as_mut() {
            let glass = pad.mode() == TouchMode::Pad;
            let events = pad.release_all();
            if glass {
                self.glass(events.into_iter().map(PagePad::Event).collect());
            } else {
                for event in events {
                    self.input.handle(&event);
                }
            }
        }
        self.game.active(false);
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
            // The pointer goes free with the focus, and is taken again when
            // the window has it back, if the game still wants it.
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                let on = focused && self.wants_capture && !self.debugger.open;
                if on != self.captured {
                    set_captured(&state.window, on);
                    self.captured = on;
                    if let Some(pad) = self.touch_pad.as_mut().filter(|p| !on && p.mode() == TouchMode::Look) {
                        for event in pad.release_all() {
                            self.input.handle(&event);
                        }
                    }
                }
                for event in translate(&event) {
                    self.input.handle(&event);
                }
            }
            // Captured, the pointer's position means nothing: only how far
            // it moves counts, and that comes as device motion.
            WindowEvent::CursorMoved { .. } if self.captured => {}
            WindowEvent::Touch(touch) if self.touch_pad.is_some() && !self.touch_pad_off => {
                let pad = self.touch_pad.as_mut().expect("just checked");
                let phase = match touch.phase {
                    winit::event::TouchPhase::Started => crate::input::TouchPhase::Started,
                    winit::event::TouchPhase::Moved => crate::input::TouchPhase::Moved,
                    winit::event::TouchPhase::Ended => crate::input::TouchPhase::Ended,
                    winit::event::TouchPhase::Cancelled => crate::input::TouchPhase::Cancelled,
                };
                let (x, y) = (touch.location.x as f32, touch.location.y as f32);
                let glass = pad.mode() == TouchMode::Pad;
                // The glass's pad back with a touch, on its slot again.
                let mut said = Vec::new();
                if glass && !pad.shown() && phase == crate::input::TouchPhase::Started {
                    pad.show(true);
                    said.push(PagePad::Shown(true));
                }
                let events = pad.touch(touch.id, phase, x, y, self.captured);
                let at = state.content.map_or((0.0, 0.0), |c| (c.0 as f32, c.1 as f32));
                if glass {
                    said.extend(events.into_iter().map(PagePad::Event));
                    self.glass(said);
                } else {
                    for event in events {
                        self.input.handle(&within(event, at));
                    }
                }
            }
            other => {
                let at = state.content.map_or((0.0, 0.0), |c| (c.0 as f32, c.1 as f32));
                let events: Vec<InputEvent> = translate(&other).into_iter().map(|e| within(e, at)).collect();
                // A key: the glass's pad steps aside (a touch brings it back).
                if !self.keyboard_shown && events.iter().any(|e| matches!(e, InputEvent::KeyDown(_))) {
                    self.hide_glass();
                }
                for event in events {
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
        for said in crate::web::drain() {
            let slots = match self.pads.as_mut() {
                Some((_, slots)) => slots,
                None => &mut self.page_slots,
            };
            for event in page_pad_events(slots, said) {
                self.input.handle(&event);
            }
        }
        if let Some((pads, slots)) = self.pads.as_mut() {
            // Pads in before the loop began say nothing of it: they take
            // their slots first, in the platform's order.
            for (id, _) in pads.gamepads() {
                let device = usize::from(id) as u64;
                if slots.slot(device).is_none() {
                    let slot = slots.connect(device);
                    self.input.handle(&InputEvent::PadConnected(slot));
                }
            }
            let mut pressed = false;
            while let Some(event) = pads.next_event() {
                let device = usize::from(event.id) as u64;
                pressed |= matches!(event.event, gilrs::EventType::ButtonPressed(..));
                for event in pad_events(slots, device, event.event) {
                    self.input.handle(&event);
                }
            }
            // A real pad in the hands: the glass's steps aside.
            if pressed {
                self.hide_glass();
            }
        }
        // Asking for a redraw here rather than drawing here is what keeps the
        // frame on the compositor's schedule instead of ahead of it.
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if self.suspended {
            return;
        }
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

/// The window's safe area as insets in pixels from its edges (left, top,
/// right, bottom): on iOS winit's inner rectangle is the safe area and the
/// outer the whole screen; elsewhere the two are the same window.
fn safe_area(window: &Window) -> glam::Vec4 {
    #[cfg(target_os = "ios")]
    {
        let outer = window.outer_size();
        let inner = window.inner_size();
        let at = window.inner_position().unwrap_or_default();
        let outer_at = window.outer_position().unwrap_or_default();
        let left = (at.x - outer_at.x).max(0) as f32;
        let top = (at.y - outer_at.y).max(0) as f32;
        let right = (outer.width as f32 - inner.width as f32 - left).max(0.0);
        let bottom = (outer.height as f32 - inner.height as f32 - top).max(0.0);
        glam::Vec4::new(left, top, right, bottom)
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = window;
        glam::Vec4::ZERO
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
        content: None,
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

/// One gilrs event of a device as ours, by the player slot the device is
/// in ([`scrap_core::input::PadSlots`]): a pad coming or going, or its
/// button or stick on its slot — which the merged pad sees as well.
fn pad_events(
    slots: &mut scrap_core::input::PadSlots,
    device: u64,
    event: gilrs::EventType,
) -> Vec<InputEvent> {
    use gilrs::EventType;
    match event {
        EventType::Connected => vec![InputEvent::PadConnected(slots.connect(device))],
        EventType::Disconnected => slots
            .disconnect(device)
            .map(InputEvent::PadDisconnected)
            .into_iter()
            .collect(),
        other => {
            let pad = slots.slot(device).unwrap_or_else(|| slots.connect(device));
            translate_pad(other).and_then(|e| on_slot(pad, e)).into_iter().collect()
        }
    }
}

/// What the page's pad on the screen said (the browser's `web.rs`): it
/// came or went, or a stick or button of it moved.
#[derive(Debug, Clone, PartialEq)]
pub enum PagePad {
    Shown(bool),
    Event(InputEvent),
}

/// The device id the page's pad takes its slot by (no real pad has it).
pub const PAGE_PAD: u64 = u64::MAX;

/// The page's pad as a pad of its own: a slot taken when it comes (or at
/// its first touch), its sticks and buttons on that slot, the slot left
/// when it goes. The glass's pad on a phone ([`TouchMode::Pad`]) is one too.
fn page_pad_events(slots: &mut scrap_core::input::PadSlots, said: PagePad) -> Vec<InputEvent> {
    match said {
        PagePad::Shown(true) => match slots.slot(PAGE_PAD) {
            Some(_) => Vec::new(),
            None => vec![InputEvent::PadConnected(slots.connect(PAGE_PAD))],
        },
        PagePad::Shown(false) => slots.disconnect(PAGE_PAD).map(InputEvent::PadDisconnected).into_iter().collect(),
        PagePad::Event(event) => {
            let mut out = Vec::new();
            let pad = match slots.slot(PAGE_PAD) {
                Some(pad) => pad,
                None => {
                    let pad = slots.connect(PAGE_PAD);
                    out.push(InputEvent::PadConnected(pad));
                    pad
                }
            };
            out.extend(on_slot(pad, event));
            out
        }
    }
}

/// A pad's event as one of slot `pad`'s.
fn on_slot(pad: usize, event: InputEvent) -> Option<InputEvent> {
    match event {
        InputEvent::PadDown(button) => Some(InputEvent::PadDownOn { pad, button }),
        InputEvent::PadUp(button) => Some(InputEvent::PadUpOn { pad, button }),
        InputEvent::PadMoved { axis, value } => Some(InputEvent::PadMovedOn { pad, axis, value }),
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

    #[test]
    fn each_pad_reports_on_its_own_slot() {
        let mut slots = scrap_core::input::PadSlots::new();
        assert_eq!(pad_events(&mut slots, 7, EventType::Connected), [InputEvent::PadConnected(0)]);
        assert_eq!(pad_events(&mut slots, 3, EventType::Connected), [InputEvent::PadConnected(1)]);
        assert_eq!(
            on_slot(1, InputEvent::PadDown(PadButton::South)),
            Some(InputEvent::PadDownOn { pad: 1, button: PadButton::South })
        );
        assert_eq!(pad_events(&mut slots, 7, EventType::Disconnected), [InputEvent::PadDisconnected(0)]);
        assert_eq!(pad_events(&mut slots, 7, EventType::Disconnected), [], "gone already");
    }

    #[test]
    fn the_pages_pad_is_a_pad_of_its_own() {
        let mut slots = scrap_core::input::PadSlots::new();
        assert_eq!(pad_events(&mut slots, 7, EventType::Connected), [InputEvent::PadConnected(0)]);
        // Shown: the next slot, said once.
        assert_eq!(page_pad_events(&mut slots, PagePad::Shown(true)), [InputEvent::PadConnected(1)]);
        assert_eq!(page_pad_events(&mut slots, PagePad::Shown(true)), []);
        assert_eq!(
            page_pad_events(&mut slots, PagePad::Event(InputEvent::PadDown(PadButton::South))),
            [InputEvent::PadDownOn { pad: 1, button: PadButton::South }]
        );
        assert_eq!(
            page_pad_events(&mut slots, PagePad::Event(InputEvent::PadMoved { axis: PadAxis::LeftX, value: -1.0 })),
            [InputEvent::PadMovedOn { pad: 1, axis: PadAxis::LeftX, value: -1.0 }]
        );
        // Hidden: its slot left, as a pad unplugged; a touch after brings it back.
        assert_eq!(page_pad_events(&mut slots, PagePad::Shown(false)), [InputEvent::PadDisconnected(1)]);
        assert_eq!(
            page_pad_events(&mut slots, PagePad::Event(InputEvent::PadUp(PadButton::South))),
            [InputEvent::PadConnected(1), InputEvent::PadUpOn { pad: 1, button: PadButton::South }]
        );
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

#[cfg(test)]
mod letterbox_tests {
    use super::letterbox;

    /// An iPhone 15 held sideways: 2556 × 1179 pixels, the island's 59
    /// points (177 px) each side, the home bar's 21 (63 px) below. A 16:9
    /// picture is as tall as the safe area lets it be and centred in it.
    #[test]
    fn a_letterbox_is_the_largest_of_its_aspect_inside_the_safe_area() {
        let (x, y, w, h) = letterbox((2556, 1179), glam::Vec4::new(177.0, 0.0, 177.0, 63.0), 16.0 / 9.0);
        assert_eq!((w, h), (1984, 1116));
        assert_eq!(y, 0);
        assert_eq!(x, 286);
        // A screen narrower than the aspect: as wide as it, centred down.
        let (x, y, w, h) = letterbox((1200, 1600), glam::Vec4::ZERO, 16.0 / 9.0);
        assert_eq!((x, w, h), (0, 1200, 675));
        assert_eq!(y, 463);
    }
}
