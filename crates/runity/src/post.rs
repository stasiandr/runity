//! Post-processing: URP's Volume, as numbers on a frame.
//!
//! The scene is drawn in high dynamic range, and what is here turns it into
//! a picture: bloom from whatever is brighter than white, exposure, white
//! balance, the colour grade, the tonemapper, and what a lens adds —
//! vignette, chromatic fringes, grain — then FXAA. DNA, postulate 7: one
//! render path, beautiful by default — tonemapping and antialiasing are on
//! without anyone asking, and every effect has URP's name and meaning, so
//! a Unity person reads a scene's `post` without a manual.
//!
//! A scene carries its own (`post: (...)` in the file); what is not said is
//! the default. Unlike URP there are no volumes blending by position yet —
//! one set per frame, which is what a scene or a camera wants; a game that
//! blends two (walking into a cave) lerps them itself ([`PostProcess::lerp`]).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// Which curve brings high dynamic range into what a screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Tonemapping {
    /// Clipped at white: what a frame looked like before.
    None,
    /// URP's Neutral: highlights compressed, hues and mid-tones kept.
    #[default]
    Neutral,
    /// URP's ACES: filmic, contrasty, highlights desaturating to white.
    Aces,
}

/// Glow around what is brighter than white: the sun on water, an ember, a
/// lit window at night. URP's Bloom.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bloom {
    /// How much of the glow is added; 0 is off.
    pub intensity: f32,
    /// How bright, in linear light, before something glows. 1.0 is white:
    /// only what is lit past white or emits does.
    pub threshold: f32,
    /// How far the glow spreads, 0 to 1.
    pub scatter: f32,
    pub tint: [f32; 3],
    /// The brightest a pixel counts as, so a single spark does not flood.
    pub clamp: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            intensity: 0.3,
            threshold: 1.0,
            scatter: 0.7,
            tint: [1.0, 1.0, 1.0],
            clamp: 65_000.0,
        }
    }
}

/// Darker towards the corners. URP's Vignette.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    /// 0 is off, 1 is heavy.
    pub intensity: f32,
    /// How gradual the edge is, 0 to 1.
    pub smoothness: f32,
    pub color: [f32; 3],
    pub center: [f32; 2],
}

impl Default for Vignette {
    /// Off, as in URP: a vignette is a look someone chooses, not a default
    /// that quietly darkens every corner of every frame.
    fn default() -> Self {
        Self {
            intensity: 0.0,
            smoothness: 0.4,
            color: [0.0, 0.0, 0.0],
            center: [0.5, 0.5],
        }
    }
}

/// Each output channel as a mix of the inputs: URP's Channel Mixer, as
/// shares rather than percent (1.0 is URP's 100).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelMixer {
    pub red: [f32; 3],
    pub green: [f32; 3],
    pub blue: [f32; 3],
}

impl Default for ChannelMixer {
    fn default() -> Self {
        Self {
            red: [1.0, 0.0, 0.0],
            green: [0.0, 1.0, 0.0],
            blue: [0.0, 0.0, 1.0],
        }
    }
}

/// The three trackballs of a grade: `lift` raises the darks (added,
/// fading to nothing at white), `gamma` bends the middle (a power), `gain`
/// scales the brights. URP's Lift Gamma Gain.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiftGammaGain {
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
}

impl Default for LiftGammaGain {
    fn default() -> Self {
        Self {
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
        }
    }
}

/// A colour for the darks, one for the middle and one for the lights, each
/// multiplied in where the picture is that bright. URP's Shadows Midtones
/// Highlights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadowsMidtonesHighlights {
    pub shadows: [f32; 3],
    pub midtones: [f32; 3],
    pub highlights: [f32; 3],
    /// Where the shadows end: fully shadows below the first, none above
    /// the second, by luminance.
    pub shadows_range: [f32; 2],
    /// Where the highlights begin.
    pub highlights_range: [f32; 2],
}

impl Default for ShadowsMidtonesHighlights {
    fn default() -> Self {
        Self {
            shadows: [1.0; 3],
            midtones: [1.0; 3],
            highlights: [1.0; 3],
            shadows_range: [0.0, 0.3],
            highlights_range: [0.55, 1.0],
        }
    }
}

/// One tint in the darks and another in the lights, soft-lit in: the
/// teal-and-orange grade. URP's Split Toning; grey (0.5) is no tint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitToning {
    pub shadows: [f32; 3],
    pub highlights: [f32; 3],
    /// -100 (all highlights) to 100 (all shadows).
    pub balance: f32,
}

impl Default for SplitToning {
    fn default() -> Self {
        Self {
            shadows: [0.5; 3],
            highlights: [0.5; 3],
            balance: 0.0,
        }
    }
}

/// A cheap lens bending the picture: barrel (positive) or pincushion
/// (negative). URP's Lens Distortion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensDistortion {
    /// -1 to 1; 0 is off.
    pub intensity: f32,
    /// How much of it along x and along y, 0 to 1.
    pub x_multiplier: f32,
    pub y_multiplier: f32,
    /// Where the bend is centred, 0 to 1 across the screen.
    pub center: [f32; 2],
    /// Zooms in (above 1) to hide the edges the bend pulls in.
    pub scale: f32,
}

impl LensDistortion {
    pub const OFF: LensDistortion = LensDistortion {
        intensity: 0.0,
        x_multiplier: 1.0,
        y_multiplier: 1.0,
        center: [0.5, 0.5],
        scale: 1.0,
    };
}

impl Default for LensDistortion {
    fn default() -> Self {
        Self::OFF
    }
}

/// A wide view kept from stretching at its sides: a cylinder's
/// projection, as a painter's. URP's Panini Projection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaniniProjection {
    /// 0 (plain perspective) to 1 (a full cylinder).
    pub distance: f32,
    /// 0 to 1: how much to zoom so no edge is left empty.
    pub crop_to_fit: f32,
}

impl PaniniProjection {
    pub const OFF: PaniniProjection = PaniniProjection {
        distance: 0.0,
        crop_to_fit: 1.0,
    };
}

impl Default for PaniniProjection {
    fn default() -> Self {
        Self::OFF
    }
}

/// Ghosts of what is bright, mirrored through the middle of the frame, a
/// halo and a streak — what light scattered inside a lens makes. URP's
/// Screen Space Lens Flare: it is made from the bloom, so bloom must be on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensFlare {
    /// 0 is off.
    pub intensity: f32,
    pub tint: [f32; 3],
    /// The ghosts mirrored through the middle.
    pub ghosts: f32,
    /// The ring around the middle.
    pub halo: f32,
    /// The horizontal streak through each bright thing (anamorphic).
    pub streaks: f32,
    /// Red and blue apart in the ghosts, 0 to 1.
    pub chromatic_aberration: f32,
}

impl LensFlare {
    pub const OFF: LensFlare = LensFlare {
        intensity: 0.0,
        tint: [1.0, 1.0, 1.0],
        ghosts: 1.0,
        halo: 0.2,
        streaks: 0.5,
        chromatic_aberration: 0.5,
    };
}

impl Default for LensFlare {
    fn default() -> Self {
        Self::OFF
    }
}

/// Everything done to a frame after it is drawn, with URP's names and
/// ranges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcess {
    /// Off: the frame as drawn, clipped at white, no antialiasing pass.
    pub enabled: bool,
    /// Post exposure, in stops (EV): +1 is twice as bright.
    pub exposure: f32,
    pub tonemapping: Tonemapping,
    pub bloom: Bloom,
    /// Colour Adjustments: -100 to 100, as URP.
    pub contrast: f32,
    /// -100 (grey) to 100. `07-look.md`'s tired player is a negative one.
    pub saturation: f32,
    /// Degrees, -180 to 180.
    pub hue_shift: f32,
    /// Multiplied into every pixel, linear.
    pub color_filter: [f32; 3],
    /// White Balance: -100 (cool) to 100 (warm), and -100 (green) to 100
    /// (magenta).
    pub temperature: f32,
    pub tint: f32,
    pub vignette: Vignette,
    pub channel_mixer: ChannelMixer,
    pub lift_gamma_gain: LiftGammaGain,
    pub shadows_midtones_highlights: ShadowsMidtonesHighlights,
    pub split_toning: SplitToning,
    /// Red and blue apart at the edges, 0 to 1.
    pub chromatic_aberration: f32,
    /// Film grain, 0 to 1.
    pub film_grain: f32,
    /// Fast approximate antialiasing on the finished picture, over the
    /// multisampling the scene is already drawn with: it catches what MSAA
    /// cannot — edges inside a texture, the alpha-cut leaf.
    pub fxaa: bool,
    /// A little noise, below a step of the screen's precision, so a gentle
    /// gradient — a sky, a lit wall — does not show its steps as bands.
    /// URP's camera Dithering.
    pub dithering: bool,
    /// Depth of Field, off unless asked ([`crate::lens`]).
    pub depth_of_field: crate::lens::DepthOfField,
    /// Camera Motion Blur, off unless asked ([`crate::lens`]).
    pub motion_blur: crate::lens::MotionBlur,
    pub lens_distortion: LensDistortion,
    pub panini_projection: PaniniProjection,
    pub lens_flare: LensFlare,
    /// Shimmering hot air and the mirage ([`crate::lens::HeatHaze`]).
    pub heat_haze: crate::lens::HeatHaze,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            enabled: true,
            exposure: 0.0,
            tonemapping: Tonemapping::Neutral,
            bloom: Bloom::default(),
            contrast: 0.0,
            saturation: 0.0,
            hue_shift: 0.0,
            color_filter: [1.0, 1.0, 1.0],
            temperature: 0.0,
            tint: 0.0,
            vignette: Vignette::default(),
            channel_mixer: ChannelMixer::default(),
            lift_gamma_gain: LiftGammaGain::default(),
            shadows_midtones_highlights: ShadowsMidtonesHighlights::default(),
            split_toning: SplitToning::default(),
            chromatic_aberration: 0.0,
            film_grain: 0.0,
            fxaa: false,
            dithering: true,
            depth_of_field: crate::lens::DepthOfField::OFF,
            motion_blur: crate::lens::MotionBlur::OFF,
            lens_distortion: LensDistortion::OFF,
            panini_projection: PaniniProjection::OFF,
            lens_flare: LensFlare::OFF,
            heat_haze: crate::lens::HeatHaze::OFF,
        }
    }
}

impl PostProcess {
    /// Nothing done: the frame as drawn, clipped at white. What a test that
    /// counts exact colours wants.
    pub const OFF: PostProcess = PostProcess {
        enabled: false,
        exposure: 0.0,
        tonemapping: Tonemapping::None,
        bloom: Bloom {
            intensity: 0.0,
            threshold: 1.0,
            scatter: 0.7,
            tint: [1.0, 1.0, 1.0],
            clamp: 65_000.0,
        },
        contrast: 0.0,
        saturation: 0.0,
        hue_shift: 0.0,
        color_filter: [1.0, 1.0, 1.0],
        temperature: 0.0,
        tint: 0.0,
        vignette: Vignette {
            intensity: 0.0,
            smoothness: 0.4,
            color: [0.0, 0.0, 0.0],
            center: [0.5, 0.5],
        },
        channel_mixer: ChannelMixer {
            red: [1.0, 0.0, 0.0],
            green: [0.0, 1.0, 0.0],
            blue: [0.0, 0.0, 1.0],
        },
        lift_gamma_gain: LiftGammaGain {
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
        },
        shadows_midtones_highlights: ShadowsMidtonesHighlights {
            shadows: [1.0; 3],
            midtones: [1.0; 3],
            highlights: [1.0; 3],
            shadows_range: [0.0, 0.3],
            highlights_range: [0.55, 1.0],
        },
        split_toning: SplitToning {
            shadows: [0.5; 3],
            highlights: [0.5; 3],
            balance: 0.0,
        },
        chromatic_aberration: 0.0,
        film_grain: 0.0,
        fxaa: false,
        dithering: false,
        depth_of_field: crate::lens::DepthOfField::OFF,
        motion_blur: crate::lens::MotionBlur::OFF,
        lens_distortion: LensDistortion::OFF,
        panini_projection: PaniniProjection::OFF,
        lens_flare: LensFlare::OFF,
        heat_haze: crate::lens::HeatHaze::OFF,
    };

    /// Part way from `self` to `other`: `t` 0 is self, 1 is other. What a
    /// game blends when walking from one mood into another; the switches
    /// (tonemapper, FXAA) flip half way.
    pub fn lerp(&self, other: &PostProcess, t: f32) -> PostProcess {
        let t = t.clamp(0.0, 1.0);
        let f = |a: f32, b: f32| a + (b - a) * t;
        let v3 = |a: [f32; 3], b: [f32; 3]| [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])];
        let half = t >= 0.5;
        PostProcess {
            enabled: if half { other.enabled } else { self.enabled },
            exposure: f(self.exposure, other.exposure),
            tonemapping: if half {
                other.tonemapping
            } else {
                self.tonemapping
            },
            bloom: Bloom {
                intensity: f(self.bloom.intensity, other.bloom.intensity),
                threshold: f(self.bloom.threshold, other.bloom.threshold),
                scatter: f(self.bloom.scatter, other.bloom.scatter),
                tint: v3(self.bloom.tint, other.bloom.tint),
                clamp: f(self.bloom.clamp, other.bloom.clamp),
            },
            contrast: f(self.contrast, other.contrast),
            saturation: f(self.saturation, other.saturation),
            hue_shift: f(self.hue_shift, other.hue_shift),
            color_filter: v3(self.color_filter, other.color_filter),
            temperature: f(self.temperature, other.temperature),
            tint: f(self.tint, other.tint),
            vignette: Vignette {
                intensity: f(self.vignette.intensity, other.vignette.intensity),
                smoothness: f(self.vignette.smoothness, other.vignette.smoothness),
                color: v3(self.vignette.color, other.vignette.color),
                center: [
                    f(self.vignette.center[0], other.vignette.center[0]),
                    f(self.vignette.center[1], other.vignette.center[1]),
                ],
            },
            channel_mixer: ChannelMixer {
                red: v3(self.channel_mixer.red, other.channel_mixer.red),
                green: v3(self.channel_mixer.green, other.channel_mixer.green),
                blue: v3(self.channel_mixer.blue, other.channel_mixer.blue),
            },
            lift_gamma_gain: LiftGammaGain {
                lift: v3(self.lift_gamma_gain.lift, other.lift_gamma_gain.lift),
                gamma: v3(self.lift_gamma_gain.gamma, other.lift_gamma_gain.gamma),
                gain: v3(self.lift_gamma_gain.gain, other.lift_gamma_gain.gain),
            },
            shadows_midtones_highlights: {
                let (a, b) = (
                    &self.shadows_midtones_highlights,
                    &other.shadows_midtones_highlights,
                );
                ShadowsMidtonesHighlights {
                    shadows: v3(a.shadows, b.shadows),
                    midtones: v3(a.midtones, b.midtones),
                    highlights: v3(a.highlights, b.highlights),
                    shadows_range: [
                        f(a.shadows_range[0], b.shadows_range[0]),
                        f(a.shadows_range[1], b.shadows_range[1]),
                    ],
                    highlights_range: [
                        f(a.highlights_range[0], b.highlights_range[0]),
                        f(a.highlights_range[1], b.highlights_range[1]),
                    ],
                }
            },
            split_toning: SplitToning {
                shadows: v3(self.split_toning.shadows, other.split_toning.shadows),
                highlights: v3(self.split_toning.highlights, other.split_toning.highlights),
                balance: f(self.split_toning.balance, other.split_toning.balance),
            },
            chromatic_aberration: f(self.chromatic_aberration, other.chromatic_aberration),
            film_grain: f(self.film_grain, other.film_grain),
            fxaa: if half { other.fxaa } else { self.fxaa },
            dithering: if half {
                other.dithering
            } else {
                self.dithering
            },
            depth_of_field: self.depth_of_field.lerp(&other.depth_of_field, t),
            motion_blur: self.motion_blur.lerp(&other.motion_blur, t),
            lens_distortion: LensDistortion {
                intensity: f(
                    self.lens_distortion.intensity,
                    other.lens_distortion.intensity,
                ),
                x_multiplier: f(
                    self.lens_distortion.x_multiplier,
                    other.lens_distortion.x_multiplier,
                ),
                y_multiplier: f(
                    self.lens_distortion.y_multiplier,
                    other.lens_distortion.y_multiplier,
                ),
                center: [
                    f(
                        self.lens_distortion.center[0],
                        other.lens_distortion.center[0],
                    ),
                    f(
                        self.lens_distortion.center[1],
                        other.lens_distortion.center[1],
                    ),
                ],
                scale: f(self.lens_distortion.scale, other.lens_distortion.scale),
            },
            panini_projection: PaniniProjection {
                distance: f(
                    self.panini_projection.distance,
                    other.panini_projection.distance,
                ),
                crop_to_fit: f(
                    self.panini_projection.crop_to_fit,
                    other.panini_projection.crop_to_fit,
                ),
            },
            heat_haze: self.heat_haze.lerp(&other.heat_haze, t),
            lens_flare: LensFlare {
                intensity: f(self.lens_flare.intensity, other.lens_flare.intensity),
                tint: v3(self.lens_flare.tint, other.lens_flare.tint),
                ghosts: f(self.lens_flare.ghosts, other.lens_flare.ghosts),
                halo: f(self.lens_flare.halo, other.lens_flare.halo),
                streaks: f(self.lens_flare.streaks, other.lens_flare.streaks),
                chromatic_aberration: f(
                    self.lens_flare.chromatic_aberration,
                    other.lens_flare.chromatic_aberration,
                ),
            },
        }
    }
}

/// White balance as LMS scales, the way Unity's `ColorUtils` works it out
/// from temperature and tint: shift the white point along the Planckian
/// locus, and scale each cone response to bring it back to D65.
pub fn white_balance_coefficients(temperature: f32, tint: f32) -> Vec3 {
    let t1 = temperature / 65.0;
    let t2 = tint / 65.0;
    let x = 0.31271 - t1 * if t1 < 0.0 { 0.1 } else { 0.05 };
    let standard_y = 2.87 * x - 3.0 * x * x - 0.275_095_07;
    let y = standard_y + t2 * 0.05;
    let lms = |x: f32, y: f32| {
        let big_y = 1.0;
        let big_x = big_y * x / y;
        let big_z = big_y * (1.0 - x - y) / y;
        Vec3::new(
            0.7328 * big_x + 0.4296 * big_y - 0.1624 * big_z,
            -0.7036 * big_x + 1.6975 * big_y + 0.0061 * big_z,
            0.0030 * big_x + 0.0136 * big_y + 0.9834 * big_z,
        )
    };
    let w1 = Vec3::new(0.949_237, 1.035_42, 1.087_28);
    w1 / lms(x, y)
}

/// URP's Panini constants: the view's extents at unit distance, the
/// distance, and how far to zoom so the edges are covered. Nothing for an
/// orthographic camera (`fov` 0), which has no perspective to unbend.
fn panini_params(p: &PaniniProjection, fov_y_degrees: f32, aspect: f32) -> [f32; 4] {
    if p.distance <= 0.0 || fov_y_degrees <= 0.0 {
        return [1.0, 1.0, 0.0, 1.0];
    }
    let d = p.distance.clamp(0.0, 1.0);
    let half = (fov_y_degrees.to_radians() * 0.5).tan();
    let view = [half * aspect, half];
    // Where the view's corner lands on the cylinder.
    let view_distance = 1.0 + d;
    let hypotenuse = (view[0] * view[0] + 1.0).sqrt();
    let cylinder_minus_d = 1.0 / hypotenuse;
    let cylinder = cylinder_minus_d + d;
    let crop = [
        view[0] * cylinder_minus_d * (view_distance / cylinder),
        view[1] * cylinder_minus_d * (view_distance / cylinder),
    ];
    let scale = (crop[0] / view[0]).min(crop[1] / view[1]);
    let zoom = 1.0 + (scale.clamp(0.0, 1.0) - 1.0) * p.crop_to_fit.clamp(0.0, 1.0);
    [view[0], view[1], d, zoom]
}

/// The post shader, as compiled in.
pub const SHADER: &str = include_str!("post.wgsl");

/// The high-dynamic-range format the scene is drawn in.
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    a: [f32; 4],
    bloom_tint: [f32; 4],
    filter_contrast: [f32; 4],
    b: [f32; 4],
    white_balance: [f32; 4],
    vignette_color: [f32; 4],
    vignette: [f32; 4],
    texel: [f32; 4],
    bloom: [f32; 4],
    mixer_red: [f32; 4],
    mixer_green: [f32; 4],
    mixer_blue: [f32; 4],
    /// Shadows' colour; `w` where the shadows start fading.
    smh_shadows: [f32; 4],
    /// Midtones' colour; `w` where the shadows are gone.
    smh_midtones: [f32; 4],
    /// Highlights' colour; `w` where the highlights start.
    smh_highlights: [f32; 4],
    /// Lift; `w` where the highlights are whole.
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    /// Split toning's shadows tint; `w` the balance, -1 to 1.
    split_shadows: [f32; 4],
    split_highlights: [f32; 4],
    /// Lens Distortion: centre (−1..1), x and y amounts.
    distortion_axis: [f32; 4],
    /// Lens Distortion: theta (or its inverse), sigma, 1/scale, intensity.
    distortion: [f32; 4],
    /// Panini: the view's half extents (tangents), distance, crop scale.
    panini: [f32; 4],
    /// Lens flare: tint, intensity.
    flare_tint: [f32; 4],
    /// Lens flare: ghosts, halo, streaks, chromatic aberration.
    flare: [f32; 4],
}

/// One uniform slot per pass of a frame, at the device's alignment.
const SLOTS: u64 = 32;
/// Steps down the bloom chain at most.
const BLOOM_LEVELS: usize = 6;

/// A texture and a view of it.
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

fn target(
    gpu: &Gpu,
    label: &str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
) -> Target {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Target {
        _texture: texture,
        view,
        size: (width.max(1), height.max(1)),
    }
}

/// The GPU half: pipelines made once, targets remade when the size changes.
pub(crate) struct PostRenderer {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    stride: u64,
    sampler: wgpu::Sampler,
    prefilter: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// The composite into the intermediate that FXAA reads.
    composite_hdr: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
    /// The bloom chain, half size down.
    levels: Vec<Target>,
    /// Where the finished picture waits for FXAA.
    graded: Option<Target>,
    /// One black texel: the bloom a frame without bloom adds.
    black: Target,
    size: (u32, u32),
    /// Whether the output format encodes sRGB itself.
    output_srgb: bool,
    started: std::time::Instant,
    /// The camera's vertical field of view, 0 for an orthographic one:
    /// what Panini unbends.
    pub(crate) fov_y_degrees: f32,
}

impl PostRenderer {
    pub(crate) fn new(gpu: &Gpu, output: wgpu::TextureFormat) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::post"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let alignment = gpu
            .device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(1) as u64;
        let size = std::mem::size_of::<PostUniform>() as u64;
        let stride = size.div_ceil(alignment) * alignment;
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post"),
            size: stride * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("post"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(size),
                        },
                        count: None,
                    },
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    texture_entry(3),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::post"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = |entry: &str, format: wgpu::TextureFormat, add: bool| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_fullscreen"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: add.then_some(wgpu::BlendState {
                                color: wgpu::BlendComponent {
                                    src_factor: wgpu::BlendFactor::One,
                                    dst_factor: wgpu::BlendFactor::One,
                                    operation: wgpu::BlendOperation::Add,
                                },
                                alpha: wgpu::BlendComponent::REPLACE,
                            }),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let black = target(gpu, "no bloom", (1, 1), HDR_FORMAT);
        Self {
            prefilter: pipeline("fs_prefilter", HDR_FORMAT, false),
            downsample: pipeline("fs_downsample", HDR_FORMAT, false),
            upsample: pipeline("fs_upsample", HDR_FORMAT, true),
            composite: pipeline("fs_composite", output, false),
            composite_hdr: pipeline("fs_composite", HDR_FORMAT, false),
            fxaa: pipeline("fs_fxaa", output, false),
            layout,
            uniforms,
            stride,
            sampler,
            levels: Vec::new(),
            graded: None,
            black,
            size: (0, 0),
            output_srgb: output.is_srgb(),
            started: std::time::Instant::now(),
            fov_y_degrees: 0.0,
        }
    }

    fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.size == size {
            return;
        }
        self.size = size;
        self.levels.clear();
        let (mut w, mut h) = (size.0 / 2, size.1 / 2);
        while self.levels.len() < BLOOM_LEVELS && w >= 2 && h >= 2 {
            self.levels.push(target(gpu, "bloom", (w, h), HDR_FORMAT));
            w /= 2;
            h /= 2;
        }
        self.graded = Some(target(gpu, "graded", size, HDR_FORMAT));
    }

    fn group(
        &self,
        gpu: &Gpu,
        source: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<PostUniform>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
            ],
        })
    }

    /// Turn the high-dynamic-range `scene` into the picture in `output`.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView,
        output: &wgpu::TextureView,
        size: (u32, u32),
        settings: &PostProcess,
    ) {
        self.resize(gpu, size);
        let settings = if settings.enabled {
            *settings
        } else {
            PostProcess::OFF
        };
        let base = self.uniform(&settings, size);
        let mut slots: Vec<PostUniform> = Vec::new();
        let texel = |(w, h): (u32, u32)| [1.0 / w.max(1) as f32, 1.0 / h.max(1) as f32, 0.0, 0.0];

        // Bloom: down the chain from the scene, then back up adding.
        let bloom_on = settings.bloom.intensity > 0.0 && !self.levels.is_empty();
        let mut passes: Vec<(
            &wgpu::RenderPipeline,
            wgpu::BindGroup,
            usize,
            &wgpu::TextureView,
            bool,
        )> = Vec::new();
        if bloom_on {
            for i in 0..self.levels.len() {
                let (source, pipeline, source_size) = if i == 0 {
                    (scene, &self.prefilter, size)
                } else {
                    (
                        &self.levels[i - 1].view,
                        &self.downsample,
                        self.levels[i - 1].size,
                    )
                };
                slots.push(PostUniform {
                    texel: texel(source_size),
                    ..base
                });
                let group = self.group(gpu, source, &self.black.view);
                passes.push((pipeline, group, slots.len() - 1, &self.levels[i].view, true));
            }
            for i in (0..self.levels.len() - 1).rev() {
                slots.push(PostUniform {
                    texel: texel(self.levels[i + 1].size),
                    ..base
                });
                let group = self.group(gpu, &self.levels[i + 1].view, &self.black.view);
                passes.push((
                    &self.upsample,
                    group,
                    slots.len() - 1,
                    &self.levels[i].view,
                    false,
                ));
            }
        }
        let bloom = if bloom_on {
            &self.levels[0].view
        } else {
            &self.black.view
        };
        let graded = self.graded.as_ref().expect("sized above");
        let (composite_target, composite_pipeline) = if settings.fxaa {
            (&graded.view, &self.composite_hdr)
        } else {
            (output, &self.composite)
        };
        let mut composite = PostUniform {
            texel: texel(size),
            ..base
        };
        if settings.fxaa {
            // The intermediate is a float target: left linear, undithered —
            // FXAA finishes the picture on its way out. (`finish` with
            // neither flag is the identity, give or take a rounding.)
            composite.a[3] = 0.0;
            composite.white_balance[3] = 0.0;
        }
        slots.push(composite);
        passes.push((
            composite_pipeline,
            self.group(gpu, scene, bloom),
            slots.len() - 1,
            composite_target,
            true,
        ));
        if settings.fxaa {
            slots.push(PostUniform {
                texel: texel(size),
                ..base
            });
            passes.push((
                &self.fxaa,
                self.group(gpu, &graded.view, &self.black.view),
                slots.len() - 1,
                output,
                true,
            ));
        }

        let mut bytes = vec![0u8; (self.stride * slots.len() as u64) as usize];
        for (i, slot) in slots.iter().enumerate() {
            let at = i * self.stride as usize;
            bytes[at..at + std::mem::size_of::<PostUniform>()]
                .copy_from_slice(bytemuck::bytes_of(slot));
        }
        gpu.queue.write_buffer(&self.uniforms, 0, &bytes);

        for (pipeline, group, slot, target, clear) in passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[(slot as u64 * self.stride) as u32]);
            pass.draw(0..3, 0..1);
        }
    }

    fn uniform(&self, s: &PostProcess, (w, h): (u32, u32)) -> PostUniform {
        let v4 = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        let smh = &s.shadows_midtones_highlights;
        let tonemapper = match s.tonemapping {
            Tonemapping::None => 0.0,
            Tonemapping::Neutral => 1.0,
            Tonemapping::Aces => 2.0,
        };
        let balance = white_balance_coefficients(s.temperature, s.tint);
        let knee = s.bloom.threshold * 0.5;
        PostUniform {
            a: [
                2f32.powf(s.exposure),
                s.bloom.intensity.max(0.0),
                tonemapper,
                if self.output_srgb { 0.0 } else { 1.0 },
            ],
            bloom_tint: [
                s.bloom.tint[0],
                s.bloom.tint[1],
                s.bloom.tint[2],
                s.film_grain.clamp(0.0, 1.0),
            ],
            filter_contrast: [
                s.color_filter[0],
                s.color_filter[1],
                s.color_filter[2],
                (1.0 + s.contrast / 100.0).max(0.0),
            ],
            b: [
                (1.0 + s.saturation / 100.0).max(0.0),
                s.hue_shift / 360.0,
                s.chromatic_aberration.clamp(0.0, 1.0),
                (self.started.elapsed().as_secs_f32() * 24.0).floor() % 97.0,
            ],
            white_balance: [
                balance.x,
                balance.y,
                balance.z,
                if s.dithering { 1.0 } else { 0.0 },
            ],
            vignette_color: [
                s.vignette.color[0],
                s.vignette.color[1],
                s.vignette.color[2],
                s.vignette.intensity.clamp(0.0, 1.0),
            ],
            vignette: [
                s.vignette.center[0],
                s.vignette.center[1],
                s.vignette.smoothness.clamp(0.0, 1.0),
                w as f32 / h.max(1) as f32,
            ],
            texel: [0.0; 4],
            bloom: [
                s.bloom.threshold.max(0.0),
                knee,
                s.bloom.scatter.clamp(0.0, 1.0),
                s.bloom.clamp.max(1.0),
            ],
            mixer_red: v4(s.channel_mixer.red, 0.0),
            mixer_green: v4(s.channel_mixer.green, 0.0),
            mixer_blue: v4(s.channel_mixer.blue, 0.0),
            smh_shadows: v4(smh.shadows, smh.shadows_range[0]),
            smh_midtones: v4(smh.midtones, smh.shadows_range[1]),
            smh_highlights: v4(smh.highlights, smh.highlights_range[0]),
            lift: v4(s.lift_gamma_gain.lift, smh.highlights_range[1]),
            gamma: v4(s.lift_gamma_gain.gamma.map(|g| g.max(1e-3)), 0.0),
            gain: v4(s.lift_gamma_gain.gain, 0.0),
            split_shadows: v4(
                s.split_toning.shadows,
                (s.split_toning.balance / 100.0).clamp(-1.0, 1.0),
            ),
            split_highlights: v4(s.split_toning.highlights, 0.0),
            distortion_axis: {
                let d = &s.lens_distortion;
                [
                    d.center[0] * 2.0 - 1.0,
                    d.center[1] * 2.0 - 1.0,
                    d.x_multiplier.clamp(0.0, 1.0).max(1e-4),
                    d.y_multiplier.clamp(0.0, 1.0).max(1e-4),
                ]
            },
            distortion: {
                // URP's own constants for the bend.
                let d = &s.lens_distortion;
                let intensity = d.intensity.clamp(-1.0, 1.0);
                let amount = 1.6 * (intensity * 100.0).abs().max(1.0);
                let theta = amount.min(160.0).to_radians();
                let sigma = 2.0 * (theta * 0.5).tan();
                [
                    if intensity >= 0.0 { theta } else { 1.0 / theta },
                    sigma,
                    1.0 / d.scale.clamp(0.01, 5.0),
                    intensity * 100.0,
                ]
            },
            panini: panini_params(
                &s.panini_projection,
                self.fov_y_degrees,
                w as f32 / h.max(1) as f32,
            ),
            flare_tint: v4(s.lens_flare.tint, s.lens_flare.intensity.max(0.0)),
            flare: [
                s.lens_flare.ghosts.max(0.0),
                s.lens_flare.halo.max(0.0),
                s.lens_flare.streaks.max(0.0),
                s.lens_flare.chromatic_aberration.clamp(0.0, 1.0),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_white_balance_changes_nothing() {
        let c = white_balance_coefficients(0.0, 0.0);
        // Unity's own coefficients at zero are within a hair of one.
        assert!((c - Vec3::ONE).abs().max_element() < 0.02, "{c}");
        let warm = white_balance_coefficients(50.0, 0.0);
        assert!(
            warm.x > warm.z,
            "warm lifts long wavelengths over short: {warm}"
        );
    }

    #[test]
    fn post_settings_read_from_a_scene_line_with_urp_names() {
        let post: PostProcess = ron::from_str(
            "(tonemapping: Aces, bloom: (intensity: 1.0), saturation: -40.0, fxaa: true)",
        )
        .unwrap();
        assert_eq!(post.tonemapping, Tonemapping::Aces);
        assert_eq!(post.bloom.intensity, 1.0);
        assert_eq!(post.bloom.threshold, 1.0, "what is not said is the default");
        assert!(post.fxaa && post.enabled);
        let half = PostProcess::default().lerp(&post, 0.5);
        assert_eq!(half.saturation, -20.0);
    }
}
