//! The screen-space layer: a draw list, not a widget toolkit.
//!
//! `docs/design/07-look.md` says the game has no HUD, with two exceptions.
//! That is a decision about a game, not about an engine, and it can change —
//! so what is fixed here is the *layer*, not what goes on it. Two lines of
//! dialogue use this today; a full inventory would use the same thing.
//!
//! What it deliberately is not is a widget system. Layout, focus, hit
//! testing and state are the parts that tie a renderer to one way of
//! building interfaces, and none of them are here. A rectangle and a run of
//! text are the whole vocabulary, and anything above that feeds this.
//!
//! Coordinates are pixels from the top left, which is where a mouse position
//! already is. A layer that used a different origin from the input it
//! responds to would need a conversion at every call site.

use glam::Vec4;

/// A filled rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Straight RGBA, alpha blended over what is already there.
    pub color: Vec4,
    /// Pixels its corners are rounded by; square at 0.
    pub radius: f32,
}

impl Quad {
    pub fn new(x: f32, y: f32, width: f32, height: f32, color: Vec4) -> Self {
        Self {
            x,
            y,
            width,
            height,
            color,
            radius: 0.0,
        }
    }

    /// The same, its corners rounded by `radius` pixels.
    pub fn rounded(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
}

/// A run of text at a size and a colour.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: Vec4,
    pub text: String,
    /// Glyphs outside this box — left, top, right, bottom — are not drawn:
    /// text in a scrolled list stops at the list's edge.
    pub clip: Option<[f32; 4]>,
    /// A box `.0` wide from `x`, and where across it the words sit: 0 the
    /// left, 0.5 the middle, 1 the right. The renderer measures the shaped
    /// words, so it holds for any font; `None` starts them at `x`.
    pub within: Option<(f32, f32)>,
}

impl TextRun {
    pub fn new(x: f32, y: f32, size: f32, color: Vec4, text: impl Into<String>) -> Self {
        Self {
            x,
            y,
            size,
            color,
            text: text.into(),
            clip: None,
            within: None,
        }
    }

    /// The words placed across a box `width` wide from `x`: `across` 0 at
    /// its left, 0.5 in its middle, 1 at its right.
    pub fn within(mut self, width: f32, across: f32) -> Self {
        self.within = Some((width, across));
        self
    }
}

/// Everything to draw over the frame, in the order it is drawn.
///
/// Order is the list's order and nothing else: no z, no sorting, no layers.
/// A list that sorted itself would need a key, and a key is the beginning of
/// a widget system.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ui {
    pub quads: Vec<Quad>,
    pub texts: Vec<TextRun>,
    /// Boxes what is added is cut to, innermost last.
    clips: Vec<[f32; 4]>,
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything. Called once a frame by whoever builds the list.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.texts.clear();
        self.clips.clear();
    }

    /// Cut everything added from now on to a box, until [`Ui::pop_clip`] —
    /// the inside of a scrolled list. Inside another clip, it is cut to
    /// both.
    pub fn push_clip(&mut self, x: f32, y: f32, width: f32, height: f32) -> &mut Self {
        let mut clip = [x, y, x + width, y + height];
        if let Some(outer) = self.clips.last() {
            clip = [
                clip[0].max(outer[0]),
                clip[1].max(outer[1]),
                clip[2].min(outer[2]),
                clip[3].min(outer[3]),
            ];
        }
        self.clips.push(clip);
        self
    }

    pub fn pop_clip(&mut self) -> &mut Self {
        self.clips.pop();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.quads.is_empty() && self.texts.is_empty()
    }

    pub fn quad(&mut self, quad: Quad) -> &mut Self {
        let Some(&[left, top, right, bottom]) = self.clips.last() else {
            self.quads.push(quad);
            return self;
        };
        // Rectangles cut to a rectangle stay rectangles: no GPU scissor
        // needed, and nothing outside is drawn at all.
        let (x0, y0) = (quad.x.max(left), quad.y.max(top));
        let (x1, y1) = (
            (quad.x + quad.width).min(right),
            (quad.y + quad.height).min(bottom),
        );
        if x1 > x0 && y1 > y0 {
            self.quads.push(Quad {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
                ..quad
            });
        }
        self
    }

    pub fn text(&mut self, mut run: TextRun) -> &mut Self {
        if let Some(&clip) = self.clips.last() {
            run.clip = Some(match run.clip {
                Some(own) => [
                    own[0].max(clip[0]),
                    own[1].max(clip[1]),
                    own[2].min(clip[2]),
                    own[3].min(clip[3]),
                ],
                None => clip,
            });
        }
        self.texts.push(run);
        self
    }

    /// A line of text over a dimmed strip the width of the screen — the
    /// shape `03-talk.md` describes for a reply, and the one thing a
    /// no-HUD game still draws.
    pub fn line(
        &mut self,
        y: f32,
        screen_width: f32,
        size: f32,
        text: impl Into<String>,
    ) -> &mut Self {
        self.quad(Quad::new(
            0.0,
            y - size * 0.35,
            screen_width,
            size * 1.7,
            Vec4::new(0.0, 0.0, 0.0, 0.35),
        ));
        self.text(TextRun::new(
            size,
            y,
            size,
            Vec4::new(0.92, 0.90, 0.86, 1.0),
            text,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_keeps_the_order_it_was_built_in() {
        // No sorting, because sorting needs a key and a key is the start of
        // a widget system.
        let mut ui = Ui::new();
        ui.quad(Quad::new(0.0, 0.0, 1.0, 1.0, Vec4::ONE));
        ui.quad(Quad::new(9.0, 0.0, 1.0, 1.0, Vec4::ZERO));
        assert_eq!(ui.quads[0].x, 0.0);
        assert_eq!(ui.quads[1].x, 9.0);
    }

    #[test]
    fn hit_testing_uses_the_same_origin_as_the_mouse() {
        // Top left, like a cursor position, so nothing needs converting at
        // the call site.
        let quad = Quad::new(10.0, 20.0, 100.0, 30.0, Vec4::ONE);
        assert!(quad.contains(10.0, 20.0), "the top-left corner is inside");
        assert!(quad.contains(109.0, 49.0));
        assert!(!quad.contains(110.0, 35.0), "the right edge is exclusive");
        assert!(!quad.contains(50.0, 19.0));
    }

    #[test]
    fn a_dialogue_line_brings_its_own_backing() {
        let mut ui = Ui::new();
        ui.line(400.0, 960.0, 20.0, "Я не знал, что ты здесь.");
        assert_eq!(ui.quads.len(), 1, "a strip behind it");
        assert_eq!(ui.quads[0].width, 960.0);
        assert!(ui.quads[0].color.w < 1.0, "dimmed, not opaque");
        assert_eq!(ui.texts.len(), 1);
        assert!(ui.texts[0].text.starts_with('Я'), "Cyrillic survives");
    }

    #[test]
    fn clearing_empties_both_lists() {
        let mut ui = Ui::new();
        ui.line(0.0, 100.0, 10.0, "x");
        assert!(!ui.is_empty());
        ui.clear();
        assert!(ui.is_empty());
    }
}
