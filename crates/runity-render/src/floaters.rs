//! Words over a place in the world, rising and fading: points won over the
//! window they were won at, damage over a head, "+1 wood" over a tree. The
//! place is in the world; where it lands on the screen is worked out each
//! frame from the camera, so the words stay over it as the camera moves.
//!
//! [`Floaters::push`] one when something happens, [`Floaters::draw`] every
//! frame after the camera is known. Local to each peer: made from what the
//! peer saw change, never sent.

use glam::{Vec2, Vec3, Vec4, Vec4Swizzles};

use crate::render::Camera;
use crate::ui::{TextRun, Ui};

/// One floating word.
#[derive(Debug, Clone)]
struct Floater {
    at: Vec3,
    text: String,
    color: Vec4,
    age: f32,
}

/// The words floating now, and how they float.
#[derive(Debug, Clone)]
pub struct Floaters {
    list: Vec<Floater>,
    /// Seconds a word lasts.
    pub life: f32,
    /// Pixels a word rises over its life, at 720 high.
    pub rise: f32,
    /// Its size, at 720 high; it pops in a little bigger.
    pub size: f32,
}

impl Default for Floaters {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            life: 1.4,
            rise: 80.0,
            size: 36.0,
        }
    }
}

impl Floaters {
    pub fn new() -> Self {
        Self::default()
    }

    /// A word over `at`, in `color` (straight RGBA).
    pub fn push(&mut self, at: Vec3, text: impl Into<String>, color: Vec4) {
        self.list.push(Floater {
            at,
            text: text.into(),
            color,
            age: 0.0,
        });
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Time on, and the words into `ui`, where `camera` sees their places
    /// on a screen `size` pixels: each with a soft shadow, rising, popping
    /// in and fading out. A place behind the camera draws nothing.
    pub fn draw(&mut self, camera: &Camera, size: Vec2, ui: &mut Ui, seconds: f32) {
        for f in &mut self.list {
            f.age += seconds;
        }
        let life = self.life.max(1e-3);
        self.list.retain(|f| f.age < life);
        let view = camera.view_projection(size.x / size.y.max(1.0));
        let scale = size.y / 720.0;
        for f in &self.list {
            let clip = view * f.at.extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let ndc = clip.xy() / clip.w;
            let t = f.age / life;
            let x = (ndc.x + 1.0) * 0.5 * size.x;
            let y = (1.0 - ndc.y) * 0.5 * size.y - self.rise * scale * t;
            let alpha = f.color.w * if t < 0.6 { 1.0 } else { 1.0 - (t - 0.6) / 0.4 };
            let pop = if t < 0.15 { 1.0 + 1.6 * (0.15 - t) } else { 1.0 };
            let big = self.size * scale * pop;
            let run = |dx: f32, dy: f32, c: Vec4| {
                TextRun::new(x - 200.0 + dx, y - big * 0.5 + dy, big, c, f.text.clone()).within(400.0, 0.5)
            };
            ui.text(run(2.0 * scale, 3.0 * scale, Vec4::new(0.08, 0.04, 0.0, alpha * 0.7)));
            ui.text(run(0.0, 0.0, f.color.truncate().extend(alpha)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            position: Vec3::new(0.0, 0.0, 5.0),
            target: Vec3::ZERO,
            ..Camera::default()
        }
    }

    #[test]
    fn a_word_floats_over_its_place_rises_and_is_gone() {
        let size = Vec2::new(1280.0, 720.0);
        let mut floaters = Floaters::new();
        floaters.push(Vec3::ZERO, "+20", Vec4::ONE);
        floaters.push(Vec3::new(0.0, 0.0, 10.0), "behind", Vec4::ONE);
        let mut ui = Ui::new();
        floaters.draw(&camera(), size, &mut ui, 0.0);
        let first: Vec<_> = ui.texts.iter().filter(|t| t.text == "+20").collect();
        assert_eq!(first.len(), 2, "the word and its shadow");
        assert!(ui.texts.iter().all(|t| t.text != "behind"), "behind the camera, nothing");
        let word = first[1];
        let centre = word.x + word.within.unwrap().0 / 2.0;
        assert!((centre - 640.0).abs() < 1.0, "over the middle: {centre}");
        let start = word.y;
        let mut ui = Ui::new();
        floaters.draw(&camera(), size, &mut ui, 0.7);
        let later = ui.texts.iter().find(|t| t.text == "+20" && t.color.x == 1.0).unwrap();
        assert!(later.y < start - 20.0, "risen: {} from {start}", later.y);
        let mut ui = Ui::new();
        floaters.draw(&camera(), size, &mut ui, 1.0);
        assert!(floaters.is_empty() && ui.texts.is_empty(), "and gone");
    }
}
