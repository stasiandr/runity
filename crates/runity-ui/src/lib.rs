//! runity-ui: one retained UI for the game and the editor.
//!
//! A tree of nodes lives between frames. A node has a [`Style`] (taffy's
//! flexbox layout plus a look: fill, border, radius, text), maybe some
//! text, maybe an image, and children. Changing a node marks only what the
//! change touches: new text reshapes that one run, a new size relays the
//! tree, a new colour repaints. A frame where nothing changed costs
//! nothing — the renderer draws last frame's buffers again.
//!
//! Input goes in as the engine's [`InputEvent`]s and comes out as data:
//! [`Ui::events`] is the list of what happened to which node — a click, a
//! key, a drag. A game reads it in a system, the editor in a panel's
//! update, a test by looking at the list.
//!
//! Everything is headless. [`Ui::dump`] is the tree as text, and
//! [`render::UiRenderer`] draws it into any `wgpu` target on the engine's
//! own device — a window, or an off-screen picture in a test.
//!
//! `docs/ui.md` is the card this implements.

mod field;
mod icons;
pub mod render;
mod style;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};
use runity::input::{InputEvent, Key, MouseButton};
use taffy::{AvailableSpace, TaffyTree};

pub use field::Clipboard;
pub use style::{Color, Look, Sense, Style, TextStyle};

/// The font every UI is set in unless told otherwise: Inter, shipped with
/// the crate (SIL Open Font License, `assets/fonts/Inter-LICENSE.txt`), so a
/// UI looks the same on every machine and a golden picture of it holds.
static INTER: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
/// The family name inside [`INTER`].
pub const SANS: &str = "Inter Variable";

/// A node of the tree. Stays valid until the node is removed; a removed
/// node's id never comes back as someone else's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(taffy::NodeId);

/// A picture a node shows, by a number the renderer knows it by: the Scene
/// view's frame, an icon sheet, a thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u32);

/// A rectangle in logical pixels from the top left of the window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub(crate) fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        Rect {
            x,
            y,
            width: (right - x).max(0.0),
            height: (bottom - y).max(0.0),
        }
    }

    const EVERYWHERE: Rect = Rect {
        x: -1.0e6,
        y: -1.0e6,
        width: 2.0e6,
        height: 2.0e6,
    };
}

/// What happened to a node, as [`Ui::events`] reports it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The pointer came over it.
    Enter,
    /// The pointer left it.
    Leave,
    /// A button went down on it.
    Press {
        button: MouseButton,
        x: f32,
        y: f32,
    },
    /// A button went up after going down on it — wherever the pointer is.
    Release {
        button: MouseButton,
    },
    /// Down and up on it without dragging. `count` is 2 for a double click.
    Click {
        button: MouseButton,
        count: u32,
    },
    /// Pressed on it and moved: it is being dragged. `dx`, `dy` since the
    /// last one.
    Drag {
        dx: f32,
        dy: f32,
        x: f32,
        y: f32,
    },
    /// A drag that started on it ended over `over` (a node that senses
    /// clicks), or over nothing.
    DragEnd {
        over: Option<NodeId>,
    },
    /// It has the keyboard now.
    Focus,
    /// It lost the keyboard.
    Blur,
    /// A key went down while it had the keyboard.
    KeyDown(Key),
    KeyUp(Key),
    /// Characters typed while it had the keyboard.
    Text(String),
    /// The wheel over it, when it scrolls nothing itself: the Scene view
    /// takes this to zoom.
    Wheel {
        x: f32,
        y: f32,
    },
    /// The pointer moved over it. Only for nodes that sense drags, which
    /// want to know where the pointer is — a viewport.
    Move {
        x: f32,
        y: f32,
    },
    /// A text field's text as it is being typed.
    Changed(String),
    /// A text field was committed: Enter, Tab, or the keyboard going
    /// elsewhere after a change.
    Submit(String),
    /// Escape in a text field: it went back to what it said before.
    Cancel,
}

/// A text run's shaped lines, kept between frames.
struct TextBox {
    string: String,
    buffer: Buffer,
    /// The style it was shaped with: reshaped only when this changes.
    style: TextStyle,
    /// The width it was last laid out at for drawing.
    drawn_at: Option<Option<f32>>,
}

struct Node {
    style: Style,
    name: Option<String>,
    text: Option<TextBox>,
    image: Option<ImageId>,
    /// How far its children are scrolled up, for a node that clips.
    scroll: f32,
    /// Where it was drawn, absolute, after the last layout.
    rect: Rect,
    /// What its children are clipped to where it was drawn.
    clip: Rect,
    /// The children [`Ui::sync_children`] made, by the hash of their key.
    keyed: HashMap<u64, NodeId>,
    /// Starts a new layer: drawn over everything before it, text and all.
    layer: bool,
    /// An icon, by its number in the built-in set.
    icon: Option<u16>,
    /// What makes it a text field: the caret, the selection, what it said
    /// when it got the keyboard.
    field: Option<field::FieldState>,
    /// A field's hint, shown while it is empty: a node of its own.
    placeholder: Option<NodeId>,
}

impl Node {
    fn new(style: Style) -> Self {
        Self {
            style,
            name: None,
            text: None,
            image: None,
            scroll: 0.0,
            rect: Rect::default(),
            clip: Rect::EVERYWHERE,
            keyed: HashMap::new(),
            layer: false,
            icon: None,
            field: None,
            placeholder: None,
        }
    }
}

/// One rectangle to draw: fill, border and round corners, clipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPaint {
    pub rect: Rect,
    pub fill: Color,
    pub border: Color,
    pub border_width: f32,
    pub radius: f32,
    pub clip: Rect,
}

/// A node's text, where to draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPaint {
    pub node: NodeId,
    pub x: f32,
    pub y: f32,
    pub clip: Rect,
    pub color: Color,
}

/// A picture, where to draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImagePaint {
    pub image: ImageId,
    pub rect: Rect,
    pub clip: Rect,
    pub radius: f32,
}

/// What to draw, in order: each layer's rectangles, then its pictures, then
/// its text, and the next layer over all of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub rects: Vec<RectPaint>,
    pub images: Vec<ImagePaint>,
    pub texts: Vec<TextPaint>,
    pub icons: Vec<IconPaint>,
}

/// An icon, where to draw it and in what colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconPaint {
    pub icon: u16,
    pub rect: Rect,
    pub clip: Rect,
    pub color: Color,
}

/// A pointer press that may become a click or a drag.
#[derive(Debug, Clone, Copy)]
struct Press {
    node: NodeId,
    button: MouseButton,
    at: (f32, f32),
    dragging: bool,
}

/// The tree, the fonts, and what the pointer and keyboard are doing to it.
pub struct Ui {
    tree: TaffyTree<Node>,
    root: taffy::NodeId,
    fonts: FontSystem,
    width: f32,
    height: f32,
    scale: f32,
    /// Something moved or resized: relayout before the next paint.
    layout_dirty: bool,
    /// Something looks different: repaint.
    paint_dirty: bool,
    /// Bumped every time the paint list changes, so a renderer knows
    /// whether to upload anything.
    revision: u64,
    layers: Vec<Layer>,
    /// Nodes in paint order, for hit testing from the top.
    order: Vec<NodeId>,
    pointer: (f32, f32),
    hovered: Option<NodeId>,
    press: Option<Press>,
    focused: Option<NodeId>,
    last_click: Option<(NodeId, Instant, u32)>,
    events: Vec<(NodeId, Event)>,
    shift: bool,
    ctrl: bool,
    alt: bool,
    command: bool,
    clipboard: Box<dyn Clipboard>,
}

/// How far a press moves before it is a drag rather than a click.
const DRAG_SLOP: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

fn fonts() -> FontSystem {
    let mut db = glyphon::cosmic_text::fontdb::Database::new();
    db.load_font_data(INTER.to_vec());
    db.set_sans_serif_family(SANS);
    // The system's monospace, if there is one: values that are code read
    // better in it, and a missing one falls back to Inter.
    #[cfg(target_os = "macos")]
    db.load_font_file("/System/Library/Fonts/Menlo.ttc").ok();
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Self {
        let mut tree = TaffyTree::new();
        let root = tree
            .new_leaf_with_context(
                Style::column().full().layout,
                Node::new(Style::column().full()),
            )
            .expect("a new tree takes a root");
        Self {
            tree,
            root,
            fonts: fonts(),
            width: 800.0,
            height: 600.0,
            scale: 1.0,
            layout_dirty: true,
            paint_dirty: true,
            revision: 0,
            layers: Vec::new(),
            order: Vec::new(),
            pointer: (-1.0, -1.0),
            hovered: None,
            press: None,
            focused: None,
            last_click: None,
            events: Vec::new(),
            shift: false,
            ctrl: false,
            alt: false,
            command: false,
            clipboard: Box::new(field::LocalClipboard::default()),
        }
    }

    /// The window's size in logical pixels, and its pixels per point.
    pub fn set_viewport(&mut self, width: f32, height: f32, scale: f32) {
        if (width, height, scale) != (self.width, self.height, self.scale) {
            self.width = width;
            self.height = height;
            self.scale = scale;
            self.layout_dirty = true;
        }
    }

    pub fn viewport(&self) -> (f32, f32, f32) {
        (self.width, self.height, self.scale)
    }

    /// The node everything else hangs from: the whole window.
    pub fn root(&self) -> NodeId {
        NodeId(self.root)
    }

    fn node(&self, id: NodeId) -> &Node {
        self.tree
            .get_node_context(id.0)
            .expect("a node id outlived its node")
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.tree
            .get_node_context_mut(id.0)
            .expect("a node id outlived its node")
    }

    /// Whether `id` is still in the tree.
    pub fn exists(&self, id: NodeId) -> bool {
        self.tree.get_node_context(id.0).is_some()
    }

    // --- building -----------------------------------------------------

    /// A new node at the end of `parent`'s children.
    pub fn add(&mut self, parent: NodeId, style: Style) -> NodeId {
        let id = self
            .tree
            .new_leaf_with_context(style.layout.clone(), Node::new(style))
            .expect("taffy takes any leaf");
        self.tree
            .add_child(parent.0, id)
            .expect("the parent is in the tree");
        self.layout_dirty = true;
        NodeId(id)
    }

    /// A new node that shows `text`, set in its style's [`TextStyle`].
    pub fn add_text(&mut self, parent: NodeId, style: Style, text: &str) -> NodeId {
        let id = self.add(parent, style);
        self.set_text(id, text);
        id
    }

    /// A one-line text field showing `text`. Typing into it is handled
    /// here — caret, selection, clipboard, Home and End — and reported as
    /// [`Event::Changed`] while typing, [`Event::Submit`] on Enter, Tab or
    /// leaving it changed, [`Event::Cancel`] on Escape.
    pub fn add_field(&mut self, parent: NodeId, style: Style, text: &str) -> NodeId {
        let style = style.focusable().nowrap();
        let id = self.add(parent, style);
        self.node_mut(id).field = Some(field::FieldState::default());
        self.set_text(id, text);
        id
    }

    /// What an empty field shows until something is typed: «Search».
    pub fn set_placeholder(&mut self, field: NodeId, hint: &str) {
        let hint_node = match self.node(field).placeholder {
            Some(n) => n,
            None => {
                let s = self.style(field).text.clone();
                // A box over the field, the hint centred in it as the
                // field's own text is.
                let over = self.add(
                    field,
                    Style::row()
                        .absolute(0.0, 0.0)
                        .full()
                        .padding_left(6.0)
                        .center_items(),
                );
                let n = self.add(
                    over,
                    Style::default()
                        .text_size(s.size)
                        .text_color(s.color.alpha(40))
                        .nowrap(),
                );
                self.node_mut(field).placeholder = Some(n);
                n
            }
        };
        self.set_text(hint_node, hint);
        self.sync_placeholder(field);
    }

    fn sync_placeholder(&mut self, field: NodeId) {
        let Some(hint) = self.node(field).placeholder.and_then(|h| self.parent(h)) else {
            return;
        };
        let empty = self.text(field).is_none_or(str::is_empty);
        let shown = self
            .tree
            .style(hint.0)
            .is_ok_and(|s| s.display != taffy::Display::None);
        if empty != shown {
            self.restyle(hint, |s| if empty { s.shown() } else { s.hidden() });
        }
    }

    /// Whether `id` is a text field: a window routes keys to it rather than
    /// to its own shortcuts while it has the keyboard.
    pub fn is_field(&self, id: NodeId) -> bool {
        self.exists(id) && self.node(id).field.is_some()
    }

    /// A built-in icon by name (`"play"`, `"move-3d"`), drawn in the
    /// node's text colour at the node's size. `None` for a name the set
    /// does not have.
    pub fn add_icon(&mut self, parent: NodeId, style: Style, name: &str) -> NodeId {
        let id = self.add(parent, style);
        self.set_icon(id, name);
        id
    }

    /// Show an icon, or change which one.
    pub fn set_icon(&mut self, id: NodeId, name: &str) {
        let icon = icons::find(name);
        debug_assert!(icon.is_some(), "no icon called {name:?}");
        if self.node(id).icon != icon {
            self.node_mut(id).icon = icon;
            self.paint_dirty = true;
        }
    }

    /// The names of the built-in icons.
    pub fn icon_names() -> impl Iterator<Item = &'static str> {
        icons::ICONS.iter().map(|(n, _)| *n)
    }

    /// A new node that shows a picture, stretched to its box.
    pub fn add_image(&mut self, parent: NodeId, style: Style, image: ImageId) -> NodeId {
        let id = self.add(parent, style);
        self.node_mut(id).image = Some(image);
        id
    }

    /// Make `id` start a layer of its own: it and what is under it draw
    /// over everything drawn before — a popup, a menu, a dragged thing.
    pub fn set_layer(&mut self, id: NodeId, layer: bool) {
        if self.node(id).layer != layer {
            self.node_mut(id).layer = layer;
            self.paint_dirty = true;
        }
    }

    /// Give a node a name: what [`Ui::find`] looks for and what
    /// [`Ui::dump`] prints, so a test or an agent can point at a control
    /// without knowing where it is.
    pub fn set_name(&mut self, id: NodeId, name: impl Into<String>) {
        self.node_mut(id).name = Some(name.into());
    }

    pub fn name(&self, id: NodeId) -> Option<&str> {
        self.node(id).name.as_deref()
    }

    /// Remove a node and everything under it.
    pub fn remove(&mut self, id: NodeId) {
        if id.0 == self.root || !self.exists(id) {
            return;
        }
        for child in self.children(id) {
            self.remove(child);
        }
        if let Some(parent) = self.tree.parent(id.0) {
            let _ = self.tree.remove_child(parent, id.0);
            // A keyed child forgets its key along with its parent's map.
            if let Some(p) = self.tree.get_node_context_mut(parent) {
                p.keyed.retain(|_, child| *child != id);
            }
        }
        // taffy's `remove` keeps the node's context (ours: its style, its
        // shaped text); dropping it here is what frees them.
        let _ = self.tree.set_node_context(id.0, None);
        let _ = self.tree.remove(id.0);
        for slot in [&mut self.hovered, &mut self.focused] {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        if self.press.is_some_and(|p| p.node == id) {
            self.press = None;
        }
        self.layout_dirty = true;
    }

    /// Move a node, with everything under it, to the end of another
    /// parent's children: a panel docked elsewhere keeps its nodes, its
    /// scroll and its focus.
    pub fn move_to(&mut self, id: NodeId, parent: NodeId) {
        if id.0 == self.root || !self.exists(id) || !self.exists(parent) {
            return;
        }
        if let Some(old) = self.tree.parent(id.0) {
            let _ = self.tree.remove_child(old, id.0);
            if let Some(p) = self.tree.get_node_context_mut(old) {
                p.keyed.retain(|_, child| *child != id);
            }
        }
        let _ = self.tree.add_child(parent.0, id.0);
        self.layout_dirty = true;
    }

    /// Remove every child of `id`.
    pub fn clear(&mut self, id: NodeId) {
        for child in self.children(id) {
            self.remove(child);
        }
    }

    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.tree
            .children(id.0)
            .unwrap_or_default()
            .into_iter()
            .map(NodeId)
            .collect()
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.tree.parent(id.0).map(NodeId)
    }

    /// Make `parent`'s children one per key, in the keys' order: a node
    /// whose key is still there is kept (and `update`d), a new key gets a
    /// node from `make`, a key that went takes its node with it. A list
    /// from data — the Hierarchy's lines, the Console's — changes by what
    /// changed rather than being built again.
    pub fn sync_children<K: Hash>(
        &mut self,
        parent: NodeId,
        keys: &[K],
        mut make: impl FnMut(&mut Ui, NodeId, &K) -> NodeId,
        mut update: impl FnMut(&mut Ui, NodeId, &K),
    ) {
        let hash = |key: &K| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            h.finish()
        };
        let mut wanted = Vec::with_capacity(keys.len());
        let mut seen = std::collections::HashSet::with_capacity(keys.len());
        for key in keys {
            let k = hash(key);
            seen.insert(k);
            let existing = self.node(parent).keyed.get(&k).copied();
            let id = match existing.filter(|id| self.exists(*id)) {
                Some(id) => {
                    update(self, id, key);
                    id
                }
                None => {
                    let id = make(self, parent, key);
                    self.node_mut(parent).keyed.insert(k, id);
                    id
                }
            };
            wanted.push(id);
        }
        let gone: Vec<NodeId> = self
            .node(parent)
            .keyed
            .iter()
            .filter(|(k, _)| !seen.contains(k))
            .map(|(_, id)| *id)
            .collect();
        for id in gone {
            self.remove(id);
        }
        let now: Vec<taffy::NodeId> = wanted.iter().map(|id| id.0).collect();
        if self.tree.children(parent.0).unwrap_or_default() != now {
            let _ = self.tree.set_children(parent.0, &now);
            self.layout_dirty = true;
        }
    }

    // --- changing -----------------------------------------------------

    pub fn style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }

    /// Replace a node's style. What changed decides what is redone: layout
    /// only if the layout did, a reshape only if the text style did.
    pub fn set_style(&mut self, id: NodeId, style: Style) {
        let node = self.node(id);
        if node.style == style {
            return;
        }
        let relayout = node.style.layout != style.layout || node.style.text != style.text;
        if relayout {
            let _ = self.tree.set_style(id.0, style.layout.clone());
            self.layout_dirty = true;
        }
        self.node_mut(id).style = style;
        self.paint_dirty = true;
    }

    /// Change a node's style in place: `ui.restyle(id, |s| s.background(c))`.
    pub fn restyle(&mut self, id: NodeId, change: impl FnOnce(Style) -> Style) {
        let style = change(self.style(id).clone());
        self.set_style(id, style);
    }

    pub fn text(&self, id: NodeId) -> Option<&str> {
        self.node(id).text.as_ref().map(|t| t.string.as_str())
    }

    /// Set a node's text. The same text again costs nothing.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if self.text(id) == Some(text) {
            return;
        }
        let node = self.tree.get_node_context_mut(id.0).expect("a live node");
        match &mut node.text {
            Some(t) => t.string = text.to_string(),
            None => {
                let s = &node.style.text;
                let metrics = Metrics::new(s.size, s.size * s.line_height);
                node.text = Some(TextBox {
                    string: text.to_string(),
                    buffer: Buffer::new(&mut self.fonts, metrics),
                    style: s.clone(),
                    drawn_at: None,
                });
            }
        }
        shape(&mut self.fonts, node);
        // Its size depends on its text.
        let _ = self.tree.mark_dirty(id.0);
        self.layout_dirty = true;
        self.sync_placeholder(id);
    }

    pub fn set_image(&mut self, id: NodeId, image: Option<ImageId>) {
        if self.node(id).image != image {
            self.node_mut(id).image = image;
            self.paint_dirty = true;
        }
    }

    // --- layout and paint ---------------------------------------------

    /// Lay out what changed since the last call.
    pub fn layout(&mut self) {
        if !self.layout_dirty {
            return;
        }
        let started = Instant::now();
        let root_style = self.tree.style(self.root).cloned().unwrap_or_default();
        let sized = taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::length(self.width),
                height: taffy::prelude::length(self.height),
            },
            ..root_style
        };
        let _ = self.tree.set_style(self.root, sized);
        let fonts = &mut self.fonts;
        let _ = self.tree.compute_layout_with_measure(
            self.root,
            taffy::Size {
                width: AvailableSpace::Definite(self.width),
                height: AvailableSpace::Definite(self.height),
            },
            |inputs, _id, node, style| {
                taffy::compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known, available| measure(fonts, node, known, available),
                )
            },
        );
        self.layout_dirty = false;
        self.paint_dirty = true;
        if std::env::var_os("RUNITY_UI_TIMING").is_some() {
            eprintln!(
                "runity-ui: layout {:.1} ms",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    }

    /// Whether anything needs doing before the next frame looks right: a
    /// window that gets `false` can skip the frame.
    pub fn is_dirty(&self) -> bool {
        self.layout_dirty || self.paint_dirty
    }

    /// A number that changes whenever [`Ui::paint`] would return something
    /// new.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// What to draw: laid out and painted again only if something changed.
    pub fn paint(&mut self) -> &[Layer] {
        self.layout();
        if self.paint_dirty {
            let started = Instant::now();
            self.layers.clear();
            self.layers.push(Layer::default());
            self.order.clear();
            self.paint_node(NodeId(self.root), 0.0, 0.0, Rect::EVERYWHERE, 1.0);
            self.layers.retain(|l| {
                !(l.rects.is_empty()
                    && l.texts.is_empty()
                    && l.images.is_empty()
                    && l.icons.is_empty())
            });
            self.paint_dirty = false;
            self.revision += 1;
            if std::env::var_os("RUNITY_UI_TIMING").is_some() {
                eprintln!(
                    "runity-ui: paint {:.1} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
        }
        &self.layers
    }

    fn paint_node(&mut self, id: NodeId, parent_x: f32, parent_y: f32, clip: Rect, opacity: f32) {
        let Ok(layout) = self.tree.layout(id.0).cloned() else {
            return;
        };
        if self
            .tree
            .style(id.0)
            .is_ok_and(|s| s.display == taffy::Display::None)
        {
            return;
        }
        let rect = Rect {
            x: parent_x + layout.location.x,
            y: parent_y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        // The text is laid out at the width the node ended up with; the
        // measure may have tried others on the way.
        if let Some(node) = self.tree.get_node_context_mut(id.0) {
            if let Some(text) = node.text.as_mut() {
                let width = if node.style.text.nowrap {
                    None
                } else {
                    Some(layout.content_box_width())
                };
                if text.drawn_at != Some(width) {
                    text.buffer.set_size(width, None);
                    text.buffer.shape_until_scroll(&mut self.fonts, false);
                    text.drawn_at = Some(width);
                }
            }
        }
        // Outside what its ancestors show: nothing of it can be seen or
        // clicked, so neither it nor what is under it is painted — a list
        // of thousands costs the lines in view.
        let visible = clip.intersect(&rect);
        if (visible.width <= 0.0 || visible.height <= 0.0) && !self.node(id).layer {
            self.node_mut(id).rect = rect;
            return;
        }
        let hovered = self.hovered == Some(id);
        let pressed = self.press.is_some_and(|p| p.node == id) && hovered;
        let node = self.node_mut(id);
        node.rect = rect;
        let look = node.style.look.clone();
        let opacity = opacity * look.opacity;
        let fade = |c: Color| Color {
            a: (c.a as f32 * opacity) as u8,
            ..c
        };
        if node.layer {
            self.layers.push(Layer::default());
        }
        let node = self.node(id);
        let fill = if pressed {
            look.press_background
                .or(look.hover_background)
                .unwrap_or(look.background)
        } else if hovered {
            look.hover_background.unwrap_or(look.background)
        } else {
            look.background
        };
        let border = if hovered {
            look.hover_border.unwrap_or(look.border)
        } else {
            look.border
        };
        let image = node.image;
        let icon = node.icon;
        let is_field = node.field.is_some();
        let focused = self.focused == Some(id);
        let has_text = node.text.is_some();
        let text_color = node.style.text.color;
        let scroll = node.scroll;
        let layer = self.layers.last_mut().expect("there is always a layer");
        if fill.is_visible() || (border.is_visible() && look.border_width > 0.0) {
            layer.rects.push(RectPaint {
                rect,
                fill: fade(fill),
                border: fade(border),
                border_width: look.border_width,
                radius: look.radius,
                clip,
            });
        }
        if let Some(image) = image {
            layer.images.push(ImagePaint {
                image,
                rect,
                clip,
                radius: look.radius,
            });
        }
        if let Some(icon) = icon {
            layer.icons.push(IconPaint {
                icon,
                rect,
                clip,
                color: fade(text_color),
            });
        }
        if has_text {
            let pad = &layout.padding;
            let border_w = &layout.border;
            let x = rect.x + pad.left + border_w.left;
            // The text sits in the middle of the box's height, as a label
            // in a button or the value in a field does: a box taller than
            // its text — a 22 px field with a 16 px line — would otherwise
            // hold it at the top.
            let top = rect.y + pad.top + border_w.top;
            let room = layout.size.height - pad.top - pad.bottom - border_w.top - border_w.bottom;
            let text_height = self.text_height(id);
            let y = top + ((room - text_height) / 2.0).max(0.0);
            let inner = Rect {
                x,
                y: rect.y,
                width: layout.content_box_width(),
                height: rect.height,
            };
            let offset = self.paint_field(id, x, y, inner, clip, focused);
            let layer = self.layers.last_mut().expect("there is always a layer");
            layer.texts.push(TextPaint {
                node: id,
                x: x - offset,
                y,
                clip: clip.intersect(&if is_field { inner } else { rect }),
                color: fade(text_color),
            });
        }
        self.order.push(id);
        let inner_clip = if look.clip {
            clip.intersect(&rect)
        } else {
            clip
        };
        self.node_mut(id).clip = inner_clip;
        for child in self.children(id) {
            self.paint_node(child, rect.x, rect.y - scroll, inner_clip, opacity);
        }
        if self.node(id).layer {
            // What comes after this subtree is over it again only if it
            // starts its own layer; otherwise it continues on a new one so
            // that it is not drawn under the popup's text.
            self.layers.push(Layer::default());
        }
    }

    /// Where a node was drawn, after the last [`Ui::paint`].
    pub fn rect(&self, id: NodeId) -> Rect {
        self.node(id).rect
    }

    /// How tall a node's text is as laid out: its lines times the line
    /// height.
    fn text_height(&self, id: NodeId) -> f32 {
        let Some(text) = self.node(id).text.as_ref() else {
            return 0.0;
        };
        let lines = text.buffer.layout_runs().count().max(1);
        lines as f32 * text.buffer.metrics().line_height
    }

    /// The fonts and every node's shaped text at once, for the renderer:
    /// glyphon rasterizes from one while reading the other.
    pub(crate) fn text_parts(&mut self) -> (&mut FontSystem, TextBuffers<'_>) {
        (&mut self.fonts, TextBuffers(&self.tree))
    }

    // --- input --------------------------------------------------------

    /// The top node under a point that the pointer can act on.
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeId> {
        self.order.iter().rev().copied().find(|id| {
            let node = self.node(*id);
            let s = node.style.sense;
            (s.click || s.drag || s.focus || node.style.look.clip)
                && node.rect.contains(x, y)
                && self.clip_of(*id).contains(x, y)
        })
    }

    /// What a node is clipped to: its parent's clip.
    fn clip_of(&self, id: NodeId) -> Rect {
        self.parent(id)
            .map(|p| self.node(p).clip)
            .unwrap_or(Rect::EVERYWHERE)
    }

    /// The nearest node at or above `id` that senses something other than
    /// scrolling: a click on a button's label is a click on the button.
    fn sensing(&self, mut id: NodeId, want: impl Fn(Sense) -> bool) -> Option<NodeId> {
        loop {
            if want(self.node(id).style.sense) {
                return Some(id);
            }
            id = self.parent(id)?;
        }
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Give `id` the keyboard, or nobody.
    pub fn focus(&mut self, id: Option<NodeId>) {
        if self.focused == id {
            return;
        }
        if let Some(old) = self.focused {
            self.field_blur(old);
            self.events.push((old, Event::Blur));
        }
        self.focused = id;
        if let Some(new) = id {
            self.field_focus(new);
            self.events.push((new, Event::Focus));
        }
        self.paint_dirty = true;
    }

    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    /// The node a drag started on, while it goes on.
    pub fn dragging(&self) -> Option<NodeId> {
        self.press.filter(|p| p.dragging).map(|p| p.node)
    }

    pub fn pointer(&self) -> (f32, f32) {
        self.pointer
    }

    /// Shift, Ctrl, Alt and Cmd/Win as the last event left them.
    pub fn modifiers(&self) -> (bool, bool, bool, bool) {
        (self.shift, self.ctrl, self.alt, self.command)
    }

    /// Take the pointer and keyboard. Returns whether the UI used it — a
    /// game then knows the click was on a button, not on the world.
    pub fn handle(&mut self, event: &InputEvent) -> bool {
        // Hit testing needs this frame's rectangles.
        self.paint();
        match event {
            InputEvent::MouseMoved { x, y } => {
                let (px, py) = self.pointer;
                self.pointer = (*x, *y);
                let over = self.hit(*x, *y);
                if over != self.hovered {
                    if let Some(old) = self.hovered {
                        self.events.push((old, Event::Leave));
                    }
                    if let Some(new) = over {
                        self.events.push((new, Event::Enter));
                    }
                    self.hovered = over;
                    self.paint_dirty = true;
                }
                if let Some(press) = &mut self.press {
                    let moved = (x - press.at.0).hypot(y - press.at.1);
                    if !press.dragging && moved > DRAG_SLOP {
                        press.dragging = true;
                    }
                }
                if let Some(press) = self.press {
                    let node = press.node;
                    if press.dragging && self.node(node).field.is_some() {
                        self.field_drag(node, *x);
                    } else if press.dragging {
                        self.events.push((
                            node,
                            Event::Drag {
                                dx: x - px,
                                dy: y - py,
                                x: *x,
                                y: *y,
                            },
                        ));
                    }
                } else if let Some(node) = over.and_then(|o| self.sensing(o, |s| s.drag)) {
                    self.events.push((node, Event::Move { x: *x, y: *y }));
                }
                over.is_some()
            }
            InputEvent::MouseDown(button) => {
                let (x, y) = self.pointer;
                let Some(over) = self.hit(x, y) else {
                    self.focus(None);
                    return false;
                };
                let target = self.sensing(over, |s| s.click || s.drag || s.focus);
                if let Some(node) = target {
                    self.press = Some(Press {
                        node,
                        button: *button,
                        at: (x, y),
                        dragging: false,
                    });
                    self.events.push((
                        node,
                        Event::Press {
                            button: *button,
                            x,
                            y,
                        },
                    ));
                    let focus = self.sensing(node, |s| s.focus);
                    self.focus(focus);
                    if self.node(node).field.is_some() {
                        let extend = self.shift;
                        self.field_press(node, x, extend);
                    }
                    self.paint_dirty = true;
                }
                true
            }
            InputEvent::MouseUp(button) => {
                let Some(press) = self.press.filter(|p| p.button == *button) else {
                    return false;
                };
                self.press = None;
                self.paint_dirty = true;
                self.events
                    .push((press.node, Event::Release { button: *button }));
                if press.dragging {
                    let (x, y) = self.pointer;
                    let over = self
                        .hit(x, y)
                        .and_then(|o| self.sensing(o, |s| s.click))
                        .filter(|o| *o != press.node);
                    self.events.push((press.node, Event::DragEnd { over }));
                } else if self.hovered.is_some_and(|h| {
                    self.sensing(h, |s| s.click || s.drag || s.focus) == Some(press.node)
                }) {
                    let now = Instant::now();
                    let count = match self.last_click {
                        Some((node, at, n)) if node == press.node && now - at < DOUBLE_CLICK => {
                            n + 1
                        }
                        _ => 1,
                    };
                    self.last_click = Some((press.node, now, count));
                    if count >= 2 && self.node(press.node).field.is_some() {
                        self.field_select_all(press.node);
                    }
                    self.events.push((
                        press.node,
                        Event::Click {
                            button: *button,
                            count,
                        },
                    ));
                }
                true
            }
            InputEvent::Scroll { x, y } => {
                let (px, py) = self.pointer;
                let Some(over) = self.hit(px, py) else {
                    return false;
                };
                // The nearest clipping ancestor with somewhere to go takes
                // it; otherwise the node under the pointer hears it.
                let mut at = Some(over);
                while let Some(id) = at {
                    if self.node(id).style.look.clip && self.scroll_by(id, -y * 40.0) {
                        return true;
                    }
                    at = self.parent(id);
                }
                if let Some(node) = self.sensing(over, |s| s.drag || s.click || s.focus) {
                    self.events.push((node, Event::Wheel { x: *x, y: *y }));
                }
                true
            }
            InputEvent::KeyDown(key) => {
                self.set_modifier(*key, true);
                if let Some(node) = self.focused.filter(|f| self.node(*f).field.is_some()) {
                    if self.field_key(node, *key) {
                        return true;
                    }
                }
                match self.focused {
                    Some(node) => {
                        self.events.push((node, Event::KeyDown(*key)));
                        true
                    }
                    None => false,
                }
            }
            InputEvent::KeyUp(key) => {
                self.set_modifier(*key, false);
                match self.focused {
                    Some(node) => {
                        self.events.push((node, Event::KeyUp(*key)));
                        true
                    }
                    None => false,
                }
            }
            InputEvent::Text(text) => match self.focused {
                Some(node) if self.node(node).field.is_some() => {
                    self.field_type(node, text);
                    true
                }
                Some(node) => {
                    self.events.push((node, Event::Text(text.clone())));
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    fn set_modifier(&mut self, key: Key, down: bool) {
        match key {
            Key::LeftShift | Key::RightShift => self.shift = down,
            Key::LeftControl | Key::RightControl => self.ctrl = down,
            Key::LeftAlt | Key::RightAlt => self.alt = down,
            Key::LeftSuper | Key::RightSuper => self.command = down,
            _ => {}
        }
    }

    /// Scroll a clipping node's children; `false` when it is already as far
    /// as it goes that way.
    pub fn scroll_by(&mut self, id: NodeId, dy: f32) -> bool {
        let Ok(layout) = self.tree.layout(id.0) else {
            return false;
        };
        let most = layout.scroll_height().max(0.0);
        let node = self.node_mut(id);
        let to = (node.scroll + dy).clamp(0.0, most);
        if to == node.scroll {
            return false;
        }
        node.scroll = to;
        self.paint_dirty = true;
        true
    }

    /// Scroll so that `child` (somewhere under `id`) is in view.
    pub fn scroll_to(&mut self, id: NodeId, child: NodeId) {
        self.layout();
        let (Some(view), Some(at)) = (self.layout_rect(id), self.layout_rect(child)) else {
            return;
        };
        if at.y < view.y {
            self.scroll_by(id, at.y - view.y);
        } else if at.y + at.height > view.y + view.height {
            self.scroll_by(id, at.y + at.height - view.y - view.height);
        }
    }

    /// Where a node is by the layout, scrolls included — whether or not
    /// it was painted.
    pub fn layout_rect(&self, id: NodeId) -> Option<Rect> {
        let own = self.tree.layout(id.0).ok()?;
        let (mut x, mut y) = (own.location.x, own.location.y);
        let mut at = self.parent(id);
        while let Some(p) = at {
            let l = self.tree.layout(p.0).ok()?;
            x += l.location.x;
            y += l.location.y - self.node(p).scroll;
            at = self.parent(p);
        }
        Some(Rect {
            x,
            y,
            width: own.size.width,
            height: own.size.height,
        })
    }

    pub fn scroll(&self, id: NodeId) -> f32 {
        self.node(id).scroll
    }

    /// What happened since the last call, in order.
    pub fn events(&mut self) -> Vec<(NodeId, Event)> {
        std::mem::take(&mut self.events)
    }

    // --- looking at it ------------------------------------------------

    /// The first node with this name, in tree order.
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.find_under(self.root(), name)
    }

    fn find_under(&self, id: NodeId, name: &str) -> Option<NodeId> {
        if self.node(id).name.as_deref() == Some(name) {
            return Some(id);
        }
        self.children(id)
            .into_iter()
            .find_map(|c| self.find_under(c, name))
    }

    /// The tree as text: one node a line, indented, with its name, text,
    /// where it is and what it senses. What an agent reads instead of a
    /// screenshot, and what a test compares.
    pub fn dump(&mut self) -> String {
        self.paint();
        let mut out = String::new();
        self.dump_node(self.root(), 0, &mut out);
        out
    }

    fn dump_node(&self, id: NodeId, depth: usize, out: &mut String) {
        use std::fmt::Write;
        let node = self.node(id);
        if self
            .tree
            .style(id.0)
            .is_ok_and(|s| s.display == taffy::Display::None)
        {
            return;
        }
        let r = node.rect;
        let _ = write!(out, "{}", "  ".repeat(depth));
        match (&node.name, &node.text) {
            (Some(name), _) => {
                let _ = write!(out, "#{name}");
            }
            (None, Some(_)) => {}
            (None, None) => {
                let _ = write!(out, "node");
            }
        }
        if let Some(text) = &node.text {
            let _ = write!(out, " {:?}", text.string);
        }
        let _ = write!(
            out,
            " @{},{} {}x{}",
            r.x.round(),
            r.y.round(),
            r.width.round(),
            r.height.round()
        );
        let s = node.style.sense;
        if s.click {
            let _ = write!(out, " click");
        }
        if s.drag {
            let _ = write!(out, " drag");
        }
        if self.focused == Some(id) {
            let _ = write!(out, " focused");
        }
        if self.hovered == Some(id) {
            let _ = write!(out, " hovered");
        }
        out.push('\n');
        for child in self.children(id) {
            self.dump_node(child, depth + 1, out);
        }
    }

    /// Move the pointer to the middle of `id` and click it — what a test or
    /// an agent does instead of finding the pixel.
    pub fn click(&mut self, id: NodeId) {
        self.paint();
        let (x, y) = self.rect(id).center();
        self.handle(&InputEvent::MouseMoved { x, y });
        self.handle(&InputEvent::MouseDown(MouseButton::Left));
        self.handle(&InputEvent::MouseUp(MouseButton::Left));
    }
}

/// Every node's shaped text, by node.
pub(crate) struct TextBuffers<'a>(&'a TaffyTree<Node>);

impl<'a> TextBuffers<'a> {
    pub(crate) fn get(&self, id: NodeId) -> Option<&'a Buffer> {
        self.0
            .get_node_context(id.0)
            .and_then(|n| n.text.as_ref())
            .map(|t| &t.buffer)
    }
}

/// (Re)shape a node's text in its current style.
fn shape(fonts: &mut FontSystem, node: &mut Node) {
    let style = node.style.text.clone();
    let Some(text) = node.text.as_mut() else {
        return;
    };
    text.buffer
        .set_metrics(Metrics::new(style.size, style.size * style.line_height));
    let family = if style.mono {
        Family::Monospace
    } else {
        Family::Name(SANS)
    };
    let attrs = Attrs::new().family(family).weight(Weight(style.weight));
    text.buffer
        .set_text(&text.string, &attrs, Shaping::Advanced, None);
    text.buffer.shape_until_scroll(fonts, false);
    text.style = style;
    text.drawn_at = None;
}

/// How big a node's content is: its text at the width it is offered.
fn measure(
    fonts: &mut FontSystem,
    node: Option<&mut Node>,
    known: taffy::Size<Option<f32>>,
    available: taffy::Size<AvailableSpace>,
) -> taffy::Size<f32> {
    if let taffy::Size {
        width: Some(width),
        height: Some(height),
    } = known
    {
        return taffy::Size { width, height };
    }
    let Some(node) = node else {
        return taffy::Size::ZERO;
    };
    if node
        .text
        .as_ref()
        .is_some_and(|t| t.style != node.style.text)
    {
        shape(fonts, node);
    }
    let nowrap = node.style.text.nowrap;
    let Some(text) = node.text.as_mut() else {
        return taffy::Size::ZERO;
    };
    let width = known.width.or(match available.width {
        AvailableSpace::Definite(w) if !nowrap => Some(w),
        _ => None,
    });
    text.buffer.set_size(width, None);
    text.buffer.shape_until_scroll(fonts, false);
    text.drawn_at = None;
    let mut w: f32 = 0.0;
    let mut lines = 0;
    for run in text.buffer.layout_runs() {
        w = w.max(run.line_w);
        lines += 1;
    }
    let line = text.buffer.metrics().line_height;
    taffy::Size {
        width: known.width.unwrap_or(w.ceil()),
        height: known.height.unwrap_or(line * lines.max(1) as f32),
    }
}
