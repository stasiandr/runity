//! The Scene view's hands: what a click, a drag, a wheel and a shortcut do.
//!
//! Unity's Scene view behaviour, as one function over a frame of
//! [`Input`]: whatever window hosts the view forwards its events and calls
//! [`Session::scene_view`] once a frame, and everything a person does with
//! the mouse and keyboard happens here — tested headless, the same in every
//! window (DNA, open question 1 is about how the window shows the frame,
//! not about this).
//!
//! | input | does |
//! |---|---|
//! | click | select what is under it; shift adds; empty space clears |
//! | drag from empty space | select everything the box touches; shift adds |
//! | drag a handle | move, turn or stretch the selection — one undo step |
//! | Ctrl Shift + drag a move handle | onto whatever is under the cursor |
//! | alt + drag | orbit |
//! | right drag | look around; with W A S D Q E held, fly (shift: faster, wheel: speed) |
//! | middle drag | pan |
//! | wheel | zoom |
//! | W / E / R | move / rotate / scale tool |
//! | X | handles along the world's axes or the entity's own |
//! | Z | handles on the entity's pivot or the selection's middle |
//! | F | frame the selection |
//! | End | down onto what is beneath |
//! | H / Shift H | hide the selection / show it alone (again: all) |
//! | Delete | delete the selection |
//! | Ctrl D | duplicate |
//! | Ctrl Z / Ctrl Y, Ctrl Shift Z | undo / redo |
//! | Ctrl C / Ctrl V | copy / paste |
//! | Ctrl S | save |
//! | Escape | select nothing |
//!
//! Ctrl is Cmd on a Mac.

use runity::gizmo::Tool;
use runity::input::{Input, Key, MouseButton};

use crate::{EditResult, Session};

impl Session {
    /// Do what this frame's input asks of the Scene view. Returns what was
    /// done, by name — "select", "drag", "undo", "orbit" — so the window
    /// knows to redraw and a test knows what happened.
    /// `dt` is the seconds since the last call: how far a flythrough goes.
    pub fn scene_view(&mut self, input: &Input, dt: f32) -> EditResult<Vec<&'static str>> {
        let mut did = Vec::new();
        let ctrl = [
            Key::LeftControl,
            Key::RightControl,
            Key::LeftSuper,
            Key::RightSuper,
        ]
        .iter()
        .any(|k| input.held(*k));
        let shift = input.held(Key::LeftShift) || input.held(Key::RightShift);
        let alt = input.held(Key::LeftAlt) || input.held(Key::RightAlt);
        let at = input.mouse_position();
        let (x, y) = (at.x.max(0.0) as u32, at.y.max(0.0) as u32);
        let motion = input.mouse_motion();

        // The mouse.
        if input.mouse_pressed(MouseButton::Left) && !alt {
            if self.gizmo_begin(x, y)?.is_some() {
                did.push("grab");
            } else {
                let hit = self.pick(x, y);
                match (hit, shift) {
                    (Some(id), true) => self.add_to_selection(id)?,
                    (Some(id), false) => self.select(Some(id))?,
                    (None, true) => {}
                    (None, false) => self.select(None)?,
                }
                if hit.is_none() {
                    // Empty space: a click, or the start of a box.
                    self.marquee = Some((at, at));
                }
                did.push("select");
            }
        } else if input.mouse_held(MouseButton::Left)
            && self.is_dragging()
            && motion != runity::glam::Vec2::ZERO
        {
            // Ctrl Shift: onto whatever is under the cursor instead of
            // along the handle.
            if ctrl && shift && self.tool() == Tool::Move {
                if self.surface_drag(x, y)? {
                    did.push("place");
                }
            } else if self.gizmo_drag(x, y)? {
                did.push("drag");
            }
        }
        if let Some((from, _)) = self.marquee {
            self.marquee = Some((from, at));
            if !input.mouse_held(MouseButton::Left) {
                self.marquee = None;
                // A few pixels of wobble is still a click.
                if (at - from).abs().max_element() > 3.0 {
                    self.select_in_rect(from, at, shift)?;
                    did.push("box select");
                }
            }
        }
        if input.mouse_released(MouseButton::Left) && self.is_dragging() {
            self.gizmo_end();
            did.push("drop");
        }
        if alt && input.mouse_held(MouseButton::Left) && motion != runity::glam::Vec2::ZERO {
            self.orbit(-motion.x * 0.3, motion.y * 0.3);
            did.push("orbit");
        }
        // Flythrough: the right button held turns the head, and WASD, Q
        // and E move it, as in a game; the wheel sets how fast.
        let flying = input.mouse_held(MouseButton::Right);
        if flying {
            if motion != runity::glam::Vec2::ZERO {
                self.look(-motion.x * 0.2, -motion.y * 0.2);
                did.push("look");
            }
            let along = |negative: Key, positive: Key| input.axis(negative, positive);
            let step = runity::glam::Vec3::new(
                along(Key::A, Key::D),
                along(Key::Q, Key::E),
                along(Key::S, Key::W),
            );
            if step != runity::glam::Vec3::ZERO {
                let speed = self.fly_speed * if shift { 4.0 } else { 1.0 };
                let step = step.normalize() * speed * dt.max(0.0);
                self.fly(step.z, step.x, step.y);
                did.push("fly");
            }
        }
        if input.mouse_held(MouseButton::Middle) && motion != runity::glam::Vec2::ZERO {
            let camera = self.camera();
            let scale = (camera.position - camera.target).length() * 0.0015;
            self.pan(-motion.x * scale, motion.y * scale);
            did.push("pan");
        }
        let wheel = input.scroll().y;
        if wheel != 0.0 && flying {
            self.fly_speed = (self.fly_speed * 1.25f32.powf(wheel)).clamp(0.1, 500.0);
            did.push("fly speed");
        } else if wheel != 0.0 {
            self.zoom(0.9f32.powf(wheel));
            did.push("zoom");
        }

        // The keyboard.
        let pressed = |key: Key| input.pressed(key);
        if ctrl {
            if pressed(Key::Z) && shift || pressed(Key::Y) {
                if self.redo()? {
                    did.push("redo");
                }
            } else if pressed(Key::Z) && self.undo()? {
                did.push("undo");
            }
            if pressed(Key::D) {
                self.duplicate_selection()?;
                did.push("duplicate");
            }
            if pressed(Key::C) {
                self.clipboard = self.copy_selection();
                did.push("copy");
            }
            if pressed(Key::V) && !self.clipboard.is_empty() {
                let text = self.clipboard.clone();
                let pasted = self.paste(&text, None)?;
                if let Some(first) = pasted.first() {
                    self.select(Some(*first))?;
                    for id in &pasted[1..] {
                        self.add_to_selection(*id)?;
                    }
                }
                did.push("paste");
            }
            if pressed(Key::S) {
                self.save_scene(None)?;
                did.push("save");
            }
        } else if !flying {
            for (key, tool, name) in [
                (Key::W, Tool::Move, "move tool"),
                (Key::E, Tool::Rotate, "rotate tool"),
                (Key::R, Tool::Scale, "scale tool"),
            ] {
                if pressed(key) {
                    self.set_tool(tool);
                    did.push(name);
                }
            }
            if pressed(Key::X) {
                self.set_space(match self.space() {
                    crate::Space::Global => crate::Space::Local,
                    crate::Space::Local => crate::Space::Global,
                });
                did.push("space");
            }
            if pressed(Key::Z) {
                self.set_pivot(match self.pivot() {
                    crate::Pivot::Pivot => crate::Pivot::Center,
                    crate::Pivot::Center => crate::Pivot::Pivot,
                });
                did.push("pivot");
            }
            if pressed(Key::F) && self.focus_selected() {
                did.push("frame");
            }
            if pressed(Key::End) && self.drop_to_ground()? > 0 {
                did.push("drop to ground");
            }
            if pressed(Key::H) && shift {
                if self.isolated().is_empty() {
                    let selection = self.selection();
                    self.isolate(&selection)?;
                    did.push("isolate");
                } else {
                    self.isolate(&[])?;
                    did.push("show all");
                }
            } else if pressed(Key::H) {
                did.push(if self.toggle_hidden()? {
                    "hide"
                } else {
                    "show"
                });
            }
            if pressed(Key::Escape) {
                self.select(None)?;
                did.push("deselect");
            }
        }
        if (pressed(Key::Delete) || (pressed(Key::Backspace) && ctrl))
            && self.delete_selection()? > 0
        {
            did.push("delete");
        }
        Ok(did)
    }
}
