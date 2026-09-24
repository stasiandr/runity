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

use std::sync::Arc;

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
use crate::time::{Time, TimeSettings};

/// What the window is called and how big it starts.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub time: TimeSettings,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "runity".into(),
            width: 1280,
            height: 720,
            time: TimeSettings::default(),
        }
    }
}

pub use runity_core::project::WINDOW_VAR;

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
    quit: bool,
    capture: Option<bool>,
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
    fn step(&mut self, _ctx: &mut Context) {}

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
/// In the browser the "window" is the page's `<canvas id="runity">` (one is
/// made when the page has none), the loop is the browser's own — this
/// returns at once and the game runs on `requestAnimationFrame` — and the
/// device comes up asynchronously, so `Game::start` is called a moment
/// later, when it has.
///
/// Started by the editor's Play (`RUNITY_EMBED` set) there is no window:
/// the game draws into the editor's view instead ([`runity_core::embed`]).
pub fn run<G: Game + 'static>(config: WindowConfig, game: G) -> anyhow::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(address) = std::env::var(runity_core::embed::EMBED_VAR) {
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
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut shell = Shell {
        config,
        game,
        state: None,
        proxy: Some(event_loop.create_proxy()),
        time,
        input: Input::new(),
        pads: match gilrs::Gilrs::new() {
            Ok(pads) => Some(pads),
            Err(e) => {
                eprintln!("no gamepads: {e}");
                None
            }
        },
        captured: false,
    };
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
    gpu: Gpu,
    surface: Surface,
    renderer: Renderer,
    overlay: crate::ui_render::UiRenderer,
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
    /// Gamepads. `None` where the platform has no way to ask — the game
    /// still runs, on keyboard and mouse.
    pads: Option<gilrs::Gilrs>,
    /// The pointer is captured ([`Context::capture_cursor`]).
    captured: bool,
}

impl<G: Game> Shell<G> {
    fn context<'a>(state: &'a mut Running, time: &'a Time, input: &'a Input, captured: bool) -> Context<'a> {
        Context {
            time,
            input,
            gpu: &state.gpu,
            size: (state.surface.width(), state.surface.height()),
            renderer: &mut state.renderer,
            overlay: &mut state.overlay,
            cursor_captured: captured,
            quit: false,
            capture: None,
        }
    }
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
    /// steps are owed, draw once.
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // The window's size as it is now: a resize that came while the
        // device was still coming up (in the browser it comes up after the
        // canvas is laid out) was told to nobody.
        #[cfg(not(target_arch = "wasm32"))]
        let (width, height) = state.window.inner_size().into();
        // In the browser the canvas's pixels are the page's to keep up:
        // its laid-out size times the device's pixel ratio, the space
        // winit gives pointer and touch positions in.
        #[cfg(target_arch = "wasm32")]
        let (width, height) = canvas_pixels(&state.window);
        state.surface.resize(&state.gpu, width, height);
        #[cfg(not(target_arch = "wasm32"))]
        self.time.tick();
        #[cfg(target_arch = "wasm32")]
        self.time.tick_at(
            web_sys::window()
                .and_then(|w| w.performance())
                .map_or(0.0, |p| p.now() / 1000.0),
        );

        let mut quit = false;
        // The pointer the game asked for, captured or free, applied once the
        // frame is done.
        let mut wanted: Option<bool> = None;
        if PATCHED.swap(false, std::sync::atomic::Ordering::AcqRel) {
            let mut ctx = Self::context(state, &self.time, &self.input, self.captured);
            let game = &mut self.game;
            hot(|| game.patched(&mut ctx));
            quit |= ctx.quit;
            wanted = ctx.capture.or(wanted);
        }
        while self.time.next_step().is_some() {
            // Through `subsecond::call`, so a rebuilt `step` takes effect in
            // the running process. It costs one indirection through a jump
            // table and buys not restarting to see a rule change — which is
            // the entire reason gameplay is not behind a scripting language
            // here.
            let mut ctx = Self::context(state, &self.time, &self.input, self.captured);
            let game = &mut self.game;
            hot(|| game.step(&mut ctx));
            quit |= ctx.quit;
            wanted = ctx.capture.or(wanted);
        }

        let mut ctx = Self::context(state, &self.time, &self.input, self.captured);
        let game = &mut self.game;
        let frame = hot(|| game.frame(&mut ctx));
        quit |= ctx.quit;
        wanted = ctx.capture.or(wanted);
        if let Some(on) = wanted.filter(|on| *on != self.captured) {
            set_captured(&state.window, on);
            self.captured = on;
        }

        // One acquired frame for both passes: the scene, then the overlay
        // over it, then a single present. Acquiring twice would show an
        // empty frame on top of a full one.
        match state.surface.begin_frame() {
            Ok(acquired) => {
                state
                    .renderer
                    .draw_ui_pictures(&state.gpu, &mut state.overlay, &frame);
                state
                    .renderer
                    .render_to_frame(&state.gpu, &acquired, &frame);
                state
                    .overlay
                    .render_to_frame(&state.gpu, &acquired, self.game.overlay());
                acquired.present(&state.gpu);
            }
            // Routine: the window is being dragged or is minimised. Rebuild
            // the swapchain and let the next frame have it.
            Err(SurfaceError::Outdated) => state.surface.reconfigure(&state.gpu),
            Err(e) => {
                eprintln!("{e}");
                quit = true;
            }
        }

        // Events consumed, so a press does not survive into the next frame.
        // After drawing rather than before, because the frame that reads a
        // press is the one it arrived in.
        self.input.begin_frame();

        if quit {
            event_loop.exit();
        }
    }
}

impl<G: Game> Shell<G> {
    /// The device and the window are there: the game starts.
    fn begin(&mut self, mut state: Running) {
        let mut ctx = Self::context(&mut state, &self.time, &self.input, self.captured);
        self.game.start(&mut ctx);
        if let Some(on) = ctx.capture {
            set_captured(&state.window, on);
            self.captured = on;
        }
        self.state = Some(state);
    }
}

/// The canvas the game draws on: the page's `#runity`, or a new one over
/// the whole page.
#[cfg(target_arch = "wasm32")]
fn canvas() -> Option<web_sys::HtmlCanvasElement> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()?.document()?;
    if let Some(found) = document.get_element_by_id("runity") {
        return found.dyn_into().ok();
    }
    let canvas: web_sys::HtmlCanvasElement =
        document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_id("runity");
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
/// browser has no terminal. The page's `runityFailed(why)`, if it has one.
#[cfg(target_arch = "wasm32")]
pub fn fail(why: &str) {
    web_sys::console::error_1(&why.into());
    if let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::get(&window, &"runityFailed".into())
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
        // In the browser the page's layout sizes the canvas; a size here
        // would pin it in pixels.
        if cfg!(not(target_arch = "wasm32")) {
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
                state.surface.resize(&state.gpu, size.width, size.height);
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            // Captured, the pointer's position means nothing: only how far
            // it moves counts, and that comes as device motion.
            WindowEvent::CursorMoved { .. } if self.captured => {}
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
        gpu,
        surface,
        renderer,
        overlay,
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
