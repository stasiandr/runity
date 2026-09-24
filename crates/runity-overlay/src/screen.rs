//! A game's screens as files: menus and HUDs a designer edits while the
//! game runs.
//!
//! Unity builds a menu out of a Canvas, RectTransforms with anchors and a
//! Canvas Scaler; here a screen is one RON file in `ui/`, every element
//! anchored to a corner, an edge or the middle, sized in pixels of a
//! 1280×720 screen and scaled with the window's height — or its width, on
//! a window narrower than 16:9 (a tablet, a phone held upright), so nothing
//! runs off the side — and a menu laid out once is laid out at every
//! resolution:
//!
//! ```text
//! (
//!     elements: [
//!         (id: "title",  anchor: Top,        at: (0, 60),   size: (600, 60), kind: Text("The Valley"), text_size: 40),
//!         (id: "play",   anchor: Center,     at: (0, 0),    size: (240, 48), kind: Button("Play")),
//!         (id: "volume", anchor: Center,     at: (0, 64),   size: (240, 32), kind: Slider(label: "Volume", min: 0.0, max: 1.0)),
//!         (id: "health", anchor: BottomLeft, at: (24, -24), size: (220, 18), kind: Bar),
//!     ],
//! )
//! ```
//!
//! The file says where things are and what they say; the game says what
//! they currently show — [`Screen::set_text`] for a score,
//! [`Screen::set_value`] for a health bar — and asks what the player did
//! from what [`Screen::draw`] returns: `if done.clicked("play")`. Saving
//! the file moves the button in the running game (DNA, postulate 1); a save
//! that does not parse is reported and the last good layout stays.

use std::collections::HashMap;
use std::path::Path;

use glam::{Vec2, Vec4};
use serde::{Deserialize, Serialize};

use crate::input::Input;
use crate::ui::{Quad, TextRun, Ui};
use crate::widgets::{Rect, Widgets};
use crate::Tuned;

/// Where on the screen an element is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Where the anchor is, as a fraction of the screen, and which point of
    /// the element sits on it — the same fraction of the element, so a
    /// bottom-right element hangs up and left from the corner.
    pub fn fraction(self) -> Vec2 {
        match self {
            Anchor::TopLeft => Vec2::new(0.0, 0.0),
            Anchor::Top => Vec2::new(0.5, 0.0),
            Anchor::TopRight => Vec2::new(1.0, 0.0),
            Anchor::Left => Vec2::new(0.0, 0.5),
            Anchor::Center => Vec2::new(0.5, 0.5),
            Anchor::Right => Vec2::new(1.0, 0.5),
            Anchor::BottomLeft => Vec2::new(0.0, 1.0),
            Anchor::Bottom => Vec2::new(0.5, 1.0),
            Anchor::BottomRight => Vec2::new(1.0, 1.0),
        }
    }
}

/// What an element is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// Words; the game may replace them with [`Screen::set_text`].
    Text(String),
    /// A box behind other elements.
    Panel,
    Button(String),
    Toggle(String),
    Slider {
        label: String,
        min: f32,
        max: f32,
    },
    /// A bar filled to a value in `0..=1`: health, loading, a timer.
    Bar,
    /// A line the player types; the words are shown greyed while it is
    /// empty. Read with [`Screen::entered`].
    Field(String),
    /// One of a few options; the chosen one's index is
    /// [`Screen::chosen`]. Options starting `@` are looked up in `strings/`.
    Choice {
        label: String,
        options: Vec<String>,
    },
    /// A row of tabs; the chosen one's index is [`Screen::chosen`].
    Tabs(Vec<String>),
    /// Rows the game fills ([`Screen::set_items`]) — saves, servers — one
    /// picked, [`Screen::picked`]. Rows `row` reference pixels tall.
    List {
        row: f32,
    },
}

/// One element of a screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// What the game calls it. Unique in the file.
    pub id: String,
    #[serde(default)]
    pub anchor: Anchor,
    /// From the anchor, in reference pixels; y grows down.
    #[serde(default)]
    pub at: (f32, f32),
    pub size: (f32, f32),
    pub kind: Kind,
    /// For text, in reference pixels.
    #[serde(default = "text_size")]
    pub text_size: f32,
    /// Where a text's words sit in its box: from the left (the default),
    /// in the middle, or to the right.
    #[serde(default, skip_serializing_if = "Align::is_left")]
    pub align: Align,
}

/// Where words sit across their box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    fn is_left(&self) -> bool {
        *self == Align::Left
    }

    /// Where across their box the words sit: 0 the left, 1 the right.
    pub fn across(self) -> f32 {
        match self {
            Align::Left => 0.0,
            Align::Center => 0.5,
            Align::Right => 1.0,
        }
    }
}

fn text_size() -> f32 {
    18.0
}

/// A screen's file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layout {
    pub elements: Vec<Element>,
}

/// The height a screen is laid out for; a taller window scales it up.
pub const REFERENCE_HEIGHT: f32 = 720.0;
/// The width it is laid out for: a window narrower than 16:9 scales by
/// this instead, as Unity's Canvas Scaler does in its "shrink" mode.
pub const REFERENCE_WIDTH: f32 = 1280.0;

/// How much a screen of `size` scales what was laid out at 1280×720.
pub fn scale_of(size: Vec2) -> f32 {
    (size.y / REFERENCE_HEIGHT).min(size.x / REFERENCE_WIDTH)
}

impl Layout {
    /// What cannot mean anything: two elements with one id.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, e) in self.elements.iter().enumerate() {
            if self.elements[..i].iter().any(|o| o.id == e.id) {
                out.push(format!("`{}` is the id of two elements", e.id));
            }
        }
        out
    }

    /// Where an element is on a screen of this size, in pixels.
    pub fn rect(&self, id: &str, screen: Vec2) -> Option<Rect> {
        self.elements
            .iter()
            .find(|e| e.id == id)
            .map(|e| place(e, screen))
    }
}

fn place(e: &Element, screen: Vec2) -> Rect {
    let scale = scale_of(screen);
    let size = Vec2::new(e.size.0, e.size.1) * scale;
    let f = e.anchor.fraction();
    let top_left = screen * f + Vec2::new(e.at.0, e.at.1) * scale - size * f;
    Rect::new(top_left.x, top_left.y, size.x, size.y)
}

/// What the player did to a screen this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    pub clicked: Vec<String>,
    /// Toggles flipped, sliders moved, fields typed into.
    pub changed: Vec<String>,
    /// Fields Enter was pressed in.
    pub submitted: Vec<String>,
}

impl Done {
    pub fn clicked(&self, id: &str) -> bool {
        self.clicked.iter().any(|c| c == id)
    }

    pub fn changed(&self, id: &str) -> bool {
        self.changed.iter().any(|c| c == id)
    }

    pub fn submitted(&self, id: &str) -> bool {
        self.submitted.iter().any(|c| c == id)
    }
}

/// A screen: its file, kept up with, and what its elements show now.
pub struct Screen {
    layout: Tuned<Layout>,
    texts: HashMap<String, String>,
    values: HashMap<String, f32>,
    toggles: HashMap<String, bool>,
    items: HashMap<String, Vec<String>>,
    /// Elements not drawn now, by id: a button only the host may press.
    hidden: std::collections::HashSet<String>,
}

impl Screen {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let layout = Tuned::<Layout>::load(path.as_ref())?;
        let problems = layout.problems();
        if !problems.is_empty() {
            return Err(format!(
                "{}: {}",
                path.as_ref().display(),
                problems.join("; ")
            ));
        }
        Ok(Self {
            layout,
            texts: HashMap::new(),
            values: HashMap::new(),
            toggles: HashMap::new(),
            items: HashMap::new(),
            hidden: Default::default(),
        })
    }

    /// A screen of a layout held in memory, not read from a file: an
    /// editor's picture of one being changed.
    pub fn from_layout(layout: Layout) -> Self {
        Self {
            layout: Tuned::fixed(layout),
            texts: HashMap::new(),
            values: HashMap::new(),
            toggles: HashMap::new(),
            items: HashMap::new(),
            hidden: Default::default(),
        }
    }

    /// Hide an element, or show it again: a hidden one is not drawn and
    /// cannot be pressed.
    pub fn set_hidden(&mut self, id: &str, hidden: bool) {
        if hidden {
            self.hidden.insert(id.to_string());
        } else {
            self.hidden.remove(id);
        }
    }

    /// Reread the file when it has changed, at most a few times a second:
    /// call it every frame. The error in words when the new text does not
    /// fit, with the last good layout kept.
    pub fn poll(&mut self, delta: f32) -> Option<Result<(), String>> {
        self.layout.poll(delta)
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Show these words in a text element (or on a button) instead of the
    /// file's.
    pub fn set_text(&mut self, id: &str, text: impl Into<String>) {
        self.texts.insert(id.to_string(), text.into());
    }

    /// Which option of a choice is chosen: its index, the first before any.
    pub fn chosen(&self, id: &str) -> usize {
        self.values.get(id).map_or(0, |v| v.max(0.0) as usize)
    }

    /// The rows of a list.
    pub fn set_items(&mut self, id: &str, items: Vec<String>) {
        self.items.insert(id.to_string(), items);
    }

    /// Which row of a list is picked; `None` before any.
    pub fn picked(&self, id: &str) -> Option<usize> {
        self.values
            .get(id)
            .filter(|v| **v >= 0.0)
            .map(|v| *v as usize)
    }

    /// Choose an option of a choice from code: the setting as saved.
    pub fn set_chosen(&mut self, id: &str, index: usize) {
        self.values.insert(id.to_string(), index as f32);
    }

    /// What the player typed into a field: its text, empty before any.
    pub fn entered(&self, id: &str) -> &str {
        self.texts.get(id).map_or("", String::as_str)
    }

    /// Fill a bar, or move a slider.
    pub fn set_value(&mut self, id: &str, value: f32) {
        self.values.insert(id.to_string(), value);
    }

    /// A slider's or a bar's value; 0 until set or moved.
    pub fn value(&self, id: &str) -> f32 {
        self.values.get(id).copied().unwrap_or(0.0)
    }

    pub fn set_toggled(&mut self, id: &str, on: bool) {
        self.toggles.insert(id.to_string(), on);
    }

    pub fn toggled(&self, id: &str) -> bool {
        self.toggles.get(id).copied().unwrap_or(false)
    }

    /// Draw every element for a screen of `size` pixels, in file order, and
    /// say what the player did.
    pub fn draw(&mut self, widgets: &mut Widgets, ui: &mut Ui, input: &Input, size: Vec2) -> Done {
        self.draw_in(widgets, ui, input, size, None)
    }

    /// [`Screen::draw`], with every `@key` text in the current language of
    /// `strings` (see [`crate::strings`]).
    pub fn draw_localized(
        &mut self,
        widgets: &mut Widgets,
        ui: &mut Ui,
        input: &Input,
        size: Vec2,
        strings: &crate::strings::Strings,
    ) -> Done {
        self.draw_in(widgets, ui, input, size, Some(strings))
    }

    /// Every `@key` the screen's texts ask for.
    pub fn keys(layout: &Layout) -> Vec<String> {
        layout
            .elements
            .iter()
            .flat_map(|e| match &e.kind {
                Kind::Text(t) | Kind::Button(t) | Kind::Toggle(t) | Kind::Field(t) => {
                    vec![t.as_str()]
                }
                Kind::Slider { label, .. } => vec![label.as_str()],
                Kind::Choice { label, options } => std::iter::once(label.as_str())
                    .chain(options.iter().map(String::as_str))
                    .collect(),
                Kind::Tabs(names) => names.iter().map(String::as_str).collect(),
                Kind::Panel | Kind::Bar | Kind::List { .. } => Vec::new(),
            })
            .filter_map(|t| t.strip_prefix('@').map(str::to_string))
            .collect()
    }

    fn draw_in(
        &mut self,
        widgets: &mut Widgets,
        ui: &mut Ui,
        input: &Input,
        size: Vec2,
        strings: Option<&crate::strings::Strings>,
    ) -> Done {
        let mut done = Done::default();
        let scale = scale_of(size);
        // The widgets' look grows with the screen, as the layout does, and
        // is theirs again after.
        let base = widgets.style;
        let style = base.scaled(scale);
        widgets.style = style;
        let elements = self.layout.elements.clone();
        for e in elements.iter().filter(|e| !self.hidden.contains(&e.id)) {
            let rect = place(e, size);
            let label = |own: &str| {
                let text = self
                    .texts
                    .get(&e.id)
                    .cloned()
                    .unwrap_or_else(|| own.to_string());
                match strings {
                    Some(strings) => strings.resolve(&text).to_string(),
                    None => text,
                }
            };
            match &e.kind {
                Kind::Text(text) => {
                    let words = label(text);
                    let size = e.text_size * scale;
                    ui.text(
                        TextRun::new(rect.x, rect.y, size, style.text, words)
                            .within(rect.width, e.align.across()),
                    );
                }
                Kind::Panel => {
                    if style.shadow > 0.0 {
                        let s = style.shadow;
                        ui.quad(
                            Quad::new(
                                rect.x + s,
                                rect.y + s,
                                rect.width,
                                rect.height,
                                Vec4::new(0.0, 0.0, 0.0, 0.25),
                            )
                            .rounded(style.radius),
                        );
                    }
                    ui.quad(
                        Quad::new(rect.x, rect.y, rect.width, rect.height, style.pressed)
                            .rounded(style.radius),
                    );
                }
                Kind::Button(text) => {
                    if widgets.button(ui, input, rect, &label(text)) {
                        done.clicked.push(e.id.clone());
                    }
                }
                Kind::Field(placeholder) => {
                    let mut value = self.texts.get(&e.id).cloned().unwrap_or_default();
                    let typed =
                        widgets.text_field(ui, input, rect, &label(placeholder), &mut value);
                    if typed.changed {
                        self.texts.insert(e.id.clone(), value);
                        done.changed.push(e.id.clone());
                    }
                    if typed.submitted {
                        done.submitted.push(e.id.clone());
                    }
                }
                Kind::Choice {
                    label: text,
                    options,
                } => {
                    let options: Vec<String> = options.iter().map(|o| label(o)).collect();
                    let mut chosen = self.chosen(&e.id);
                    if widgets.dropdown(ui, input, rect, &label(text), &options, &mut chosen) {
                        self.values.insert(e.id.clone(), chosen as f32);
                        done.changed.push(e.id.clone());
                    }
                }
                Kind::Tabs(names) => {
                    let names: Vec<String> = names.iter().map(|n| label(n)).collect();
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    let mut chosen = self.chosen(&e.id);
                    if widgets.tabs(ui, input, rect, &names, &mut chosen) {
                        self.values.insert(e.id.clone(), chosen as f32);
                        done.changed.push(e.id.clone());
                    }
                }
                Kind::List { row } => {
                    let items = self.items.get(&e.id).cloned().unwrap_or_default();
                    let mut picked = self.picked(&e.id);
                    if widgets.list(ui, input, rect, row * scale, &items, &mut picked) {
                        self.values
                            .insert(e.id.clone(), picked.map_or(-1.0, |p| p as f32));
                        done.changed.push(e.id.clone());
                    }
                }
                Kind::Toggle(text) => {
                    let mut on = self.toggled(&e.id);
                    if widgets.toggle(ui, input, rect, &label(text), &mut on) {
                        self.toggles.insert(e.id.clone(), on);
                        done.changed.push(e.id.clone());
                    }
                }
                Kind::Slider {
                    label: text,
                    min,
                    max,
                } => {
                    let mut value = self.values.get(&e.id).copied().unwrap_or(*min);
                    if widgets.slider(ui, input, rect, &label(text), &mut value, *min..=*max) {
                        self.values.insert(e.id.clone(), value);
                        done.changed.push(e.id.clone());
                    }
                }
                Kind::Bar => {
                    let t = self.value(&e.id).clamp(0.0, 1.0);
                    let r = style.radius.min(rect.height * 0.5);
                    ui.quad(
                        Quad::new(rect.x, rect.y, rect.width, rect.height, style.idle).rounded(r),
                    );
                    ui.quad(
                        Quad::new(rect.x, rect.y, rect.width * t, rect.height, style.accent)
                            .rounded(r),
                    );
                }
            }
        }
        widgets.style = base;
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_element_is_not_drawn_until_shown_again() {
        let layout: Layout = ron::from_str(
            r#"(elements: [(id: "start", anchor: Center, at: (0, 0), size: (200, 40), kind: Button("Start"))])"#,
        )
        .unwrap();
        let mut screen = Screen::from_layout(layout);
        let drawn = |screen: &mut Screen| {
            let mut ui = Ui::new();
            screen.draw(
                &mut Widgets::new(),
                &mut ui,
                &Input::default(),
                Vec2::new(1280.0, 720.0),
            );
            ui.texts.len()
        };
        screen.set_hidden("start", true);
        assert_eq!(drawn(&mut screen), 0);
        screen.set_hidden("start", false);
        assert_eq!(drawn(&mut screen), 1);
    }

    #[test]
    fn words_sit_left_in_the_middle_or_right_of_their_box() {
        let layout: Layout = ron::from_str(
            r#"(elements: [
                (id: "a", anchor: TopLeft, at: (0, 0), size: (400, 30), kind: Text("hi")),
                (id: "b", anchor: TopLeft, at: (0, 40), size: (400, 30), kind: Text("hi"), align: Center),
                (id: "c", anchor: TopLeft, at: (0, 80), size: (400, 30), kind: Text("hi"), align: Right),
            ])"#,
        )
        .unwrap();
        let mut screen = Screen::from_layout(layout);
        let mut ui = Ui::new();
        screen.draw(
            &mut Widgets::new(),
            &mut ui,
            &Input::default(),
            Vec2::new(1280.0, 720.0),
        );
        // The box and where across it: the renderer measures the words.
        let placed: Vec<_> = ui.texts.iter().map(|t| (t.x, t.within)).collect();
        assert_eq!(
            placed,
            [
                (0.0, Some((400.0, 0.0))),
                (0.0, Some((400.0, 0.5))),
                (0.0, Some((400.0, 1.0)))
            ]
        );
        // Left is not written: old files and new read the same.
        let text = ron::to_string(&screen.layout().elements[0]).unwrap();
        assert!(!text.contains("align"), "{text}");
    }
    use crate::input::{InputEvent, MouseButton};

    const MENU: &str = r#"(elements: [
        (id: "title", anchor: Top, at: (0, 60), size: (600, 60), kind: Text("The Valley"), text_size: 40),
        (id: "play", anchor: Center, size: (240, 48), kind: Button("Play")),
        (id: "health", anchor: BottomLeft, at: (24, -24), size: (220, 18), kind: Bar),
        (id: "mute", anchor: TopRight, at: (-20, 20), size: (120, 32), kind: Toggle("Mute")),
    ])"#;

    fn menu(name: &str) -> (Screen, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("runity-screen-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("menu.ron");
        std::fs::write(&path, MENU).unwrap();
        (Screen::load(&path).unwrap(), path)
    }

    #[test]
    fn anchors_keep_a_layout_in_place_at_every_size() {
        let (screen, _) = menu("anchors");
        let layout = screen.layout();
        let small = Vec2::new(1280.0, 720.0);
        let play = layout.rect("play", small).unwrap();
        assert_eq!((play.x, play.y), (520.0, 336.0), "centred");
        let health = layout.rect("health", small).unwrap();
        assert_eq!(
            (health.x, health.y + health.height),
            (24.0, 696.0),
            "up from the corner"
        );
        let mute = layout.rect("mute", small).unwrap();
        assert_eq!(mute.x + mute.width, 1260.0);

        // Twice as tall, and wider than 16:9: scaled by height, anchored
        // where it was — still centred, still in its corner.
        let big = Vec2::new(3440.0, 1440.0);
        let play = layout.rect("play", big).unwrap();
        assert_eq!((play.width, play.height), (480.0, 96.0));
        assert_eq!(play.x + play.width * 0.5, 1720.0);
        let mute = layout.rect("mute", big).unwrap();
        assert_eq!(mute.x + mute.width, 3440.0 - 40.0);

        // Narrower than 16:9 — 4:3 — scaled by width: the corner still in
        // the window.
        let square = Vec2::new(1024.0, 768.0);
        let mute = layout.rect("mute", square).unwrap();
        assert_eq!(mute.x + mute.width, 1024.0 - 20.0 * 0.8);
    }

    #[test]
    fn the_game_asks_what_was_clicked_and_says_what_to_show() {
        let (mut screen, _) = menu("clicks");
        let (mut widgets, mut ui) = (Widgets::new(), Ui::new());
        let size = Vec2::new(1280.0, 720.0);
        let play = screen.layout().rect("play", size).unwrap();
        let mut input = Input::new();
        input.begin_frame();
        input.handle(&InputEvent::MouseMoved {
            x: play.x + 5.0,
            y: play.y + 5.0,
        });
        input.handle(&InputEvent::MouseDown(MouseButton::Left));
        assert!(!screen
            .draw(&mut widgets, &mut ui, &input, size)
            .clicked("play"));
        input.begin_frame();
        input.handle(&InputEvent::MouseUp(MouseButton::Left));
        assert!(screen
            .draw(&mut widgets, &mut ui, &input, size)
            .clicked("play"));

        screen.set_value("health", 0.25);
        screen.set_text("title", "Night falls");
        ui.clear();
        screen.draw(&mut widgets, &mut ui, &Input::new(), size);
        assert!(ui.texts.iter().any(|t| t.text == "Night falls"));
        let health = screen.layout().rect("health", size).unwrap();
        assert!(ui
            .quads
            .iter()
            .any(|q| q.width == health.width * 0.25 && q.y == health.y));
    }

    #[test]
    fn tabs_and_a_list_the_game_fills_are_chosen_on_a_screen() {
        let layout: Layout = ron::from_str(
            r#"(elements: [
                (id: "pages", anchor: TopLeft, at: (0, 0), size: (300, 30), kind: Tabs(["Video", "Sound"])),
                (id: "saves", anchor: TopLeft, at: (0, 40), size: (300, 100), kind: List(row: 25)),
            ])"#,
        )
        .unwrap();
        let mut screen = Screen::from_layout(layout);
        screen.set_items("saves", vec!["Autumn".into(), "Spring".into()]);
        let size = Vec2::new(1280.0, 720.0);
        let scale = 720.0 / REFERENCE_HEIGHT;
        let mut widgets = Widgets::new();
        let mut ui = Ui::new();
        let mut click = |screen: &mut Screen, x: f32, y: f32| {
            let mut input = Input::new();
            input.begin_frame();
            input.handle(&InputEvent::MouseMoved {
                x: x * scale,
                y: y * scale,
            });
            input.handle(&InputEvent::MouseDown(MouseButton::Left));
            screen.draw(&mut widgets, &mut ui, &input, size);
            input.begin_frame();
            input.handle(&InputEvent::MouseUp(MouseButton::Left));
            screen.draw(&mut widgets, &mut ui, &input, size)
        };
        assert_eq!(screen.picked("saves"), None);
        assert!(click(&mut screen, 200.0, 15.0).changed("pages"));
        assert_eq!(screen.chosen("pages"), 1);
        assert!(click(&mut screen, 50.0, 75.0).changed("saves"));
        assert_eq!(screen.picked("saves"), Some(1), "Spring");
        assert!(ui.texts.iter().any(|t| t.text == "Spring"));
    }

    #[test]
    fn a_saved_layout_moves_the_button_and_a_broken_one_keeps_the_last() {
        let (mut screen, path) = menu("reload");
        let size = Vec2::new(1280.0, 720.0);
        std::fs::write(
            &path,
            MENU.replace("anchor: Center, size", "anchor: Bottom, at: (0, -40), size"),
        )
        .unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(screen.poll(1.0), Some(Ok(())));
        let play = screen.layout().rect("play", size).unwrap();
        assert_eq!(play.y + play.height, 680.0);

        std::fs::write(&path, "(elements: [(id: ").unwrap();
        let later = later + std::time::Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(matches!(screen.poll(1.0), Some(Err(_))));
        assert!(
            screen.layout().rect("play", size).is_some(),
            "the last good one"
        );
        assert!(Layout {
            elements: vec![
                screen.layout().elements[1].clone(),
                screen.layout().elements[1].clone()
            ]
        }
        .problems()[0]
            .contains("`play` is the id of two elements"));
    }

    #[test]
    fn a_screen_speaks_the_current_language() {
        let dir = std::env::temp_dir().join("runity-screen-strings");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ru.ron"), r#"{"menu.play": "Играть"}"#).unwrap();
        let path = dir.join("menu.ron");
        std::fs::write(
            &path,
            r#"(elements: [(id: "play", size: (200, 40), kind: Button("@menu.play"))])"#,
        )
        .unwrap();
        let mut screen = Screen::load(&path).unwrap();
        assert_eq!(Screen::keys(screen.layout()), ["menu.play"]);
        let strings = crate::strings::Strings::load(&dir, "ru").unwrap();
        let mut ui = Ui::new();
        screen.draw_localized(
            &mut Widgets::new(),
            &mut ui,
            &Input::new(),
            Vec2::new(1280.0, 720.0),
            &strings,
        );
        assert!(
            ui.texts.iter().any(|t| t.text == "Играть"),
            "{:?}",
            ui.texts
        );
    }
}
