//! The Scene view: the engine's picture in a GPUI element.
//!
//! What a click, a drag, a wheel or a shortcut *means* is not decided here —
//! it is `Session::scene_view`, written once and tested without a window
//! (DNA, postulate 5). This element does three things: it tells the session
//! how big the view is, it hands it this frame's input, and it puts the
//! picture on the screen.
//!
//! How the picture gets there is DNA's open question 1, and this is the
//! answer's slow half: the engine's frame is read back into memory and
//! painted as a GPUI image. It works on every platform today and costs a
//! copy of the whole frame each time — `docs/questions/01-gpui-viewport.md`
//! has the numbers and what the zero-copy path would take. Everything
//! around this element stays the same when that path lands: only
//! [`SceneView::frame`] changes.

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    canvas, div, prelude::*, Bounds, Context, Corners, Entity, FocusHandle, Focusable,
    KeyDownEvent, KeyUpEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, RenderImage,
    ScrollDelta, ScrollWheelEvent, Window,
};
use image::{Frame, RgbaImage};
use runity::glam::Vec2;
use runity::input::{Input, InputEvent};
use runity_editor::Session;

use crate::keys;
use crate::theme::RADIUS_MD;

/// The view onto the document the window has open.
pub struct SceneView {
    /// Shared with the panels: what this view does to it, they show.
    session: Entity<Session>,
    /// What the panels last saw, so that they are told only when there is
    /// something new to show rather than once a frame.
    seen: Option<Stamp>,
    /// This frame's input, filled by the window's events and drained by
    /// [`Session::scene_view`] once a frame.
    input: Input,
    focus: FocusHandle,
    /// Where the view sat when it was last drawn, so that a mouse position
    /// in the window becomes a position in the picture.
    bounds: Bounds<Pixels>,
    /// The window's pixels per point when it was last drawn. The session
    /// renders at pixels, because a retina window that renders at points
    /// draws a soft picture and picks the wrong entity at the edges.
    scale: f32,
    /// The picture on screen. Kept so that its tile in GPUI's atlas can be
    /// dropped when the next frame replaces it; without that the atlas
    /// grows by a frame every frame.
    image: Option<Arc<RenderImage>>,
    drawn: Instant,
}

impl SceneView {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            seen: None,
            input: Input::new(),
            focus: cx.focus_handle(),
            bounds: Bounds::default(),
            scale: 1.0,
            image: None,
            drawn: Instant::now(),
        }
    }

    /// Where a window position lands in the picture, in its pixels.
    fn at(&self, position: gpui::Point<Pixels>) -> Vec2 {
        Vec2::new(
            f32::from(position.x - self.bounds.origin.x) * self.scale,
            f32::from(position.y - self.bounds.origin.y) * self.scale,
        )
    }

    /// Run one frame: size the session to the view, let it do what the
    /// input asks, and read the picture back.
    fn frame(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.bounds = bounds;
        self.scale = window.scale_factor();
        let width = ((f32::from(bounds.size.width) * self.scale).round() as u32).max(1);
        let height = ((f32::from(bounds.size.height) * self.scale).round() as u32).max(1);

        let dt = self.drawn.elapsed().as_secs_f32().min(0.1);
        self.drawn = Instant::now();
        let input = &self.input;
        let seen = self.seen.take();
        let (image, stamp) = self.session.update(cx, |session, cx| {
            if session.size() != (width, height) {
                session.resize(width, height);
            }
            // What it refuses it says in the Console; there is nothing for
            // the window to do about it here.
            let _ = session.scene_view(input, dt);
            session.render();
            let image = bgra_image(session.frame_pixels(), width, height).map(Arc::new);
            // The panels hear about it when what they show has changed —
            // or all the time while something moves under them: a drag, a
            // game being played.
            let stamp = Stamp::of(session);
            if seen.as_ref() != Some(&stamp) || session.is_dragging() || session.is_playing() {
                cx.notify();
            }
            (image, stamp)
        });
        self.seen = Some(stamp);
        self.input.begin_frame();

        // The frame that was on screen until now: its tile is free the
        // moment this one replaces it.
        if let Some(previous) = self.image.take() {
            let _ = window.drop_image(previous);
        }
        self.image = image;

        // The Scene view is a running picture — settlers walk, emitters
        // spray, a flythrough keeps flying — so the next frame is asked for
        // whether or not anything was clicked.
        window.request_animation_frame();
    }

    /// The keyboard's way in: a panel that is not the Scene view but wants
    /// its shortcuts (the Hierarchy: Delete, F, Ctrl Z) hands it here.
    pub fn focus(&self, window: &mut Window, cx: &mut gpui::App) {
        window.focus(&self.focus, cx);
    }
}

/// What the panels show, in brief: when it is the same as last frame's,
/// they are not asked to draw again.
#[derive(PartialEq)]
struct Stamp {
    selection: Vec<runity::EntityId>,
    steps: usize,
    redo: Option<String>,
    entities: usize,
    console: (usize, usize, usize),
    lines: usize,
    tool: runity::gizmo::Tool,
    playing: bool,
    paused: bool,
    hidden: usize,
    isolated: usize,
    grid: bool,
    space: bool,
    pivot: bool,
}

impl Stamp {
    fn of(session: &Session) -> Self {
        Self {
            selection: session.selection(),
            steps: session.undo_steps().len(),
            redo: session.redo_label(),
            entities: session.entity_count(),
            console: session.console_counts(),
            lines: session.console().iter().map(|l| l.count as usize).sum(),
            tool: session.tool(),
            playing: session.is_playing(),
            paused: session.is_paused(),
            hidden: session.hidden().len(),
            isolated: session.isolated().len(),
            grid: session.show_grid(),
            space: session.space() == runity_editor::Space::Global,
            pivot: session.pivot() == runity_editor::Pivot::Center,
        }
    }
}

/// The engine's RGBA frame as the BGRA image GPUI paints.
///
/// The swizzle is the price of this path: the engine renders in the byte
/// order a PNG wants and GPUI's atlas wants the other one.
fn bgra_image(pixels: &[u8], width: u32, height: u32) -> Option<RenderImage> {
    if pixels.len() != (width as usize) * (height as usize) * 4 {
        // The session was resized between the render and here; the next
        // frame is a few milliseconds away and will be the right size.
        return None;
    }
    let mut bgra = pixels.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(width, height, bgra)?;
    Some(RenderImage::new(vec![Frame::new(buffer)]))
}

impl Focusable for SceneView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SceneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        div()
            .id("scene-view")
            .track_focus(&self.focus)
            .key_context("SceneView")
            .size_full()
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        view.update(cx, |view, cx| {
                            view.frame(bounds, window, cx);
                            view.image.clone()
                        })
                    },
                    |bounds, image: Option<Arc<RenderImage>>, window, _cx| {
                        if let Some(image) = image {
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                Corners::all(RADIUS_MD),
                                image,
                                0,
                                false,
                            );
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // A click into the view is what gives it the keyboard:
                    // W, E, R and Ctrl Z belong to whatever was last
                    // touched, as in every editor.
                    window.focus(&this.focus, cx);
                    this.press(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.press(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.press(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.release(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.release(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.release(event.button, event.position, &event.modifiers, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                keys::sync_modifiers(&mut this.input, &event.modifiers);
                let at = this.at(event.position);
                this.input
                    .handle(&InputEvent::MouseMoved { x: at.x, y: at.y });
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                keys::sync_modifiers(&mut this.input, &event.modifiers);
                // Lines, because that is what a wheel notch is and what the
                // session's zoom is written against. A trackpad reports
                // pixels; forty of them is one notch, as in GPUI's own
                // scroll handling.
                let delta = match event.delta {
                    ScrollDelta::Lines(lines) => (lines.x, lines.y),
                    ScrollDelta::Pixels(pixels) => {
                        (f32::from(pixels.x) / 40.0, f32::from(pixels.y) / 40.0)
                    }
                };
                this.input.handle(&InputEvent::Scroll {
                    x: delta.0,
                    y: delta.1,
                });
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.key_down(event, cx);
                // Handled here: the panels around do not hear it again.
                cx.stop_propagation();
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                this.key_up(event, cx);
                cx.stop_propagation();
            }))
    }
}

impl SceneView {
    /// A key pressed here, or in a panel that hands its keys on.
    pub fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        keys::sync_modifiers(&mut self.input, &event.keystroke.modifiers);
        if let Some(key) = keys::key_of(&event.keystroke) {
            self.input.handle(&InputEvent::KeyDown(key));
        }
        cx.notify();
    }

    pub fn key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        keys::sync_modifiers(&mut self.input, &event.keystroke.modifiers);
        if let Some(key) = keys::key_of(&event.keystroke) {
            self.input.handle(&InputEvent::KeyUp(key));
        }
        cx.notify();
    }

    fn press(
        &mut self,
        button: gpui::MouseButton,
        position: gpui::Point<Pixels>,
        modifiers: &gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        keys::sync_modifiers(&mut self.input, modifiers);
        let at = self.at(position);
        self.input
            .handle(&InputEvent::MouseMoved { x: at.x, y: at.y });
        if let Some(button) = keys::button_of(button) {
            self.input.handle(&InputEvent::MouseDown(button));
        }
        cx.notify();
    }

    fn release(
        &mut self,
        button: gpui::MouseButton,
        position: gpui::Point<Pixels>,
        modifiers: &gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        keys::sync_modifiers(&mut self.input, modifiers);
        let at = self.at(position);
        self.input
            .handle(&InputEvent::MouseMoved { x: at.x, y: at.y });
        if let Some(button) = keys::button_of(button) {
            self.input.handle(&InputEvent::MouseUp(button));
        }
        cx.notify();
    }
}
