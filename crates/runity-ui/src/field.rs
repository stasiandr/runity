//! A one-line text field: the caret, the selection, the clipboard.
//!
//! The field is a node like any other, with its text in the node and this
//! state beside it. Keys and typed text reach it while it has the keyboard;
//! what it reports is [`Event::Changed`], [`Event::Submit`] and
//! [`Event::Cancel`] — a panel never sees Backspace. Positions are byte
//! offsets into the text, always on a character boundary.

use runity_core::input::Key;

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
    /// Several lines: Enter is a new line, Cmd/Ctrl Enter commits, the
    /// arrows go up and down, and the text wraps instead of scrolling.
    pub(crate) multiline: bool,
    /// Text an input method is composing, not yet typed: where it sits in
    /// the field's text, and what it is. Shown underlined, never reported
    /// as a change.
    preedit: Option<(usize, String)>,
}

impl FieldState {
    pub(crate) fn multiline() -> Self {
        Self {
            multiline: true,
            ..Self::default()
        }
    }
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

/// The accent of the caret and the tint of a selection, drawn through the
/// palette like any other colour: a theme's accent is its caret too.
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
        let multiline = state.multiline;
        self.set_state(
            id,
            FieldState {
                multiline,
                ..FieldState::default()
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

    /// The byte offset nearest to a point (window coordinates) in a field.
    fn index_at(&self, id: NodeId, x: f32, y: f32) -> usize {
        let Some(buffer) = self.node(id).text.as_ref().map(|t| &t.buffer) else {
            return 0;
        };
        let state = self.state(id);
        let (left, top) = self.text_origin(id);
        let text = self.text(id).unwrap_or_default();
        let (lx, ly) = (x - left + state.offset, (y - top).max(0.0));
        let Some(cursor) = buffer.hit(lx, ly) else {
            return if ly <= 0.0 { 0 } else { text.len() };
        };
        line_start(text, cursor.line) + cursor.index
    }

    /// Where a byte offset is drawn, from the text's top left: `x`, and the
    /// top of the line it is on.
    fn caret_at(&self, id: NodeId, at: usize) -> (f32, f32) {
        let Some(buffer) = self.node(id).text.as_ref().map(|t| &t.buffer) else {
            return (0.0, 0.0);
        };
        let text = self.text(id).unwrap_or_default();
        let (line, index) = line_of(text, at);
        let mut last = None;
        for run in buffer.layout_runs().filter(|r| r.line_i == line) {
            for glyph in run.glyphs {
                if index <= glyph.start {
                    return (glyph.x, run.line_top);
                }
            }
            last = Some((
                run.line_w,
                run.line_top,
                run.glyphs.last().map_or(0, |g| g.end),
            ));
            // A wrapped line goes on in the next run.
            if run.glyphs.last().is_some_and(|g| index < g.end) {
                break;
            }
        }
        match last {
            Some((w, top, _)) => (w, top),
            // An empty line: its run has no glyphs, find its top anyway.
            None => (
                0.0,
                buffer
                    .layout_runs()
                    .find(|r| r.line_i == line)
                    .map_or(line as f32 * buffer.metrics().line_height, |r| r.line_top),
            ),
        }
    }

    /// Where the field's text starts, in window coordinates.
    fn text_origin(&self, id: NodeId) -> (f32, f32) {
        let rect = self.node(id).rect;
        let layout = self.tree.layout(id.0).ok();
        let left = rect.x + layout.map_or(0.0, |l| l.padding.left + l.border.left);
        let top = rect.y + layout.map_or(0.0, |l| l.padding.top + l.border.top);
        let height = layout.map_or(0.0, |l| {
            l.size.height - l.padding.top - l.padding.bottom - l.border.top - l.border.bottom
        });
        // As the text is drawn: in the middle of a box taller than it.
        (
            left,
            top + ((height - self.text_height_of(id)) / 2.0).max(0.0),
        )
    }

    pub(crate) fn field_press(&mut self, id: NodeId, x: f32, y: f32, extend: bool) {
        let at = self.index_at(id, x, y);
        let mut state = self.state(id);
        state.cursor = at;
        if !extend {
            state.anchor = at;
        }
        self.set_state(id, state);
    }

    pub(crate) fn field_drag(&mut self, id: NodeId, x: f32, y: f32) {
        let at = self.index_at(id, x, y);
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

    /// What an input method is composing, shown in the focused field in
    /// place, underlined; `""` when the composition ends (a commit follows
    /// as typed text).
    pub fn ime_preedit(&mut self, text: &str) {
        let Some(id) = self.focused.filter(|f| self.is_field(*f)) else {
            return;
        };
        let mut state = self.state(id);
        let shown = self.text(id).unwrap_or_default().to_string();
        // The text without the old composition.
        let (start, base) = match &state.preedit {
            Some((start, old)) => {
                let mut base = shown.clone();
                base.replace_range(*start..*start + old.len(), "");
                (*start, base)
            }
            None => (state.cursor.min(state.anchor), shown.clone()),
        };
        if text.is_empty() {
            state.preedit = None;
            state.cursor = start;
            state.anchor = start;
            self.set_text(id, &base);
        } else {
            let mut composite = base.clone();
            composite.insert_str(start, text);
            state.preedit = Some((start, text.to_string()));
            state.cursor = start + text.len();
            state.anchor = state.cursor;
            self.set_text(id, &composite);
        }
        self.set_state(id, state);
    }

    /// Where the caret of the focused field is, in window coordinates: what
    /// an input method places its candidate window by.
    pub fn caret_rect(&self) -> Option<Rect> {
        let id = self.focused.filter(|f| self.is_field(*f))?;
        let state = self.state(id);
        let (x, y) = self.caret_at(id, state.cursor);
        let (left, top) = self.text_origin(id);
        let line = self
            .node(id)
            .text
            .as_ref()
            .map_or(16.0, |t| t.buffer.metrics().line_height);
        Some(Rect {
            x: left + x - state.offset,
            y: top + y,
            width: 1.0,
            height: line,
        })
    }

    pub(crate) fn field_type(&mut self, id: NodeId, text: &str) {
        // A composition still showing gives way to what was committed.
        if self.state(id).preedit.is_some() {
            self.ime_preedit("");
        }
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
            Key::Up | Key::Down if state.multiline => {
                // The same x a line up or down, as the layout has it.
                let (x, y) = self.caret_at(id, state.cursor);
                let line = self
                    .node(id)
                    .text
                    .as_ref()
                    .map_or(16.0, |t| t.buffer.metrics().line_height);
                let (left, top) = self.text_origin(id);
                let y = if key == Key::Up {
                    y - line * 0.5
                } else {
                    y + line * 1.5
                };
                let to = if y < 0.0 {
                    0
                } else {
                    self.index_at(id, left + x, top + y)
                };
                moved(self, to, &mut state);
            }
            Key::Home if state.multiline => {
                let (line, _) = line_of(&text, state.cursor);
                moved(self, line_start(&text, line), &mut state);
            }
            Key::End if state.multiline => {
                let (line, _) = line_of(&text, state.cursor);
                let end = line_start(&text, line) + text.split('\n').nth(line).map_or(0, str::len);
                moved(self, end, &mut state);
            }
            Key::Home => moved(self, 0, &mut state),
            Key::End => moved(self, text.len(), &mut state),
            // Up and Down in a one-line field are not the field's: a list
            // under a search box moves its choice with them.
            Key::Up | Key::Down => return false,
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
                    let paste = if state.multiline {
                        paste
                    } else {
                        paste.lines().next().unwrap_or_default().to_string()
                    };
                    self.replace_selection(id, &paste);
                }
            }
            Key::Enter if state.multiline && !shortcut => {
                self.replace_selection(id, "\n");
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
        let (caret_x, caret_y) = self.caret_at(id, state.cursor);
        // Keep the caret inside the box, sideways; a field of several
        // lines wraps instead.
        if state.multiline {
            state.offset = 0.0;
        } else if caret_x - state.offset > inner.width - 2.0 {
            state.offset = caret_x - inner.width + 2.0;
        } else if caret_x - state.offset < 0.0 {
            state.offset = caret_x;
        }
        let offset = state.offset;
        let line = self
            .node(id)
            .text
            .as_ref()
            .map_or(16.0, |t| t.buffer.metrics().line_height);
        let range = self.field_selection(id);
        let clip = clip.intersect(&inner);
        // The selection: a band on each line it covers.
        let mut bands = Vec::new();
        if !range.is_empty() {
            let (sx, sy) = self.caret_at(id, range.start);
            let (ex, ey) = self.caret_at(id, range.end);
            let mut y = sy;
            while y <= ey + 0.5 {
                let from = if (y - sy).abs() < 0.5 { sx } else { 0.0 };
                let to = if (y - ey).abs() < 0.5 {
                    ex
                } else {
                    inner.width + offset
                };
                if to > from {
                    bands.push((from, y, to - from));
                }
                y += line;
            }
        }
        // The composition, underlined.
        if let Some((start, text)) = state.preedit.clone() {
            let (sx, sy) = self.caret_at(id, start);
            let (ex, _) = self.caret_at(id, start + text.len());
            let color = self.tint(CARET);
            bands_underline(
                &mut self.layers,
                color,
                inner.x + sx - offset,
                text_y + sy + line - 2.0,
                (ex - sx).max(1.0),
                clip,
            );
        }
        let caret = self.tint(CARET);
        let layer_rects = &mut self.layers.last_mut().expect("a layer").rects;
        for (from, y, width) in bands {
            layer_rects.push(RectPaint {
                rect: Rect {
                    x: inner.x + from - offset,
                    y: text_y + y,
                    width,
                    height: line,
                },
                fill: caret.alpha(30),
                border: Color::TRANSPARENT,
                border_width: 0.0,
                radius: 2.0,
                clip,
            });
        }
        layer_rects.push(RectPaint {
            rect: Rect {
                x: inner.x + caret_x - offset,
                y: text_y + caret_y + 1.0,
                width: 1.5,
                height: line - 2.0,
            },
            fill: caret,
            border: Color::TRANSPARENT,
            border_width: 0.0,
            radius: 0.0,
            clip,
        });
        self.node_mut(id).field = Some(state);
        offset
    }
}

/// Where line `line` (0-based, split at `\n`) starts, as a byte offset.
fn line_start(text: &str, line: usize) -> usize {
    text.split('\n')
        .take(line)
        .map(|l| l.len() + 1)
        .sum::<usize>()
        .min(text.len())
}

/// Which line a byte offset is on, and how far into it.
fn line_of(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at.min(text.len())];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, at - start)
}

/// A line under text being composed.
fn bands_underline(
    layers: &mut [crate::Layer],
    color: Color,
    x: f32,
    y: f32,
    width: f32,
    clip: Rect,
) {
    if let Some(layer) = layers.last_mut() {
        layer.rects.push(RectPaint {
            rect: Rect {
                x,
                y,
                width,
                height: 1.5,
            },
            fill: color,
            border: Color::TRANSPARENT,
            border_width: 0.0,
            radius: 0.0,
            clip,
        });
    }
}
