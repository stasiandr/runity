//! Foliage that lives: trees and grass sway in the wind, leaves glow with
//! the sun behind them, and grass bends round what walks through it.
//!
//! * **Wind** is the scene's: `wind: (direction: (1.0, 0.0, 0.3), strength:
//!   1.0)` — a gentle breeze when the scene does not say. What sways, and
//!   how much, is the material's `wind` (0 for a rock, ~1 for a tree, 5 or
//!   so for grass): the vertex shader bends a thing away from the wind by its
//!   height above its own origin — curved up to a metre, so a blade bows,
//!   straight above it, so a crown sways a hand's width — rocking slowly in
//!   gusts that roll across the ground, with
//!   a quick flutter on top. The shadow passes bend it the same way, so
//!   shadows sway with what casts them.
//! * **Translucency** (the material's `translucency`, 0 to 1): a leaf or a
//!   blade of grass with the sun behind it is lit through, brightest looking
//!   straight at the sun — what makes a backlit meadow glow.
//! * **Benders**: an entity with `bends_grass: 0.8` pushes whatever sways
//!   out of the way within that many metres of it, and down — grass parts
//!   round a player. Up to [`MAX_BENDERS`] a frame, the nearest the camera.
//! * **Trampling**: what a bender pushes down stays down a while after it
//!   has gone — a path through a meadow, the flattened ring where
//!   something lay. The renderer keeps an interaction texture round the
//!   camera ([`TrampleMap`]): each frame the benders press into it how far
//!   and which way the grass is bent, and it springs back over
//!   [`RECOVER`] seconds; the vertex shader bends grass by it.
//! * **Hierarchy**: a tree sways as a trunk, its branches and its leaves.
//!   The trunk bends from its foot by height; each branch — how far a
//!   point is out from the trunk's axis — bobs on its own phase; leaves
//!   flutter fast at the tips. Grass, with no reach from its axis, is only
//!   a trunk.

use glam::Vec3;

/// The most benders a frame bends by.
pub const MAX_BENDERS: usize = 8;

pub use runity_core::wind::Wind;

/// Something that bends grass round it: where, and how far it reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bender {
    pub position: Vec3,
    pub radius: f32,
}

/// What the vertex shaders — the lit one and the shadow ones — need to bend
/// foliage, in the frame's uniform and each shadow pass's.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FoliageUniform {
    /// Wind: level direction x and z, strength, time in seconds.
    pub wind: [f32; 4],
    /// Benders: position and radius, nearest first; unused slots have
    /// radius zero and bend nothing.
    pub benders: [[f32; 4]; MAX_BENDERS],
    /// The trample map: its middle's x and z, its size in metres, and 1
    /// when there is one.
    pub trample: [f32; 4],
}

impl FoliageUniform {
    pub(crate) fn new(wind: &Wind, benders: &[Bender], eye: Vec3, time: f32) -> Self {
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or(Vec3::X);
        let mut near: Vec<&Bender> = benders.iter().filter(|b| b.radius > 0.0).collect();
        near.sort_by(|a, b| {
            (a.position - eye)
                .length_squared()
                .total_cmp(&(b.position - eye).length_squared())
        });
        let mut out = [[0.0; 4]; MAX_BENDERS];
        for (slot, bender) in out.iter_mut().zip(near) {
            *slot = [
                bender.position.x,
                bender.position.y,
                bender.position.z,
                bender.radius,
            ];
        }
        Self {
            wind: [level.x, level.z, wind.strength.max(0.0), time],
            benders: out,
            trample: [0.0; 4],
        }
    }
}

/// Seconds trampled grass takes to spring back.
pub const RECOVER: f32 = 10.0;
/// Cells a side of the trample map.
pub const TRAMPLE_CELLS: u32 = 256;
/// Metres a side of it.
pub const TRAMPLE_SIZE: f32 = 64.0;

/// How the grass round the camera is trampled: each cell how far it is
/// pressed (0 to 1) and which way, kept frame to frame and following the
/// camera a whole cell at a time, so the ground under it stays put.
#[derive(Debug, Clone, PartialEq)]
pub struct TrampleMap {
    /// Its middle, snapped to cells, in the world's x and z.
    pub centre: glam::Vec2,
    /// Per cell: pressed, and the way out (x, z) from −1 to 1.
    pub cells: Vec<[f32; 3]>,
    /// The clock it was last pressed at.
    time: Option<f32>,
}

impl Default for TrampleMap {
    fn default() -> Self {
        let n = (TRAMPLE_CELLS * TRAMPLE_CELLS) as usize;
        Self { centre: glam::Vec2::ZERO, cells: vec![[0.0; 3]; n], time: None }
    }
}

impl TrampleMap {
    fn cell(&self) -> f32 {
        TRAMPLE_SIZE / TRAMPLE_CELLS as f32
    }

    /// The map moved to be round `eye`, what it holds kept where it lies
    /// in the world.
    fn follow(&mut self, eye: Vec3) {
        let cell = self.cell();
        let want = glam::Vec2::new((eye.x / cell).round() * cell, (eye.z / cell).round() * cell);
        let shift = ((want - self.centre) / cell).round();
        if shift == glam::Vec2::ZERO {
            return;
        }
        let n = TRAMPLE_CELLS as i64;
        let (sx, sz) = (shift.x as i64, shift.y as i64);
        let mut moved = vec![[0.0; 3]; self.cells.len()];
        for z in 0..n {
            for x in 0..n {
                let (fx, fz) = (x + sx, z + sz);
                if fx >= 0 && fz >= 0 && fx < n && fz < n {
                    moved[(z * n + x) as usize] = self.cells[(fz * n + fx) as usize];
                }
            }
        }
        self.cells = moved;
        self.centre = want;
    }

    /// One frame at `time`: sprung back a little, pressed where the
    /// benders are, round `eye`.
    pub fn press(&mut self, benders: &[Bender], eye: Vec3, time: f32) {
        self.follow(eye);
        let dt = self.time.map_or(0.0, |t| (time - t).clamp(0.0, 0.5));
        self.time = Some(time);
        let back = dt / RECOVER;
        for c in &mut self.cells {
            c[0] = (c[0] - back).max(0.0);
        }
        let cell = self.cell();
        let n = TRAMPLE_CELLS as i64;
        let low = self.centre - glam::Vec2::splat(TRAMPLE_SIZE * 0.5);
        for b in benders.iter().filter(|b| b.radius > 0.0) {
            let at = (glam::Vec2::new(b.position.x, b.position.z) - low) / cell;
            let r = (b.radius / cell).ceil() as i64;
            for z in (at.y as i64 - r).max(0)..=(at.y as i64 + r).min(n - 1) {
                for x in (at.x as i64 - r).max(0)..=(at.x as i64 + r).min(n - 1) {
                    let p = low + (glam::Vec2::new(x as f32, z as f32) + 0.5) * cell;
                    let away = p - glam::Vec2::new(b.position.x, b.position.z);
                    let d = away.length();
                    let pressed = 1.0 - (d / b.radius).clamp(0.0, 1.0);
                    let c = &mut self.cells[(z * n + x) as usize];
                    if pressed > c[0] {
                        let way = away / d.max(1e-3);
                        *c = [pressed, way.x, way.y];
                    }
                }
            }
        }
    }

    /// Its texels: pressed, and the way out in 0–255.
    pub fn texels(&self) -> Vec<[u8; 4]> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
        self.cells.iter().map(|c| [byte(c[0]), byte(c[1] * 0.5 + 0.5), byte(c[2] * 0.5 + 0.5), 255]).collect()
    }

    /// How pressed the grass is at a point of the world.
    pub fn pressed_at(&self, x: f32, z: f32) -> f32 {
        let cell = self.cell();
        let low = self.centre - glam::Vec2::splat(TRAMPLE_SIZE * 0.5);
        let at = ((glam::Vec2::new(x, z) - low) / cell).floor();
        let n = TRAMPLE_CELLS as f32;
        if at.x < 0.0 || at.y < 0.0 || at.x >= n || at.y >= n {
            return 0.0;
        }
        self.cells[(at.y as u32 * TRAMPLE_CELLS + at.x as u32) as usize][0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_benders_are_kept_and_the_wind_is_levelled() {
        let benders: Vec<Bender> = (0..12)
            .map(|i| Bender {
                position: Vec3::new(i as f32, 0.0, 0.0),
                radius: 1.0,
            })
            .collect();
        let u = FoliageUniform::new(
            &Wind {
                direction: Vec3::new(0.0, 5.0, 2.0),
                strength: 2.0,
            },
            &benders,
            Vec3::new(11.0, 0.0, 0.0),
            3.5,
        );
        assert_eq!(u.wind, [0.0, 1.0, 2.0, 3.5]);
        assert_eq!(u.benders[0][0], 11.0, "nearest first");
        assert_eq!(u.benders[MAX_BENDERS - 1][0], 4.0);
    }

    #[test]
    fn a_path_walked_through_grass_stays_pressed_and_springs_back() {
        let mut map = TrampleMap::default();
        // Walked along x at half a metre a frame, from x = 0 to x = 5.
        for i in 0..=10 {
            let at = Vec3::new(i as f32 * 0.5, 0.0, 0.0);
            map.press(&[Bender { position: at, radius: 0.6 }], Vec3::ZERO, i as f32 * 0.1);
        }
        // Behind the walker, the path is still pressed; beside it, not.
        assert!(map.pressed_at(1.0, 0.0) > 0.5, "{}", map.pressed_at(1.0, 0.0));
        assert_eq!(map.pressed_at(1.0, 2.0), 0.0);
        // The camera moves on: the path stays where it lies in the world.
        map.press(&[], Vec3::new(10.0, 0.0, 3.0), 1.1);
        assert!(map.pressed_at(1.0, 0.0) > 0.5);
        // Some seconds later, it has sprung back.
        map.press(&[], Vec3::new(10.0, 0.0, 3.0), 1.1 + 0.5);
        for k in 0..20 {
            map.press(&[], Vec3::new(10.0, 0.0, 3.0), 1.6 + k as f32 * 0.5);
        }
        assert_eq!(map.pressed_at(1.0, 0.0), 0.0);
    }

    #[test]
    fn wind_reads_from_a_scene_line() {
        let w: Wind = ron::from_str("(strength: 2.5)").unwrap();
        assert_eq!(w.strength, 2.5);
        assert_eq!(w.direction, Wind::default().direction);
    }
}
