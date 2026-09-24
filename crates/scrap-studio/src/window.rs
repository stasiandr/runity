//! The window: winit around a [`Studio`].
//!
//! All this does is carry events in and frames out. The surface is made on
//! the session's own GPU, so the Scene view's texture and the window's
//! frame live on one device and the UI shows the one inside the other with
//! no copy. Events come through the engine's own translation
//! (`scrap::shell::translate`), in physical pixels, and go on to the studio
//! in logical ones.
//!
//! On macOS the menus are the system's menu bar (`crate::native_menu`); a
//! line chosen there comes back as a user event, and a key that chose one
//! goes on to the studio as that key.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scrap::input::{InputEvent, Key};
use scrap::surface::{Surface, SurfaceError};
use scrap_ui::render::UiRenderer;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::native_menu::Chosen;
use crate::studio::Studio;

struct Running {
    window: Arc<Window>,
    surface: Surface,
    renderer: UiRenderer,
    studio: Studio,
    title: String,
    /// Whether the input method is on: while a text field is focused.
    ime: bool,
    /// Text that came as a key and text an input method committed, with
    /// when: some platforms send both for one keystroke, and it is typed
    /// once.
    last_key_text: Option<(String, Instant)>,
    last_commit: Option<(String, Instant)>,
    /// The pointer is held for the game in the view, as it asked.
    captured: bool,
}

/// A panel's own window: the same UI tree, drawn from the panel's frame.
struct FloatWindow {
    name: String,
    window: Arc<Window>,
    surface: Surface,
    renderer: UiRenderer,
    /// The studio's picture generation this renderer has been given.
    seen: u64,
}

impl FloatWindow {
    fn draw(&mut self, studio: &mut Studio) {
        let gpu = studio.session.gpu();
        match self.surface.begin_frame() {
            Ok(frame) => {
                let view = frame.ui_view();
                let (w, h) = (frame.width, frame.height);
                studio.draw_float(&self.name, &mut self.renderer, &mut self.seen, &view, w, h);
                frame.present(studio.session.gpu());
            }
            Err(SurfaceError::Outdated | SurfaceError::Lost) => self.surface.reconfigure(gpu),
            Err(_) => {}
        }
    }
}

struct App {
    /// The session until the window exists to put it in.
    pending: Option<(scrap_editor::Session, String)>,
    running: Option<Running>,
    /// Panels torn off into windows of their own.
    floats: Vec<FloatWindow>,
    /// Closing over unsaved work asks once, in the Console.
    close_asked: bool,
    /// Keys a menu of the menu bar took and handed on: let go after the
    /// frame that saw them pressed.
    keys_up: Vec<Key>,
    /// The system's menu bar, and where it sends what is chosen.
    #[cfg(target_os = "macos")]
    menu: Option<crate::native_menu::mac::MenuBar>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    proxy: winit::event_loop::EventLoopProxy<Chosen>,
    /// When the next frame is due. Vsync paces a visible window; this
    /// paces one that is hidden or on a display that does not, which
    /// would otherwise draw as fast as the GPU goes for nobody.
    next: Instant,
}

/// The most frames a second the editor draws.
const FRAME: Duration = Duration::from_micros(1_000_000 / 120);

impl App {
    /// The window asked to close, by its button or by Quit: over unsaved
    /// work, once more first.
    fn close(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(run) = self.running.as_mut() {
            if run.studio.session.is_modified() && !self.close_asked {
                self.close_asked = true;
                run.studio.session.say(
                    scrap_editor::console::Level::Warning,
                    "unsaved changes: save first (Ctrl/Cmd S), or close again to discard them",
                );
                run.studio.refresh();
                run.window.request_redraw();
                return;
            }
        }
        event_loop.exit();
    }
}

impl ApplicationHandler<Chosen> for App {
    /// A line of the system's menu bar.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn user_event(&mut self, event_loop: &ActiveEventLoop, chosen: Chosen) {
        #[cfg(target_os = "macos")]
        {
            use crate::native_menu::mac::Asked;
            let Some(asked) = self.menu.as_ref().and_then(|m| m.asked(&chosen)) else {
                return;
            };
            let Some(run) = self.running.as_mut() else {
                return;
            };
            match asked {
                Asked::Quit => return self.close(event_loop),
                // The key's own way: the studio answers keys (see
                // `native_menu`), a field first.
                Asked::Key(shortcut) => {
                    run.studio.handle(&InputEvent::KeyDown(shortcut.key));
                    self.keys_up.push(shortcut.key);
                }
                Asked::Run(action) => run.studio.run(action),
            }
            run.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(run) = self.running.as_ref() else {
            return;
        };
        // Idle, a few frames a second: the disk is still watched and
        // emitters still play, but nothing burns a core for a still scene.
        // Idle, as many a second as Preferences › General says (four).
        let due = if run.studio.wants_frame() {
            self.next
        } else {
            self.next + run.studio.personal().idle().saturating_sub(FRAME)
        };
        if Instant::now() >= due {
            run.window.request_redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(due));
        }
    }

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
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut studio = studio;
        #[cfg(target_os = "macos")]
        {
            // After launch, or winit's own menu would replace it.
            // With the person's keys (Preferences › Keys).
            self.menu = Some(crate::native_menu::mac::MenuBar::install(studio.menu_bar()));
            let proxy = std::sync::Mutex::new(self.proxy.clone());
            crate::native_menu::mac::listen(move |chosen| {
                if let Ok(proxy) = proxy.lock() {
                    let _ = proxy.send_event(chosen);
                }
            });
            studio.set_native_menu(true);
        }
        let renderer = studio.renderer(surface.format());
        window.request_redraw();
        self.running = Some(Running {
            window,
            surface,
            renderer,
            studio,
            title: String::new(),
            ime: false,
            last_key_text: None,
            last_commit: None,
            captured: false,
        });
    }

    /// Raw motion, for the game in the view looking around while it holds
    /// the pointer.
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        let Some(run) = self.running.as_mut() else {
            return;
        };
        if let (true, winit::event::DeviceEvent::MouseMotion { delta }) = (run.captured, event) {
            run.studio.handle(&InputEvent::MouseMotion {
                dx: delta.0 as f32,
                dy: delta.1 as f32,
            });
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(run) = self.running.as_mut() else {
            return;
        };
        if id != run.window.id() {
            if let Some(float) = self.floats.iter_mut().find(|f| f.window.id() == id) {
                float_event(run, float, &event);
            }
            return;
        }
        match &event {
            WindowEvent::CloseRequested => return self.close(event_loop),
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
            WindowEvent::Ime(ime) => {
                match ime {
                    winit::event::Ime::Preedit(text, _) => run.studio.ime_preedit(text),
                    winit::event::Ime::Commit(text) => {
                        let twice = run.last_key_text.as_ref().is_some_and(|(t, at)| {
                            t == text && at.elapsed() < Duration::from_millis(60)
                        });
                        if !twice {
                            run.studio.handle(&InputEvent::Text(text.clone()));
                        }
                        run.last_commit = Some((text.clone(), Instant::now()));
                    }
                    _ => {}
                }
                return;
            }
            WindowEvent::RedrawRequested => {
                run.studio.frame();
                for key in self.keys_up.drain(..) {
                    run.studio.handle(&InputEvent::KeyUp(key));
                }
                #[cfg(target_os = "macos")]
                if let Some(menu) = &mut self.menu {
                    menu.refresh(&run.studio);
                }
                let cursor = match run.studio.cursor() {
                    crate::studio::Cursor::Default => winit::window::CursorIcon::Default,
                    crate::studio::Cursor::Text => winit::window::CursorIcon::Text,
                    crate::studio::Cursor::ResizeColumn => winit::window::CursorIcon::ColResize,
                    crate::studio::Cursor::ResizeRow => winit::window::CursorIcon::RowResize,
                    crate::studio::Cursor::Brush => winit::window::CursorIcon::Crosshair,
                };
                run.window.set_cursor(cursor);
                // The game in the view looks around with the pointer held.
                let capture = run.studio.captures_cursor();
                if capture != run.captured {
                    set_captured(&run.window, capture);
                    run.captured = capture;
                }
                // The input method only while typing, its candidates at
                // the caret.
                let typing = run.studio.typing();
                if typing != run.ime {
                    run.window.set_ime_allowed(typing);
                    run.ime = typing;
                }
                if typing {
                    if let Some(r) = run.studio.ime_area() {
                        run.window.set_ime_cursor_area(
                            winit::dpi::LogicalPosition::new(r.x, r.y),
                            winit::dpi::LogicalSize::new(r.width.max(1.0), r.height),
                        );
                    }
                }
                let title = run.studio.title();
                if title != run.title {
                    run.window.set_title(&title);
                    // The dot in the close button, as a Mac document has.
                    #[cfg(target_os = "macos")]
                    {
                        use winit::platform::macos::WindowExtMacOS;
                        run.window
                            .set_document_edited(run.studio.session.is_modified());
                    }
                    run.title = title;
                }
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
                sync_floats(event_loop, run, &mut self.floats);
                for float in &mut self.floats {
                    float.draw(&mut run.studio);
                }
                self.next = Instant::now() + FRAME;
                return;
            }
            _ => {}
        }
        let scale = run.window.scale_factor() as f32;
        for input in scrap::shell::translate(&event) {
            let input = match input {
                InputEvent::MouseMoved { x, y } => InputEvent::MouseMoved {
                    x: x / scale,
                    y: y / scale,
                },
                InputEvent::Text(text) => {
                    let twice = run.last_commit.as_ref().is_some_and(|(t, at)| {
                        *t == text && at.elapsed() < Duration::from_millis(60)
                    });
                    run.last_key_text = Some((text.clone(), Instant::now()));
                    if twice {
                        continue;
                    }
                    InputEvent::Text(text)
                }
                other => other,
            };
            run.studio.handle(&input);
        }
    }
}

/// Hold the pointer in the window and hide it, or let it go: locked where
/// the platform can, else confined.
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

/// Open a window for each panel the studio has floating, and close the
/// ones it has docked back.
fn sync_floats(event_loop: &ActiveEventLoop, run: &mut Running, floats: &mut Vec<FloatWindow>) {
    let wanted = run.studio.floating();
    floats.retain(|f| wanted.iter().any(|(name, _)| *name == f.name));
    // One asked for again (⌘, with Preferences open): to the front.
    if let Some(name) = run.studio.take_raise() {
        if let Some(float) = floats.iter().find(|f| f.name == name) {
            float.window.focus_window();
        }
    }
    for (name, title) in wanted {
        if floats.iter().any(|f| f.name == name) {
            continue;
        }
        // Where it was left, and as big.
        let ([width, height], at) = run.studio.float_place(&name);
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(width, height));
        if let Some([x, y]) = at {
            attrs = attrs.with_position(LogicalPosition::new(x, y));
        }
        let Ok(window) = event_loop.create_window(attrs) else {
            continue;
        };
        let window = Arc::new(window);
        let Ok(surface) = Surface::from_window(run.studio.session.gpu(), window.clone()) else {
            continue;
        };
        let renderer = run.studio.renderer(surface.format());
        floats.push(FloatWindow {
            name,
            window,
            surface,
            renderer,
            seen: u64::MAX,
        });
    }
}

/// An event from a floating panel's window.
fn float_event(run: &mut Running, float: &mut FloatWindow, event: &WindowEvent) {
    let scale = float.window.scale_factor() as f32;
    match event {
        // Closing the window docks the panel; the window goes at the next
        // frame, when the studio no longer lists it.
        WindowEvent::CloseRequested => {
            run.studio.close_float(&float.name);
            run.window.request_redraw();
        }
        WindowEvent::Resized(size) => {
            float
                .surface
                .resize(run.studio.session.gpu(), size.width, size.height);
            run.studio.resize_float(
                &float.name,
                size.width as f32 / scale,
                size.height as f32 / scale,
            );
            run.window.request_redraw();
        }
        // Moved: it opens there next time.
        WindowEvent::Moved(at) => {
            let at = at.to_logical::<f32>(scale as f64);
            run.studio.moved_float(&float.name, at.x, at.y);
        }
        WindowEvent::RedrawRequested => float.draw(&mut run.studio),
        _ => {
            for input in scrap::shell::translate(event) {
                let input = match input {
                    InputEvent::MouseMoved { x, y } => InputEvent::MouseMoved {
                        x: x / scale,
                        y: y / scale,
                    },
                    other => other,
                };
                run.studio.handle_float(&float.name, &input);
            }
            run.window.request_redraw();
        }
    }
}

/// Open the window over `session` and run until it closes.
pub fn run(session: scrap_editor::Session, title: String) {
    let mut builder = EventLoop::<Chosen>::with_user_event();
    // The menu bar is ours (`crate::native_menu`), not winit's default.
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_default_menu(false);
    }
    let event_loop = match builder.build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("no event loop: {e}");
            return;
        }
    };
    let mut app = App {
        pending: Some((session, title)),
        running: None,
        floats: Vec::new(),
        close_asked: false,
        keys_up: Vec::new(),
        #[cfg(target_os = "macos")]
        menu: None,
        proxy: event_loop.create_proxy(),
        next: Instant::now(),
    };

    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("{e}");
    }
}
