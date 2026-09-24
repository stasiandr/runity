//! The open sea: a line's `ocean`. `ocean: (size: 64.0)` spreads a sea of
//! wind-raised waves round the entity — tiles of one patch of water whose
//! waves are summed from thousands of sine waves of the right lengths and
//! heights for the scene's wind, stepped by a fast Fourier transform each
//! frame (Tessendorf 2001, "Simulating Ocean Water"): the Phillips
//! spectrum, each wave moving at the speed deep water gives its length,
//! and the crests pulled sharp by `choppy`. What stands in it floats
//! ([`water_height`](crate::heightfield::water_height)).

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::wind::Wind;
use runity_core::world::WorldTransform;
use runity_geometry::mesh_asset::Vertex;

/// An ocean, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ocean {
    /// Metres across one patch; it tiles.
    pub size: f32,
    /// Points across a patch: a power of two.
    pub resolution: u32,
    /// Patches across, round the entity.
    pub tiles: u32,
    /// How high the waves are: 1 is a fresh breeze's.
    pub height: f32,
    /// How sharp the crests are pulled: 0 round, 1 sharp.
    pub choppy: f32,
    /// Metres a second of wind at the scene's `strength: 1`.
    pub wind_speed: f32,
}

impl Default for Ocean {
    fn default() -> Self {
        Self { size: 64.0, resolution: 64, tiles: 3, height: 1.0, choppy: 0.8, wind_speed: 8.0 }
    }
}

runity_core::impl_parts! {
    Ocean => "ocean";
}

/// The ocean of a line, read off it.
pub trait OceanLine {
    fn ocean(&self) -> Option<Ocean>;
}

impl OceanLine for runity_core::EntityDesc {
    fn ocean(&self) -> Option<Ocean> {
        self.part()
    }
}

impl OceanLine for runity_core::scene::Override {
    fn ocean(&self) -> Option<Ocean> {
        self.part()
    }
}

const G: f32 = 9.81;

/// A complex number.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct C(f32, f32);

impl C {
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn sub(self, o: C) -> C {
        C(self.0 - o.0, self.1 - o.1)
    }
    fn conj(self) -> C {
        C(self.0, -self.1)
    }
    fn scale(self, s: f32) -> C {
        C(self.0 * s, self.1 * s)
    }
    fn turn(angle: f32) -> C {
        C(angle.cos(), angle.sin())
    }
}

/// An inverse Fourier transform along a line of `n` (a power of two),
/// every `stride`th value from `start`, in place.
fn fft_line(values: &mut [C], start: usize, stride: usize, n: usize) {
    // Bit-reversed order.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            values.swap(start + i * stride, start + j * stride);
        }
    }
    let mut len = 2;
    while len <= n {
        let w = C::turn(std::f32::consts::TAU / len as f32);
        for s in (0..n).step_by(len) {
            let mut t = C(1.0, 0.0);
            for k in 0..len / 2 {
                let a = start + (s + k) * stride;
                let b = start + (s + k + len / 2) * stride;
                let u = values[a];
                let v = values[b].mul(t);
                values[a] = u.add(v);
                values[b] = u.sub(v);
                t = t.mul(w);
            }
        }
        len <<= 1;
    }
}

/// A two-dimensional inverse transform of an `n` × `n` grid, in place.
fn fft_2d(values: &mut [C], n: usize) {
    for row in 0..n {
        fft_line(values, row * n, 1, n);
    }
    for column in 0..n {
        fft_line(values, column, n, n);
    }
}

/// An ocean as it moves: the component [`run_oceans`] steps.
#[derive(Debug, Clone)]
pub struct OceanState {
    pub ocean: Ocean,
    n: usize,
    /// Each wave's starting amplitude, h₀(k), and that of the wave the
    /// other way, h₀(−k)*.
    h0: Vec<(C, C)>,
    /// Its wave vectors.
    k: Vec<Vec2>,
    /// The patch now: height, and how far each point is pulled across.
    pub heights: Vec<f32>,
    shifts: Vec<Vec2>,
    time: f32,
    centre: Vec3,
    /// The wind its waves were made for.
    wind: Wind,
    /// The scene's wind: whoever spawns the scene sets it.
    pub blowing: Wind,
}

impl OceanState {
    pub fn new(ocean: Ocean) -> Self {
        let n = (ocean.resolution.clamp(8, 256) as usize).next_power_of_two();
        Self { ocean, n, h0: Vec::new(), k: Vec::new(), heights: vec![0.0; n * n], shifts: vec![Vec2::ZERO; n * n], time: 0.0, centre: Vec3::ZERO, wind: Wind::default(), blowing: Wind::default() }
    }

    /// The waves for a wind: the Phillips spectrum, with random phases.
    fn spectrum(&mut self, wind: Wind) {
        let n = self.n;
        let l = self.ocean.size.max(1.0);
        let dir = Vec2::new(wind.direction.x, wind.direction.z).normalize_or(Vec2::X);
        let speed = (self.ocean.wind_speed * wind.strength.max(0.05)).max(0.5);
        let longest = speed * speed / G;
        let a = 3e-4 * self.ocean.height.max(0.0);
        let mut dice = 0x2545_f491_4f6c_dd1du64;
        let mut gauss = || {
            let mut u = || {
                dice ^= dice << 13;
                dice ^= dice >> 7;
                dice ^= dice << 17;
                ((dice >> 40) as f32 + 0.5) / (1u64 << 24) as f32
            };
            let (a, b) = (u(), u());
            let r = (-2.0 * a.ln()).sqrt();
            (r * (std::f32::consts::TAU * b).cos(), r * (std::f32::consts::TAU * b).sin())
        };
        let phillips = |k: Vec2| {
            let kl = k.length();
            if kl < 1e-6 {
                return 0.0;
            }
            let along = (k / kl).dot(dir);
            // Only what goes with the wind, and the smallest waves let go.
            let with = if along < 0.0 { along * along * 0.07 } else { along * along };
            a * (-1.0 / (kl * longest).powi(2)).exp() / kl.powi(4) * with * (-(kl * l / n as f32 * 0.5).powi(2)).exp()
        };
        self.k.clear();
        self.h0.clear();
        for m in 0..n {
            for i in 0..n {
                let k = Vec2::new(i as f32 - n as f32 / 2.0, m as f32 - n as f32 / 2.0) * (std::f32::consts::TAU / l);
                let (g1, g2) = gauss();
                let (g3, g4) = gauss();
                let h = C(g1, g2).scale((phillips(k) / 2.0).sqrt());
                let h_back = C(g3, g4).scale((phillips(-k) / 2.0).sqrt()).conj();
                self.k.push(k);
                self.h0.push((h, h_back));
            }
        }
        self.wind = wind;
    }

    /// The sea at `time` seconds: every wave moved on, and summed.
    fn waves(&mut self) {
        let n = self.n;
        let mut h = vec![C::default(); n * n];
        let mut dx = vec![C::default(); n * n];
        let mut dz = vec![C::default(); n * n];
        for (idx, (k, (h0, h0_back))) in self.k.iter().zip(&self.h0).enumerate() {
            let w = (G * k.length()).sqrt();
            let turn = C::turn(w * self.time);
            let hk = h0.mul(turn).add(h0_back.mul(turn.conj()));
            h[idx] = hk;
            let kl = k.length();
            if kl > 1e-6 {
                // −i k/|k| h: the crest pulled toward where it goes.
                let side = C(hk.1, -hk.0);
                dx[idx] = side.scale(k.x / kl);
                dz[idx] = side.scale(k.y / kl);
            }
        }
        fft_2d(&mut h, n);
        fft_2d(&mut dx, n);
        fft_2d(&mut dz, n);
        let choppy = self.ocean.choppy.max(0.0);
        for m in 0..n {
            for i in 0..n {
                // The spectrum was centred: every other point turned over.
                let sign = if (i + m) % 2 == 0 { 1.0 } else { -1.0 };
                let at = m * n + i;
                self.heights[at] = h[at].0 * sign;
                self.shifts[at] = Vec2::new(dx[at].0, dz[at].0) * sign * choppy;
            }
        }
    }

    /// Along by `seconds`, where the entity is, in `wind`.
    pub fn advance(&mut self, placed: Mat4, wind: Wind, seconds: f32) {
        if self.h0.is_empty() || self.wind != wind {
            self.spectrum(wind);
        }
        self.blowing = wind;
        self.centre = placed.w_axis.truncate();
        self.time += seconds.max(0.0);
        self.waves();
    }

    /// The patch's value at a point, tiled, bilinear.
    fn sample<T: Copy + std::ops::Mul<f32, Output = T> + std::ops::Add<Output = T>>(&self, values: &[T], p: Vec3) -> T {
        let n = self.n;
        let l = self.ocean.size;
        let x = ((p.x - self.centre.x) / l * n as f32).rem_euclid(n as f32);
        let z = ((p.z - self.centre.z) / l * n as f32).rem_euclid(n as f32);
        let (i, k) = (x as usize % n, z as usize % n);
        let (fx, fz) = (x.fract(), z.fract());
        let v = |a: usize, b: usize| values[(b % n) * n + a % n];
        let low = v(i, k) * (1.0 - fx) + v(i + 1, k) * fx;
        let high = v(i, k + 1) * (1.0 - fx) + v(i + 1, k + 1) * fx;
        low * (1.0 - fz) + high * fz
    }

    /// The sea's surface at a point.
    pub fn height_at(&self, p: Vec3) -> Option<f32> {
        if self.h0.is_empty() {
            return None;
        }
        let half = self.ocean.size * self.ocean.tiles.max(1) as f32 * 0.5;
        if (p.x - self.centre.x).abs() > half || (p.z - self.centre.z).abs() > half {
            return None;
        }
        // Where a point pulled across ends up is near enough where it
        // started for a floating body.
        Some(self.centre.y + self.sample(&self.heights, p))
    }

    /// Its tiles, in the space of `placed`.
    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        let n = self.n;
        let l = self.ocean.size;
        let tiles = self.ocean.tiles.clamp(1, 9) as usize;
        let side = n * tiles + 1;
        let back = placed.inverse();
        let start = -l * tiles as f32 * 0.5;
        let step = l / n as f32;
        let point = |i: usize, k: usize| {
            let at = (k % n) * n + (i % n);
            let shift = self.shifts[at];
            Vec3::new(start + i as f32 * step + shift.x, self.heights[at], start + k as f32 * step + shift.y)
        };
        let mut vertices = Vec::with_capacity(side * side);
        for k in 0..side {
            for i in 0..side {
                let p = point(i, k);
                let (e, w) = (point(i + 1, k), point(i + n - 1, k) - Vec3::new(l, 0.0, 0.0));
                let (s, nn) = (point(i, k + 1), point(i, k + n - 1) - Vec3::new(0.0, 0.0, l));
                let normal = (s - nn).cross(e - w).normalize_or(Vec3::Y);
                let world = self.centre + p;
                vertices.push(Vertex {
                    position: back.transform_point3(world).to_array(),
                    normal: back.transform_vector3(normal).normalize_or(Vec3::Y).to_array(),
                    uv: [world.x, world.z],
                });
            }
        }
        let mut indices = Vec::with_capacity((side - 1) * (side - 1) * 6);
        for k in 0..side as u32 - 1 {
            for i in 0..side as u32 - 1 {
                let a = k * side as u32 + i;
                let (b, c, d) = (a + 1, a + side as u32, a + side as u32 + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (vertices, indices)
    }
}

/// Every ocean on by `seconds` in its scene's wind.
pub fn run_oceans(world: &mut hecs::World, seconds: f32) {
    // The same sea on every peer (docs/netsim.md): its waves are the same
    // spectrum drawn at the session's time.
    let clock = runity_core::netsim::session_time(world);
    for (state, placed) in world.query_mut::<(&mut OceanState, &WorldTransform)>() {
        let wind = state.blowing;
        runity_soft::net::keep_time(&mut state.time, clock);
        state.advance(placed.0, wind, seconds);
    }
}

/// Every ocean raised by this wind: the scene's.
pub fn set_wind(world: &mut hecs::World, wind: Wind) {
    for state in world.query_mut::<&mut OceanState>() {
        state.blowing = wind;
    }
}

/// The fluid module's dresser for oceans.
pub struct OceanDress;

impl runity_core::world::Dress for OceanDress {
    fn parts(&self) -> &[&'static str] {
        &["ocean"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.ocean() {
            Some(o) => {
                let _ = world.insert_one(entity, OceanState::new(o));
            }
            None => {
                let _ = world.remove_one::<OceanState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transform_turns_one_wave_back_into_a_sine() {
        // A single frequency in: one wave out, of the right size.
        let n = 16;
        let mut values = vec![C::default(); n * n];
        values[1] = C(0.5, 0.0);
        values[n - 1] = C(0.5, 0.0);
        fft_2d(&mut values, n);
        for (i, v) in values.iter().take(n).enumerate() {
            let expected = (std::f32::consts::TAU * i as f32 / n as f32).cos();
            assert!((v.0 - expected).abs() < 1e-4, "{i}: {} vs {expected}", v.0);
        }
    }

    #[test]
    fn the_wind_raises_waves_that_move_downwind_and_a_stronger_wind_higher() {
        let sea = |strength: f32| {
            let mut s = OceanState::new(Ocean { size: 64.0, resolution: 64, ..Ocean::default() });
            s.advance(Mat4::IDENTITY, Wind { direction: Vec3::X, strength }, 1.0);
            s
        };
        let rms = |s: &OceanState| (s.heights.iter().map(|h| h * h).sum::<f32>() / s.heights.len() as f32).sqrt();
        let (calm, gale) = (sea(0.5), sea(2.0));
        assert!(rms(&calm) > 0.01, "waves: {}", rms(&calm));
        assert!(rms(&gale) > rms(&calm) * 2.0, "higher in a gale: {} vs {}", rms(&gale), rms(&calm));
        // Longer along the wind than across it: the crests lie across.
        let n = 64;
        let along: f32 = (0..n - 1).map(|i| (calm.heights[i + 1] - calm.heights[i]).abs()).sum();
        let across: f32 = (0..n - 1).map(|k| (calm.heights[(k + 1) * n] - calm.heights[k * n]).abs()).sum();
        assert!(across < along, "crests across the wind: along {along} across {across}");
        // A point bobs: the sea moves.
        let mut later = calm.clone();
        later.advance(Mat4::IDENTITY, Wind { direction: Vec3::X, strength: 0.5 }, 0.5);
        assert_ne!(later.heights, calm.heights);
        assert!(later.height_at(Vec3::new(10.0, 0.0, 10.0)).is_some());
        let (vertices, indices) = calm.mesh(Mat4::IDENTITY);
        assert_eq!(vertices.len(), (64 * 3 + 1) * (64 * 3 + 1));
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
    }
}
