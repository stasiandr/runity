//! An analytic sky, and the image-based lighting that comes out of it.
//!
//! Ambient light is what makes a render look like it belongs somewhere. A
//! single "ambient color" term cannot do it: light arriving from the sky above
//! is blue and bright, light from the ground below is dim and warm, and a
//! shiny surface should reflect *that*, not a constant.
//!
//! Everything here is generated, never loaded: a gradient dome with a sun,
//! turned into two things the shading passes ask for —
//!
//! * a small equirectangular map with a mip chain, where each level stands in
//!   for a rougher reflection ([`Sky::sample`]);
//! * nine spherical-harmonic coefficients, which reconstruct the cosine-weighted
//!   irradiance for any normal in about twenty multiplications
//!   ([`Sky::irradiance`]).
//!
//! The sun's disc is deliberately *not* baked into the prefiltered map — it is
//! far smaller than a texel there. It belongs to the directional light, whose
//! GGX lobe gives a far better highlight than a blurred dot ever would.

use crate::color::Color;
use crate::pbr::Light;
use runity_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyParams {
    /// Direction *towards* the sun.
    pub sun_direction: Vec3,
    pub sun_color: Color,
    /// Radiance of the disc itself. Sunlight is orders of magnitude brighter
    /// than the sky around it, which is the entire reason for an HDR pipeline.
    pub sun_intensity: f32,
    /// Irradiance the sun delivers to a surface facing it — what the
    /// directional light carries.
    ///
    /// Kept separate from `sun_intensity` because the two are calibrated
    /// against different things: the disc against the tone curve, the light
    /// against the albedo of what it falls on.
    pub sun_irradiance: f32,
    pub sun_angular_radius: f32,
    pub zenith_color: Color,
    pub horizon_color: Color,
    pub ground_color: Color,
    /// How tightly the horizon color hugs the horizon.
    pub horizon_falloff: f32,
    /// Overall multiplier on the dome (not the sun).
    pub intensity: f32,
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(0.45, 0.55, 0.35).normalized(),
            sun_color: Color::rgb(1.0, 0.92, 0.78),
            sun_intensity: 24.0,
            sun_irradiance: 3.2,
            sun_angular_radius: 0.035,
            zenith_color: Color::rgb(0.12, 0.26, 0.58),
            horizon_color: Color::rgb(0.62, 0.72, 0.86),
            ground_color: Color::rgb(0.14, 0.13, 0.12),
            horizon_falloff: 3.0,
            intensity: 1.0,
        }
    }
}

impl SkyParams {
    /// A low, warm sun — long shadows and a strong color split between sky and
    /// sun, which flatters any material.
    pub fn golden_hour() -> Self {
        Self {
            sun_direction: Vec3::new(0.72, 0.16, 0.35).normalized(),
            sun_color: Color::rgb(1.0, 0.66, 0.36),
            sun_intensity: 26.0,
            sun_irradiance: 2.6,
            zenith_color: Color::rgb(0.10, 0.20, 0.44),
            horizon_color: Color::rgb(0.86, 0.60, 0.42),
            ground_color: Color::rgb(0.10, 0.09, 0.08),
            horizon_falloff: 2.2,
            ..Self::default()
        }
    }

    /// No sun, bright uniform dome: the lighting equivalent of a softbox.
    pub fn overcast() -> Self {
        Self {
            sun_intensity: 0.0,
            sun_irradiance: 0.0,
            zenith_color: Color::rgb(0.55, 0.58, 0.62),
            horizon_color: Color::rgb(0.42, 0.44, 0.47),
            ground_color: Color::rgb(0.18, 0.18, 0.18),
            horizon_falloff: 1.5,
            ..Self::default()
        }
    }
}

/// One level of the equirectangular environment map.
#[derive(Debug, Clone)]
struct Mip {
    width: usize,
    height: usize,
    texels: Vec<Color>,
}

impl Mip {
    /// Bilinear lookup; `u` wraps around, `v` clamps at the poles.
    fn sample(&self, u: f32, v: f32) -> Color {
        let x = u * self.width as f32 - 0.5;
        let y = (v * self.height as f32 - 0.5).clamp(0.0, self.height as f32 - 1.0);
        let x0 = x.floor();
        let y0 = y.floor();
        let tx = x - x0;
        let ty = y - y0;
        let wrap = |x: i64| x.rem_euclid(self.width as i64) as usize;
        let clamp = |y: i64| y.clamp(0, self.height as i64 - 1) as usize;
        let (x0i, y0i) = (x0 as i64, y0 as i64);
        let texel = |x: i64, y: i64| self.texels[clamp(y) * self.width + wrap(x)];
        let top = texel(x0i, y0i).lerp(texel(x0i + 1, y0i), tx);
        let bottom = texel(x0i, y0i + 1).lerp(texel(x0i + 1, y0i + 1), tx);
        top.lerp(bottom, ty)
    }
}

/// Equirectangular mapping: longitude across, latitude down.
#[inline]
fn direction_to_uv(direction: Vec3) -> (f32, f32) {
    let d = direction.normalized();
    let u = d.z.atan2(d.x) / core::f32::consts::TAU + 0.5;
    let v = d.y.clamp(-1.0, 1.0).acos() / core::f32::consts::PI;
    (u, v)
}

#[inline]
fn uv_to_direction(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * core::f32::consts::TAU;
    let theta = v * core::f32::consts::PI;
    let (sin_theta, cos_theta) = theta.sin_cos();
    Vec3::new(sin_theta * phi.cos(), cos_theta, sin_theta * phi.sin())
}

/// A generated environment: background, reflections and ambient light.
#[derive(Debug, Clone)]
pub struct Sky {
    params: SkyParams,
    mips: Vec<Mip>,
    /// Nine coefficients of the dome's radiance, RGB each.
    sh: [Color; 9],
}

/// Base resolution of the prefiltered map. Small on purpose: it is only ever
/// read with a wide filter, and the sun does not live in it.
const BASE_WIDTH: usize = 128;
const BASE_HEIGHT: usize = 64;

impl Default for Sky {
    fn default() -> Self {
        Self::new(SkyParams::default())
    }
}

impl Sky {
    pub fn new(params: SkyParams) -> Self {
        let base = Self::render_dome(&params, BASE_WIDTH, BASE_HEIGHT);
        let sh = Self::project_sh(&base);
        let mips = Self::build_mip_chain(base);
        Self { params, mips, sh }
    }

    pub fn params(&self) -> &SkyParams {
        &self.params
    }

    pub fn set_params(&mut self, params: SkyParams) {
        *self = Self::new(params);
    }

    /// The directional light matching the sun, or `None` for a sunless sky.
    pub fn sun_light(&self) -> Option<Light> {
        if self.params.sun_irradiance <= 0.0 {
            return None;
        }
        Some(Light::directional(
            -self.params.sun_direction,
            self.params.sun_color,
            self.params.sun_irradiance,
        ))
    }

    /// Radiance arriving from `direction`, sun included — the background.
    pub fn radiance(&self, direction: Vec3) -> Color {
        let d = direction.normalized();
        let dome = self.dome_radiance(d);
        if self.params.sun_intensity <= 0.0 {
            return dome;
        }
        let cos_angle = d.dot(self.params.sun_direction);
        let disc = if cos_angle >= self.params.sun_angular_radius.cos() {
            self.params.sun_color.scale_rgb(self.params.sun_intensity)
        } else {
            Color::BLACK
        };
        Color::rgb(dome.r + disc.r, dome.g + disc.g, dome.b + disc.b)
    }

    /// The sky without the sun's disc: gradient, ground, and the glow around
    /// the sun. This is what gets prefiltered.
    fn dome_radiance(&self, direction: Vec3) -> Color {
        let p = &self.params;
        let d = direction.normalized();

        let sky = if d.y >= 0.0 {
            let t = (1.0 - d.y).powf(p.horizon_falloff);
            p.zenith_color.lerp(p.horizon_color, t)
        } else {
            // Fade into the ground instead of cutting at the horizon line.
            let t = (-d.y * 12.0).clamp(0.0, 1.0);
            p.horizon_color.lerp(p.ground_color, t)
        };

        let mut color = sky.scale_rgb(p.intensity);
        if p.sun_intensity > 0.0 {
            let cos_angle = d.dot(p.sun_direction).max(0.0);
            // Two lobes: a tight one for the glare around the disc, a wide one
            // for the haze that brightens the whole quarter of the sky.
            let glare = cos_angle.powf(350.0) * 0.6;
            let haze = cos_angle.powf(6.0) * 0.12;
            let sun = p
                .sun_color
                .scale_rgb(p.sun_intensity * (glare + haze) * 0.1);
            color = Color::rgb(color.r + sun.r, color.g + sun.g, color.b + sun.b);
        }
        color
    }

    fn render_dome(params: &SkyParams, width: usize, height: usize) -> Mip {
        let sky = Sky {
            params: *params,
            mips: Vec::new(),
            sh: [Color::BLACK; 9],
        };
        let mut texels = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let v = (y as f32 + 0.5) / height as f32;
                texels.push(sky.dome_radiance(uv_to_direction(u, v)));
            }
        }
        Mip {
            width,
            height,
            texels,
        }
    }

    /// Successive halving. A box filter is not a true GGX convolution, but over
    /// a gradient dome the difference is invisible, and it costs nothing.
    fn build_mip_chain(base: Mip) -> Vec<Mip> {
        let mut mips = vec![base];
        while mips.last().unwrap().width > 4 && mips.last().unwrap().height > 2 {
            let previous = mips.last().unwrap();
            let width = previous.width / 2;
            let height = previous.height / 2;
            let mut texels = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let mut sum = Color::BLACK;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(previous.width - 1);
                        let sy = (y * 2 + dy).min(previous.height - 1);
                        let c = previous.texels[sy * previous.width + sx];
                        sum = Color::rgb(sum.r + c.r, sum.g + c.g, sum.b + c.b);
                    }
                    texels.push(sum.scale_rgb(0.25));
                }
            }
            mips.push(Mip {
                width,
                height,
                texels,
            });
        }
        mips
    }

    /// Project the dome onto the first nine real spherical harmonics.
    fn project_sh(base: &Mip) -> [Color; 9] {
        let mut coefficients = [Color::BLACK; 9];
        for y in 0..base.height {
            let v = (y as f32 + 0.5) / base.height as f32;
            let theta = v * core::f32::consts::PI;
            // Solid angle of this texel: sinθ dθ dφ.
            let solid_angle = theta.sin()
                * (core::f32::consts::PI / base.height as f32)
                * (core::f32::consts::TAU / base.width as f32);
            for x in 0..base.width {
                let u = (x as f32 + 0.5) / base.width as f32;
                let direction = uv_to_direction(u, v);
                let radiance = base.texels[y * base.width + x];
                let basis = sh_basis(direction);
                for (coefficient, weight) in coefficients.iter_mut().zip(basis) {
                    let scale = weight * solid_angle;
                    *coefficient = Color::rgb(
                        coefficient.r + radiance.r * scale,
                        coefficient.g + radiance.g * scale,
                        coefficient.b + radiance.b * scale,
                    );
                }
            }
        }
        coefficients
    }

    /// Cosine-weighted incoming light for a normal, already divided by π —
    /// multiply it by albedo to get diffuse ambient.
    pub fn irradiance(&self, normal: Vec3) -> Color {
        // Ramamoorthi & Hanrahan's convolution constants, over π.
        const A: [f32; 3] = [
            1.0,       // π / π
            2.0 / 3.0, // (2π/3) / π
            0.25,      // (π/4) / π
        ];
        let basis = sh_basis(normal.normalized());
        let band = [0usize, 1, 1, 1, 2, 2, 2, 2, 2];
        let mut result = Color::BLACK;
        for ((coefficient, weight), band) in self.sh.iter().zip(basis).zip(band) {
            let scale = weight * A[band];
            result = Color::rgb(
                result.r + coefficient.r * scale,
                result.g + coefficient.g * scale,
                result.b + coefficient.b * scale,
            );
        }
        Color::rgb(result.r.max(0.0), result.g.max(0.0), result.b.max(0.0))
    }

    /// Prefiltered radiance along `direction` for a surface of this roughness.
    pub fn sample(&self, direction: Vec3, roughness: f32) -> Color {
        let (u, v) = direction_to_uv(direction);
        let levels = self.mips.len() as f32 - 1.0;
        let level = (roughness.clamp(0.0, 1.0).sqrt() * levels).clamp(0.0, levels);
        let low = level.floor() as usize;
        let high = (low + 1).min(self.mips.len() - 1);
        let t = level - low as f32;
        self.mips[low]
            .sample(u, v)
            .lerp(self.mips[high].sample(u, v), t)
    }
}

/// The nine real spherical harmonic basis functions, evaluated for a direction.
#[inline]
fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_sky(radiance: f32) -> Sky {
        let c = Color::rgb(radiance, radiance, radiance);
        Sky::new(SkyParams {
            sun_intensity: 0.0,
            sun_irradiance: 0.0,
            zenith_color: c,
            horizon_color: c,
            ground_color: c,
            intensity: 1.0,
            ..SkyParams::default()
        })
    }

    #[test]
    fn equirect_mapping_round_trips() {
        for direction in [
            Vec3::Y,
            -Vec3::Y,
            Vec3::X,
            Vec3::new(0.3, -0.5, 0.8).normalized(),
            Vec3::new(-0.7, 0.2, -0.1).normalized(),
        ] {
            let (u, v) = direction_to_uv(direction);
            let back = uv_to_direction(u, v);
            assert!(
                (back - direction).length() < 1e-4,
                "{direction:?} -> {back:?}"
            );
        }
    }

    /// Under a uniform sky of radiance L, the cosine-weighted irradiance is
    /// exactly L from every direction. It catches any missing factor in the
    /// projection, the solid angle, or the convolution constants.
    #[test]
    fn a_uniform_sky_gives_uniform_irradiance() {
        let sky = uniform_sky(0.7);
        for normal in [
            Vec3::Y,
            -Vec3::Y,
            Vec3::X,
            Vec3::new(0.5, 0.5, 0.7).normalized(),
        ] {
            let e = sky.irradiance(normal);
            assert!((e.r - 0.7).abs() < 0.02, "normal {normal:?} gave {e:?}");
        }
    }

    #[test]
    fn irradiance_follows_the_bright_side_of_the_sky() {
        let sky = Sky::new(SkyParams::default());
        let up = sky.irradiance(Vec3::Y);
        let down = sky.irradiance(-Vec3::Y);
        assert!(
            up.luminance() > down.luminance() * 1.5,
            "{up:?} vs {down:?}"
        );
        // The sky is blue, so upward-facing surfaces pick up blue.
        assert!(up.b > up.r);
    }

    #[test]
    fn the_prefiltered_map_blurs_with_roughness() {
        let sky = Sky::new(SkyParams::default());
        let towards_sun = sky.params.sun_direction;
        let sharp = sky.sample(towards_sun, 0.0);
        let blurred = sky.sample(towards_sun, 1.0);
        assert!(
            sharp.luminance() > blurred.luminance(),
            "the glow should wash out"
        );

        // A rough sample of the sky is close to its average from any direction.
        let a = sky.sample(Vec3::new(1.0, 0.2, 0.0).normalized(), 1.0);
        let b = sky.sample(Vec3::new(-1.0, 0.2, 0.0).normalized(), 1.0);
        assert!(
            (a.luminance() - b.luminance()).abs() < 0.2,
            "{a:?} vs {b:?}"
        );
    }

    #[test]
    fn the_sun_disc_is_only_in_the_background_not_in_the_reflections() {
        let sky = Sky::new(SkyParams::default());
        let direction = sky.params.sun_direction;
        let with_disc = sky.radiance(direction);
        let dome_only = sky.dome_radiance(direction);
        assert!(
            with_disc.luminance() > dome_only.luminance() * 5.0,
            "the disc is bright"
        );
        // ...and the prefiltered map never reaches that brightness.
        assert!(sky.sample(direction, 0.0).luminance() < with_disc.luminance());
    }

    #[test]
    fn the_sun_becomes_a_directional_light_pointing_the_right_way() {
        let sky = Sky::new(SkyParams::default());
        let light = sky.sun_light().expect("the default sky has a sun");
        let (to_light, radiance) = light
            .sample(Vec3::ZERO)
            .expect("directional lights always hit");
        assert!((to_light - sky.params.sun_direction).length() < 1e-5);
        assert!(
            radiance.luminance() > 1.0,
            "midday sun is brighter than white"
        );

        assert!(Sky::new(SkyParams::overcast()).sun_light().is_none());
    }

    #[test]
    fn the_ground_is_darker_than_the_sky() {
        let sky = Sky::new(SkyParams::default());
        assert!(sky.radiance(-Vec3::Y).luminance() < sky.radiance(Vec3::Y).luminance());
    }
}
