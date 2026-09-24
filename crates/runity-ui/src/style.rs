//! How a node is laid out and what it looks like, in one value.
//!
//! Layout is taffy's flexbox, wrapped so that a panel reads as a sentence —
//! `Style::row().gap(6.0).padding(8.0).fill()` — rather than a struct
//! literal of `LengthPercentageAuto`s. Look is ours: a fill, a border, a
//! radius, the text's size and colour, and what hover and press change.
//! Units are logical pixels; the renderer multiplies by the window's scale.

use taffy::prelude::{auto, length, percent};

/// A colour as written in a design file: sRGB, straight alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `0x9184d9` — the way a design system writes it.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
    }

    /// The same colour at `percent` opacity — CSS's `color-mix(in srgb,
    /// X n%, transparent)`.
    pub const fn alpha(self, percent: u32) -> Self {
        Self {
            a: ((self.a as u32 * percent) / 100) as u8,
            ..self
        }
    }

    pub fn is_visible(self) -> bool {
        self.a > 0
    }

    /// As the shader takes it: sRGB values, straight alpha, 0 to 1. The UI
    /// blends in sRGB, as a browser does (see `runity-ui`'s shader).
    pub fn to_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// Linear RGBA, straight alpha: for a caller that shades in linear
    /// light, as the 3D renderer does.
    pub fn linear(self) -> [f32; 4] {
        fn channel(c: u8) -> f32 {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        [
            channel(self.r),
            channel(self.g),
            channel(self.b),
            self.a as f32 / 255.0,
        ]
    }
}

/// What a node looks like. Everything here changes paint, never layout, so
/// changing it costs no relayout.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub background: Color,
    pub border: Color,
    pub border_width: f32,
    pub radius: f32,
    /// Under the pointer.
    pub hover_background: Option<Color>,
    pub hover_border: Option<Color>,
    /// Pressed, pointer still down on it.
    pub press_background: Option<Color>,
    /// Children outside this node's box are not drawn, and a wheel over it
    /// scrolls them.
    pub clip: bool,
    /// Faded as a whole: a disabled control.
    pub opacity: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            border: Color::TRANSPARENT,
            border_width: 0.0,
            radius: 0.0,
            hover_background: None,
            hover_border: None,
            press_background: None,
            clip: false,
            opacity: 1.0,
        }
    }
}

/// How a node's text is set.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// Line height as a multiple of the size.
    pub line_height: f32,
    pub color: Color,
    /// 400 regular, 500 medium, 700 bold.
    pub weight: u16,
    pub mono: bool,
    /// One line, cut at the node's width rather than wrapped.
    pub nowrap: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 13.0,
            line_height: 1.35,
            color: Color::hex(0xe9e9ed),
            weight: 400,
            mono: false,
            nowrap: false,
        }
    }
}

/// What the pointer can do to a node. A node that senses nothing is
/// transparent to the pointer: a click falls through to what is under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sense {
    pub click: bool,
    pub drag: bool,
    /// Takes the keyboard when clicked.
    pub focus: bool,
}

impl Sense {
    pub const NONE: Sense = Sense {
        click: false,
        drag: false,
        focus: false,
    };
    pub const CLICK: Sense = Sense {
        click: true,
        drag: false,
        focus: false,
    };
}

/// A node's layout, look and sense, built up in a chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub layout: taffy::Style,
    pub look: Look,
    pub text: TextStyle,
    pub sense: Sense,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            layout: taffy::Style::default(),
            look: Look::default(),
            text: TextStyle::default(),
            sense: Sense::NONE,
        }
    }
}

fn edges(v: f32) -> taffy::Rect<taffy::LengthPercentage> {
    taffy::Rect {
        left: length(v),
        right: length(v),
        top: length(v),
        bottom: length(v),
    }
}

impl Style {
    /// Children side by side.
    pub fn row() -> Self {
        let mut s = Self::default();
        s.layout.display = taffy::Display::Flex;
        s.layout.flex_direction = taffy::FlexDirection::Row;
        s
    }

    /// Children one under another.
    pub fn column() -> Self {
        let mut s = Self::default();
        s.layout.display = taffy::Display::Flex;
        s.layout.flex_direction = taffy::FlexDirection::Column;
        s
    }

    // --- size -------------------------------------------------------

    pub fn width(mut self, w: f32) -> Self {
        self.layout.size.width = length(w);
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.layout.size.height = length(h);
        self
    }

    pub fn size(self, w: f32, h: f32) -> Self {
        self.width(w).height(h)
    }

    /// A fraction of the parent's width: a progress bar's fill.
    pub fn width_fraction(mut self, f: f32) -> Self {
        self.layout.size.width = percent(f.clamp(0.0, 1.0));
        self
    }

    /// As wide as the parent allows.
    pub fn full_width(mut self) -> Self {
        self.layout.size.width = percent(1.0);
        self
    }

    pub fn full_height(mut self) -> Self {
        self.layout.size.height = percent(1.0);
        self
    }

    pub fn full(self) -> Self {
        self.full_width().full_height()
    }

    /// As tall as its content: undoes a fixed height.
    pub fn auto_height(mut self) -> Self {
        self.layout.size.height = auto();
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.layout.min_size.width = length(w);
        self
    }

    pub fn max_width(mut self, w: f32) -> Self {
        self.layout.max_size.width = length(w);
        self
    }

    pub fn min_height(mut self, h: f32) -> Self {
        self.layout.min_size.height = length(h);
        self
    }

    /// Takes the space its siblings leave, and may shrink below its
    /// content — the middle of a toolbar, the Scene view between panels.
    pub fn fill(mut self) -> Self {
        self.layout.flex_grow = 1.0;
        self.layout.flex_shrink = 1.0;
        self.layout.flex_basis = length(0.0);
        self.layout.min_size.width = length(0.0);
        self.layout.min_size.height = length(0.0);
        self
    }

    /// Shares the room its siblings leave evenly with the others that
    /// share it, but never gets narrower than its content: the two sides
    /// of a toolbar, which keep what is between them in the middle for as
    /// long as there is room, and push it aside rather than run under it.
    pub fn share(mut self) -> Self {
        self.layout.flex_grow = 1.0;
        self.layout.flex_shrink = 1.0;
        self.layout.flex_basis = length(0.0);
        self.layout.min_size = taffy::Size::auto();
        self
    }

    /// Undo [`Style::fill`]: its own size again, kept when the parent is
    /// short of room — a panel back from being maximized.
    pub fn unfilled(mut self) -> Self {
        self.layout.flex_grow = 0.0;
        self.layout.flex_shrink = 0.0;
        self.layout.flex_basis = auto();
        self.layout.min_size = taffy::Size::auto();
        self
    }

    /// Keeps its size when the parent is short of room.
    pub fn fixed(mut self) -> Self {
        self.layout.flex_shrink = 0.0;
        self
    }

    // --- spacing ----------------------------------------------------

    pub fn gap(mut self, g: f32) -> Self {
        self.layout.gap = taffy::Size {
            width: length(g),
            height: length(g),
        };
        self
    }

    pub fn padding(mut self, p: f32) -> Self {
        self.layout.padding = edges(p);
        self
    }

    pub fn padding_x(mut self, p: f32) -> Self {
        self.layout.padding.left = length(p);
        self.layout.padding.right = length(p);
        self
    }

    pub fn padding_y(mut self, p: f32) -> Self {
        self.layout.padding.top = length(p);
        self.layout.padding.bottom = length(p);
        self
    }

    pub fn padding_left(mut self, p: f32) -> Self {
        self.layout.padding.left = length(p);
        self
    }

    pub fn margin(mut self, m: f32) -> Self {
        self.layout.margin = taffy::Rect {
            left: length(m),
            right: length(m),
            top: length(m),
            bottom: length(m),
        };
        self
    }

    // --- alignment --------------------------------------------------

    /// Children centred across the main axis — a toolbar's icons on its
    /// midline.
    pub fn center_items(mut self) -> Self {
        self.layout.align_items = Some(taffy::AlignItems::CENTER);
        self
    }

    /// Children centred along the main axis too.
    pub fn center(mut self) -> Self {
        self.layout.align_items = Some(taffy::AlignItems::CENTER);
        self.layout.justify_content = Some(taffy::JustifyContent::CENTER);
        self
    }

    /// Undo [`Style::center`]: children at the start again.
    pub fn center_items_reset(mut self) -> Self {
        self.layout.align_items = None;
        self.layout.justify_content = None;
        self
    }

    pub fn space_between(mut self) -> Self {
        self.layout.justify_content = Some(taffy::JustifyContent::SPACE_BETWEEN);
        self
    }

    /// Children go on to the next line when a line is full. The lines are
    /// packed at the start, as text is: a grid taller than its tiles keeps
    /// them together rather than spreading its rows down the room.
    pub fn wrap(mut self) -> Self {
        self.layout.flex_wrap = taffy::FlexWrap::Wrap;
        self.layout.align_content = Some(taffy::AlignContent::FLEX_START);
        self
    }

    /// Placed over its parent at an offset, out of the flow: a popup, a
    /// badge.
    pub fn absolute(mut self, left: f32, top: f32) -> Self {
        self.layout.position = taffy::Position::Absolute;
        self.layout.inset = taffy::Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        };
        self
    }

    pub fn hidden(mut self) -> Self {
        self.layout.display = taffy::Display::None;
        self
    }

    /// Undo [`Style::hidden`]: laid out and drawn again.
    pub fn shown(mut self) -> Self {
        self.layout.display = taffy::Display::Flex;
        self
    }

    // --- look -------------------------------------------------------

    pub fn background(mut self, c: Color) -> Self {
        self.look.background = c;
        self
    }

    pub fn border(mut self, width: f32, c: Color) -> Self {
        self.look.border = c;
        self.look.border_width = width;
        // Taffy lays out inside the border, as CSS's border-box does.
        self.layout.border = edges(width);
        self
    }

    pub fn radius(mut self, r: f32) -> Self {
        self.look.radius = r;
        self
    }

    pub fn hover(mut self, c: Color) -> Self {
        self.look.hover_background = Some(c);
        self.sense.click = true;
        self
    }

    pub fn hover_border(mut self, c: Color) -> Self {
        self.look.hover_border = Some(c);
        self
    }

    pub fn pressed(mut self, c: Color) -> Self {
        self.look.press_background = Some(c);
        self.sense.click = true;
        self
    }

    pub fn clip(mut self) -> Self {
        self.look.clip = true;
        self.layout.overflow = taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        };
        self
    }

    pub fn opacity(mut self, o: f32) -> Self {
        self.look.opacity = o;
        self
    }

    // --- text -------------------------------------------------------

    pub fn text_size(mut self, s: f32) -> Self {
        self.text.size = s;
        self
    }

    pub fn text_color(mut self, c: Color) -> Self {
        self.text.color = c;
        self
    }

    pub fn weight(mut self, w: u16) -> Self {
        self.text.weight = w;
        self
    }

    pub fn mono(mut self) -> Self {
        self.text.mono = true;
        self
    }

    pub fn nowrap(mut self) -> Self {
        self.text.nowrap = true;
        self
    }

    // --- sense ------------------------------------------------------

    pub fn clickable(mut self) -> Self {
        self.sense.click = true;
        self
    }

    pub fn draggable(mut self) -> Self {
        self.sense.drag = true;
        self
    }

    pub fn focusable(mut self) -> Self {
        self.sense.focus = true;
        self.sense.click = true;
        self
    }
}
