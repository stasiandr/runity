//! How a Unity scene looks, as a runity scene says it: the fog and the
//! sky from its RenderSettings, the post-processing from its global
//! Volume's profile.
//!
//! URP's Volume overrides and runity's post settings share names and
//! units by design ([`runity::post`]), so most of it is copying; where URP
//! prepares a value before its shader sees it — the trackballs of Lift
//! Gamma Gain, the weights of Shadows Midtones Highlights — the same
//! preparation is done here, so the numbers in the file are what the
//! picture was graded with. What a profile does not override is URP's
//! default, not runity's: no tonemapping unless the profile says so.

use runity::post::PostProcess;

use super::yaml::{self, Get};
use super::{Report, Unity};

/// The scene's fog: Unity's three modes are runity's three. Off in Unity
/// is fog too far off to see.
pub fn fog(text: &str) -> Option<runity::scene::Fog> {
    let docs = yaml::documents(text);
    let settings = docs.iter().find(|d| d.kind == "RenderSettings")?;
    let b = &settings.body;
    let c = b.color("m_FogColor").unwrap_or([0.5, 0.5, 0.5, 1.0]);
    let mut fog = runity::scene::Fog {
        color: [c[0], c[1], c[2]],
        start: b.f32("m_LinearFogStart").unwrap_or(0.0),
        end: b.f32("m_LinearFogEnd").unwrap_or(300.0),
        mode: match b.i64("m_FogMode") {
            Some(2) => runity::render::FogMode::Exponential,
            Some(3) => runity::render::FogMode::ExponentialSquared,
            _ => runity::render::FogMode::Linear,
        },
        density: b.f32("m_FogDensity").unwrap_or(0.01),
    };
    if b.i64("m_Fog") == Some(0) {
        fog.mode = runity::render::FogMode::Linear;
        fog.start = 1.0e5;
        fog.end = 1.0e6;
    }
    Some(fog)
}

/// The light from all round a scene says itself: Unity's gradient
/// (Trilight) as its three colours, a flat colour as one colour three
/// times. A skybox's light is left to runity's sky.
pub fn ambient(text: &str) -> Option<runity::scene::Ambient> {
    let docs = yaml::documents(text);
    let b = &docs.iter().find(|d| d.kind == "RenderSettings")?.body;
    let intensity = b.f32("m_AmbientIntensity").unwrap_or(1.0);
    let colour = |key: &str| {
        let c = b.color(key)?;
        Some([c[0], c[1], c[2]].map(|v| v * intensity))
    };
    let sky = colour("m_AmbientSkyColor")?;
    match b.i64("m_AmbientMode") {
        Some(1) => Some(runity::scene::Ambient {
            sky,
            equator: colour("m_AmbientEquatorColor")?,
            ground: colour("m_AmbientGroundColor")?,
        }),
        Some(3) => Some(runity::scene::Ambient { sky, equator: sky, ground: sky }),
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

fn luminance(c: [f32; 3]) -> f32 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

/// URP's ColorUtils.PrepareLiftGammaGain: each trackball's colour about
/// its own luminance, plus its offset.
fn lift_gamma_gain(lift: [f32; 4], gamma: [f32; 4], gain: [f32; 4]) -> runity::post::LiftGammaGain {
    let about = |v: [f32; 4], scale: f32, offset: f32| {
        let c = [v[0] * scale, v[1] * scale, v[2] * scale];
        let l = luminance(c);
        [
            c[0] - l + v[3] * scale + offset,
            c[1] - l + v[3] * scale + offset,
            c[2] - l + v[3] * scale + offset,
        ]
    };
    let lift = about(lift, 0.2, 0.0);
    let inverse_gamma = about(gamma, 0.8, 1.0).map(|g| 1.0 / g.max(1e-3));
    let gain = about(gain, 0.8, 1.0);
    runity::post::LiftGammaGain {
        lift,
        // runity's gamma is the power's inverse: the picture to 1/gamma.
        gamma: inverse_gamma.map(|g| 1.0 / g),
        gain,
    }
}

/// URP's ColorUtils.PrepareShadowsMidtonesHighlights: the offset weighs
/// four times as much brightening as darkening.
fn tone(v: [f32; 4]) -> [f32; 3] {
    let weight = v[3] * if v[3] < 0.0 { 1.0 } else { 4.0 };
    [
        (v[0] + weight).max(0.0),
        (v[1] + weight).max(0.0),
        (v[2] + weight).max(0.0),
    ]
}

/// A Volume profile as runity's post settings.
pub fn post_of_profile(text: &str, report: &mut Report) -> PostProcess {
    let mut post = PostProcess {
        tonemapping: runity::post::Tonemapping::None,
        ..PostProcess::default()
    };
    for doc in yaml::documents(text) {
        let c = &doc.body;
        if c.i64("active") == Some(0) {
            continue;
        }
        match c.str("m_Name").unwrap_or("") {
            "Bloom" => {
                let b = &mut post.bloom;
                b.intensity = number(c, "intensity").unwrap_or(0.0);
                b.threshold = number(c, "threshold").unwrap_or(b.threshold);
                b.scatter = number(c, "scatter").unwrap_or(b.scatter);
                b.clamp = number(c, "clamp").unwrap_or(b.clamp);
                b.tint = rgb(c, "tint").unwrap_or(b.tint);
            }
            "Tonemapping" => {
                post.tonemapping = match value(c, "mode").and_then(yaml::integer) {
                    Some(1) => runity::post::Tonemapping::Neutral,
                    Some(2) => runity::post::Tonemapping::Aces,
                    _ => runity::post::Tonemapping::None,
                }
            }
            "ColorAdjustments" => {
                post.exposure = number(c, "postExposure").unwrap_or(0.0);
                post.contrast = number(c, "contrast").unwrap_or(0.0);
                post.saturation = number(c, "saturation").unwrap_or(0.0);
                post.hue_shift = number(c, "hueShift").unwrap_or(0.0);
                post.color_filter = rgb(c, "colorFilter").unwrap_or(post.color_filter);
            }
            "WhiteBalance" => {
                post.temperature = number(c, "temperature").unwrap_or(0.0);
                post.tint = number(c, "tint").unwrap_or(0.0);
            }
            "Vignette" => {
                let v = &mut post.vignette;
                v.intensity = number(c, "intensity").unwrap_or(0.0);
                v.smoothness = number(c, "smoothness").unwrap_or(v.smoothness);
                v.color = rgb(c, "color").unwrap_or(v.color);
                if let Some(at) = value(c, "center") {
                    v.center = [at.f32("x").unwrap_or(0.5), at.f32("y").unwrap_or(0.5)];
                }
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
                t.balance = number(c, "balance").unwrap_or(0.0);
            }
            "ChromaticAberration" => {
                post.chromatic_aberration = number(c, "intensity").unwrap_or(0.0);
            }
            "FilmGrain" => post.film_grain = number(c, "intensity").unwrap_or(0.0),
            "DepthOfField" => {
                let d = &mut post.depth_of_field;
                d.mode = match value(c, "mode").and_then(yaml::integer) {
                    Some(1) => runity::lens::FocusMode::Gaussian,
                    Some(2) => runity::lens::FocusMode::Bokeh,
                    _ => runity::lens::FocusMode::Off,
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
                m.intensity = number(c, "intensity").unwrap_or(0.0);
                m.clamp = number(c, "clamp").unwrap_or(m.clamp);
            }
            "" => {}
            other if doc.kind == "MonoBehaviour" && c.get("components").is_badvalue() => {
                report.skip(format!("Volume override {other}"));
            }
            _ => {}
        }
    }
    post
}

/// The scene's global Volume's profile, as post settings: the first
/// Volume with `isGlobal` and a shared profile.
pub fn post(unity: &Unity, text: &str, report: &mut Report) -> Option<PostProcess> {
    let docs = yaml::documents(text);
    let profile = docs.iter().find_map(|d| {
        let b = &d.body;
        (d.kind == "MonoBehaviour" && b.i64("isGlobal") == Some(1))
            .then(|| b.reference("sharedProfile"))
            .flatten()
            .and_then(|r| r.guid)
    })?;
    let path = unity.guids.get(&profile)?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(post_of_profile(&text, report))
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
            runity::post::Tonemapping::None,
            "URP's default"
        );
    }
}
