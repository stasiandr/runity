//! A one-line text field: the caret, the selection, the clipboard.
//!
//! The field is a node like any other, with its text in the node and this
//! state beside it. Keys and typed text reach it while it has the keyboard;
//! what it reports is [`Event::Changed`], [`Event::Submit`] and
//! [`Event::Cancel`] — a panel never sees Backspace. Positions are byte
//! offsets into the text, always on a character boundary.

use runity::input::Key;

use crate::{Color, Event, NodeId, Rect, RectPaint, Ui};

/// A field's caret and selection.
#[derive(Debug, Clone, Default)]
pub(crate) struct FieldState {
    /// Where the caret is.
    cursor: usize,
    /// The other end of the selection; equal to `cursor` when nothing is
    /// selected.
    anchor: usize,
    /// How far the text is scrolled left so that the caret stays in view.
    offset: f32,
    /// What it said when it got the keyboard: Escape goes back to it, and
    /// leaving it changed is a Submit.
    original: String,
}

/// Where copy and paste go. The window gives the UI the system's; without
/// one, a clipboard of its own inside the UI.
pub trait Clipboard {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: String);
}

/// The clipboard a UI starts with: a string, private to it.
#[derive(Default)]
pub(crate) struct LocalClipboard(String);

impl Clipboard for LocalClipboard {
    fn get(&mut self) -> Option<String> {
        Some(self.0.clone())
    }
    fn set(&mut self, text: String) {
        self.0 = text;
    }
}

/// The accent of the caret and the tint of a selection.
const CARET: Color = Color::hex(0x9184d9);

fn prev_boundary(text: &str, at: usize) -> usize {
    text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_boundary(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

impl Ui {
    /// Where copy and paste go: the system clipboard, given by the window.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
    }

    fn state(&self, id: NodeId) -> FieldState {
        self.node(id).field.clone().unwrap_or_default()
    }

    fn set_state(&mut self, id: NodeId, state: FieldState) {
        self.node_mut(id).field = Some(state);
        self.paint_dirty = true;
    }

    /// The selected part of a field, as a byte range.
    pub fn field_selection(&self, id: NodeId) -> std::ops::Range<usize> {
        let s = self.state(id);
        s.cursor.min(s.anchor)..s.cursor.max(s.anchor)
    }

    pub(crate) fn field_focus(&mut self, id: NodeId) {
        if self.node(id).field.is_none() {
            return;
        }
        let text = self.text(id).unwrap_or_default().to_string();
        let mut state = self.state(id);
        state.original = text;
        self.set_state(id, state);
    }

    pub(crate) fn field_blur(&mut self, id: NodeId) {
        let Some(state) = self.node(id).field.clone() else {
            return;
        };
        let text = self.text(id).unwrap_or_default().to_string();
        if text != state.original {
            self.events.push((id, Event::Submit(text)));
        }
        self.set_state(
            id,
            FieldState {
                cursor: 0,
                anchor: 0,
                offset: 0.0,
                original: String::new(),
            },
        );
    }

    pub(crate) fn field_select_all(&mut self, id: NodeId) {
        let len = self.text(id).map_or(0, str::len);
        let mut state = self.state(id);
        state.anchor = 0;
        state.cursor = len;
        self.set_state(id, state);
    }

    /// The byte offset nearest to `x` (window coordinates) in a field.
    fn index_at(&self, id: NodeId, x: f32) -> usize {
        let Some(buffer) = self.node(id).text.as_ref().map(|t| &t.buffer) else {
            return 0;
        };
        let state = self.state(id);
        let left = self.text_left(id);
        let x = x - left + state.offset;
        let text = self.text(id).unwrap_or_default();
        let Some(run) = buffer.layout_runs().next() else {
            return 0;
        };
        for glyph in run.glyphs {
            if x < glyph.x + glyph.w / 2.0 {
                return glyph.start;
            }
        }
        text.len()
    }

    /// Where a byte offset is drawn, from the text's left edge.
    fn caret_x(&self, id: NodeId, at: usize) -> f32 {
        let Some(buffer) = self.node(id).text.as_ref().map(|t| &t.buffer) else {
            return 0.0;
        };
        let Some(run) = buffer.layout_runs().next() else {
            return 0.0;
        };
        for glyph in run.glyphs {
            if at <= glyph.start {
                return glyph.x;
            }
        }
        run.line_w
    }

    /// The field's text starts here, in window coordinates.
    fn text_left(&self, id: NodeId) -> f32 {
        let rect = self.node(id).rect;
        let layout = self.tree.layout(id.0).ok();
        rect.x + layout.map_or(0.0, |l| l.padding.left + l.border.left)
    }

    pub(crate) fn field_press(&mut self, id: NodeId, x: f32, extend: bool) {
        let at = self.index_at(id, x);
        let mut state = self.state(id);
        state.cursor = at;
        if !extend {
            state.anchor = at;
        }
        self.set_state(id, state);
    }

    pub(crate) fn field_drag(&mut self, id: NodeId, x: f32) {
        let at = self.index_at(id, x);
        let mut state = self.state(id);
        state.cursor = at;
        self.set_state(id, state);
    }

    /// Replace the selection with `insert` and put the caret after it.
    fn replace_selection(&mut self, id: NodeId, insert: &str) {
        let text = self.text(id).unwrap_or_default().to_string();
        let range = self.field_selection(id);
        let mut next = String::with_capacity(text.len() + insert.len());
        next.push_str(&text[..range.start]);
        next.push_str(insert);
        next.push_str(&text[range.end..]);
        let caret = range.start + insert.len();
        self.set_text(id, &next);
        let mut state = self.state(id);
        state.cursor = caret;
        state.anchor = caret;
        self.set_state(id, state);
        self.events.push((id, Event::Changed(next)));
    }

    pub(crate) fn field_type(&mut self, id: NodeId, text: &str) {
        // Control characters arrive as text on some platforms alongside
        // the key that made them; the key is what handles them.
        let printable: String = text.chars().filter(|c| !c.is_control()).collect();
        if printable.is_empty() {
            return;
        }
        let (_, ctrl, _, command) = self.modifiers();
        if ctrl || command {
            return;
        }
        self.replace_selection(id, &printable);
    }

    /// A key while a field has the keyboard. `true` when the field used it.
    pub(crate) fn field_key(&mut self, id: NodeId, key: Key) -> bool {
        let (shift, ctrl, _, command) = self.modifiers();
        let shortcut = ctrl || command;
        let text = self.text(id).unwrap_or_default().to_string();
        let mut state = self.state(id);
        let range = self.field_selection(id);
        let moved = |ui: &mut Ui, to: usize, state: &mut FieldState| {
            state.cursor = to;
            if !shift {
                state.anchor = to;
            }
            ui.set_state(id, state.clone());
        };
        match key {
            Key::Left => {
                let to = if !shift && !range.is_empty() {
                    range.start
                } else {
                    prev_boundary(&text, state.cursor)
                };
                moved(self, to, &mut state);
            }
            Key::Right => {
                let to = if !shift && !range.is_empty() {
                    range.end
                } else {
                    next_boundary(&text, state.cursor)
                };
                moved(self, to, &mut state);
            }
            Key::Home | Key::Up => moved(self, 0, &mut state),
            Key::End | Key::Down => moved(self, text.len(), &mut state),
            Key::Backspace => {
                if range.is_empty() {
                    state.anchor = prev_boundary(&text, state.cursor);
                    self.set_state(id, state);
                }
                self.replace_selection(id, "");
            }
            Key::Delete => {
                if range.is_empty() {
                    state.anchor = next_boundary(&text, state.cursor);
                    self.set_state(id, state);
                }
                self.replace_selection(id, "");
            }
            Key::A if shortcut => self.field_select_all(id),
            Key::C if shortcut => {
                if !range.is_empty() {
                    self.clipboard.set(text[range].to_string());
                }
            }
            Key::X if shortcut => {
                if !range.is_empty() {
                    self.clipboard.set(text[range].to_string());
                    self.replace_selection(id, "");
                }
            }
            Key::V if shortcut => {
                if let Some(paste) = self.clipboard.get() {
                    // One line: a newline in a paste would be a second
                    // line the field cannot show.
                    let paste = paste.lines().next().unwrap_or_default().to_string();
                    self.replace_selection(id, &paste);
                }
            }
            Key::Enter | Key::Tab => {
                self.events.push((id, Event::Submit(text.clone())));
                state.original = text;
                self.set_state(id, state);
                self.field_select_all(id);
            }
            Key::Escape => {
                let original = state.original.clone();
                self.set_text(id, &original);
                self.events.push((id, Event::Cancel));
                self.focus(None);
            }
            // Holding a modifier is the field's too: it is half of a
            // shortcut the field may take.
            Key::LeftShift
            | Key::RightShift
            | Key::LeftControl
            | Key::RightControl
            | Key::LeftAlt
            | Key::RightAlt
            | Key::LeftSuper
            | Key::RightSuper => {}
            // Other shortcuts are not the field's: Cmd S still saves.
            _ if shortcut => return false,
            // A letter's key: the text event that follows does the typing.
            _ => {}
        }
        true
    }

    /// The caret and selection of a focused field, painted behind its text;
    /// returns how far the text is scrolled left to keep the caret in view.
    pub(crate) fn paint_field(
        &mut self,
        id: NodeId,
        _text_x: f32,
        text_y: f32,
        inner: Rect,
        clip: Rect,
        focused: bool,
    ) -> f32 {
        if self.node(id).field.is_none() {
            return 0.0;
        }
        let mut state = self.state(id);
        if !focused {
            if state.offset != 0.0 {
                state.offset = 0.0;
                self.node_mut(id).field = Some(state);
            }
            return 0.0;
        }
        let caret = self.caret_x(id, state.cursor);
        // Keep the caret inside the box.
        if caret - state.offset > inner.width - 2.0 {
            state.offset = caret - inner.width + 2.0;
        } else if caret - state.offset < 0.0 {
            state.offset = caret;
        }
        let offset = state.offset;
        let line = self
            .node(id)
            .text
            .as_ref()
            .map_or(16.0, |t| t.buffer.metrics().line_height);
        let range = self.field_selection(id);
        let clip = clip.intersect(&inner);
        let from = self.caret_x(id, range.start);
        let to = self.caret_x(id, range.end);
        let layer_rects = &mut self.layers.last_mut().expect("a layer").rects;
        if from != to {
            layer_rects.push(RectPaint {
                rect: Rect {
                    x: inner.x + from - offset,
                    y: text_y,
                    width: to - from,
                    height: line,
                },
                fill: CARET.alpha(30),
                border: Color::TRANSPARENT,
                border_width: 0.0,
                radius: 2.0,
                clip,
            });
        }
        layer_rects.push(RectPaint {
            rect: Rect {
                x: inner.x + caret - offset,
                y: text_y + 1.0,
                width: 1.5,
                height: line - 2.0,
            },
            fill: CARET,
            border: Color::TRANSPARENT,
            border_width: 0.0,
            radius: 0.0,
            clip,
        });
        self.node_mut(id).field = Some(state);
        offset
    }
}
