//! A window the caller's own loop drives ([`Pumped`]): for code that
//! cannot be turned inside out into [`super::Game`]'s callbacks — a bot
//! that walks a whole dungeon in one call, a tool that draws as it works —
//! and wants each of its steps seen in a real window, at the screen's
//! pace. It opens the window and the device, then each [`Pumped::show`]
//! takes the window's events (keys into [`Pumped::input`], a close, a
//! resize) without waiting and draws one frame. Desktop only: winit pumps
//! events on macOS, Windows and Linux.

use std::sync::Arc;
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowId};

use super::{draw_frame, running, translate, Drawn, Running, WindowConfig};
use crate::gpu::Gpu;
use crate::input::Input;
use crate::render::{Frame, Renderer};

/// A window and its device, drawn when the caller says.
pub struct Pumped {
    event_loop: EventLoop<Running>,
    app: App,
}

struct App {
    config: WindowConfig,
    state: Option<Running>,
    input: Input,
    closed: bool,
    failed: Option<String>,
}

impl ApplicationHandler<Running> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let mut attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.config.width, self.config.height));
        if std::env::var("SCRAP_FULLSCREEN").is_ok_and(|v| v == "1") {
            attributes = attributes.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        let made = event_loop
            .create_window(attributes)
            .map_err(|e| e.to_string())
            .and_then(|window| {
                let gpu = Gpu::headless_blocking(false).map_err(|e| e.to_string())?;
                running(gpu, Arc::new(window))
            });
        match made {
            Ok(state) => self.state = Some(state),
            Err(e) => self.failed = Some(e),
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => self.closed = true,
            WindowEvent::Resized(size) => {
                if let Some(state) = self.state.as_mut() {
                    if let Some(d) = state.drawing.as_mut() {
                        d.surface.resize(&state.gpu, size.width, size.height);
                    }
                }
            }
            _ => {}
        }
        for e in translate(&event) {
            self.input.handle(&e);
        }
    }
}

impl Pumped {
    /// Open the window and its device.
    pub fn open(config: WindowConfig) -> anyhow::Result<Self> {
        let mut event_loop = EventLoop::<Running>::with_user_event().build()?;
        let mut app = App { config, state: None, input: Input::new(), closed: false, failed: None };
        for _ in 0..500 {
            if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut app) {
                anyhow::bail!("the window's loop ended ({code})");
            }
            if let Some(e) = app.failed.take() {
                anyhow::bail!(e);
            }
            if app.state.is_some() {
                return Ok(Self { event_loop, app });
            }
        }
        anyhow::bail!("no window after five seconds")
    }

    fn drawing(&mut self) -> &mut super::Drawing {
        let state = self.app.state.as_mut().expect("opened");
        state.drawing.as_deref_mut().expect("never handed away")
    }

    pub fn gpu(&self) -> &Gpu {
        &self.app.state.as_ref().expect("opened").gpu
    }

    /// The device, the renderer and the overlay's renderer: for uploads,
    /// a game's font, its pictures.
    pub fn parts(&mut self) -> (&Gpu, &mut Renderer, &mut crate::ui_render::UiRenderer) {
        let state = self.app.state.as_mut().expect("opened");
        let d = state.drawing.as_deref_mut().expect("never handed away");
        (&state.gpu, &mut d.renderer, &mut d.overlay)
    }

    /// The overlay's renderer (a game's font goes in here).
    pub fn overlay(&mut self) -> &mut crate::ui_render::UiRenderer {
        &mut self.drawing().overlay
    }

    /// The drawable's size in pixels.
    pub fn size(&mut self) -> (u32, u32) {
        let d = self.drawing();
        (d.surface.width(), d.surface.height())
    }

    /// What the keys and the mouse did since the last frame shown.
    pub fn input(&self) -> &Input {
        &self.app.input
    }

    /// Whether the window has been closed (its close button).
    pub fn closed(&self) -> bool {
        self.app.closed
    }

    /// Take the window's events and draw `frame` with `ui` over it; false
    /// once the window is closed.
    pub fn show(&mut self, frame: &Frame, ui: &crate::ui::Ui) -> bool {
        self.app.input.begin_frame();
        if let PumpStatus::Exit(_) = self.event_loop.pump_app_events(Some(Duration::ZERO), &mut self.app) {
            self.app.closed = true;
        }
        if self.app.closed {
            return false;
        }
        let state = self.app.state.as_mut().expect("opened");
        let (width, height) = state.window.inner_size().into();
        let d = state.drawing.as_deref_mut().expect("never handed away");
        d.surface.resize(&state.gpu, width, height);
        let mut times = Vec::new();
        match draw_frame(&state.gpu, &d.surface, &mut d.renderer, &mut d.overlay, None, frame, ui, &mut times) {
            Drawn::Shown => {}
            Drawn::Outdated => {
                d.surface.reconfigure(&state.gpu);
                std::thread::sleep(Duration::from_millis(8));
            }
            Drawn::Failed(e) => {
                eprintln!("{e}");
                return false;
            }
        }
        true
    }
}
