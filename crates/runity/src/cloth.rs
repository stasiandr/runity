//! Cloth: a banner, an awning, a sheet on a line, flapping in the scene's
//! wind. `cloth: (size: (2.0, 3.0), pinned: Top)` on an entity hangs a
//! sheet from it, its material the entity's.
//!
//! A grid of points, each carried on by where it was going (Verlet), pulled
//! down, pushed by the wind as much as it faces it — harder in gusts — and
//! held to its neighbours: across, down, the diagonals and two apart, which
//! is what keeps it from folding like paper. The pinned points go where the
//! entity is, so a moved pole drags its flag. Drawn from both sides.
//!
//! Only a look: every machine flaps its own, with nothing sent (DNA,
//! postulate 4 — nothing in a game should stand on where a flag's corner
//! is). Its steps are fixed, so the same wind flaps it the same way.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::world::{LiveMesh, WorldTransform};

/// A sheet of cloth, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cloth {
    /// Metres: across (the entity's x) and down (its −y), or for an awning
    /// across and deep (its −z).
    pub size: [f32; 2],
    /// Points across and down.
    pub cells: [u32; 2],
    /// What holds it up.
    pub pinned: Pinned,
    /// How much the wind takes it: 1 a flag, less a heavy canvas.
    pub catch: f32,
    /// How much it holds its shape, as rounds of pulling points back to
    /// their neighbours each step: more is stiffer, and dearer.
    pub stiffness: u32,
}

/// What holds a cloth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Pinned {
    /// Its whole top edge: a banner, a curtain.
    #[default]
    Top,
    /// Its two top corners: a sheet on a line, sagging between.
    TopCorners,
    /// Its left edge: a flag on a pole.
    Left,
    /// Its four corners, lying level: an awning, bellying in the wind.
    Corners,
}

impl Default for Cloth {
    fn default() -> Self {
        Self {
            size: [2.0, 2.0],
            cells: [16, 16],
            pinned: Pinned::Top,
            catch: 1.0,
            stiffness: 8,
        }
    }
}

/// Seconds a step: the cloth runs at its own fixed rate.
const STEP: f32 = 1.0 / 120.0;
/// Metres a second of wind at `strength: 1`, as the physics' wind.
const WIND_SPEED: f32 = 4.0;

/// A cloth as it moves: the component [`run_cloth`] steps.
#[derive(Debug, Clone)]
pub struct ClothState {
    pub cloth: Cloth,
    across: usize,
    down: usize,
    /// Where each point is and was, in the world.
    now: Vec<Vec3>,
    was: Vec<Vec3>,
    /// Each point's place on the entity, when it is pinned there.
    rest: Vec<Vec3>,
    pinned: Vec<bool>,
    /// Pairs held apart at their resting distance.
    links: Vec<(u32, u32, f32)>,
    /// Seconds stepped, and seconds owed a step.
    time: f32,
    owed: f32,
    placed: bool,
}

impl ClothState {
    pub fn new(cloth: Cloth) -> Self {
        let across = cloth.cells[0].clamp(2, 64) as usize;
        let down = cloth.cells[1].clamp(2, 64) as usize;
        let (w, h) = (cloth.size[0].max(0.01), cloth.size[1].max(0.01));
        let mut rest = Vec::with_capacity(across * down);
        let mut pinned = Vec::with_capacity(across * down);
        for j in 0..down {
            for i in 0..across {
                let u = i as f32 / (across - 1) as f32;
                let v = j as f32 / (down - 1) as f32;
                let x = (u - 0.5) * w;
                rest.push(match cloth.pinned {
                    Pinned::Corners => Vec3::new(x, 0.0, -v * h),
                    _ => Vec3::new(x, -v * h, 0.0),
                });
                pinned.push(match cloth.pinned {
                    Pinned::Top => j == 0,
                    Pinned::TopCorners => j == 0 && (i == 0 || i == across - 1),
                    Pinned::Left => i == 0,
                    Pinned::Corners => (j == 0 || j == down - 1) && (i == 0 || i == across - 1),
                });
            }
        }
        let mut links = Vec::new();
        let at = |i: usize, j: usize| (j * across + i) as u32;
        let mut link = |a: u32, b: u32| {
            let d = rest[a as usize].distance(rest[b as usize]);
            links.push((a, b, d));
        };
        for j in 0..down {
            for i in 0..across {
                if i + 1 < across {
                    link(at(i, j), at(i + 1, j));
                }
                if j + 1 < down {
                    link(at(i, j), at(i, j + 1));
                }
                if i + 1 < across && j + 1 < down {
                    link(at(i, j), at(i + 1, j + 1));
                    link(at(i + 1, j), at(i, j + 1));
                }
                // Two apart: it bends rather than creases.
                if i + 2 < across {
                    link(at(i, j), at(i + 2, j));
                }
                if j + 2 < down {
                    link(at(i, j), at(i, j + 2));
                }
            }
        }
        Self {
            cloth,
            across,
            down,
            now: rest.clone(),
            was: rest.clone(),
            rest,
            pinned,
            links,
            time: 0.0,
            owed: 0.0,
            placed: false,
        }
    }

    /// Where its points are, in the world.
    pub fn points(&self) -> &[Vec3] {
        &self.now
    }

    /// Hung where the entity is, as it rests.
    fn place(&mut self, placed: Mat4) {
        for (i, r) in self.rest.iter().enumerate() {
            let p = placed.transform_point3(*r);
            self.now[i] = p;
            self.was[i] = p;
        }
        self.placed = true;
    }

    /// Along by `seconds`, in the scene's wind, where the entity now is.
    pub fn advance(&mut self, placed: Mat4, wind: &crate::foliage::Wind, seconds: f32) {
        if !self.placed {
            self.place(placed);
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
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        let normals = self.normals();
        let catch = self.cloth.catch.max(0.0);
        // Four arrays side by side, one point in each.
        #[allow(clippy::needless_range_loop)]
        for k in 0..self.now.len() {
            if self.pinned[k] {
                continue;
            }
            let p = self.now[k];
            let moving = (p - self.was[k]) / STEP;
            // Gusts: slow waves rolling downwind through it, never quite
            // repeating, so each part of the sheet takes them in turn.
            let along = p.dot(level);
            let gust = 0.55
                + 0.3 * (self.time * 1.3 - along * 0.35).sin()
                + 0.25 * (self.time * 3.7 + p.y * 0.8 - along * 0.9).sin().max(0.0);
            // And turbulence, across the wind and up: what sets a flag
            // flapping even when the wind runs along it.
            let side = level.cross(Vec3::Y);
            let swirl = side * (self.time * 5.1 + along * 2.3 + p.y * 1.7).sin() * 0.3
                + Vec3::Y * (self.time * 4.3 - along * 1.9).sin() * 0.15;
            let air = (level * gust + swirl) * speed - moving;
            let n = normals[k];
            let push = n * (n.dot(air) * catch * 3.0) + air * 0.08 * catch;
            let acceleration = gravity + push;
            let next = p + (p - self.was[k]) * 0.992 + acceleration * STEP * STEP;
            self.was[k] = p;
            self.now[k] = next;
        }
        for (k, r) in self.rest.iter().enumerate() {
            if self.pinned[k] {
                let p = placed.transform_point3(*r);
                self.was[k] = self.now[k];
                self.now[k] = p;
            }
        }
        for _ in 0..self.cloth.stiffness.clamp(1, 32) {
            for &(a, b, length) in &self.links {
                let (a, b) = (a as usize, b as usize);
                let d = self.now[b] - self.now[a];
                let apart = d.length();
                if apart < 1e-6 {
                    continue;
                }
                let off = d * ((apart - length) / apart);
                match (self.pinned[a], self.pinned[b]) {
                    (true, true) => {}
                    (true, false) => self.now[b] -= off,
                    (false, true) => self.now[a] += off,
                    (false, false) => {
                        self.now[a] += off * 0.5;
                        self.now[b] -= off * 0.5;
                    }
                }
            }
        }
    }

    /// Each point's facing, from its neighbours across and down.
    fn normals(&self) -> Vec<Vec3> {
        let (w, h) = (self.across, self.down);
        let at = |i: usize, j: usize| self.now[j * w + i];
        let mut out = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                let across = at((i + 1).min(w - 1), j) - at(i.saturating_sub(1), j);
                let down = at(i, (j + 1).min(h - 1)) - at(i, j.saturating_sub(1));
                out.push(down.cross(across).normalize_or(Vec3::Z));
            }
        }
        out
    }

    /// Its triangles, in the entity's own space, both faces.
    pub fn mesh(&self, placed: Mat4) -> (Vec<crate::asset::Vertex>, Vec<u32>) {
        let back = placed.inverse();
        let turn = back.transform_vector3(Vec3::Z).length().max(1e-6);
        let normals = self.normals();
        let (w, h) = (self.across, self.down);
        let count = w * h;
        let mut vertices = Vec::with_capacity(count * 2);
        for side in [1.0f32, -1.0] {
            for (k, (now, normal)) in self.now.iter().zip(&normals).enumerate().take(count) {
                let p = back.transform_point3(*now);
                let n = (back.transform_vector3(*normal) / turn).normalize_or(Vec3::Z) * side;
                let (i, j) = (k % w, k / w);
                vertices.push(crate::asset::Vertex {
                    position: p.to_array(),
                    normal: n.to_array(),
                    uv: [i as f32 / (w - 1) as f32, j as f32 / (h - 1) as f32],
                });
            }
        }
        let mut indices = Vec::with_capacity((w - 1) * (h - 1) * 12);
        for j in 0..h - 1 {
            for i in 0..w - 1 {
                let a = (j * w + i) as u32;
                let b = a + 1;
                let c = a + w as u32;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
                let o = count as u32;
                indices.extend_from_slice(&[a + o, b + o, c + o, b + o, d + o, c + o]);
            }
        }
        (vertices, indices)
    }
}

/// Step every cloth in the world by `seconds` in `wind`, and put where it
/// went in its mesh. Call once a frame, or once a fixed step.
pub fn run_cloth(world: &mut hecs::World, seconds: f32, wind: &crate::foliage::Wind) {
    for (state, placed, live) in world.query_mut::<(&mut ClothState, &WorldTransform, &mut LiveMesh)>() {
        state.advance(placed.0, wind, seconds);
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wind(strength: f32) -> crate::foliage::Wind {
        // Blowing into the banner's face, along +z.
        crate::foliage::Wind {
            direction: Vec3::Z,
            strength,
        }
    }

    #[test]
    fn a_banner_hangs_in_still_air_and_streams_out_downwind_in_a_gale() {
        let banner = Cloth {
            size: [1.0, 2.0],
            cells: [8, 12],
            pinned: Pinned::Top,
            ..Cloth::default()
        };
        let pole = Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0));
        let hem = |state: &ClothState| {
            let p = state.points();
            p[p.len() - state.across / 2]
        };
        let settle = |state: &mut ClothState, strength: f32| {
            for _ in 0..120 {
                state.advance(pole, &wind(strength), 1.0 / 30.0);
            }
        };
        let mut still = ClothState::new(banner);
        settle(&mut still, 0.0);
        let hung = hem(&still);
        let below = pole.transform_point3(still.rest[still.rest.len() - still.across / 2]);
        // Straight down, its full length and no more.
        assert!((hung.x - below.x).abs() < 0.05 && hung.z.abs() < 0.05, "{hung} under {below}");
        assert!((hung.y - 3.0).abs() < 0.15, "hangs its length: {hung}");

        let mut blown = ClothState::new(banner);
        settle(&mut blown, 3.0);
        let streamed = hem(&blown);
        // Out along the wind, lifted.
        assert!(streamed.z > 0.6, "streams downwind: {streamed}");
        assert!(streamed.y > hung.y + 0.3, "lifted: {streamed}");
        // And it has not torn or stretched much: its top still where pinned.
        assert!(blown.points()[0].distance(pole.transform_point3(Vec3::new(-0.5, 0.0, 0.0))) < 1e-4);
        let (vertices, indices) = blown.mesh(pole);
        assert_eq!(vertices.len(), 8 * 12 * 2);
        assert_eq!(indices.len(), 7 * 11 * 12);
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn the_same_wind_flaps_it_the_same_way() {
        let flag = Cloth {
            pinned: Pinned::Left,
            ..Cloth::default()
        };
        let run = || {
            let mut s = ClothState::new(flag);
            for _ in 0..90 {
                s.advance(Mat4::IDENTITY, &wind(2.0), 1.0 / 30.0);
            }
            s.points().to_vec()
        };
        assert_eq!(run(), run());
    }
}
