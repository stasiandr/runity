//! A game's screens as files: menus and HUDs a designer edits while the
//! game runs.
//!
//! Unity builds a menu out of a Canvas, RectTransforms with anchors and a
//! Canvas Scaler; here a screen is one RON file in `ui/`, every element
//! anchored to a corner, an edge or the middle, sized in pixels of a
//! 1280×720 screen and scaled with the window's height — so a menu laid out
//! once is laid out at every resolution:
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

use glam::Vec2;
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
    fn fraction(self) -> Vec2 {
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
    let scale = screen.y / REFERENCE_HEIGHT;
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
        })
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
            .filter_map(|e| match &e.kind {
                Kind::Text(t) | Kind::Button(t) | Kind::Toggle(t) | Kind::Field(t) => {
                    Some(t.as_str())
                }
                Kind::Slider { label, .. } => Some(label.as_str()),
                Kind::Panel | Kind::Bar => None,
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
        let style = widgets.style;
        let scale = size.y / REFERENCE_HEIGHT;
        let elements = self.layout.elements.clone();
        for e in &elements {
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
                    ui.text(TextRun::new(
                        rect.x,
                        rect.y,
                        e.text_size * scale,
                        style.text,
                        label(text),
                    ));
                }
                Kind::Panel => {
                    ui.quad(Quad::new(
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        style.pressed,
                    ));
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
                    ui.quad(Quad::new(
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        style.idle,
                    ));
                    ui.quad(Quad::new(
                        rect.x,
                        rect.y,
                        rect.width * t,
                        rect.height,
                        style.accent,
                    ));
                }
            }
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
