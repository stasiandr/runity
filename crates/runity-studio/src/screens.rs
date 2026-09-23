//! Screens: the game's menus and HUDs, laid out by hand — Unity's UI
//! Builder, for this engine's screens.
//!
//! A screen is a RON file in `ui/` (`runity::screen`): elements anchored
//! to a corner, an edge or the middle, sized in pixels of a 720-high
//! screen. This panel draws one exactly as the game does — `Screen::draw`
//! into the engine's own screen layer, on the session's GPU — at the
//! reference 1280×720, and lets it be edited: a click on the canvas picks
//! an element, a drag moves it, the boxes on the right set its anchor,
//! place, size, text size and what it is. Every change is written to the
//! file at once, so a running game moves the button too (DNA, postulate 1).
//!
//! Only the elements that changed are rewritten, each in its own place in
//! the text: comments and hand-made layout survive an edit made here.

use std::path::PathBuf;

use runity::gpu::OffscreenTarget;
use runity::screen::{Anchor, Element, Kind, Layout};
use runity_editor::console::Level;
use runity_editor::Session;
use runity_ui::{Event, ImageId, NodeId, Style, Ui};

use crate::theme::*;

/// The picture of the screen being edited.
pub const CANVAS: ImageId = ImageId(3);
/// The size a screen is laid out for.
const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;

const ANCHORS: [(Anchor, &str); 9] = [
    (Anchor::TopLeft, "TopLeft"),
    (Anchor::Top, "Top"),
    (Anchor::TopRight, "TopRight"),
    (Anchor::Left, "Left"),
    (Anchor::Center, "Center"),
    (Anchor::Right, "Right"),
    (Anchor::BottomLeft, "BottomLeft"),
    (Anchor::Bottom, "Bottom"),
    (Anchor::BottomRight, "BottomRight"),
];

pub struct Screens {
    pub root: NodeId,
    files_list: NodeId,
    elements_list: NodeId,
    canvas: NodeId,
    fields: NodeId,
    wide_button: NodeId,
    /// The panel over the whole middle of the window, the Scene view
    /// hidden: Unity's UI Builder is a window of its own, with room.
    pub wide: bool,
    files: Vec<(NodeId, PathBuf)>,
    elements: Vec<(NodeId, String)>,
    boxes: Vec<(NodeId, &'static str)>,
    anchors: Vec<(NodeId, Anchor)>,
    open: Option<(PathBuf, Layout)>,
    selected: Option<String>,
    listed: bool,
    /// The picture: drawn again when something changed.
    target: Option<OffscreenTarget>,
    renderer: Option<runity::ui_render::UiRenderer>,
    pub dirty: bool,
    /// The target was made anew: the window's renderer must be told.
    pub new_target: bool,
}

impl Screens {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::row()
                .fill()
                .full_width()
                .gap(SPACE_2)
                .padding(SPACE_2),
        );
        ui.set_name(root, "screens");
        let left = ui.add(
            root,
            Style::column()
                .width(180.0)
                .full_height()
                .fixed()
                .gap(SPACE_2),
        );
        let head = ui.add(left, Style::row().full_width().center_items());
        ui.add_text(head, caption(), "SCREENS");
        spacer(ui, head);
        let wide_button = icon_button(ui, head, "screen wide", "expand", false);
        let files_list = ui.add(left, Style::column().full_width().gap(2.0));
        ui.add_text(left, caption(), "ELEMENTS");
        let elements_list = ui.add(left, Style::column().fill().full_width().gap(2.0).clip());
        let middle = ui.add(root, Style::column().fill().full_height().center());
        let canvas = ui.add_image(
            middle,
            Style::default()
                .size(320.0, 180.0)
                .radius(RADIUS_SM)
                .border(1.0, DIVIDER)
                .draggable(),
            CANVAS,
        );
        ui.set_name(canvas, "screen canvas");
        let fields = ui.add(
            root,
            Style::row()
                .width(440.0)
                .full_height()
                .fixed()
                .gap(SPACE_2)
                .clip(),
        );
        Self {
            root,
            files_list,
            elements_list,
            canvas,
            fields,
            wide_button,
            wide: false,
            files: Vec::new(),
            elements: Vec::new(),
            boxes: Vec::new(),
            anchors: Vec::new(),
            open: None,
            selected: None,
            listed: false,
            target: None,
            renderer: None,
            dirty: false,
            new_target: false,
        }
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == self.root {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    /// The canvas fitted to the room it has, keeping 16:9.
    fn fit_canvas(&self, ui: &mut Ui) {
        let Some(middle) = ui.parent(self.canvas) else {
            return;
        };
        let room = ui.rect(middle);
        let (w, h) = if room.width / room.height.max(1.0) > WIDTH / HEIGHT {
            (room.height * WIDTH / HEIGHT, room.height)
        } else {
            (room.width, room.width * HEIGHT / WIDTH)
        };
        let (w, h) = ((w - 4.0).max(64.0).floor(), (h - 4.0).max(36.0).floor());
        if (ui.rect(self.canvas).width - w).abs() > 0.5 {
            ui.restyle(self.canvas, |s| s.size(w, h));
        }
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        self.fit_canvas(ui);
        if self.listed {
            return;
        }
        self.listed = true;
        ui.clear(self.files_list);
        self.files.clear();
        let Some(dir) = session
            .project()
            .map(|p| p.root().join(runity::project::UI))
        else {
            return;
        };
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "ron"))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        if paths.is_empty() {
            ui.add_text(
                self.files_list,
                Style::default().text_size(11.5).text_color(MUTED),
                "No screens in ui/ yet.",
            );
        }
        for path in paths {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let row = line(
                ui,
                self.files_list,
                &format!("screen {name}"),
                "layout-dashboard",
                &name,
            );
            self.files.push((row, path));
        }
    }

    fn open(&mut self, ui: &mut Ui, session: &mut Session, path: PathBuf) {
        let layout = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| runity::ron::from_str::<Layout>(&t).map_err(|e| e.to_string()));
        match layout {
            Ok(layout) => {
                for (row, p) in &self.files {
                    let on = *p == path;
                    ui.restyle(*row, |s| {
                        s.background(if on {
                            ACCENT_900
                        } else {
                            runity_ui::Color::TRANSPARENT
                        })
                    });
                }
                self.open = Some((path, layout));
                self.selected = None;
                self.show_elements(ui);
                self.show_fields(ui);
                self.dirty = true;
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    fn show_elements(&mut self, ui: &mut Ui) {
        ui.clear(self.elements_list);
        self.elements.clear();
        let Some((_, layout)) = &self.open else {
            return;
        };
        for e in layout.elements.clone() {
            let row = line(
                ui,
                self.elements_list,
                &format!("element {}", e.id),
                kind_icon(&e.kind),
                &e.id,
            );
            let on = self.selected.as_deref() == Some(e.id.as_str());
            ui.restyle(row, |s| {
                s.background(if on {
                    ACCENT_900
                } else {
                    runity_ui::Color::TRANSPARENT
                })
            });
            self.elements.push((row, e.id));
        }
    }

    /// The selected element's boxes.
    fn show_fields(&mut self, ui: &mut Ui) {
        ui.clear(self.fields);
        self.boxes.clear();
        self.anchors.clear();
        let Some(e) = self.element().cloned() else {
            ui.add_text(
                self.fields,
                Style::default().text_size(12.0).text_color(MUTED),
                "Pick an element on the canvas or the list.",
            );
            return;
        };
        let row = |ui: &mut Ui, parent: NodeId, label: &str| {
            let r = ui.add(
                parent,
                Style::row().full_width().gap(SPACE_2).center_items(),
            );
            ui.add_text(
                r,
                Style::default()
                    .width(64.0)
                    .fixed()
                    .text_size(12.0)
                    .text_color(LABEL)
                    .nowrap(),
                label,
            );
            r
        };
        // Numbers on the left; the anchor and what it is on the right.
        let numbers = ui.add(self.fields, Style::column().fill().gap(3.0));
        let fields = ui.add(self.fields, Style::column().width(200.0).fixed().gap(3.0));
        let values = [
            ("id", e.id.clone()),
            ("x", trim(e.at.0)),
            ("y", trim(e.at.1)),
            ("width", trim(e.size.0)),
            ("height", trim(e.size.1)),
            ("text size", trim(e.text_size)),
            ("kind", runity::ron::to_string(&e.kind).unwrap_or_default()),
        ];
        // The anchor: a 3×3 of its nine places.
        let r = row(ui, fields, "anchor");
        let grid = ui.add(r, Style::column().gap(2.0));
        for chunk in ANCHORS.chunks(3) {
            let line = ui.add(grid, Style::row().gap(2.0));
            for (anchor, name) in chunk {
                let on = e.anchor == *anchor;
                let cell = ui.add(
                    line,
                    Style::row()
                        .size(22.0, 16.0)
                        .radius(3.0)
                        .border(1.0, if on { ACCENT } else { DIVIDER })
                        .clickable()
                        .background(if on {
                            ACCENT.alpha(30)
                        } else {
                            runity_ui::Color::TRANSPARENT
                        })
                        .hover(HOVER),
                );
                ui.set_name(cell, format!("anchor {name}"));
                self.anchors.push((cell, *anchor));
            }
        }
        for (label, value) in values {
            let r = row(ui, if label == "kind" { fields } else { numbers }, label);
            let b = if label == "kind" {
                ui.add_textarea(
                    r,
                    field_style()
                        .fill()
                        .auto_height()
                        .min_height(22.0)
                        .padding_y(3.0)
                        .mono()
                        .text_size(11.5),
                    &value,
                )
            } else {
                ui.add_field(r, field_style().fill().mono().text_size(11.5), &value)
            };
            ui.set_name(b, format!("screen field {label}"));
            let key: &'static str = match label {
                "id" => "id",
                "x" => "x",
                "y" => "y",
                "width" => "width",
                "height" => "height",
                "text size" => "text size",
                _ => "kind",
            };
            self.boxes.push((b, key));
        }
    }

    fn element(&self) -> Option<&Element> {
        let (_, layout) = self.open.as_ref()?;
        let id = self.selected.as_ref()?;
        layout.elements.iter().find(|e| &e.id == id)
    }

    fn element_mut(&mut self) -> Option<&mut Element> {
        let id = self.selected.clone()?;
        let (_, layout) = self.open.as_mut()?;
        layout.elements.iter_mut().find(|e| e.id == id)
    }

    /// Write the screen back; the game picks it up from disk. Only the
    /// elements that changed are rewritten, each on its own span of the
    /// file, so comments and the rest of the text stay as they were — and
    /// the diff is the change (DNA, postulate 2).
    fn save(&mut self, session: &mut Session) {
        let Some((path, layout)) = &self.open else {
            return;
        };
        let problems = layout.problems();
        if let Some(p) = problems.first() {
            session.say(Level::Error, format!("{}: not saved, {p}", path.display()));
            return;
        }
        let old = std::fs::read_to_string(path).unwrap_or_default();
        let text = patched(&old, layout).unwrap_or_else(|| {
            let pretty = runity::ron::ser::PrettyConfig::new();
            runity::ron::ser::to_string_pretty(layout, pretty).unwrap_or_default() + "\n"
        });
        if text != old {
            if let Err(e) = std::fs::write(path, text) {
                session.say(Level::Error, format!("{}: {e}", path.display()));
            }
        }
        self.dirty = true;
    }

    /// The element under a point of the canvas, topmost first.
    fn pick(&self, ui: &Ui, x: f32, y: f32) -> Option<String> {
        let (_, layout) = self.open.as_ref()?;
        let c = ui.rect(self.canvas);
        let (sx, sy) = ((x - c.x) / c.width * WIDTH, (y - c.y) / c.height * HEIGHT);
        let screen = runity::glam::Vec2::new(WIDTH, HEIGHT);
        layout
            .elements
            .iter()
            .rev()
            .find(|e| {
                layout.rect(&e.id, screen).is_some_and(|r| {
                    sx >= r.x && sy >= r.y && sx < r.x + r.width && sy < r.y + r.height
                })
            })
            .map(|e| e.id.clone())
    }

    fn select(&mut self, ui: &mut Ui, id: Option<String>) {
        self.selected = id;
        self.show_elements(ui);
        self.show_fields(ui);
        self.dirty = true;
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        if node == self.wide_button {
            if let Event::Click { .. } = event {
                self.wide = !self.wide;
                set_icon_button(ui, self.wide_button, "expand", self.wide, true);
            }
            return;
        }
        if let Some(path) = self
            .files
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, p)| p.clone())
        {
            if let Event::Click { .. } = event {
                self.open(ui, session, path);
            }
            return;
        }
        if let Some(id) = self
            .elements
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, i)| i.clone())
        {
            if let Event::Click { .. } = event {
                self.select(ui, Some(id));
            }
            return;
        }
        if let Some(anchor) = self
            .anchors
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, a)| *a)
        {
            if let Event::Click { .. } = event {
                if let Some(e) = self.element_mut() {
                    // Where it is on screen stays: a new anchor, a new
                    // offset. Its corner is `at + (screen - size) * f`.
                    let (was, now) = (e.anchor.fraction(), anchor.fraction());
                    e.at.0 = (e.at.0 + (WIDTH - e.size.0) * (was.x - now.x)).round();
                    e.at.1 = (e.at.1 + (HEIGHT - e.size.1) * (was.y - now.y)).round();
                    e.anchor = anchor;
                }
                self.save(session);
                self.show_fields(ui);
            }
            return;
        }
        if node == self.canvas {
            match event {
                Event::Press { x, y, .. } => {
                    let hit = self.pick(ui, *x, *y);
                    if hit != self.selected {
                        self.select(ui, hit);
                    }
                }
                Event::Drag { dx, dy, .. } => {
                    let c = ui.rect(self.canvas);
                    let (kx, ky) = (WIDTH / c.width.max(1.0), HEIGHT / c.height.max(1.0));
                    if let Some(e) = self.element_mut() {
                        e.at.0 = (e.at.0 + dx * kx).round();
                        e.at.1 = (e.at.1 + dy * ky).round();
                        self.dirty = true;
                    }
                }
                Event::DragEnd { .. } => {
                    self.save(session);
                    self.show_fields(ui);
                }
                _ => {}
            }
            return;
        }
        if let Some(key) = self.boxes.iter().find(|(n, _)| *n == node).map(|(_, k)| *k) {
            let Event::Submit(value) = event else { return };
            let value = value.trim().to_string();
            let number = value.parse::<f32>().ok();
            let mut error = None;
            let mut renamed = None;
            if let Some(e) = self.element_mut() {
                match (key, number) {
                    ("id", _) if !value.is_empty() => {
                        renamed = Some(value.clone());
                        e.id = value.clone();
                    }
                    ("x", Some(n)) => e.at.0 = n,
                    ("y", Some(n)) => e.at.1 = n,
                    ("width", Some(n)) => e.size.0 = n.max(1.0),
                    ("height", Some(n)) => e.size.1 = n.max(1.0),
                    ("text size", Some(n)) => e.text_size = n.max(1.0),
                    ("kind", _) => match runity::ron::from_str::<Kind>(&value) {
                        Ok(kind) => e.kind = kind,
                        Err(err) => error = Some(format!("kind: {err}")),
                    },
                    _ => error = Some(format!("{key}: a number")),
                }
            }
            if let Some(id) = renamed {
                self.selected = Some(id);
            }
            match error {
                Some(err) => session.say(Level::Error, err),
                None => self.save(session),
            }
            self.show_elements(ui);
            self.show_fields(ui);
        }
    }

    /// Draw the screen as the game would, with the selected element
    /// outlined. The target is the session's GPU's; the window shows it.
    pub fn draw(&mut self, session: &Session) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let gpu = session.gpu();
        if self.target.is_none() {
            let target = OffscreenTarget::new(gpu, WIDTH as u32, HEIGHT as u32);
            self.renderer = Some(runity::ui_render::UiRenderer::new(gpu, &target));
            self.target = Some(target);
            self.new_target = true;
        }
        let (Some(target), Some(renderer)) = (&self.target, &mut self.renderer) else {
            return;
        };
        let mut ui = runity::ui::Ui::new();
        // The game's own backdrop is its scene; here, a plain dark one.
        ui.quad(runity::ui::Quad::new(
            0.0,
            0.0,
            WIDTH,
            HEIGHT,
            runity::glam::Vec4::from(BG.linear()),
        ));
        if let Some((_, layout)) = &self.open {
            let mut screen = runity::screen::Screen::from_layout(layout.clone());
            let mut widgets = runity::widgets::Widgets::new();
            let input = runity::input::Input::new();
            screen.draw(
                &mut widgets,
                &mut ui,
                &input,
                runity::glam::Vec2::new(WIDTH, HEIGHT),
            );
            if let Some(id) = &self.selected {
                if let Some(r) = layout.rect(id, runity::glam::Vec2::new(WIDTH, HEIGHT)) {
                    let c = runity::glam::Vec4::from(ACCENT.linear());
                    let t = 2.0;
                    for (x, y, w, h) in [
                        (r.x - t, r.y - t, r.width + 2.0 * t, t),
                        (r.x - t, r.y + r.height, r.width + 2.0 * t, t),
                        (r.x - t, r.y, t, r.height),
                        (r.x + r.width, r.y, t, r.height),
                    ] {
                        ui.quad(runity::ui::Quad::new(x, y, w, h, c));
                    }
                }
            }
        }
        renderer.render(gpu, target, &ui);
    }

    /// The canvas picture, for the window's renderer.
    pub fn target(&self) -> Option<&OffscreenTarget> {
        self.target.as_ref()
    }
}

fn line(ui: &mut Ui, parent: NodeId, name: &str, glyph: &str, label: &str) -> NodeId {
    let row = ui.add(
        parent,
        Style::row()
            .full_width()
            .height(22.0)
            .fixed()
            .padding_x(SPACE_2)
            .gap(SPACE_2)
            .center_items()
            .radius(RADIUS_SM)
            .hover(HOVER)
            .clickable(),
    );
    ui.set_name(row, name.to_string());
    icon(ui, row, glyph, MUTED);
    ui.add_text(row, text(), label);
    row
}

fn kind_icon(kind: &Kind) -> &'static str {
    match kind {
        Kind::Text(_) => "type",
        Kind::Panel => "square",
        Kind::Button(_) => "mouse-pointer-2",
        Kind::Toggle(_) => "check",
        Kind::Slider { .. } => "sliders-horizontal",
        Kind::Bar => "minus",
        Kind::Field(_) => "pencil",
        Kind::Choice { .. } => "chevron-down",
    }
}

fn trim(n: f32) -> String {
    let s = format!("{n:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `old` with every element that differs from `layout` rewritten in place,
/// or `None` when the file's elements cannot be matched one to one (then
/// the whole file is written anew).
fn patched(old: &str, layout: &Layout) -> Option<String> {
    let on_disk: Layout = runity::ron::from_str(old).ok()?;
    let spans = element_spans(old)?;
    if spans.len() != on_disk.elements.len() || spans.len() != layout.elements.len() {
        return None;
    }
    let mut text = old.to_string();
    for ((span, was), now) in spans
        .iter()
        .zip(&on_disk.elements)
        .zip(&layout.elements)
        .rev()
    {
        if was != now {
            text.replace_range(span.clone(), &element_text(now));
        }
    }
    let check: Layout = runity::ron::from_str(&text).ok()?;
    (check == *layout).then_some(text)
}

/// Where each element's `( … )` is in a screen file: the groups one level
/// inside the `elements` list, strings and comments skipped.
fn element_spans(text: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = text.as_bytes();
    let list = text.find("elements")?;
    let mut i = list + text[list..].find('[')? + 1;
    let (mut depth, mut start, mut spans) = (0usize, 0usize, Vec::new());
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += text[i + 2..].find("*/")? + 3;
            }
            b'(' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    spans.push(start..i + 1);
                }
            }
            b']' if depth == 0 => return Some(spans),
            _ => {}
        }
        i += 1;
    }
    None
}

/// One element on one line, as the files are written by hand.
fn element_text(e: &Element) -> String {
    let kind = runity::ron::to_string(&e.kind).unwrap_or_default();
    let size = if e.text_size == 18.0 {
        String::new()
    } else {
        format!(", text_size: {}", trim(e.text_size))
    };
    format!(
        "(id: {:?}, anchor: {:?}, at: ({}, {}), size: ({}, {}), kind: {kind}{size})",
        e.id,
        e.anchor,
        trim(e.at.0),
        trim(e.at.1),
        trim(e.size.0),
        trim(e.size.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_keeps_comments_and_the_other_lines() {
        let old = r#"// The main menu.
(elements: [
    // Big and on top.
    (id: "title", anchor: Top, at: (0, 60), size: (600, 60), kind: Text("A (b) \"c\""), text_size: 40),
    /* the one that starts */ (id: "play", anchor: Center, at: (0, 0), size: (240, 48), kind: Button("Play")),
])
"#;
        let mut layout: Layout = runity::ron::from_str(old).unwrap();
        layout.elements[1].at = (10.0, -4.5);
        let new = patched(old, &layout).expect("patched in place");
        assert!(new.contains("// The main menu."));
        assert!(new.contains("// Big and on top."));
        assert!(new.contains("/* the one that starts */ (id: \"play\""));
        assert!(new.contains("at: (10, -4.5)"));
        let title_line = old.lines().nth(3).unwrap();
        assert!(
            new.contains(title_line),
            "untouched line kept byte for byte"
        );
        let back: Layout = runity::ron::from_str(&new).unwrap();
        assert_eq!(back, layout);
    }
}
