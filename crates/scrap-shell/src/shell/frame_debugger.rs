//! The Frame Debugger's window over a running game, as Unity's is: F9
//! stops the game on the frame it is drawing, and lists that frame's passes
//! and draws; picking one shows the picture of its pass as it stood after
//! it ([`scrap_render::frame_debugger`]). F9 again, and the game goes on.
//!
//! Every game the shell runs has it, and none has to do anything for it:
//! the shell holds the frame the game last made and draws it again each
//! turn, stopped where the list's selection is, while the game's steps and
//! `frame` wait. The window is the overlay's draw list, like any HUD.
//!
//! Keys: ↑/↓ an event, ←/→ a pass, PgUp/PgDn a page, Home the whole
//! frame, End the last event; the mouse picks a row and the wheel scrolls.

use crate::input::{Input, Key, MouseButton};
use crate::ui::{Quad, TextRun, Ui};
use scrap_core::glam::Vec4;
use scrap_render::frame_debugger::{FrameCapture, Step};

/// The key that opens and closes it.
pub(super) const KEY: Key = Key::F9;

/// The window's state: open or not, the event picked, the list's scroll.
#[derive(Default)]
pub(super) struct Panel {
    pub(super) open: bool,
    /// The event the frame stops at; `None` draws it whole.
    selected: Option<usize>,
    /// The first row the list shows.
    top: usize,
    /// The pointer was captured when it opened: given back when it closes.
    pub(super) was_captured: bool,
    pub(super) ui: Ui,
}

/// Sizes, in pixels of a screen this tall: a Retina window's text is as
/// big to the eye as a plain one's.
struct Metrics {
    scale: f32,
    row: f32,
    text: f32,
    list_width: f32,
    header: f32,
}

impl Metrics {
    fn of(size: (u32, u32)) -> Self {
        let scale = (size.1 as f32 / 800.0).clamp(1.0, 3.0);
        Self {
            scale,
            row: 17.0 * scale,
            text: 12.5 * scale,
            list_width: (size.0 as f32 * 0.36).max(260.0 * scale).min(size.0 as f32 * 0.6),
            header: 44.0 * scale,
        }
    }

    fn rows(&self, size: (u32, u32)) -> usize {
        (((size.1 as f32 - self.header) / self.row).floor() as usize).max(1)
    }
}

const BACK: Vec4 = Vec4::new(0.09, 0.09, 0.1, 0.94);
const TEXT: Vec4 = Vec4::new(0.86, 0.86, 0.88, 1.0);
const DIM: Vec4 = Vec4::new(0.55, 0.56, 0.6, 1.0);
const PASS: Vec4 = Vec4::new(0.55, 0.78, 1.0, 1.0);
const PICKED: Vec4 = Vec4::new(0.22, 0.4, 0.7, 1.0);
const HOVER: Vec4 = Vec4::new(1.0, 1.0, 1.0, 0.06);

impl Panel {
    /// The event the frame is drawn stopped at.
    pub(super) fn stop(&self) -> Option<usize> {
        self.selected
    }

    /// Where the picture goes: right of the list, above the details.
    pub(super) fn picture_area(size: (u32, u32)) -> (f32, f32, f32, f32) {
        let m = Metrics::of(size);
        let (w, h) = (size.0 as f32, size.1 as f32);
        let x = m.list_width + 8.0 * m.scale;
        (x, 8.0 * m.scale, (w - x - 8.0 * m.scale).max(1.0), (h * 0.72).max(1.0))
    }

    /// The keys and the pointer, on the list of `capture`.
    pub(super) fn update(&mut self, input: &Input, capture: Option<&FrameCapture>, size: (u32, u32)) {
        let Some(capture) = capture else { return };
        let count = capture.events.len();
        if count == 0 {
            self.selected = None;
            return;
        }
        let m = Metrics::of(size);
        let page = m.rows(size);
        let last = count - 1;
        let at = self.selected;
        let pass_starts: Vec<usize> = (0..count).filter(|&i| matches!(capture.events[i].step, Step::Pass(_))).collect();
        let mut pick = at;
        if input.pressed(Key::Down) {
            pick = Some(at.map_or(0, |i| (i + 1).min(last)));
        }
        if input.pressed(Key::Up) {
            pick = at.and_then(|i| i.checked_sub(1));
        }
        if input.pressed(Key::PageDown) {
            pick = Some(at.map_or(0, |i| (i + page).min(last)));
        }
        if input.pressed(Key::PageUp) {
            pick = at.map(|i| i.saturating_sub(page));
        }
        if input.pressed(Key::Right) {
            pick = pass_starts.iter().copied().find(|&p| at.is_none_or(|i| p > i)).or(at);
        }
        if input.pressed(Key::Left) {
            pick = at.and_then(|i| pass_starts.iter().copied().rev().find(|&p| p < i));
        }
        if input.pressed(Key::Home) {
            pick = None;
        }
        if input.pressed(Key::End) {
            pick = Some(last);
        }
        // The wheel scrolls; a click on a row picks it.
        let wheel = input.scroll().y;
        if wheel != 0.0 {
            let lines = (wheel.abs() / 3.0).ceil().max(1.0) as usize;
            self.top = if wheel > 0.0 { self.top.saturating_sub(lines) } else { (self.top + lines).min(last) };
        }
        let mouse = input.mouse_position();
        if input.mouse_pressed(MouseButton::Left) && mouse.x < m.list_width {
            if mouse.y < m.header {
                pick = None;
            } else {
                let row = ((mouse.y - m.header) / m.row) as usize + self.top;
                if row < count {
                    pick = Some(row);
                }
            }
        }
        self.selected = pick.map(|p| p.min(last));
        // The selection kept in sight when the keys moved it.
        if pick != at {
            if let Some(p) = self.selected {
                if p < self.top {
                    self.top = p;
                } else if p >= self.top + page {
                    self.top = p + 1 - page;
                }
            }
        }
        self.top = self.top.min(count.saturating_sub(page.min(count)));
    }

    /// The window drawn into [`Self::ui`]: the list, and the picked event's
    /// particulars under the picture.
    pub(super) fn draw(&mut self, capture: Option<&FrameCapture>, input: &Input, size: (u32, u32), times: &[(String, f32)]) {
        let ui = &mut self.ui;
        ui.clear();
        let m = Metrics::of(size);
        let (w, h) = (size.0 as f32, size.1 as f32);
        ui.quad(Quad::new(0.0, 0.0, m.list_width, h, BACK));
        let pad = 8.0 * m.scale;
        let Some(capture) = capture else {
            ui.text(TextRun::new(pad, pad, m.text, TEXT, "Frame Debugger — recording the frame…"));
            return;
        };
        let total: f32 = times.iter().map(|t| t.1).sum();
        let title = format!(
            "Frame Debugger · {} draws · {} tris{}",
            capture.draws(),
            scrap_render::frame_debugger::count(capture.triangles()),
            if total > 0.0 { format!(" · gpu {total:.2} ms") } else { String::new() }
        );
        ui.text(TextRun::new(pad, pad, m.text * 1.1, TEXT, title));
        let whole = self.selected.is_none();
        ui.text(TextRun::new(
            pad,
            pad + m.row,
            m.text,
            if whole { PASS } else { DIM },
            "▸ whole frame (Home) · ↑↓ event · ←→ pass · F9 back to the game",
        ));
        ui.push_clip(0.0, m.header, m.list_width, h - m.header);
        let mouse = input.mouse_position();
        for (row, i) in (self.top..capture.events.len()).take(m.rows(size) + 1).enumerate() {
            let y = m.header + row as f32 * m.row;
            let picked = self.selected == Some(i);
            // Stopped here: everything after it in its pass is not drawn.
            let after = self.selected.is_some_and(|s| i > s);
            if picked {
                ui.quad(Quad::new(0.0, y, m.list_width, m.row, PICKED));
            } else if mouse.x < m.list_width && mouse.y >= y && mouse.y < y + m.row {
                ui.quad(Quad::new(0.0, y, m.list_width, m.row, HOVER));
            }
            let is_pass = matches!(capture.events[i].step, Step::Pass(_));
            let colour = match (is_pass, after && !picked) {
                (_, true) => DIM,
                (true, false) => PASS,
                (false, false) => TEXT,
            };
            let run = TextRun::new(pad, y + (m.row - m.text) * 0.4, m.text, colour, format!("{i:>5}  {}", capture.line(i)));
            ui.text(run);
            // A pass's time on the GPU, every pass of its name together
            // (the four cascades are one time), on its first row.
            let e = &capture.events[i];
            let first_of_name = capture.events.iter().find(|o| o.pass == e.pass).map(|o| o.pass_number) == Some(e.pass_number);
            if is_pass && first_of_name {
                if let Some((_, ms)) = times.iter().find(|t| t.0 == e.pass) {
                    let ms = TextRun::new(pad, y + (m.row - m.text) * 0.4, m.text, DIM, format!("{ms:.2} ms"))
                        .within(m.list_width - pad * 2.0, 1.0);
                    ui.text(ms);
                }
            }
        }
        ui.pop_clip();
        // The picked event's particulars, under the picture.
        let (px, py, _, ph) = Self::picture_area(size);
        let top = py + ph + pad;
        ui.quad(Quad::new(px - pad, top - pad * 0.5, w - px + pad, h - top + pad * 0.5, BACK));
        let mut lines = match self.selected {
            Some(i) => capture.details(i),
            None => vec!["The whole frame: pick an event to see the frame as it stood after it.".to_string()],
        };
        match (&capture.picture, self.selected) {
            (Some(p), Some(_)) => lines.insert(0, format!("showing {} of pass {}, {}x{}", p.target, p.pass, p.size.0, p.size.1)),
            (None, Some(_)) => lines.insert(0, "no picture of this pass: the finished frame shows".to_string()),
            _ => {}
        }
        for (n, line) in lines.iter().enumerate() {
            let y = top + n as f32 * m.row;
            if y + m.row > h {
                break;
            }
            ui.text(TextRun::new(px, y, m.text, if n == 0 { PASS } else { TEXT }, line.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputEvent;
    use scrap_core::glam::{Mat4, Vec3};
    use scrap_render::render::{Camera, Draw, Frame, Renderer, Sky, SkyMode, TextureHandle};

    fn key(input: &mut Input, key: Key) {
        input.begin_frame();
        input.handle(&InputEvent::KeyDown(key));
        input.handle(&InputEvent::KeyUp(key));
    }

    /// The window as the shell draws it, offscreen: the frame stopped where
    /// the keys moved the selection, its picture, the list over it.
    #[test]
    fn the_keys_walk_the_frame_and_the_window_shows_where_it_stopped() {
        let Ok(gpu) = scrap_gpu::gpu::Gpu::headless_blocking(false) else {
            eprintln!("skipping: no adapter");
            return;
        };
        let size = (1280u32, 720u32);
        let target = scrap_gpu::gpu::OffscreenTarget::new(&gpu, size.0, size.1);
        let mut renderer = Renderer::new(&gpu, &target);
        let mut overlay = crate::ui_render::UiRenderer::new(&gpu, &target);
        let mut boxes = Vec::new();
        for (name, x, c) in [("red box", -1.0, [1.0, 0.1, 0.1]), ("blue box", 1.0, [0.1, 0.2, 1.0])] {
            let mut cube = scrap_geometry::builtin::cube(1.0);
            cube.name = name.into();
            boxes.push(Draw {
                mesh: renderer.upload_mesh_owned(&gpu, &cube),
                transform: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
                texture: TextureHandle::WHITE,
                material: scrap_render::Material::new(c[0], c[1], c[2]),
                pose: None,
            });
        }
        let frame = Frame {
            sky: Sky { mode: SkyMode::Color, ..Default::default() },
            camera: Camera { position: Vec3::new(0.0, 0.5, 4.0), target: Vec3::ZERO, ..Camera::default() },
            clear_color: Vec3::new(0.05, 0.05, 0.05),
            draws: boxes,
            ..Frame::default()
        };
        let mut panel = Panel { open: true, ..Default::default() };
        let mut input = Input::new();
        // As a turn of the shell's draw_debugged goes.
        let mut turn = |panel: &mut Panel, input: &Input| {
            panel.update(input, renderer.frame_capture(), size);
            renderer.debug_frame(panel.stop());
            renderer.render(&gpu, &target, &frame);
            renderer.show_debug_picture(&gpu, &target.view, Panel::picture_area(size));
            let times = renderer.gpu_times();
            panel.draw(renderer.frame_capture(), input, size, &times);
            overlay.render(&gpu, &target, &panel.ui);
            renderer.frame_capture().cloned().unwrap()
        };
        let capture = turn(&mut panel, &input);
        assert_eq!(panel.stop(), None);
        // → to the passes in turn, until the scene's.
        while !capture.events.get(panel.stop().unwrap_or(0)).is_some_and(|e| e.pass == "scene" && panel.stop().is_some()) {
            key(&mut input, Key::Right);
            turn(&mut panel, &input);
            assert!(panel.stop().is_some(), "→ always lands on a pass");
        }
        // ↓ onto its first draw.
        key(&mut input, Key::Down);
        let capture = turn(&mut panel, &input);
        let stop = panel.stop().unwrap();
        assert!(matches!(capture.events[stop].step, Step::Draw(_)), "{}", capture.text());
        assert_eq!(capture.picture.as_ref().map(|p| p.pass), Some("scene"));
        let text: Vec<&str> = panel.ui.texts.iter().map(|t| t.text.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("Frame Debugger")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("red box") || t.contains("blue box")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("showing hdr of pass scene")), "{text:?}");
        let pixels = target.read_rgba(&gpu);
        let shot = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/frame_debugger_window.png");
        let _ = std::fs::create_dir_all(shot.parent().unwrap());
        image::save_buffer(&shot, &pixels, size.0, size.1, image::ExtendedColorType::Rgba8).unwrap();
        eprintln!("{}", shot.display());
        // Home: the whole frame again.
        key(&mut input, Key::Home);
        turn(&mut panel, &input);
        assert_eq!(panel.stop(), None);
    }
}
