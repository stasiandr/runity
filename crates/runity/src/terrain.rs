//! Terrain: ground with a shape, made from a few numbers rather than
//! modelled — for now, a field of dunes.
//!
//! An entity's `terrain: (size: 400.0, dunes: (height: 8.0))` is a square of
//! ground that many metres across, centred on the entity and turned with
//! it, raised into dunes shaped by the wind: long crests square to the
//! entity's +x (the way the wind blew them), a gentle climb up the windward
//! back and a short slip face down the lee at about the angle sand rests
//! at, the crests wandering and, with `barchans`, broken into crescents.
//! The mesh is made from these the first time it is drawn
//! ([`upload_terrains`]) and again only when they change; with
//! `collider: Model` the same mesh is what walkers stand on.
//!
//! Its material draws it: `shading: Sand` puts the wind's ripples, glints
//! and drifting sand on it ([`crate::material::Shading::Sand`]), and the
//! ripples leave the slip faces bare, as real ones do.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::asset::{MeshAsset, Vertex};

/// A patch of shaped ground: its line's `terrain`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terrain {
    /// Metres across, square.
    pub size: f32,
    /// Cells across: the mesh is this many squared, twice as many
    /// triangles. More is finer crests and a heavier mesh.
    pub cells: u32,
    pub dunes: Dunes,
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            size: 400.0,
            cells: 256,
            dunes: Dunes::default(),
        }
    }
}

/// How the dunes stand.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dunes {
    /// Metres from trough to crest, at the tallest.
    pub height: f32,
    /// Metres from one crest to the next, along the wind.
    pub wavelength: f32,
    /// How much the crests wander from straight lines, 0 to 1.
    pub sinuosity: f32,
    /// How much the crests break into crescents, 0 (long ridges) to 1.
    pub barchans: f32,
    /// Which dunes: another number, another field.
    pub seed: u32,
    /// The share of the size, at its edges, over which the dunes flatten
    /// into level ground.
    pub fade: f32,
}

impl Default for Dunes {
    fn default() -> Self {
        Self {
            height: 8.0,
            wavelength: 60.0,
            sinuosity: 0.5,
            barchans: 0.4,
            seed: 1,
            fade: 0.1,
        }
    }
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Value noise, smooth, 0 to 1.
fn noise(p: Vec2, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash(x, y, seed);
    let b = hash(x + 1, y, seed);
    let c = hash(x, y + 1, seed);
    let d = hash(x + 1, y + 1, seed);
    let top = a + (b - a) * u.x;
    let bottom = c + (d - c) * u.x;
    top + (bottom - top) * u.y
}

/// Where the windward back ends and the slip face begins, in a wavelength.
const CREST: f32 = 0.78;

impl Terrain {
    /// The ground's height at `(x, z)` in its own space (metres from its
    /// middle; the wind along +x).
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let d = &self.dunes;
        let lambda = d.wavelength.max(1.0);
        let seed = d.seed;
        let across = Vec2::new(x, z) / lambda;
        // The crest lines, bent.
        let bend = (noise(Vec2::new(across.y * 0.8, across.x * 0.15), seed) - 0.5) * 1.6
            + (noise(Vec2::new(across.y * 2.1, across.x * 0.4) + 11.0, seed) - 0.5) * 0.4;
        let phase = across.x + bend * d.sinuosity;
        let p = phase - phase.floor();
        // A slow climb, then the fall down the lee.
        let profile = if p < CREST {
            let t = p / CREST;
            t * t * (1.6 - 0.6 * t)
        } else {
            let t = (p - CREST) / (1.0 - CREST);
            (1.0 - t).powf(1.4)
        };
        // Taller here, lower there; with barchans, gaps between.
        let swell = noise(Vec2::new(across.x * 0.45, across.y * 0.6) + 37.0, seed);
        let breakup = ((swell - 0.3) / 0.4).clamp(0.0, 1.0);
        let amplitude = (0.6 + 0.4 * swell) * (1.0 - d.barchans + d.barchans * breakup);
        let rolling = noise(across * 0.25 + 91.0, seed) * 0.3;
        let h = d.height * (amplitude * profile * 0.85 + rolling * 0.5);
        // Level at the edges, to meet whatever ground is round it.
        let half = self.size * 0.5;
        let edge = (half - x.abs()).min(half - z.abs()) / (self.size * d.fade.max(1e-3));
        let edge = edge.clamp(0.0, 1.0);
        h * edge * edge * (3.0 - 2.0 * edge)
    }

    /// The ground as a mesh: a grid of `cells`² squares, normals from the
    /// slope, texture coordinates in metres.
    pub fn mesh(&self) -> MeshAsset {
        let n = self.cells.clamp(2, 2048);
        let size = self.size.max(1.0);
        let step = size / n as f32;
        let half = size * 0.5;
        let heights: Vec<f32> = (0..=n)
            .flat_map(|z| (0..=n).map(move |x| (x, z)))
            .map(|(x, z)| self.height(-half + x as f32 * step, -half + z as f32 * step))
            .collect();
        let at = |x: u32, z: u32| heights[(z.min(n) * (n + 1) + x.min(n)) as usize];
        let mut vertices = Vec::with_capacity(heights.len());
        for z in 0..=n {
            for x in 0..=n {
                let dx = at(x + 1, z) - at(x.saturating_sub(1), z);
                let dz = at(x, z + 1) - at(x, z.saturating_sub(1));
                let wide = |i: u32| if i == 0 || i == n { 1.0 } else { 2.0 };
                let normal =
                    Vec3::new(-dx / (wide(x) * step), 1.0, -dz / (wide(z) * step)).normalize();
                vertices.push(Vertex {
                    position: [-half + x as f32 * step, at(x, z), -half + z as f32 * step],
                    normal: normal.to_array(),
                    uv: [x as f32 * step, z as f32 * step],
                });
            }
        }
        let stride = n + 1;
        let mut indices = Vec::with_capacity((n * n * 6) as usize);
        for z in 0..n {
            for x in 0..n {
                let a = z * stride + x;
                let (b, c, d) = (a + 1, a + stride, a + stride + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        crate::builtin::finish("terrain", vertices, indices)
    }
}

/// A terrain on an entity: its settings, and the mesh drawn once made.
#[derive(Debug, Clone)]
pub struct Relief {
    pub terrain: Terrain,
    /// The uploaded mesh, and the settings it was made from.
    made: Option<(crate::render::MeshHandle, Terrain)>,
}

impl Relief {
    pub fn new(terrain: Terrain) -> Self {
        Self {
            terrain,
            made: None,
        }
    }
}

/// Make and upload the mesh of every terrain that has none, or whose
/// settings changed since: call it before drawing, as the material maps
/// are. Its entity then draws it like any model.
pub fn upload_terrains(
    world: &mut hecs::World,
    gpu: &crate::gpu::Gpu,
    renderer: &mut crate::render::Renderer,
) {
    let mut drawn = Vec::new();
    for (entity, relief) in world.query_mut::<(hecs::Entity, &mut Relief)>() {
        let current = relief.made.filter(|(_, t)| *t == relief.terrain);
        let handle = match current {
            Some((handle, _)) => handle,
            None => {
                let handle = renderer.upload_mesh_owned(gpu, &relief.terrain.mesh());
                relief.made = Some((handle, relief.terrain));
                handle
            }
        };
        drawn.push((entity, handle));
    }
    for (entity, handle) in drawn {
        let has = world
            .get::<&crate::world::Model>(entity)
            .is_ok_and(|m| m.0 == handle);
        if !has {
            let _ = world.insert_one(entity, crate::world::Model(handle));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dunes_climb_gently_and_fall_steeply_downwind() {
        let t = Terrain {
            dunes: Dunes {
                sinuosity: 0.0,
                barchans: 0.0,
                ..Dunes::default()
            },
            ..Terrain::default()
        };
        // Along the wind, the steepest rise and the steepest fall.
        let (mut up, mut down) = (0.0f32, 0.0f32);
        let step = 0.5;
        let mut x = -100.0;
        while x < 100.0 {
            let slope = (t.height(x + step, 0.0) - t.height(x, 0.0)) / step;
            up = up.max(slope);
            down = down.min(slope);
            x += step;
        }
        assert!(
            down.abs() > up * 1.8,
            "the lee is steep: up {up}, down {down}"
        );
        let angle = down.abs().atan().to_degrees();
        assert!(
            angle > 20.0 && angle < 38.0,
            "about as steep as sand rests: {angle}°"
        );
    }

    #[test]
    fn the_edges_are_level_and_the_mesh_is_whole() {
        let t = Terrain {
            size: 100.0,
            cells: 32,
            ..Terrain::default()
        };
        assert_eq!(t.height(50.0, 10.0), 0.0);
        assert_eq!(t.height(-10.0, -50.0), 0.0);
        let mesh = t.mesh();
        assert_eq!(mesh.vertices.len(), 33 * 33);
        assert_eq!(mesh.indices.len(), 32 * 32 * 6);
        assert!(
            mesh.vertices.iter().all(|v| v.normal[1] > 0.0),
            "all face up"
        );
    }
}
