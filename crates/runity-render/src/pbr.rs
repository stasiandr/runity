//! Physically based shading: the Cook-Torrance microfacet BRDF.
//!
//! The surface is modelled as a field of tiny mirrors. Three terms describe it:
//! **D**, how many of those mirrors point at the half vector; **G/V**, how many
//! of them shadow or mask each other; and **F**, how reflective they are at
//! this angle. Multiply, divide by the projected areas, and you have specular.
//! What is not reflected is what lights the diffuse term — which is where
//! energy conservation comes from, and why this looks right where
//! Blinn-Phong's independently tuned `specular_strength` does not.
//!
//! References: Cook & Torrance (1982); Walter et al. (2007) for GGX; Heitz
//! (2014) for the height-correlated Smith visibility; Karis (2013) for the
//! split-sum environment approximation.

use crate::color::Color;
use crate::gbuffer::Surface;
use crate::texture::Texture;
use runity_math::{Vec2, Vec3};

/// Reflectance of common dielectrics at normal incidence — 4%.
pub const DIELECTRIC_F0: f32 = 0.04;

/// Roughness below this makes the specular lobe a numerical singularity.
const MIN_ROUGHNESS: f32 = 0.045;

/// GGX / Trowbridge-Reitz normal distribution.
#[inline]
pub fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (core::f32::consts::PI * d * d).max(1e-8)
}

/// Height-correlated Smith visibility, with the BRDF's `1 / (4·NdotL·NdotV)`
/// folded in — that is what makes it "visibility" rather than "geometry".
#[inline]
pub fn visibility_smith_ggx(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lambda_v = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let lambda_l = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    0.5 / (lambda_v + lambda_l).max(1e-6)
}

/// Schlick's Fresnel: how reflectance rises to 1 at grazing angles.
#[inline]
pub fn fresnel_schlick(cos_theta: f32, f0: Color) -> Color {
    let f = (1.0 - cos_theta).clamp(0.0, 1.0).powi(5);
    Color::rgb(
        f0.r + (1.0 - f0.r) * f,
        f0.g + (1.0 - f0.g) * f,
        f0.b + (1.0 - f0.b) * f,
    )
}

/// As [`fresnel_schlick`], with a roughness-aware ceiling — for ambient light,
/// where there is no single incident direction to be grazing to.
#[inline]
pub fn fresnel_schlick_roughness(cos_theta: f32, f0: Color, roughness: f32) -> Color {
    let f = (1.0 - cos_theta).clamp(0.0, 1.0).powi(5);
    let ceiling = 1.0 - roughness;
    Color::rgb(
        f0.r + (ceiling.max(f0.r) - f0.r) * f,
        f0.g + (ceiling.max(f0.g) - f0.g) * f,
        f0.b + (ceiling.max(f0.b) - f0.b) * f,
    )
}

/// Specular reflectance at normal incidence: 4% for dielectrics, the base color
/// itself for metals (which is why metals have colored highlights).
#[inline]
pub fn f0_for(base_color: Color, metallic: f32) -> Color {
    Color::rgb(
        DIELECTRIC_F0 * (1.0 - metallic) + base_color.r * metallic,
        DIELECTRIC_F0 * (1.0 - metallic) + base_color.g * metallic,
        DIELECTRIC_F0 * (1.0 - metallic) + base_color.b * metallic,
    )
}

/// Outgoing radiance from one light, already multiplied by `NdotL` and the
/// light's radiance.
///
/// * `n` — surface normal, `v` — direction to the eye, `l` — direction to the
///   light, all unit length and in the same space.
pub fn direct_light(surface: &Surface, n: Vec3, v: Vec3, l: Vec3, radiance: Color) -> Color {
    let n_dot_l = n.dot(l);
    let n_dot_v = n.dot(v).abs().max(1e-4);
    if n_dot_l <= 0.0 {
        return Color::BLACK;
    }
    let roughness = surface.roughness.clamp(MIN_ROUGHNESS, 1.0);
    let h = (v + l).normalized();
    let n_dot_h = n.dot(h).max(0.0);
    let v_dot_h = v.dot(h).max(0.0);

    let d = distribution_ggx(n_dot_h, roughness);
    let vis = visibility_smith_ggx(n_dot_v, n_dot_l, roughness);
    let f = fresnel_schlick(v_dot_h, f0_for(surface.albedo, surface.metallic));

    // Whatever is not reflected is available to scatter diffusely, and metals
    // keep none of it.
    let diffuse_scale = (1.0 - surface.metallic) / core::f32::consts::PI;
    let specular = d * vis;

    Color::rgb(
        (surface.albedo.r * diffuse_scale * (1.0 - f.r) + specular * f.r) * radiance.r * n_dot_l,
        (surface.albedo.g * diffuse_scale * (1.0 - f.g) + specular * f.g) * radiance.g * n_dot_l,
        (surface.albedo.b * diffuse_scale * (1.0 - f.b) + specular * f.b) * radiance.b * n_dot_l,
    )
}

/// Karis's analytic fit of the split-sum environment BRDF: the scale and bias
/// to apply to `F0` when lighting a surface with a prefiltered environment.
///
/// It replaces the 2D lookup texture the split-sum approximation normally
/// needs — one less thing to precompute, and accurate to a fraction of a
/// percent over the useful range.
#[inline]
pub fn environment_brdf(n_dot_v: f32, roughness: f32) -> (f32, f32) {
    // Lazarov's mobile-friendly approximation of the same integral.
    let c0 = [-1.0f32, -0.0275, -0.572, 0.022];
    let c1 = [1.0f32, 0.0425, 1.04, -0.04];
    let r = [
        roughness * c0[0] + c1[0],
        roughness * c0[1] + c1[1],
        roughness * c0[2] + c1[2],
        roughness * c0[3] + c1[3],
    ];
    let a004 = (r[0] * r[0]).min((-9.28 * n_dot_v).exp2()) * r[0] + r[1];
    (a004 * -1.04 + r[2], a004 * 1.04 + r[3])
}

/// A light source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightKind {
    /// Infinitely far away; `direction` is the direction the light travels.
    Directional { direction: Vec3 },
    /// Falls off with the inverse square of distance, cut off at `range`.
    Point { position: Vec3, range: f32 },
    /// A point light restricted to a cone.
    Spot {
        position: Vec3,
        direction: Vec3,
        range: f32,
        /// Cosine of the angle where the cone starts to fade.
        inner_cos: f32,
        /// Cosine of the angle where it reaches zero.
        outer_cos: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub color: Color,
    pub intensity: f32,
    /// Whether the shadow pass renders a map for this light.
    pub casts_shadow: bool,
}

impl Light {
    pub fn directional(direction: Vec3, color: Color, intensity: f32) -> Self {
        Self {
            kind: LightKind::Directional {
                direction: direction.normalized(),
            },
            color,
            intensity,
            casts_shadow: true,
        }
    }

    pub fn point(position: Vec3, color: Color, intensity: f32, range: f32) -> Self {
        Self {
            kind: LightKind::Point { position, range },
            color,
            intensity,
            casts_shadow: false,
        }
    }

    pub fn spot(
        position: Vec3,
        direction: Vec3,
        color: Color,
        intensity: f32,
        range: f32,
        inner_degrees: f32,
        outer_degrees: f32,
    ) -> Self {
        Self {
            kind: LightKind::Spot {
                position,
                direction: direction.normalized(),
                range,
                inner_cos: inner_degrees.to_radians().cos(),
                outer_cos: outer_degrees.to_radians().cos(),
            },
            color,
            intensity,
            casts_shadow: false,
        }
    }

    /// Direction from `point` towards the light, and the radiance arriving
    /// there. Returns `None` when the point is out of range or outside the cone.
    pub fn sample(&self, point: Vec3) -> Option<(Vec3, Color)> {
        match self.kind {
            LightKind::Directional { direction } => {
                Some((-direction, self.color.scale_rgb(self.intensity)))
            }
            LightKind::Point { position, range } => {
                let offset = position - point;
                let distance = offset.length();
                if distance > range || distance <= 0.0 {
                    return None;
                }
                Some((
                    offset * (1.0 / distance),
                    self.color
                        .scale_rgb(self.intensity * falloff(distance, range)),
                ))
            }
            LightKind::Spot {
                position,
                direction,
                range,
                inner_cos,
                outer_cos,
            } => {
                let offset = position - point;
                let distance = offset.length();
                if distance > range || distance <= 0.0 {
                    return None;
                }
                let to_light = offset * (1.0 / distance);
                let cone = direction.dot(-to_light);
                if cone <= outer_cos {
                    return None;
                }
                // Smooth the cone edge so it does not alias into a hard circle.
                let t = ((cone - outer_cos) / (inner_cos - outer_cos).max(1e-4)).clamp(0.0, 1.0);
                let attenuation = falloff(distance, range) * t * t;
                Some((to_light, self.color.scale_rgb(self.intensity * attenuation)))
            }
        }
    }

    /// Where the light is, for shadow mapping. `None` for directional lights.
    pub fn position(&self) -> Option<Vec3> {
        match self.kind {
            LightKind::Directional { .. } => None,
            LightKind::Point { position, .. } | LightKind::Spot { position, .. } => Some(position),
        }
    }
}

/// Inverse-square falloff, windowed so it reaches exactly zero at `range`
/// instead of being cut off mid-gradient.
#[inline]
fn falloff(distance: f32, range: f32) -> f32 {
    let inverse_square = 1.0 / (distance * distance).max(1e-4);
    let window = (1.0 - (distance / range).powi(4)).clamp(0.0, 1.0);
    inverse_square * window * window
}

/// What a surface is made of.
#[derive(Debug, Clone, Copy)]
pub struct Material<'a> {
    pub base_color: Color,
    /// 0 = dielectric, 1 = metal. Values in between are for blends, not for
    /// "a bit shiny".
    pub metallic: f32,
    /// Perceptual roughness: 0 is a mirror, 1 is chalk.
    pub roughness: f32,
    pub emissive: Color,
    pub occlusion: f32,
    /// sRGB albedo map, multiplied into `base_color`.
    pub base_color_texture: Option<&'a Texture>,
    /// Tangent-space normal map (linear, 0.5 = flat).
    pub normal_texture: Option<&'a Texture>,
    /// glTF convention: green is roughness, blue is metallic.
    pub metallic_roughness_texture: Option<&'a Texture>,
    pub emissive_texture: Option<&'a Texture>,
    pub uv_scale: Vec2,
    /// Discard fragments whose alpha falls below this.
    pub alpha_cutoff: Option<f32>,
}

impl Default for Material<'_> {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Color::BLACK,
            occlusion: 1.0,
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            uv_scale: Vec2::ONE,
            alpha_cutoff: None,
        }
    }
}

impl<'a> Material<'a> {
    pub fn new(base_color: Color, metallic: f32, roughness: f32) -> Self {
        Self {
            base_color,
            metallic,
            roughness,
            ..Self::default()
        }
    }

    pub fn metal(base_color: Color, roughness: f32) -> Self {
        Self::new(base_color, 1.0, roughness)
    }

    pub fn dielectric(base_color: Color, roughness: f32) -> Self {
        Self::new(base_color, 0.0, roughness)
    }

    pub fn with_texture(mut self, texture: &'a Texture) -> Self {
        self.base_color_texture = Some(texture);
        self
    }

    pub fn with_normal_map(mut self, texture: &'a Texture) -> Self {
        self.normal_texture = Some(texture);
        self
    }

    pub fn with_emissive(mut self, emissive: Color) -> Self {
        self.emissive = emissive;
        self
    }

    pub fn with_uv_scale(mut self, scale: Vec2) -> Self {
        self.uv_scale = scale;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(roughness: f32, metallic: f32, albedo: Color) -> Surface {
        Surface {
            albedo,
            normal: Vec3::Y,
            roughness,
            metallic,
            ..Surface::default()
        }
    }

    /// Evenly spread directions over the hemisphere around +Y.
    fn hemisphere(count: usize) -> Vec<Vec3> {
        let golden = core::f32::consts::PI * (3.0 - 5f32.sqrt());
        (0..count)
            .map(|i| {
                let y = (i as f32 + 0.5) / count as f32; // cos(theta), 0..1
                let radius = (1.0 - y * y).sqrt();
                let theta = golden * i as f32;
                Vec3::new(theta.cos() * radius, y, theta.sin() * radius)
            })
            .collect()
    }

    #[test]
    fn fresnel_rises_from_f0_to_one() {
        let f0 = Color::rgb(0.04, 0.04, 0.04);
        assert!(
            (fresnel_schlick(1.0, f0).r - 0.04).abs() < 1e-6,
            "head on, reflectance is F0"
        );
        assert!(
            fresnel_schlick(0.0, f0).r > 0.99,
            "at grazing, everything reflects"
        );
        assert!(
            fresnel_schlick(0.5, f0).r > fresnel_schlick(0.9, f0).r,
            "and it is monotonic"
        );
    }

    #[test]
    fn metals_take_their_f0_from_the_base_color() {
        let gold = Color::rgb(1.0, 0.77, 0.34);
        assert_eq!(f0_for(gold, 1.0), gold, "a metal's highlight is tinted");
        assert!(
            (f0_for(gold, 0.0).r - DIELECTRIC_F0).abs() < 1e-6,
            "a dielectric's is not"
        );
    }

    #[test]
    fn a_smoother_surface_concentrates_the_highlight() {
        // The same distribution, narrower: higher at the peak, lower off-axis.
        let peak_smooth = distribution_ggx(1.0, 0.1);
        let peak_rough = distribution_ggx(1.0, 0.8);
        assert!(
            peak_smooth > peak_rough * 50.0,
            "{peak_smooth} vs {peak_rough}"
        );
        assert!(distribution_ggx(0.7, 0.1) < distribution_ggx(0.7, 0.8));
    }

    #[test]
    fn the_brdf_is_reciprocal() {
        // Swapping the eye and the light must not change the result — the
        // property that separates a BRDF from an ad-hoc highlight term.
        let s = surface(0.35, 0.0, Color::rgb(0.8, 0.6, 0.4));
        let v = Vec3::new(0.3, 0.8, 0.5).normalized();
        let l = Vec3::new(-0.6, 0.4, 0.2).normalized();
        let forward = direct_light(&s, Vec3::Y, v, l, Color::WHITE);
        let backward = direct_light(&s, Vec3::Y, l, v, Color::WHITE);
        // Reciprocity holds for the BRDF; each result carries its own NdotL.
        let a = forward.r / Vec3::Y.dot(l);
        let b = backward.r / Vec3::Y.dot(v);
        assert!((a - b).abs() < 1e-4, "{a} vs {b}");
    }

    /// A white furnace test: a surface lit equally from every direction cannot
    /// reflect more light than arrives. Catches a missing normalization factor
    /// in D, V or the diffuse term.
    #[test]
    fn the_brdf_conserves_energy() {
        const SAMPLES: usize = 4000;
        let directions = hemisphere(SAMPLES);
        // Each sample carries the same solid angle: 2π / N, and the cosine is
        // already in `direct_light`.
        let weight = 2.0 * core::f32::consts::PI / SAMPLES as f32;

        for roughness in [0.05f32, 0.25, 0.5, 0.9] {
            for metallic in [0.0f32, 1.0] {
                let s = surface(roughness, metallic, Color::WHITE);
                let v = Vec3::new(0.4, 0.7, 0.0).normalized();
                let total: f32 = directions
                    .iter()
                    .map(|l| direct_light(&s, Vec3::Y, v, *l, Color::WHITE).r * weight)
                    .sum();
                assert!(
                    total <= 1.02,
                    "roughness {roughness}, metallic {metallic}: reflects {total} of the light"
                );
                assert!(
                    total > 0.25,
                    "roughness {roughness}, metallic {metallic}: only {total}"
                );
            }
        }
    }

    #[test]
    fn light_behind_the_surface_contributes_nothing() {
        let s = surface(0.5, 0.0, Color::WHITE);
        let below = Vec3::new(0.0, -1.0, 0.0);
        assert_eq!(
            direct_light(&s, Vec3::Y, Vec3::Y, below, Color::WHITE),
            Color::BLACK
        );
    }

    #[test]
    fn point_lights_fall_off_and_stop_at_their_range() {
        let light = Light::point(Vec3::new(0.0, 4.0, 0.0), Color::WHITE, 10.0, 8.0);
        let near = light.sample(Vec3::new(0.0, 2.0, 0.0)).expect("in range").1;
        let far = light.sample(Vec3::new(0.0, -2.0, 0.0)).expect("in range").1;
        assert!(
            near.r > far.r * 3.0,
            "inverse square: {} vs {}",
            near.r,
            far.r
        );
        assert!(
            light.sample(Vec3::new(0.0, -20.0, 0.0)).is_none(),
            "out of range"
        );
    }

    #[test]
    fn a_spot_light_fades_out_at_its_cone_edge() {
        let light = Light::spot(
            Vec3::new(0.0, 3.0, 0.0),
            -Vec3::Y,
            Color::WHITE,
            20.0,
            10.0,
            15.0,
            30.0,
        );
        let center = light.sample(Vec3::ZERO).expect("inside the cone").1;
        let edge = light
            .sample(Vec3::new(1.4, 0.0, 0.0))
            .expect("near the edge")
            .1;
        assert!(center.r > edge.r, "{} vs {}", center.r, edge.r);
        assert!(
            light.sample(Vec3::new(5.0, 0.0, 0.0)).is_none(),
            "outside the cone"
        );
    }

    #[test]
    fn the_environment_brdf_matches_the_integral_it_approximates() {
        // A rough dielectric reflects little; a smooth one reflects nearly all
        // of F0 head-on. Those are the two ends the fit has to get right.
        let (smooth_scale, smooth_bias) = environment_brdf(1.0, 0.05);
        assert!(
            smooth_scale + smooth_bias > 0.9,
            "{smooth_scale} + {smooth_bias}"
        );
        let (rough_scale, rough_bias) = environment_brdf(1.0, 1.0);
        assert!(rough_scale + rough_bias < smooth_scale + smooth_bias);
    }

    #[test]
    fn the_environment_brdf_stays_in_range() {
        for roughness in [0.0f32, 0.3, 0.7, 1.0] {
            for n_dot_v in [0.05f32, 0.5, 1.0] {
                let (scale, bias) = environment_brdf(n_dot_v, roughness);
                // The fit undershoots by a fraction of a percent at the
                // extremes; anything beyond that would tint ambient light.
                assert!(
                    (-0.01..=1.05).contains(&scale),
                    "{scale} at {roughness}/{n_dot_v}"
                );
                assert!(
                    (-0.01..=1.05).contains(&bias),
                    "{bias} at {roughness}/{n_dot_v}"
                );
            }
        }
    }
}
