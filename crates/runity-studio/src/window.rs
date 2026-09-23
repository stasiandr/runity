//! The window: winit around a [`Studio`].
//!
//! All this does is carry events in and frames out. The surface is made on
//! the session's own GPU, so the Scene view's texture and the window's
//! frame live on one device and the UI shows the one inside the other with
//! no copy. Events come through the engine's own translation
//! (`runity::shell::translate`), in physical pixels, and go on to the studio
//! in logical ones.

use std::sync::Arc;

use runity::input::InputEvent;
use runity::surface::{Surface, SurfaceError};
use runity_ui::render::UiRenderer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::studio::Studio;

struct Running {
    window: Arc<Window>,
    surface: Surface,
    renderer: UiRenderer,
    studio: Studio,
}

struct App {
    /// The session until the window exists to put it in.
    pending: Option<(runity_editor::Session, String)>,
    running: Option<Running>,
    /// Closing over unsaved work asks once, in the Console.
    close_asked: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let Some((session, title)) = self.pending.take() else {
            return;
        };
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1440.0, 900.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("no window: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface = match Surface::from_window(session.gpu(), window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("no surface: {e}");
                event_loop.exit();
                return;
            }
        };
        let scale = window.scale_factor() as f32;
        let size = window.inner_size();
        let studio = Studio::new(
            session,
            size.width as f32 / scale,
            size.height as f32 / scale,
            scale,
        );
        let renderer = studio.renderer(surface.format());
        window.request_redraw();
        self.running = Some(Running {
            window,
            surface,
            renderer,
            studio,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(run) = self.running.as_mut() else {
            return;
        };
        match &event {
            WindowEvent::CloseRequested => {
                if run.studio.session.is_modified() && !self.close_asked {
                    self.close_asked = true;
                    run.studio.session.say(
                        runity_editor::console::Level::Warning,
                        "unsaved changes: save first (Ctrl/Cmd S), or close again to discard them",
                    );
                    run.studio.refresh();
                    return;
                }
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                run.surface
                    .resize(run.studio.session.gpu(), size.width, size.height);
                let scale = run.window.scale_factor() as f32;
                run.studio
                    .resize(size.width as f32 / scale, size.height as f32 / scale, scale);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = run.window.inner_size();
                let scale = *scale_factor as f32;
                run.studio
                    .resize(size.width as f32 / scale, size.height as f32 / scale, scale);
            }
            WindowEvent::DroppedFile(path) => run.studio.drop_file(path),
            WindowEvent::RedrawRequested => {
                run.studio.frame();
                let gpu = run.studio.session.gpu();
                match run.surface.begin_frame() {
                    Ok(frame) => {
                        let view = frame.ui_view();
                        let (w, h) = (frame.width, frame.height);
                        run.studio.draw(&mut run.renderer, &view, w, h);
                        frame.present(run.studio.session.gpu());
                    }
                    Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                        run.surface.reconfigure(gpu)
                    }
                    Err(_) => {}
                }
                run.window.request_redraw();
                return;
            }
            _ => {}
        }
        let scale = run.window.scale_factor() as f32;
        for input in runity::shell::translate(&event) {
            let input = match input {
                InputEvent::MouseMoved { x, y } => InputEvent::MouseMoved {
                    x: x / scale,
                    y: y / scale,
                },
                other => other,
            };
            run.studio.handle(&input);
        }
    }
}

/// Open the window over `session` and run until it closes.
pub fn run(session: runity_editor::Session, title: String) {
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("no event loop: {e}");
            return;
        }
    };
    let mut app = App {
        pending: Some((session, title)),
        running: None,
        close_asked: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("{e}");
    }
}
