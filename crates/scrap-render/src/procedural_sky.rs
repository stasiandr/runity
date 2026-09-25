//! Unity's Procedural skybox (`Skybox/Procedural`, the built-in shader
//! every URP scene starts with): O'Neil's two-sample scattering through a
//! thin shell of air, lit by the sun — so the sky follows the sun as
//! Unity's does, deep blue overhead and pale at the horizon, warmer as the
//! sun goes down, the ground's colour below.
//!
//! The same maths is in `render.wgsl` (`unity_sky`) for the picture; here
//! it is for the light the sky gives all round, which Unity works out from
//! the skybox when a scene's ambient source is the Skybox.

use glam::Vec3;

use crate::render::Sky;

const OUTER: f32 = 1.025;
const CAMERA_HEIGHT: f32 = 0.0001;
const MIE: f32 = 0.0010;
const SUN_BRIGHTNESS: f32 = 20.0;
const MAX_SCATTER: f32 = 50.0;
const SCALE: f32 = 1.0 / (OUTER - 1.0);
const SCALE_DEPTH: f32 = 0.25;
const SCALE_OVER_SCALE_DEPTH: f32 = SCALE / SCALE_DEPTH;

/// A sky's numbers as the shader wants them, from the scene's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitySky {
    /// 1/λ⁴ of the wavelengths the tint picks.
    pub inv_wavelength: Vec3,
    /// Rayleigh's constant for the air's thickness.
    pub rayleigh: f32,
    /// Below the horizon, linear.
    pub ground: Vec3,
    pub exposure: f32,
}

impl UnitySky {
    pub fn new(sky: &Sky) -> Self {
        // Unity takes the tint in gamma, so its middle grey is the
        // default wavelengths.
        let tint = Vec3::from(sky.tint).max(Vec3::ZERO).powf(1.0 / 2.2);
        let wavelength = Vec3::new(0.65, 0.57, 0.475);
        let range = Vec3::splat(0.15);
        let scattering = wavelength - range + 2.0 * range * (Vec3::ONE - tint);
        Self {
            inv_wavelength: Vec3::ONE / scattering.powf(4.0),
            rayleigh: 0.0025 * sky.thickness.max(0.0).powf(2.5),
            ground: Vec3::from(sky.ground),
            exposure: sky.exposure.max(0.0),
        }
    }

    /// The sky's light coming along `-ray` (looking along `ray`), with the
    /// sun toward `to_sun`; no sun's disc.
    pub fn radiance(&self, ray: Vec3, to_sun: Vec3) -> Vec3 {
        self.scatter(ray, to_sun).0
    }

    /// The sky that way, and the light that comes through the air unscattered
    /// (what the sun's disc is made of).
    fn scatter(&self, ray: Vec3, to_sun: Vec3) -> (Vec3, Vec3) {
        let ray = ray.normalize_or(Vec3::Y);
        let kr_esun = self.rayleigh * SUN_BRIGHTNESS;
        let kr_4pi = self.rayleigh * 4.0 * std::f32::consts::PI;
        let km_esun = MIE * SUN_BRIGHTNESS;
        let km_4pi = MIE * 4.0 * std::f32::consts::PI;
        let extinction = self.inv_wavelength * kr_4pi + Vec3::splat(km_4pi);
        let camera = Vec3::new(0.0, 1.0 + CAMERA_HEIGHT, 0.0);
        let (c_in, c_out) = if ray.y >= 0.0 {
            let far = (OUTER * OUTER + ray.y * ray.y - 1.0).sqrt() - ray.y;
            let height = 1.0 + CAMERA_HEIGHT;
            let depth = (SCALE_OVER_SCALE_DEPTH * -CAMERA_HEIGHT).exp();
            let start_offset = depth * scale(ray.dot(camera) / height);
            let length = far / 2.0;
            let step = ray * length;
            let mut p = camera + step * 0.5;
            let mut front = Vec3::ZERO;
            for _ in 0..2 {
                let h = p.length();
                let d = (SCALE_OVER_SCALE_DEPTH * (1.0 - h)).exp();
                let scatter = start_offset + d * (scale(to_sun.dot(p) / h) - scale(ray.dot(p) / h));
                let attenuate = exp3(-scatter.clamp(0.0, MAX_SCATTER) * extinction);
                front += attenuate * (d * length * SCALE);
                p += step;
            }
            (front * (self.inv_wavelength * kr_esun), front * km_esun)
        } else {
            let far = -CAMERA_HEIGHT / ray.y.min(-0.001);
            let at = camera + ray * far;
            let depth = (-CAMERA_HEIGHT / SCALE_DEPTH).exp();
            let camera_scale = scale((-ray).dot(at));
            let light_scale = scale(to_sun.dot(at));
            let length = far / 2.0;
            let p = camera + ray * length * 0.5;
            let d = (SCALE_OVER_SCALE_DEPTH * (1.0 - p.length())).exp();
            let scatter = d * (light_scale + camera_scale) - depth * camera_scale;
            let attenuate = exp3(-scatter.clamp(0.0, MAX_SCATTER) * extinction);
            let front = attenuate * (d * length * SCALE);
            (
                front * (self.inv_wavelength * kr_esun + Vec3::splat(km_esun)),
                attenuate.clamp(Vec3::ZERO, Vec3::ONE),
            )
        };
        let ground = (c_in + self.ground * c_out) * self.exposure;
        let cos = to_sun.dot(-ray);
        let sky = c_in * (0.75 + 0.75 * cos * cos) * self.exposure;
        // Blended over a fiftieth below the horizon.
        (sky.lerp(ground, (-ray.y / 0.02).clamp(0.0, 1.0)), c_out)
    }

    /// The light from all round, as Unity's ambient probe has it from the
    /// skybox — the sky and the sun's disc (`sun_size` degrees across, lit
    /// by `sun`, the sun's colour times its intensity) as second-order
    /// spherical harmonics: what a face turned up, sideways (the four ways
    /// round, on average) and down sees.
    pub fn ambient(&self, to_sun: Vec3, sun: Vec3, sun_size: f32) -> [Vec3; 3] {
        const RINGS: usize = 24;
        const SPOKES: usize = 48;
        let to_sun = to_sun.normalize_or(Vec3::Y);
        let mut sh = [Vec3::ZERO; 9];
        let each = 4.0 * std::f32::consts::PI / (RINGS * SPOKES) as f32;
        for i in 0..RINGS {
            // Rings of equal area, pole to pole.
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / RINGS as f32;
            let r = (1.0 - y * y).max(0.0).sqrt();
            for j in 0..SPOKES {
                let a = (j as f32 + 0.5) / SPOKES as f32 * std::f32::consts::TAU;
                let ray = Vec3::new(r * a.cos(), y, r * a.sin());
                let l = self.radiance(ray, to_sun) * each;
                for (c, b) in sh.iter_mut().zip(basis(ray)) {
                    *c += l * b;
                }
            }
        }
        // The disc, too small for the rings: Unity's simple one, a spot
        // fading as the square of the way out to its radius — its light
        // all from where the sun is.
        let radius = (sun_size.max(0.0) * 0.5).to_radians();
        if radius > 0.0 && to_sun.y >= 0.0 {
            // ∫ (1 − smoothstep(0, 1, u))² u du over the disc.
            const SPOT: f32 = 3.0 / 35.0;
            let solid = std::f32::consts::TAU * radius * radius * SPOT;
            let c_out = self.scatter(to_sun, to_sun).1;
            let bright = sun.length().clamp(0.25, 1.0);
            let disc = 27.0 * (c_out * 400.0 * SUN_BRIGHTNESS).clamp(Vec3::ZERO, Vec3::ONE) * sun / bright;
            for (c, b) in sh.iter_mut().zip(basis(to_sun)) {
                *c += disc * solid * b;
            }
        }
        // Irradiance over π: each band as the cosine lobe convolves it.
        let seen = |n: Vec3| {
            let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
            sh.iter().zip(basis(n)).zip(band).map(|((c, b), k)| *c * b * k).sum::<Vec3>().max(Vec3::ZERO)
        };
        let side = (seen(Vec3::X) + seen(Vec3::NEG_X) + seen(Vec3::Z) + seen(Vec3::NEG_Z)) * 0.25;
        [seen(Vec3::Y), side, seen(Vec3::NEG_Y)]
    }
}

/// Unity's gradient light (Trilight: `sky`, `equator`, `ground`) as its
/// ambient probe has it: what a face turned up, sideways and down sees —
/// up half the sky and half the equator, sideways mostly the equator. The
/// shares are Unity's own, read off its probe (the oracle's
/// lit_GDScene.json); in between they follow the probe's curve, a
/// quadratic in how far the face is turned up.
pub fn gradient_seen(sky: Vec3, equator: Vec3, ground: Vec3) -> [Vec3; 3] {
    const UP_SKY: f32 = 0.5433;
    const UP_EQUATOR: f32 = 0.458;
    const SIDE_SKY: f32 = 0.1148;
    const SIDE_EQUATOR: f32 = 0.7705;
    [
        sky * UP_SKY + equator * UP_EQUATOR,
        (sky + ground) * SIDE_SKY + equator * SIDE_EQUATOR,
        ground * UP_SKY + equator * UP_EQUATOR,
    ]
}

/// The real spherical harmonics to the second band, at unit `d`.
fn basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

fn scale(cos: f32) -> f32 {
    let x = 1.0 - cos;
    0.25 * (-0.00287 + x * (0.459 + x * (3.83 + x * (-6.80 + x * 5.25)))).exp()
}

fn exp3(v: Vec3) -> Vec3 {
    Vec3::new(v.x.exp(), v.y.exp(), v.z.exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unity_default() -> UnitySky {
        UnitySky::new(&Sky {
            mode: crate::render::SkyMode::Procedural,
            ground: [0.112, 0.1, 0.095],
            exposure: 1.3,
            ..Default::default()
        })
    }

    #[test]
    fn overhead_is_deeper_blue_than_the_horizon() {
        let sky = unity_default();
        let to_sun = Vec3::new(0.0, 0.43, 0.9).normalize();
        let zenith = sky.radiance(Vec3::Y, to_sun);
        let horizon = sky.radiance(Vec3::new(1.0, 0.02, 0.0).normalize(), to_sun);
        assert!(zenith.z > zenith.x * 2.0, "blue overhead: {zenith}");
        assert!(horizon.x / horizon.z > zenith.x / zenith.z, "paler, warmer at the horizon: {horizon} {zenith}");
        assert!(horizon.length() > zenith.length(), "brighter at the horizon: {horizon} {zenith}");
    }

    #[test]
    fn the_light_all_round_is_unitys_probe() {
        // Level1's sun (Unity's (0.3214, 0.766, −0.5567), mirrored) and
        // what Unity's ambient probe says there (lit_Level1.json).
        let to_sun = Vec3::new(0.3214, 0.766, 0.5567).normalize();
        let [up, side, down] = unity_default().ambient(to_sun, Vec3::splat(1.1), 4.5837);
        let near = |a: Vec3, b: Vec3, tolerance: f32| (a - b).abs().max_element() < tolerance;
        assert!(near(up, Vec3::new(0.1392, 0.2182, 0.3762), 0.01), "up {up}");
        assert!(near(down, Vec3::new(0.1506, 0.1356, 0.1213), 0.01), "down {down}");
        assert!(near(side, Vec3::new(0.198, 0.25, 0.336), 0.02), "sideways {side}");
    }

    #[test]
    fn a_gradient_is_seen_as_unitys_probe_sees_it() {
        // GDScene's colours at its morning, and Unity's probe for them.
        let [up, side, down] = gradient_seen(
            Vec3::new(0.2238, 0.1706, 0.0929),
            Vec3::new(0.0406, 0.0302, 0.1526),
            Vec3::new(0.1082, 0.1505, 0.1945),
        );
        let near = |a: Vec3, b: Vec3| (a - b).abs().max_element() < 0.002;
        assert!(near(up, Vec3::new(0.14, 0.1063, 0.1201)), "{up}");
        assert!(near(side, Vec3::new(0.0693, 0.0601, 0.1506)), "{side}");
        assert!(near(down, Vec3::new(0.0771, 0.0954, 0.1754)), "{down}");
    }
}
