//! How a Unity scene looks, as a scrap scene says it: the fog and the
//! sky from its RenderSettings, the post-processing from its global
//! Volume's profile.
//!
//! URP's Volume overrides and scrap's post settings share names and
//! units by design ([`scrap::post`]), so most of it is copying; where URP
//! prepares a value before its shader sees it — the trackballs of Lift
//! Gamma Gain, the weights of Shadows Midtones Highlights — the same
//! preparation is done here, so the numbers in the file are what the
//! picture was graded with. What a profile does not override is URP's
//! default, not scrap's: no tonemapping unless the profile says so.

use scrap::post::PostProcess;

use yaml_rust2::Yaml;
use super::yaml::{self, Get};
use super::{Report, Unity};

/// The scene's fog: Unity's three modes are scrap's three. Off in Unity
/// is fog too far off to see.
pub fn fog(text: &str) -> Option<scrap::scene::Fog> {
    let docs = yaml::documents(text);
    let settings = docs.iter().find(|d| d.kind == "RenderSettings")?;
    let b = &settings.body;
    let c = b.color("m_FogColor").unwrap_or([0.5, 0.5, 0.5, 1.0]);
    // Unity saves a colour as the picker shows it, sRGB; scrap's fog is
    // the linear colour it blends towards.
    let linear = |v: f32| scrap::material::srgb_to_linear(v.clamp(0.0, 1.0));
    let mut fog = scrap::scene::Fog {
        color: [linear(c[0]), linear(c[1]), linear(c[2])],
        start: b.f32("m_LinearFogStart").unwrap_or(0.0),
        end: b.f32("m_LinearFogEnd").unwrap_or(300.0),
        mode: match b.i64("m_FogMode") {
            Some(2) => scrap::render::FogMode::Exponential,
            Some(3) => scrap::render::FogMode::ExponentialSquared,
            _ => scrap::render::FogMode::Linear,
        },
        density: b.f32("m_FogDensity").unwrap_or(0.01),
    };
    if b.i64("m_Fog") == Some(0) {
        fog.mode = scrap::render::FogMode::Linear;
        fog.start = 1.0e5;
        fog.end = 1.0e6;
    }
    Some(fog)
}

/// The scene's sky: Unity's Procedural skybox as its material sets it —
/// the built-in Default-Skybox's numbers when it is that one, or when the
/// material is not a file of the project. A scene with no skybox is its
/// fog's colour.
pub fn sky(unity: &Unity, text: &str) -> Option<scrap::render::Sky> {
    let docs = yaml::documents(text);
    let b = &docs.iter().find(|d| d.kind == "RenderSettings")?.body;
    let skybox = b.reference("m_SkyboxMaterial");
    if skybox.as_ref().is_none_or(|r| r.is_none()) {
        return Some(scrap::render::Sky {
            mode: scrap::render::SkyMode::Color,
            ..Default::default()
        });
    }
    let material = skybox
        .and_then(|r| r.guid)
        .and_then(|g| unity.guids.get(&g))
        .and_then(|p| super::material::material_body(unity, p));
    let float = |name: &str, default: f32| {
        material
            .as_ref()
            .and_then(|m| super::material::float(m, name))
            .unwrap_or(default)
    };
    // Saved as the picker shows them, sRGB; the sky's colours are linear.
    let colour = |name: &str, default: [f32; 3]| {
        let c = material
            .as_ref()
            .and_then(|m| super::material::color(m, name))
            .map_or(default, |c| [c[0], c[1], c[2]]);
        c.map(|v| scrap::material::srgb_to_linear(v.clamp(0.0, 1.0)))
    };
    // A skybox of the project's own shader that paints a gradient
    // (3D Game Kit's SimpleSky: _AmbientSky, _AmbientHorizon,
    // _AmbientGround): scrap's Gradient sky, its three colours.
    if let Some(m) = material.as_ref().filter(|m| super::material::own_shader(unity, m).is_some()) {
        let named = |words: &[&str]| {
            m["m_SavedProperties"].list("m_Colors").iter().find_map(|item| {
                let Yaml::Hash(h) = item else { return None };
                let (k, _) = h.iter().next()?;
                let key = k.as_str()?;
                let lower = key.to_lowercase();
                words.iter().any(|w| lower.contains(w)).then(|| key.to_string())
            })
        };
        if let (Some(top), Some(middle), Some(bottom)) = (
            named(&["sky", "zenith", "top"]),
            named(&["horizon", "equator"]),
            named(&["ground", "bottom"]),
        ) {
            return Some(scrap::render::Sky {
                mode: scrap::render::SkyMode::Gradient,
                zenith: colour(&top, [0.2, 0.4, 0.7]),
                horizon: colour(&middle, [0.6, 0.7, 0.8]),
                ground: colour(&bottom, [0.3, 0.3, 0.3]),
                sun_size: 0.0,
                reflection_intensity: b.f32("m_ReflectionIntensity").unwrap_or(1.0),
                ..Default::default()
            });
        }
    }
    // _SunSize is the disc's radius, as a length between unit
    // directions; scrap's is degrees across. _SunDisk 0 is none.
    let sun_size = if float("_SunDisk", 1.0) < 0.5 {
        0.0
    } else {
        2.0 * float("_SunSize", 0.04).to_degrees()
    };
    Some(scrap::render::Sky {
        mode: scrap::render::SkyMode::Procedural,
        tint: colour("_SkyTint", [0.5, 0.5, 0.5]),
        ground: colour("_GroundColor", [0.369, 0.349, 0.341]),
        thickness: float("_AtmosphereThickness", 1.0),
        exposure: float("_Exposure", 1.3),
        sun_size,
        reflection_intensity: b.f32("m_ReflectionIntensity").unwrap_or(1.0),
        ..Default::default()
    })
}

/// The light from all round a scene says itself: Unity's gradient
/// (Trilight) as its three colours, a flat colour as one colour three
/// times. A skybox's light is left to scrap's sky.
pub fn ambient(text: &str) -> Option<scrap::scene::Ambient> {
    let docs = yaml::documents(text);
    let b = &docs.iter().find(|d| d.kind == "RenderSettings")?.body;
    let intensity = b.f32("m_AmbientIntensity").unwrap_or(1.0);
    let colour = |key: &str| {
        let c = b.color(key)?;
        Some([c[0], c[1], c[2]].map(|v| v * intensity))
    };
    let sky = colour("m_AmbientSkyColor")?;
    match b.i64("m_AmbientMode") {
        Some(1) => Some(scrap::scene::Ambient {
            sky,
            equator: colour("m_AmbientEquatorColor")?,
            ground: colour("m_AmbientGroundColor")?,
        }),
        Some(3) => Some(scrap::scene::Ambient { sky, equator: sky, ground: sky }),
        _ => None,
    }
}

/// The value of an override that is on, from a profile's component.
fn value<'a>(component: &'a yaml_rust2::Yaml, key: &str) -> Option<&'a yaml_rust2::Yaml> {
    let field = &component[key];
    (field.i64("m_OverrideState") == Some(1)).then(|| &field["m_Value"])
}

fn number(component: &yaml_rust2::Yaml, key: &str) -> Option<f32> {
    value(component, key)
        .and_then(yaml::number)
        .map(|n| n as f32)
}

fn rgb(component: &yaml_rust2::Yaml, key: &str) -> Option<[f32; 3]> {
    let v = value(component, key)?;
    Some([v.f32("r")?, v.f32("g")?, v.f32("b")?])
}

fn vec4(component: &yaml_rust2::Yaml, key: &str) -> Option<[f32; 4]> {
    let v = value(component, key)?;
    Some([v.f32("x")?, v.f32("y")?, v.f32("z")?, v.f32("w")?])
}

/// ColorUtils.Luminance: sRGB primaries, D65.
fn luminance(c: [f32; 3]) -> f32 {
    c[0] * 0.2126729 + c[1] * 0.7151522 + c[2] * 0.072175
}

/// A trackball's colour, saved as the picker shows it, in linear light:
/// ColorUtils' GammaToLinearSpace.
fn trackball(v: [f32; 4], scale: f32) -> [f32; 3] {
    [v[0], v[1], v[2]].map(|c| scrap::material::srgb_to_linear(c.max(0.0)) * scale)
}

/// URP's ColorUtils.PrepareLiftGammaGain (Unity 6): each trackball's
/// colour in linear light, scaled, about its own luminance, plus its
/// offset — lift's added as it is, gamma's and gain's about one.
fn lift_gamma_gain(lift: [f32; 4], gamma: [f32; 4], gain: [f32; 4]) -> scrap::post::LiftGammaGain {
    let about = |c: [f32; 3], offset: f32| {
        let l = luminance(c);
        c.map(|v| v - l + offset)
    };
    let lift = about(trackball(lift, 0.15), lift[3]);
    // URP's shader raises to 1/this; so does scrap's.
    let gamma = about(trackball(gamma, 0.8), gamma[3] + 1.0).map(|g| g.max(1e-3));
    let gain = about(trackball(gain, 0.8), gain[3] + 1.0);
    scrap::post::LiftGammaGain { lift, gamma, gain }
}

/// URP's ColorUtils.PrepareShadowsMidtonesHighlights: the colour in linear
/// light, and the offset weighing four times as much brightening as
/// darkening.
fn tone(v: [f32; 4]) -> [f32; 3] {
    let weight = v[3] * if v[3] < 0.0 { 1.0 } else { 4.0 };
    trackball(v, 1.0).map(|c| (c + weight).max(0.0))
}

/// What URP does with no profile overriding it: no tonemapping, no bloom,
/// no vignette — and no eye adapting to the light: a picture is as
/// exposed as its profiles say.
fn urp_defaults() -> PostProcess {
    let mut post = PostProcess {
        tonemapping: scrap::post::Tonemapping::None,
        ..PostProcess::default()
    };
    post.bloom.intensity = 0.0;
    post.bloom.threshold = 0.9;
    post.auto_exposure.enabled = false;
    post
}

/// A Volume profile as scrap's post settings, over URP's defaults.
#[cfg(test)]
fn post_of_profile(text: &str, report: &mut Report) -> PostProcess {
    let mut post = urp_defaults();
    apply_profile(&mut post, text, report);
    post
}

/// A Volume profile's overrides laid over `post`: what it does not
/// override stays as it was.
fn apply_profile(post: &mut PostProcess, text: &str, report: &mut Report) {
    for doc in yaml::documents(text) {
        let c = &doc.body;
        if c.i64("active") == Some(0) {
            continue;
        }
        match c.str("m_Name").unwrap_or("") {
            "Bloom" => {
                let b = &mut post.bloom;
                b.intensity = number(c, "intensity").unwrap_or(b.intensity);
                b.threshold = number(c, "threshold").unwrap_or(b.threshold);
                b.scatter = number(c, "scatter").unwrap_or(b.scatter);
                b.clamp = number(c, "clamp").unwrap_or(b.clamp);
                b.tint = rgb(c, "tint").unwrap_or(b.tint);
            }
            "Tonemapping" => {
                post.tonemapping = match value(c, "mode").and_then(yaml::integer) {
                    Some(1) => scrap::post::Tonemapping::Neutral,
                    Some(2) => scrap::post::Tonemapping::Aces,
                    Some(_) => scrap::post::Tonemapping::None,
                    None => post.tonemapping,
                }
            }
            "ColorAdjustments" => {
                post.exposure = number(c, "postExposure").unwrap_or(post.exposure);
                post.contrast = number(c, "contrast").unwrap_or(post.contrast);
                post.saturation = number(c, "saturation").unwrap_or(post.saturation);
                post.hue_shift = number(c, "hueShift").unwrap_or(post.hue_shift);
                post.color_filter = rgb(c, "colorFilter").unwrap_or(post.color_filter);
            }
            "WhiteBalance" => {
                post.temperature = number(c, "temperature").unwrap_or(post.temperature);
                post.tint = number(c, "tint").unwrap_or(post.tint);
            }
            "Vignette" => {
                let v = &mut post.vignette;
                v.intensity = number(c, "intensity").unwrap_or(v.intensity);
                v.smoothness = number(c, "smoothness").unwrap_or(v.smoothness);
                v.color = rgb(c, "color").unwrap_or(v.color);
                if let Some(at) = value(c, "center") {
                    v.center = [at.f32("x").unwrap_or(0.5), at.f32("y").unwrap_or(0.5)];
                }
                v.rounded = number(c, "rounded").map_or(v.rounded, |r| r > 0.5);
            }
            "LiftGammaGain" => {
                let neutral = [1.0, 1.0, 1.0, 0.0];
                post.lift_gamma_gain = lift_gamma_gain(
                    vec4(c, "lift").unwrap_or(neutral),
                    vec4(c, "gamma").unwrap_or(neutral),
                    vec4(c, "gain").unwrap_or(neutral),
                );
            }
            "ShadowsMidtonesHighlights" => {
                let neutral = [1.0, 1.0, 1.0, 0.0];
                let s = &mut post.shadows_midtones_highlights;
                s.shadows = tone(vec4(c, "shadows").unwrap_or(neutral));
                s.midtones = tone(vec4(c, "midtones").unwrap_or(neutral));
                s.highlights = tone(vec4(c, "highlights").unwrap_or(neutral));
                if let (Some(a), Some(b)) = (number(c, "shadowsStart"), number(c, "shadowsEnd")) {
                    s.shadows_range = [a, b];
                }
                if let (Some(a), Some(b)) =
                    (number(c, "highlightsStart"), number(c, "highlightsEnd"))
                {
                    s.highlights_range = [a, b];
                }
            }
            "SplitToning" => {
                let t = &mut post.split_toning;
                t.shadows = rgb(c, "shadows").unwrap_or(t.shadows);
                t.highlights = rgb(c, "highlights").unwrap_or(t.highlights);
                t.balance = number(c, "balance").unwrap_or(t.balance);
            }
            "ChromaticAberration" => {
                post.chromatic_aberration = number(c, "intensity").unwrap_or(post.chromatic_aberration);
            }
            "FilmGrain" => post.film_grain = number(c, "intensity").unwrap_or(post.film_grain),
            "DepthOfField" => {
                let d = &mut post.depth_of_field;
                d.mode = match value(c, "mode").and_then(yaml::integer) {
                    Some(1) => scrap::lens::FocusMode::Gaussian,
                    Some(2) => scrap::lens::FocusMode::Bokeh,
                    _ => scrap::lens::FocusMode::Off,
                };
                d.gaussian_start = number(c, "gaussianStart").unwrap_or(d.gaussian_start);
                d.gaussian_end = number(c, "gaussianEnd").unwrap_or(d.gaussian_end);
                d.gaussian_max_radius =
                    number(c, "gaussianMaxRadius").unwrap_or(d.gaussian_max_radius);
                d.focus_distance = number(c, "focusDistance").unwrap_or(d.focus_distance);
                d.focal_length = number(c, "focalLength").unwrap_or(d.focal_length);
                d.aperture = number(c, "aperture").unwrap_or(d.aperture);
            }
            "MotionBlur" => {
                let m = &mut post.motion_blur;
                m.intensity = number(c, "intensity").unwrap_or(m.intensity);
                m.clamp = number(c, "clamp").unwrap_or(m.clamp);
            }
            "" => {}
            other if doc.kind == "MonoBehaviour" && c.get("components").is_badvalue() => {
                report.skip(format!("Volume override {other}"));
            }
            _ => {}
        }
    }
}

/// The scene's post settings as URP layers them: its defaults, the render
/// pipeline asset's own Volume Profile under every scene (Unity 6), then
/// the scene's global Volume — the first with `isGlobal` and a shared
/// profile. `None` for a project with no URP pipeline and a scene with no
/// Volume.
pub fn post(unity: &Unity, text: &str, report: &mut Report) -> Option<PostProcess> {
    let docs = yaml::documents(text);
    let profile = docs.iter().find_map(|d| {
        let b = &d.body;
        // `isGlobal` before Unity 6, `m_IsGlobal` since.
        let global = b.i64("isGlobal").or_else(|| b.i64("m_IsGlobal"));
        (d.kind == "MonoBehaviour" && global == Some(1))
            .then(|| b.reference("sharedProfile"))
            .flatten()
            .and_then(|r| r.guid)
    });
    let read = |guid: &String| unity.guids.get(guid).and_then(|p| std::fs::read_to_string(p).ok());
    let pipeline = pipeline_asset(unity);
    let under = pipeline.as_ref().and_then(|text| {
        let guid = yaml::documents(text)
            .into_iter()
            .find_map(|d| d.body.reference("m_VolumeProfile"))
            .filter(|r| r.file_id != 0)?
            .guid?;
        read(&guid)
    });
    let scene = profile.as_ref().and_then(read);
    if pipeline.is_none() && scene.is_none() {
        return None;
    }
    let mut post = urp_defaults();
    for text in [under, scene].into_iter().flatten() {
        apply_profile(&mut post, &text, report);
    }
    post.grading = grading(pipeline.as_deref());
    Some(post)
}

/// The project's render pipeline asset, as text: the quality level's own
/// pipeline, or the graphics settings'.
fn pipeline_asset(unity: &Unity) -> Option<String> {
    let settings = |name: &str| {
        let text = std::fs::read_to_string(unity.root.join("ProjectSettings").join(name)).ok()?;
        yaml::documents(&text).into_iter().next().map(|d| d.body)
    };
    let usable = |r: Option<yaml::Ref>| r.filter(|r| r.file_id != 0).and_then(|r| r.guid);
    let from_quality = settings("QualitySettings.asset").and_then(|q| {
        let current = q.i64("m_CurrentQuality").unwrap_or(0).max(0) as usize;
        usable(q.list("m_QualitySettings").get(current)?.reference("customRenderPipeline"))
    });
    let pipeline = from_quality.or_else(|| {
        settings("GraphicsSettings.asset").and_then(|g| usable(g.reference("m_CustomRenderPipeline")))
    })?;
    std::fs::read_to_string(unity.guids.get(&pipeline)?).ok()
}

/// The sun's shadows as the render pipeline asset casts them: URP's
/// shadow distance, cascades and their splits, last border, depth and
/// normal bias (in texels, as scrap's are) and soft shadow quality — soft
/// only when the pipeline allows it and the scene's sun asks for it. Its
/// cascades share one atlas: past one, each is half its side. URP has no
/// contact shadows. `None` for a project with no URP pipeline.
pub fn shadows(unity: &Unity, scene: &str) -> Option<scrap::render::ShadowSettings> {
    let text = pipeline_asset(unity)?;
    let p = yaml::documents(&text).into_iter().find(|d| d.body.f32("m_ShadowDistance").is_some())?.body;
    let mut out = scrap::render::ShadowSettings {
        contact: 0.0,
        ..Default::default()
    };
    out.enabled = p.i64("m_MainLightShadowsSupported").unwrap_or(1) != 0;
    out.max_distance = p.f32("m_ShadowDistance").unwrap_or(50.0);
    out.cascades = p.i64("m_ShadowCascadeCount").unwrap_or(1).clamp(1, 4) as u32;
    if let Some(split) = p.vec3("m_Cascade4Split") {
        out.cascade_splits = split;
    }
    out.cascade_border = p.f32("m_CascadeBorder").unwrap_or(0.2);
    out.depth_bias = p.f32("m_ShadowDepthBias").unwrap_or(1.0);
    out.normal_bias = p.f32("m_ShadowNormalBias").unwrap_or(1.0);
    let atlas = p.i64("m_MainLightShadowmapResolution").unwrap_or(2048).max(1) as u32;
    out.resolution = if out.cascades > 1 { atlas / 2 } else { atlas };
    // The sun's own shadow type: 1 hard, 2 soft.
    let sun_soft = yaml::documents(scene)
        .iter()
        .find(|d| d.kind == "Light" && d.body.i64("m_Type") == Some(1))
        .and_then(|l| l.body["m_Shadows"].i64("m_Type"))
        .is_none_or(|t| t == 2);
    out.soft = if p.i64("m_SoftShadowsSupported").unwrap_or(0) != 0 && sun_soft {
        match p.i64("m_SoftShadowQuality").unwrap_or(2) {
            1 => scrap::render::SoftShadows::Low,
            3 => scrap::render::SoftShadows::High,
            _ => scrap::render::SoftShadows::Medium,
        }
    } else {
        scrap::render::SoftShadows::Hard
    };
    Some(out)
}

/// The pipeline's default renderer's Screen Space Ambient Occlusion
/// feature, when it is there and on: URP's SSAO, its numbers as they are.
/// `None` when the renderer has none.
pub fn ambient_occlusion(unity: &Unity) -> Option<scrap::ssao::AmbientOcclusion> {
    let text = pipeline_asset(unity)?;
    let p = yaml::documents(&text).into_iter().find(|d| d.body.f32("m_ShadowDistance").is_some())?.body;
    let list = p.list("m_RendererDataList");
    let index = p.i64("m_DefaultRendererIndex").unwrap_or(0).max(0) as usize;
    let guid = yaml::reference(list.get(index)?)?.guid?;
    let renderer = std::fs::read_to_string(unity.guids.get(&guid)?).ok()?;
    let docs = yaml::documents(&renderer);
    let feature = docs.iter().find(|d| {
        d.body.str("m_Name") == Some("ScreenSpaceAmbientOcclusion") && d.body.i64("m_Active") != Some(0)
    })?;
    let s = &feature.body["m_Settings"];
    let defaults = scrap::ssao::AmbientOcclusion::default();
    Some(scrap::ssao::AmbientOcclusion {
        enabled: true,
        intensity: s.f32("Intensity").unwrap_or(3.0),
        radius: s.f32("Radius").unwrap_or(0.035),
        direct_lighting_strength: s.f32("DirectLightingStrength").unwrap_or(0.25),
        falloff_distance: s.f32("Falloff").unwrap_or(100.0),
        // URP's High, Medium and Low.
        samples: match s.i64("Samples").unwrap_or(1) {
            0 => 12,
            2 => 4,
            _ => 8,
        },
        method: scrap::ssao::Method::Ssao,
        ..defaults
    })
}

/// Where a render pipeline asset grades: its Grading Mode. High dynamic
/// range when it cannot be told.
pub fn grading(pipeline: Option<&str>) -> scrap::post::Grading {
    let mode = pipeline.and_then(|text| {
        yaml::documents(text).into_iter().find_map(|d| d.body.i64("m_ColorGradingMode"))
    });
    match mode {
        Some(0) => scrap::post::Grading::LowDynamicRange,
        _ => scrap::post::Grading::HighDynamicRange,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_trackballs_grade_nothing() {
        let n = [1.0, 1.0, 1.0, 0.0];
        let g = lift_gamma_gain(n, n, n);
        for i in 0..3 {
            assert!(g.lift[i].abs() < 1e-5, "{g:?}");
            assert!((g.gamma[i] - 1.0).abs() < 1e-5, "{g:?}");
            assert!((g.gain[i] - 1.0).abs() < 1e-5, "{g:?}");
        }
        assert_eq!(tone(n), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn trackballs_are_prepared_as_urps_color_utils_does() {
        // GDScene's profile, and what Unity 6's ColorUtils makes of it.
        let g = lift_gamma_gain(
            [0.9135216, 0.9521028, 1.0, 0.0],
            [1.0, 0.9882, 0.96140367, -0.09930486],
            [1.0, 0.9315856, 0.8925388, 0.63555104],
        );
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-3);
        assert!(near(g.lift, [-0.0106, 0.0014, 0.0172]), "{g:?}");
        assert!(near(g.gamma.map(|g| 1.0 / g), [1.0859, 1.1116, 1.1731]), "{g:?}");
        assert!(near(g.gain, [1.7337, 1.6148, 1.5519]), "{g:?}");
        assert!(near(tone([0.7274276, 0.86546403, 1.0, 0.079443894]), [0.8058, 1.0386, 1.3178]));
    }

    #[test]
    fn a_profile_comes_over_by_its_overrides() {
        let text = r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!114 &1
MonoBehaviour:
  m_Name: Bloom
  active: 1
  threshold:
    m_OverrideState: 1
    m_Value: 0.9
  intensity:
    m_OverrideState: 1
    m_Value: 1.64
  scatter:
    m_OverrideState: 0
    m_Value: 0.2
--- !u!114 &2
MonoBehaviour:
  m_Name: ColorAdjustments
  active: 1
  contrast:
    m_OverrideState: 1
    m_Value: 5.9
  saturation:
    m_OverrideState: 1
    m_Value: -17
"#;
        let mut report = Report::default();
        let post = post_of_profile(text, &mut report);
        assert_eq!(post.bloom.intensity, 1.64);
        assert_eq!(post.bloom.threshold, 0.9);
        assert_eq!(
            post.bloom.scatter,
            PostProcess::default().bloom.scatter,
            "not overridden"
        );
        assert_eq!(post.contrast, 5.9);
        assert_eq!(post.saturation, -17.0);
        assert_eq!(
            post.tonemapping,
            scrap::post::Tonemapping::None,
            "URP's default"
        );
    }
}
