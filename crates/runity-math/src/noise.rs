//! Coherent noise: value, Perlin and Worley, plus fractal summation.
//!
//! World generation needs functions that look random but are smooth and, above
//! all, *reproducible*: the same seed and the same coordinate must give the
//! same number on every machine, every run, forever. So there is no table that
//! gets shuffled at startup and no state that a sample mutates. Every value is
//! derived by hashing the integer lattice coordinates together with the seed,
//! which makes sampling order-independent — you can generate chunk 700 before
//! chunk 3 and get identical terrain.

use crate::{vec2, vec3, Vec2, Vec3};

/// A seeded noise field.
///
/// Cheap to copy; hold one per "channel" of your world (height, moisture,
/// ore density) so that tuning one does not shift the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Noise {
    seed: u64,
}

impl Noise {
    /// A field seeded with a raw number.
    pub fn new(seed: u64) -> Self {
        Self {
            seed: mix(seed ^ SEED_SALT),
        }
    }

    /// A field seeded by world seed *and* a name, so `"height"` and
    /// `"moisture"` decorrelate without you having to invent magic offsets.
    pub fn named(seed: u64, name: &str) -> Self {
        Self {
            seed: mix(seed ^ hash_str(name)),
        }
    }

    /// The raw seed, for serialization.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    // ---------------------------------------------------------------- value

    /// Smoothed random values on the integer lattice, in `[-1, 1]`.
    ///
    /// Cheaper than Perlin and blobbier; good for masks and jitter.
    pub fn value_2d(&self, p: Vec2) -> f32 {
        let (ix, iy) = (floor_i(p.x), floor_i(p.y));
        let (fx, fy) = (p.x - ix as f32, p.y - iy as f32);
        let (u, v) = (fade(fx), fade(fy));

        let c00 = unit(hash2(self.seed, ix, iy));
        let c10 = unit(hash2(self.seed, ix + 1, iy));
        let c01 = unit(hash2(self.seed, ix, iy + 1));
        let c11 = unit(hash2(self.seed, ix + 1, iy + 1));

        let a = lerp(c00, c10, u);
        let b = lerp(c01, c11, u);
        lerp(a, b, v) * 2.0 - 1.0
    }

    /// Value noise in three dimensions, in `[-1, 1]`.
    pub fn value_3d(&self, p: Vec3) -> f32 {
        let (ix, iy, iz) = (floor_i(p.x), floor_i(p.y), floor_i(p.z));
        let (fx, fy, fz) = (p.x - ix as f32, p.y - iy as f32, p.z - iz as f32);
        let (u, v, w) = (fade(fx), fade(fy), fade(fz));

        let mut face = [0.0f32; 2];
        for (dz, out) in face.iter_mut().enumerate() {
            let z = iz + dz as i32;
            let c00 = unit(hash3(self.seed, ix, iy, z));
            let c10 = unit(hash3(self.seed, ix + 1, iy, z));
            let c01 = unit(hash3(self.seed, ix, iy + 1, z));
            let c11 = unit(hash3(self.seed, ix + 1, iy + 1, z));
            *out = lerp(lerp(c00, c10, u), lerp(c01, c11, u), v);
        }
        lerp(face[0], face[1], w) * 2.0 - 1.0
    }

    // --------------------------------------------------------------- perlin

    /// Classic gradient noise, in `[-1, 1]`, zero at every lattice point.
    ///
    /// The quintic fade curve gives a continuous second derivative, so
    /// normals derived from a Perlin heightfield do not crease along cell
    /// boundaries.
    pub fn perlin_2d(&self, p: Vec2) -> f32 {
        let (ix, iy) = (floor_i(p.x), floor_i(p.y));
        let (fx, fy) = (p.x - ix as f32, p.y - iy as f32);
        let (u, v) = (fade(fx), fade(fy));

        let g = |cx: i32, cy: i32, dx: f32, dy: f32| {
            let grad = gradient_2d(hash2(self.seed, cx, cy));
            grad.x * dx + grad.y * dy
        };

        let n00 = g(ix, iy, fx, fy);
        let n10 = g(ix + 1, iy, fx - 1.0, fy);
        let n01 = g(ix, iy + 1, fx, fy - 1.0);
        let n11 = g(ix + 1, iy + 1, fx - 1.0, fy - 1.0);

        let a = lerp(n00, n10, u);
        let b = lerp(n01, n11, u);
        // Unit gradients bound the result to +-sqrt(2)/2; rescale to +-1.
        lerp(a, b, v) * std::f32::consts::SQRT_2
    }

    /// Gradient noise in three dimensions, in `[-1, 1]`.
    pub fn perlin_3d(&self, p: Vec3) -> f32 {
        let (ix, iy, iz) = (floor_i(p.x), floor_i(p.y), floor_i(p.z));
        let (fx, fy, fz) = (p.x - ix as f32, p.y - iy as f32, p.z - iz as f32);
        let (u, v, w) = (fade(fx), fade(fy), fade(fz));

        let g = |cx: i32, cy: i32, cz: i32, dx: f32, dy: f32, dz: f32| {
            let grad = gradient_3d(hash3(self.seed, cx, cy, cz));
            grad.x * dx + grad.y * dy + grad.z * dz
        };

        let mut face = [0.0f32; 2];
        for (dz, out) in face.iter_mut().enumerate() {
            let z = iz + dz as i32;
            let tz = fz - dz as f32;
            let n00 = g(ix, iy, z, fx, fy, tz);
            let n10 = g(ix + 1, iy, z, fx - 1.0, fy, tz);
            let n01 = g(ix, iy + 1, z, fx, fy - 1.0, tz);
            let n11 = g(ix + 1, iy + 1, z, fx - 1.0, fy - 1.0, tz);
            *out = lerp(lerp(n00, n10, u), lerp(n01, n11, u), v);
        }
        // +-sqrt(3)/2 for unit gradients.
        lerp(face[0], face[1], w) * (2.0 / SQRT_3)
    }

    // --------------------------------------------------------------- worley

    /// Cellular noise: scatters one feature point per unit cell and reports
    /// the distances to the two nearest ones.
    ///
    /// Useful for anything that wants irregular *regions* rather than smooth
    /// undulation — biome patches, ore veins, cracked ground, the plot layout
    /// of a village that grew without a planner.
    pub fn worley_2d(&self, p: Vec2) -> Cell {
        let (ix, iy) = (floor_i(p.x), floor_i(p.y));
        let mut nearest = f32::INFINITY;
        let mut second = f32::INFINITY;
        let mut id = 0;

        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (ix + dx, iy + dy);
                let h = hash2(self.seed, cx, cy);
                let feature = vec2(cx as f32 + unit(h), cy as f32 + unit(mix(h ^ FEATURE_SALT)));
                let d = (feature - p).length();
                if d < nearest {
                    second = nearest;
                    nearest = d;
                    id = h;
                } else if d < second {
                    second = d;
                }
            }
        }

        Cell {
            distance: nearest,
            second,
            id,
        }
    }

    /// Distance to the nearest feature point, remapped to `[-1, 1]` so it
    /// composes with the other generators.
    pub fn worley_value_2d(&self, p: Vec2) -> f32 {
        crate::clamp(self.worley_2d(p).distance, 0.0, 1.0) * 2.0 - 1.0
    }

    // -------------------------------------------------------------- fractal

    /// Sum several octaves of Perlin noise, normalized back to `[-1, 1]`.
    pub fn fbm_2d(&self, p: Vec2, fbm: Fbm) -> f32 {
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut point = p * fbm.frequency;

        for _ in 0..fbm.octaves.max(1) {
            sum += self.perlin_2d(point) * amplitude;
            total += amplitude;
            amplitude *= fbm.gain;
            point = rotate(point, OCTAVE_ROTATION) * fbm.lacunarity + OCTAVE_OFFSET_2D;
        }

        if total > 0.0 {
            sum / total
        } else {
            0.0
        }
    }

    /// Fractal Perlin noise in three dimensions.
    pub fn fbm_3d(&self, p: Vec3, fbm: Fbm) -> f32 {
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut point = p * fbm.frequency;

        for _ in 0..fbm.octaves.max(1) {
            sum += self.perlin_3d(point) * amplitude;
            total += amplitude;
            amplitude *= fbm.gain;
            point = point * fbm.lacunarity + OCTAVE_OFFSET_3D;
        }

        if total > 0.0 {
            sum / total
        } else {
            0.0
        }
    }

    /// Fractal noise folded around zero and inverted, in `[-1, 1]`.
    ///
    /// The sharp creases where the fold happens read as mountain ridges,
    /// which is exactly what plain fbm fails to produce.
    pub fn ridged_2d(&self, p: Vec2, fbm: Fbm) -> f32 {
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut point = p * fbm.frequency;

        for _ in 0..fbm.octaves.max(1) {
            let ridge = 1.0 - self.perlin_2d(point).abs();
            sum += ridge * ridge * amplitude;
            total += amplitude;
            amplitude *= fbm.gain;
            point = rotate(point, OCTAVE_ROTATION) * fbm.lacunarity + OCTAVE_OFFSET_2D;
        }

        if total > 0.0 {
            sum / total * 2.0 - 1.0
        } else {
            0.0
        }
    }

    /// Fractal noise of `|perlin|`, in `[-1, 1]`: puffy, cloud-like lobes.
    pub fn billow_2d(&self, p: Vec2, fbm: Fbm) -> f32 {
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut point = p * fbm.frequency;

        for _ in 0..fbm.octaves.max(1) {
            sum += self.perlin_2d(point).abs() * amplitude;
            total += amplitude;
            amplitude *= fbm.gain;
            point = rotate(point, OCTAVE_ROTATION) * fbm.lacunarity + OCTAVE_OFFSET_2D;
        }

        if total > 0.0 {
            sum / total * 2.0 - 1.0
        } else {
            0.0
        }
    }

    /// Push the sample point around by another noise field before sampling.
    ///
    /// One call turns the tell-tale "blobs on a grid" of raw fbm into
    /// meandering, river-like structure. `strength` is in world units.
    pub fn warped_2d(&self, p: Vec2, fbm: Fbm, strength: f32) -> f32 {
        let warp = vec2(
            self.fbm_2d(p + WARP_OFFSET_X, fbm),
            self.fbm_2d(p + WARP_OFFSET_Y, fbm),
        );
        self.fbm_2d(p + warp * strength, fbm)
    }

    /// Analytic-ish gradient of [`Noise::fbm_2d`] by central differences.
    ///
    /// Handy for terrain normals and for slope-driven rules ("no houses on
    /// anything steeper than this").
    pub fn fbm_gradient_2d(&self, p: Vec2, fbm: Fbm, epsilon: f32) -> Vec2 {
        let e = if epsilon > 0.0 { epsilon } else { 1e-3 };
        let dx = self.fbm_2d(p + vec2(e, 0.0), fbm) - self.fbm_2d(p - vec2(e, 0.0), fbm);
        let dy = self.fbm_2d(p + vec2(0.0, e), fbm) - self.fbm_2d(p - vec2(0.0, e), fbm);
        vec2(dx, dy) * (0.5 / e)
    }
}

/// What [`Noise::worley_2d`] found near a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// Distance to the nearest feature point.
    pub distance: f32,
    /// Distance to the second nearest one.
    pub second: f32,
    /// A stable identifier for the nearest cell — hash it further to pick a
    /// biome, a colour, an owner.
    pub id: u64,
}

impl Cell {
    /// `second - distance`: near zero exactly on the boundary between two
    /// cells, which draws crisp region borders.
    pub fn edge(&self) -> f32 {
        self.second - self.distance
    }
}

/// Fractal summation parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fbm {
    /// How many layers to sum. Each one costs a full noise evaluation.
    pub octaves: u32,
    /// Frequency of the first layer: how many features per world unit.
    pub frequency: f32,
    /// Frequency multiplier between layers. `2.0` is the usual choice.
    pub lacunarity: f32,
    /// Amplitude multiplier between layers. Below `0.5` the detail barely
    /// shows; above it the result gets noisy.
    pub gain: f32,
}

impl Fbm {
    /// Sensible terrain defaults: five octaves, classic 2.0/0.5 falloff.
    pub const TERRAIN: Fbm = Fbm {
        octaves: 5,
        frequency: 1.0,
        lacunarity: 2.0,
        gain: 0.5,
    };

    /// A single octave — plain Perlin at the given frequency.
    pub fn single(frequency: f32) -> Self {
        Fbm {
            octaves: 1,
            frequency,
            ..Fbm::TERRAIN
        }
    }

    /// The same settings at a different base frequency.
    pub fn at(self, frequency: f32) -> Self {
        Fbm { frequency, ..self }
    }

    /// The same settings with a different octave count.
    pub fn octaves(self, octaves: u32) -> Self {
        Fbm { octaves, ..self }
    }
}

impl Default for Fbm {
    fn default() -> Self {
        Fbm::TERRAIN
    }
}

// ------------------------------------------------------------------ interior

const SEED_SALT: u64 = 0x5BF0_3635_1E1B_9A11;
const FEATURE_SALT: u64 = 0x2545_F491_4F6C_DD1D;
/// ~0.6 rad: an irrational-ish turn per octave so lattice axes never stack up.
const OCTAVE_ROTATION: f32 = 0.617_251_8;
const OCTAVE_OFFSET_2D: Vec2 = Vec2 { x: 17.31, y: 5.77 };
const OCTAVE_OFFSET_3D: Vec3 = Vec3 {
    x: 17.31,
    y: 5.77,
    z: 29.13,
};
const WARP_OFFSET_X: Vec2 = Vec2 {
    x: 137.21,
    y: 41.09,
};
const WARP_OFFSET_Y: Vec2 = Vec2 {
    x: -83.57,
    y: 219.63,
};
const SQRT_3: f32 = 1.732_050_8;

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Quintic smoothstep, `6t^5 - 15t^4 + 10t^3`. First *and* second derivatives
/// vanish at both ends, which is what keeps lattice seams invisible.
#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn floor_i(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

#[inline]
fn rotate(p: Vec2, angle: f32) -> Vec2 {
    let (s, c) = (angle.sin(), angle.cos());
    vec2(p.x * c - p.y * s, p.x * s + p.y * c)
}

/// SplitMix64's finalizer: strong avalanche, no state, no tables.
#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn hash2(seed: u64, x: i32, y: i32) -> u64 {
    let key = (x as u32 as u64) | ((y as u32 as u64) << 32);
    mix(seed ^ mix(key.wrapping_add(0x9E37_79B9_7F4A_7C15)))
}

#[inline]
fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let key = (x as u32 as u64) | ((y as u32 as u64) << 32);
    let zk = (z as u32 as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    mix(seed ^ mix(key.wrapping_add(0x9E37_79B9_7F4A_7C15) ^ zk))
}

/// Top bits of a hash as a float in `[0, 1)`.
#[inline]
fn unit(h: u64) -> f32 {
    ((h >> 40) as f32) * (1.0 / 16_777_216.0)
}

/// Eight unit directions: the four axes and the four diagonals. Using unit
/// vectors (rather than Perlin's `(1,1)`-style set) means the noise bound is
/// exactly `sqrt(2)/2` and the rescale above is exact.
#[inline]
fn gradient_2d(h: u64) -> Vec2 {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match h & 7 {
        0 => vec2(1.0, 0.0),
        1 => vec2(-1.0, 0.0),
        2 => vec2(0.0, 1.0),
        3 => vec2(0.0, -1.0),
        4 => vec2(D, D),
        5 => vec2(-D, D),
        6 => vec2(D, -D),
        _ => vec2(-D, -D),
    }
}

/// The twelve cube-edge directions, normalized.
#[inline]
fn gradient_3d(h: u64) -> Vec3 {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match h % 12 {
        0 => vec3(D, D, 0.0),
        1 => vec3(-D, D, 0.0),
        2 => vec3(D, -D, 0.0),
        3 => vec3(-D, -D, 0.0),
        4 => vec3(D, 0.0, D),
        5 => vec3(-D, 0.0, D),
        6 => vec3(D, 0.0, -D),
        7 => vec3(-D, 0.0, -D),
        8 => vec3(0.0, D, D),
        9 => vec3(0.0, -D, D),
        10 => vec3(0.0, D, -D),
        _ => vec3(0.0, -D, -D),
    }
}

/// FNV-1a, so that `Noise::named` matches `Rng::named` byte for byte.
fn hash_str(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk a deterministic spiral of sample points so the tests cover a
    /// spread of lattice cells rather than one lucky spot.
    fn samples(count: usize) -> impl Iterator<Item = Vec2> {
        (0..count).map(|i| {
            let t = i as f32 * 0.137;
            vec2(t.cos() * t * 0.31, t.sin() * t * 0.29)
        })
    }

    #[test]
    fn the_same_seed_and_point_always_give_the_same_value() {
        let a = Noise::new(7);
        let b = Noise::new(7);
        for p in samples(200) {
            assert_eq!(a.perlin_2d(p), b.perlin_2d(p));
            assert_eq!(a.value_2d(p), b.value_2d(p));
            assert_eq!(a.fbm_2d(p, Fbm::TERRAIN), b.fbm_2d(p, Fbm::TERRAIN));
        }
    }

    #[test]
    fn sampling_order_does_not_matter() {
        // The whole point of hash-based noise: generating chunk 9 before
        // chunk 1 must not change chunk 1.
        let noise = Noise::new(11);
        let forward: Vec<f32> = samples(64).map(|p| noise.perlin_2d(p)).collect();
        let mut reversed: Vec<f32> = samples(64)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|p| noise.perlin_2d(p))
            .collect();
        reversed.reverse();
        assert_eq!(forward, reversed);
    }

    #[test]
    fn different_seeds_and_names_give_different_fields() {
        let a = Noise::new(1);
        let b = Noise::new(2);
        let height = Noise::named(1, "height");
        let moisture = Noise::named(1, "moisture");

        let differing = samples(100)
            .filter(|&p| a.perlin_2d(p) != b.perlin_2d(p))
            .count();
        assert!(differing > 90, "{differing}");

        let decorrelated = samples(100)
            .filter(|&p| height.perlin_2d(p) != moisture.perlin_2d(p))
            .count();
        assert!(decorrelated > 90, "{decorrelated}");
    }

    #[test]
    fn perlin_stays_in_range_and_vanishes_on_the_lattice() {
        let noise = Noise::new(3);
        for p in samples(500) {
            let v = noise.perlin_2d(p);
            assert!((-1.0..=1.0).contains(&v), "{v} at {p:?}");
        }
        for y in -4..4 {
            for x in -4..4 {
                let v = noise.perlin_2d(vec2(x as f32, y as f32));
                assert!(v.abs() < 1e-6, "lattice point ({x}, {y}) gave {v}");
                let v3 = noise.perlin_3d(vec3(x as f32, y as f32, 1.0));
                assert!(v3.abs() < 1e-6, "lattice point ({x}, {y}, 1) gave {v3}");
            }
        }
    }

    #[test]
    fn perlin_actually_reaches_a_useful_part_of_its_range() {
        // A normalization bug that shrinks everything toward zero would still
        // pass the bounds test above, so check the spread too.
        let noise = Noise::new(5);
        let mut peak: f32 = 0.0;
        let mut sum = 0.0;
        let mut count = 0;
        for i in 0..4_000 {
            let p = vec2(i as f32 * 0.0731, (i % 97) as f32 * 0.113);
            let v = noise.perlin_2d(p);
            peak = peak.max(v.abs());
            sum += v;
            count += 1;
        }
        assert!(peak > 0.7, "peak amplitude only {peak}");
        let mean = sum / count as f32;
        assert!(mean.abs() < 0.05, "mean {mean} is far from zero");
    }

    #[test]
    fn noise_is_continuous_across_cell_boundaries() {
        // Step across a lattice line in tiny increments; a seam would show up
        // as a jump much larger than the local slope.
        let noise = Noise::new(13);
        let mut previous = noise.perlin_2d(vec2(0.9, 0.37));
        let mut x = 0.9;
        while x < 1.1 {
            x += 0.001;
            let current = noise.perlin_2d(vec2(x, 0.37));
            assert!((current - previous).abs() < 0.02, "jump at x={x}");
            previous = current;
        }
    }

    #[test]
    fn value_noise_stays_in_range() {
        let noise = Noise::new(17);
        for p in samples(300) {
            let v = noise.value_2d(p);
            assert!((-1.0..=1.0).contains(&v), "{v}");
            let v3 = noise.value_3d(vec3(p.x, p.y, p.x * 0.5));
            assert!((-1.0..=1.0).contains(&v3), "{v3}");
        }
    }

    #[test]
    fn perlin_3d_agrees_with_itself_along_a_slice() {
        let noise = Noise::new(19);
        let a = noise.perlin_3d(vec3(1.25, 0.5, 2.75));
        let b = noise.perlin_3d(vec3(1.25, 0.5, 2.75));
        assert_eq!(a, b);
        assert!(a.abs() <= 1.0);
    }

    #[test]
    fn one_octave_of_fbm_is_just_perlin() {
        let noise = Noise::new(23);
        for p in samples(100) {
            let single = Fbm::single(1.0);
            assert_eq!(noise.fbm_2d(p, single), noise.perlin_2d(p));
            assert_eq!(
                noise.fbm_3d(vec3(p.x, p.y, 0.25), single),
                noise.perlin_3d(vec3(p.x, p.y, 0.25))
            );
        }
    }

    #[test]
    fn frequency_controls_how_fast_the_field_changes() {
        // Coarse noise must vary less between neighbouring samples than fine
        // noise does; that is the whole meaning of the parameter.
        let noise = Noise::new(29);
        let variation = |frequency: f32| {
            let fbm = Fbm::single(frequency);
            let mut total = 0.0;
            let mut previous = noise.fbm_2d(vec2(0.0, 0.0), fbm);
            for i in 1..500 {
                let v = noise.fbm_2d(vec2(i as f32 * 0.01, 0.0), fbm);
                total += (v - previous).abs();
                previous = v;
            }
            total
        };
        assert!(variation(8.0) > variation(1.0) * 3.0);
    }

    #[test]
    fn more_octaves_add_detail_without_leaving_the_range() {
        let noise = Noise::new(31);
        let detail = |octaves: u32| {
            let fbm = Fbm::TERRAIN.octaves(octaves);
            let mut total = 0.0;
            let mut previous = noise.fbm_2d(vec2(0.0, 0.0), fbm);
            for i in 1..800 {
                let p = vec2(i as f32 * 0.004, 0.0);
                let v = noise.fbm_2d(p, fbm);
                assert!((-1.0..=1.0).contains(&v), "{v} with {octaves} octaves");
                total += (v - previous).abs();
                previous = v;
            }
            total
        };
        assert!(detail(6) > detail(1));
    }

    #[test]
    fn ridged_and_billow_stay_in_range_and_differ_from_fbm() {
        let noise = Noise::new(37);
        let fbm = Fbm::TERRAIN.at(0.5);
        let mut differences = 0;
        for p in samples(200) {
            let plain = noise.fbm_2d(p, fbm);
            let ridged = noise.ridged_2d(p, fbm);
            let billow = noise.billow_2d(p, fbm);
            assert!((-1.0..=1.0).contains(&ridged), "{ridged}");
            assert!((-1.0..=1.0).contains(&billow), "{billow}");
            if (ridged - plain).abs() > 1e-3 {
                differences += 1;
            }
        }
        assert!(differences > 180, "{differences}");
    }

    #[test]
    fn ridged_noise_peaks_are_sharper_than_fbm_peaks() {
        // Ridges are made of creases: the second difference around a local
        // maximum should be markedly larger than smooth fbm's.
        let noise = Noise::new(41);
        let fbm = Fbm::TERRAIN.at(0.7);
        let curvature = |f: &dyn Fn(Vec2) -> f32| {
            let mut worst: f32 = 0.0;
            for i in 0..2_000 {
                let x = i as f32 * 0.002;
                let a = f(vec2(x - 0.002, 0.3));
                let b = f(vec2(x, 0.3));
                let c = f(vec2(x + 0.002, 0.3));
                worst = worst.max((a - 2.0 * b + c).abs());
            }
            worst
        };
        let smooth = curvature(&|p| noise.fbm_2d(p, fbm));
        let sharp = curvature(&|p| noise.ridged_2d(p, fbm));
        assert!(sharp > smooth * 2.0, "smooth {smooth}, sharp {sharp}");
    }

    #[test]
    fn worley_reports_ordered_distances_and_a_stable_id() {
        let noise = Noise::new(43);
        for p in samples(400) {
            let cell = noise.worley_2d(p);
            assert!(cell.distance >= 0.0);
            assert!(cell.second >= cell.distance, "{cell:?}");
            assert!(cell.distance.is_finite() && cell.second.is_finite());
            // Within a 3x3 neighbourhood the nearest feature is never far.
            assert!(cell.distance < 2.0, "{cell:?}");
            assert!(cell.edge() >= 0.0);
            assert_eq!(noise.worley_2d(p).id, cell.id);
        }
    }

    #[test]
    fn worley_cells_are_regions_with_shared_ids() {
        // Points close together usually belong to the same cell; points far
        // apart usually do not. That is what makes the id usable as a biome.
        let noise = Noise::new(47);
        let mut same_near = 0;
        let mut same_far = 0;
        for i in 0..300 {
            let p = vec2(i as f32 * 0.021, i as f32 * 0.017);
            if noise.worley_2d(p).id == noise.worley_2d(p + vec2(0.01, 0.0)).id {
                same_near += 1;
            }
            if noise.worley_2d(p).id == noise.worley_2d(p + vec2(7.3, 4.1)).id {
                same_far += 1;
            }
        }
        assert!(same_near > 250, "{same_near}");
        assert!(same_far < 10, "{same_far}");
    }

    #[test]
    fn domain_warping_bends_the_field() {
        let noise = Noise::new(53);
        let fbm = Fbm::TERRAIN.at(0.4);
        let mut moved = 0;
        for p in samples(200) {
            let plain = noise.fbm_2d(p, fbm);
            let warped = noise.warped_2d(p, fbm, 0.8);
            assert!((-1.0..=1.0).contains(&warped), "{warped}");
            if (warped - plain).abs() > 1e-3 {
                moved += 1;
            }
        }
        assert!(moved > 190, "{moved}");
        // Zero strength must collapse back to the unwarped field.
        for p in samples(50) {
            assert_eq!(noise.warped_2d(p, fbm, 0.0), noise.fbm_2d(p, fbm));
        }
    }

    #[test]
    fn the_gradient_matches_measured_slope() {
        let noise = Noise::new(59);
        let fbm = Fbm::single(0.5);
        for p in samples(50) {
            let g = noise.fbm_gradient_2d(p, fbm, 1e-2);
            // Step along the gradient and check the field really rose by
            // roughly the predicted amount.
            let step = 1e-3;
            let predicted = g.x * step;
            let measured = noise.fbm_2d(p + vec2(step, 0.0), fbm) - noise.fbm_2d(p, fbm);
            assert!(
                (predicted - measured).abs() < 5e-4,
                "{predicted} vs {measured}"
            );
        }
    }

    #[test]
    fn a_named_field_is_reproducible_across_constructions() {
        let a = Noise::named(0xDEAD_BEEF, "ore");
        let b = Noise::named(0xDEAD_BEEF, "ore");
        assert_eq!(a.seed(), b.seed());
        assert_eq!(
            a.fbm_2d(vec2(3.5, -2.25), Fbm::TERRAIN),
            b.fbm_2d(vec2(3.5, -2.25), Fbm::TERRAIN)
        );
    }

    #[test]
    fn negative_coordinates_behave_like_positive_ones() {
        // A `as i32` truncation bug instead of a floor would make the field
        // mirror around the origin; check it does not.
        let noise = Noise::new(61);
        assert_ne!(
            noise.perlin_2d(vec2(-0.3, -0.7)),
            noise.perlin_2d(vec2(0.3, 0.7))
        );
        assert_eq!(floor_i(-0.3), -1);
        assert_eq!(floor_i(-1.0), -1);
        assert_eq!(floor_i(0.9), 0);
        for p in samples(200) {
            let mirrored = vec2(-p.x, -p.y);
            let v = noise.perlin_2d(mirrored);
            assert!((-1.0..=1.0).contains(&v), "{v}");
        }
    }
}
