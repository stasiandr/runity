//! Rope: a line strung between two points, sagging and swaying in the
//! wind — a washing line, a string of bunting, a rope bridge's rail, a
//! hanging cable. `rope: (to: (6.0, 0.0, 0.0), slack: 0.08)` on an entity
//! strings one from it to a point in its space; its material the entity's.
//!
//! A chain of points (Verlet), each held to the next at its share of the
//! rope's length — `slack` longer than the straight line, so it sags into
//! a catenary by itself — pulled down, dragged by the wind, and pinned at
//! both ends to the entity, so it moves with it. Drawn as a tube.
//!
//! Only a look, like cloth: each machine sways its own (DNA, postulate 4).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::world::{LiveMesh, WorldTransform};

/// A rope, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rope {
    /// Where its far end is tied, in the entity's space; the near end is
    /// the entity itself.
    pub to: [f32; 3],
    /// How much longer than the straight line it is: 0 taut, 0.1 a good
    /// sag.
    pub slack: f32,
    /// Points along it.
    pub segments: u32,
    /// How thick, metres.
    pub thickness: f32,
    /// How much the wind takes it: a thin cord more, a hawser less.
    pub catch: f32,
}

impl Default for Rope {
    fn default() -> Self {
        Self {
            to: [4.0, 0.0, 0.0],
            slack: 0.08,
            segments: 24,
            thickness: 0.025,
            catch: 0.4,
        }
    }
}

const STEP: f32 = 1.0 / 120.0;
const WIND_SPEED: f32 = 4.0;
/// Sides of the tube round it.
const SIDES: usize = 6;

/// A rope as it moves: the component [`run_ropes`] steps.
#[derive(Debug, Clone)]
pub struct RopeState {
    pub rope: Rope,
    now: Vec<Vec3>,
    was: Vec<Vec3>,
    /// Each link's length.
    link: f32,
    time: f32,
    owed: f32,
    placed: bool,
}

impl RopeState {
    pub fn new(rope: Rope) -> Self {
        let count = rope.segments.clamp(3, 128) as usize;
        let end = Vec3::from_array(rope.to);
        let length = end.length() * (1.0 + rope.slack.max(0.0));
        let points: Vec<Vec3> = (0..count)
            .map(|i| end * (i as f32 / (count - 1) as f32))
            .collect();
        Self {
            rope,
            was: points.clone(),
            now: points,
            link: length / (count - 1) as f32,
            time: 0.0,
            owed: 0.0,
            placed: false,
        }
    }

    /// Where its points are, in the world.
    pub fn points(&self) -> &[Vec3] {
        &self.now
    }

    fn ends(&self, placed: Mat4) -> (Vec3, Vec3) {
        (
            placed.transform_point3(Vec3::ZERO),
            placed.transform_point3(Vec3::from_array(self.rope.to)),
        )
    }

    /// Along by `seconds`, in the wind, where the entity now is.
    pub fn advance(&mut self, placed: Mat4, wind: &crate::foliage::Wind, seconds: f32) {
        if !self.placed {
            let (a, b) = self.ends(placed);
            let n = self.now.len();
            for i in 0..n {
                let p = a.lerp(b, i as f32 / (n - 1) as f32);
                self.now[i] = p;
                self.was[i] = p;
            }
            self.placed = true;
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.25);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(placed, wind);
        }
    }

    fn step(&mut self, placed: Mat4, wind: &crate::foliage::Wind) {
        self.time += STEP;
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or_zero();
        let speed = WIND_SPEED * wind.strength.max(0.0);
        let n = self.now.len();
        for k in 1..n - 1 {
            let p = self.now[k];
            let moving = (p - self.was[k]) / STEP;
            let along = p.dot(level);
            let gust = 0.6 + 0.4 * (self.time * 1.1 - along * 0.3 + k as f32 * 0.05).sin();
            let air = level * speed * gust - moving;
            // A cord takes the wind across itself, not along.
            let tangent = (self.now[k + 1] - self.now[k - 1]).normalize_or_zero();
            let across = air - tangent * air.dot(tangent);
            let acceleration = Vec3::new(0.0, -9.81, 0.0) + across * self.rope.catch.max(0.0);
            let next = p + (p - self.was[k]) * 0.995 + acceleration * STEP * STEP;
            self.was[k] = p;
            self.now[k] = next;
        }
        let (a, b) = self.ends(placed);
        self.was[0] = self.now[0];
        self.now[0] = a;
        self.was[n - 1] = self.now[n - 1];
        self.now[n - 1] = b;
        for _ in 0..12 {
            for k in 0..n - 1 {
                let d = self.now[k + 1] - self.now[k];
                let apart = d.length();
                if apart < 1e-6 {
                    continue;
                }
                let off = d * ((apart - self.link) / apart);
                let (first, last) = (k == 0, k + 1 == n - 1);
                match (first, last) {
                    (true, true) => {}
                    (true, false) => self.now[k + 1] -= off,
                    (false, true) => self.now[k] += off,
                    (false, false) => {
                        self.now[k] += off * 0.5;
                        self.now[k + 1] -= off * 0.5;
                    }
                }
            }
        }
    }

    /// A tube along it, in the entity's space.
    pub fn mesh(&self, placed: Mat4) -> (Vec<crate::asset::Vertex>, Vec<u32>) {
        let back = placed.inverse();
        let local: Vec<Vec3> = self.now.iter().map(|p| back.transform_point3(*p)).collect();
        let n = local.len();
        let r = self.rope.thickness.max(0.002) * 0.5;
        let mut vertices = Vec::with_capacity(n * SIDES);
        let mut side = Vec3::Y;
        for k in 0..n {
            let t = (local[(k + 1).min(n - 1)] - local[k.saturating_sub(1)]).normalize_or(Vec3::X);
            // A frame carried along it, so the tube does not twist.
            let mut s = side - t * side.dot(t);
            if s.length_squared() < 1e-6 {
                s = t.any_orthonormal_vector();
            }
            side = s.normalize();
            let up = t.cross(side);
            for j in 0..SIDES {
                let a = j as f32 / SIDES as f32 * std::f32::consts::TAU;
                let normal = side * a.cos() + up * a.sin();
                vertices.push(crate::asset::Vertex {
                    position: (local[k] + normal * r).to_array(),
                    normal: normal.to_array(),
                    uv: [j as f32 / SIDES as f32, k as f32 / (n - 1) as f32],
                });
            }
        }
        let mut indices = Vec::with_capacity((n - 1) * SIDES * 6);
        for k in 0..n - 1 {
            for j in 0..SIDES {
                let a = (k * SIDES + j) as u32;
                let b = (k * SIDES + (j + 1) % SIDES) as u32;
                let c = a + SIDES as u32;
                let d = b + SIDES as u32;
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        (vertices, indices)
    }
}

/// Step every rope by `seconds` in `wind`, and put where it went in its
/// mesh.
pub fn run_ropes(world: &mut hecs::World, seconds: f32, wind: &crate::foliage::Wind) {
    for (state, placed, live) in world.query_mut::<(&mut RopeState, &WorldTransform, &mut LiveMesh)>() {
        state.advance(placed.0, wind, seconds);
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still() -> crate::foliage::Wind {
        crate::foliage::Wind {
            direction: Vec3::X,
            strength: 0.0,
        }
    }

    #[test]
    fn a_slack_rope_sags_in_the_middle_as_long_as_it_is_and_swings_in_a_crosswind() {
        let line = Rope {
            to: [6.0, 0.0, 0.0],
            slack: 0.08,
            ..Rope::default()
        };
        let posts = Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0));
        let settle = |state: &mut RopeState, wind: crate::foliage::Wind| {
            for _ in 0..150 {
                state.advance(posts, &wind, 1.0 / 30.0);
            }
        };
        let mut hung = RopeState::new(line);
        settle(&mut hung, still());
        let p = hung.points();
        let middle = p[p.len() / 2];
        // A rope 8% longer than 6 m sags about 1.2 m in its middle.
        let sag = 3.0 - middle.y;
        assert!((0.8..1.6).contains(&sag), "sags {sag}");
        // Its ends stay tied.
        assert!(p[0].distance(Vec3::new(0.0, 3.0, 0.0)) < 1e-4);
        assert!(p[p.len() - 1].distance(Vec3::new(6.0, 3.0, 0.0)) < 1e-4);
        // And it is as long as it is, near enough.
        let length: f32 = p.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!((length - 6.48).abs() < 0.1, "{length}");

        let mut blown = RopeState::new(line);
        settle(
            &mut blown,
            crate::foliage::Wind {
                direction: Vec3::Z,
                strength: 3.0,
            },
        );
        let swung = blown.points()[blown.points().len() / 2];
        assert!(swung.z > 0.15, "swings downwind: {swung}");
        let (vertices, indices) = blown.mesh(posts);
        assert_eq!(vertices.len(), 24 * SIDES);
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
    }
}
