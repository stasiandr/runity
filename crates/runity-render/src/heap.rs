//! A heap of sand growing where sand pours: under a torn sack, a chute, a
//! hand letting it run through the fingers. `heap: (rate: 0.002)` on an
//! entity on the ground raises a cone there, a little rough, at sand's
//! angle of repose; its material the entity's. The stream falling onto it
//! is particles on an entity above (`particles: (...)`, `gravity` down).
//!
//! It grows by the clock — so much a second, up to `most` — so every
//! machine has the same heap at the same time with nothing sent (DNA,
//! postulate 4).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::world::LiveMesh;

/// A heap, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Heap {
    /// Cubic metres it grows by a second.
    pub rate: f32,
    /// Cubic metres there at the start.
    pub start: f32,
    /// The most it grows to, cubic metres.
    pub most: f32,
    /// How steep its sides stand, degrees: dry sand's angle of repose is
    /// about 34.
    pub angle_deg: f32,
}

impl Default for Heap {
    fn default() -> Self {
        Self {
            rate: 0.002,
            start: 0.0,
            most: 0.5,
            angle_deg: 34.0,
        }
    }
}

impl Heap {
    /// How much there is after `seconds`.
    pub fn volume(&self, seconds: f32) -> f32 {
        (self.start + self.rate.max(0.0) * seconds.max(0.0)).clamp(0.0, self.most.max(0.0))
    }

    /// How high a cone of `volume` stands at the heap's angle: V = π r² h / 3
    /// with r = h / tan(angle).
    pub fn height(&self, volume: f32) -> f32 {
        let slope = self.angle_deg.clamp(5.0, 80.0).to_radians().tan();
        (3.0 * volume.max(0.0) * slope * slope / std::f32::consts::PI).cbrt()
    }

    /// Its mesh at `volume`: a cone at the angle, its sides a little rough,
    /// its tip rounded, in the entity's space.
    pub fn mesh(&self, volume: f32) -> (Vec<crate::asset::Vertex>, Vec<u32>) {
        let h = self.height(volume).max(0.002);
        let slope = self.angle_deg.clamp(5.0, 80.0).to_radians().tan();
        let r = h / slope;
        let around = 28usize;
        let rings = 8usize;
        let rough = |a: f32, f: f32| {
            1.0 + 0.06 * (a * 5.0 + 1.3).sin() * (f * 3.1).sin()
                + 0.04 * (a * 11.0 + f * 7.0).sin()
        };
        let mut vertices = Vec::with_capacity(around * (rings + 1));
        for j in 0..=rings {
            // From the rim (f = 0) to the tip (f = 1), rounding off the top.
            let f = j as f32 / rings as f32;
            let lift = 1.0 - (1.0 - f).powf(1.0) * 1.0;
            let y = h * (lift - 0.12 * f * f * f);
            for i in 0..around {
                let a = i as f32 / around as f32 * std::f32::consts::TAU;
                let out = r * (1.0 - f) * rough(a, f);
                let p = Vec3::new(a.cos() * out, y.max(0.0), a.sin() * out);
                let n = Vec3::new(a.cos(), 1.0 / slope, a.sin()).normalize();
                vertices.push(crate::asset::Vertex {
                    position: p.to_array(),
                    normal: n.to_array(),
                    uv: [i as f32 / around as f32, f],
                });
            }
        }
        let mut indices = Vec::with_capacity(around * rings * 6);
        for j in 0..rings {
            for i in 0..around {
                let a = (j * around + i) as u32;
                let b = (j * around + (i + 1) % around) as u32;
                let c = a + around as u32;
                let d = b + around as u32;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (vertices, indices)
    }
}

/// A heap as it grows: the component [`run_heaps`] steps.
#[derive(Debug, Clone)]
pub struct HeapState {
    pub heap: Heap,
    /// Seconds it has been growing.
    pub seconds: f32,
    /// The volume its mesh was last made for.
    made: f32,
}

impl HeapState {
    pub fn new(heap: Heap) -> Self {
        Self {
            heap,
            seconds: 0.0,
            made: -1.0,
        }
    }
}

/// Grow every heap by `seconds`, and make its mesh again when it has grown
/// enough to see.
pub fn run_heaps(world: &mut hecs::World, seconds: f32) {
    for (state, live) in world.query_mut::<(&mut HeapState, &mut LiveMesh)>() {
        state.seconds += seconds.max(0.0);
        let volume = state.heap.volume(state.seconds);
        if (volume - state.made).abs() > state.made.max(1e-4) * 0.01 {
            let (vertices, indices) = state.heap.mesh(volume);
            live.set(vertices, indices);
            state.made = volume;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heap_grows_at_its_rate_to_its_most_standing_at_its_angle() {
        let heap = Heap {
            rate: 0.01,
            most: 0.3,
            ..Heap::default()
        };
        assert_eq!(heap.volume(0.0), 0.0);
        assert!((heap.volume(10.0) - 0.1).abs() < 1e-6);
        assert_eq!(heap.volume(1000.0), 0.3);
        // A cone of that volume at 34 degrees.
        let h = heap.height(0.3);
        let r = h / 34f32.to_radians().tan();
        let v = std::f32::consts::PI * r * r * h / 3.0;
        assert!((v - 0.3).abs() < 1e-4, "{v}");
        let (vertices, indices) = heap.mesh(0.3);
        let top = vertices.iter().map(|v| v.position[1]).fold(0.0f32, f32::max);
        assert!(top <= h + 1e-4 && top > h * 0.8, "{top} against {h}");
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
    }
}
