//! Moods: a whole scene's look in one word — golden hour, overcast, a misty
//! morning, rain, snow, night, a storm, desert noon, a sandstorm — for a person picking from a list
//! and an agent told "make it a stormy evening" alike (DNA 5 and 8).
//!
//! A mood is only a set of the scene's own fields — `sun`, `sky`, `fog`,
//! `volumetric_fog`, `weather`, `wind`, `post`, `screen_space_reflections`
//! — written as the file writes them. Applying one is an ordinary edit:
//! one undo step, and what it did is right there in the scene's text to
//! tune further. Nothing about a mood is remembered after.

/// A mood: its name, what it is for, and the fields it sets.
pub struct Mood {
    pub name: &'static str,
    pub about: &'static str,
    pub fields: &'static [(&'static str, &'static str)],
}

/// The scene fields a mood, or the look tool, sets.
pub const LOOK_FIELDS: [&str; 11] = [
    "sun",
    "fog",
    "sky",
    "post",
    "ambient_occlusion",
    "shadows",
    "volumetric_fog",
    "weather",
    "wind",
    "screen_space_reflections",
    "ray_tracing",
];

const CLEAR: &str = "None";

pub const MOODS: &[Mood] = &[
    Mood {
        name: "clear noon",
        about: "high sun, a blue sky with a few clouds, crisp shadows",
        fields: &[
            ("sun", "(hour: 12.5, intensity: 1.2)"),
            ("sky", "(mode: Physical, clouds: (coverage: 0.2))"),
            ("fog", "(color: (0.62, 0.68, 0.74), start: 60.0, end: 400.0)"),
            ("volumetric_fog", CLEAR),
            ("weather", CLEAR),
            ("wind", "(strength: 0.8)"),
            ("post", CLEAR),
        ],
    },
    Mood {
        name: "golden hour",
        about: "a low warm sun, long shadows, glowing haze and backlit leaves",
        fields: &[
            ("sun", "(hour: 17.6, intensity: 1.3)"),
            ("sky", "(mode: Physical, clouds: (coverage: 0.3))"),
            ("fog", "(color: (0.62, 0.68, 0.74), start: 40.0, end: 300.0)"),
            ("volumetric_fog", "(enabled: true, density: 0.015, anisotropy: 0.75, height_falloff: 0.2)"),
            ("weather", CLEAR),
            ("wind", "(strength: 0.6)"),
            ("post", "(bloom: (intensity: 0.45), temperature: 12.0, saturation: 8.0)"),
        ],
    },
    Mood {
        name: "overcast",
        about: "a grey sky from edge to edge, soft light, no hard shadows",
        fields: &[
            ("sun", "(hour: 13.0, intensity: 0.8)"),
            ("sky", "(mode: Physical, clouds: (coverage: 0.95, shadows: 0.9))"),
            ("fog", "(color: (0.6, 0.63, 0.66), start: 20.0, end: 180.0)"),
            ("volumetric_fog", CLEAR),
            ("weather", CLEAR),
            ("wind", "(strength: 1.2)"),
            ("post", "(saturation: -15.0, contrast: -5.0)"),
        ],
    },
    Mood {
        name: "misty morning",
        about: "early light through mist lying in the low ground",
        fields: &[
            ("sun", "(hour: 7.3, intensity: 1.0)"),
            ("sky", "(mode: Physical, clouds: (coverage: 0.15))"),
            ("fog", "(color: (0.66, 0.7, 0.74), start: 10.0, end: 120.0)"),
            ("volumetric_fog", "(enabled: true, density: 0.07, height_falloff: 0.35, anisotropy: 0.7)"),
            ("weather", "(wetness: 0.3)"),
            ("wind", "(strength: 0.3)"),
            ("post", "(temperature: -8.0, bloom: (intensity: 0.35))"),
        ],
    },
    Mood {
        name: "rainy",
        about: "steady rain, wet ground and puddles that mirror the world",
        fields: &[
            ("sun", "(hour: 14.0, intensity: 0.45)"),
            ("sky", "(mode: Physical, clouds: (coverage: 1.0, density: 1.3))"),
            ("fog", "(color: (0.5, 0.54, 0.58), start: 10.0, end: 120.0)"),
            ("volumetric_fog", "(enabled: true, density: 0.02, height_falloff: 0.1)"),
            ("weather", "(rain: 1.0, wetness: 1.0, puddles: 0.55)"),
            ("wind", "(strength: 1.5)"),
            ("screen_space_reflections", "(enabled: true)"),
            ("post", "(saturation: -20.0)"),
        ],
    },
    Mood {
        name: "snowy",
        about: "a white ground under a pale sky, snow still falling",
        fields: &[
            ("sun", "(hour: 12.0, intensity: 0.9, ground: (0.85, 0.87, 0.9))"),
            ("sky", "(mode: Physical, clouds: (coverage: 0.7))"),
            ("fog", "(color: (0.8, 0.83, 0.86), start: 15.0, end: 160.0)"),
            ("volumetric_fog", CLEAR),
            ("weather", "(snow: 0.9, snowfall: 0.6)"),
            ("wind", "(strength: 0.7)"),
            ("post", "(temperature: -12.0)"),
        ],
    },
    Mood {
        name: "night",
        about: "moonlight, stars and the Milky Way, a dark blue sky, lamps glowing in the air",
        fields: &[
            ("sun", "(hour: 22.5, intensity: 1.2)"),
            ("sky", "(mode: Physical)"),
            ("fog", "(color: (0.03, 0.04, 0.07), start: 20.0, end: 150.0)"),
            ("volumetric_fog", "(enabled: true, density: 0.03, ambient: 0.3, lamps: 6.0)"),
            ("weather", CLEAR),
            ("wind", "(strength: 0.4)"),
            ("post", "(exposure: 0.8, bloom: (intensity: 0.6), temperature: -20.0)"),
        ],
    },
    Mood {
        name: "storm",
        about: "heavy dark clouds, a gale bending the trees, driving rain",
        fields: &[
            ("sun", "(hour: 16.0, intensity: 0.35)"),
            ("sky", "(mode: Physical, clouds: (coverage: 1.0, density: 1.8, shadows: 1.0))"),
            ("fog", "(color: (0.36, 0.39, 0.43), start: 8.0, end: 90.0)"),
            ("volumetric_fog", "(enabled: true, density: 0.03, height_falloff: 0.05)"),
            ("weather", "(rain: 1.0, wetness: 1.0, puddles: 0.75)"),
            ("wind", "(strength: 3.0)"),
            ("screen_space_reflections", "(enabled: true)"),
            ("post", "(saturation: -30.0, contrast: 12.0)"),
        ],
    },
    Mood {
        name: "desert noon",
        about: "a white-hot sun over sand, a dusty pale sky, the air shimmering and a mirage at the horizon",
        fields: &[
            ("sun", "(hour: 12.5, intensity: 1.5, ground: (0.78, 0.6, 0.38))"),
            ("sky", "(mode: Physical, atmosphere: (mie: 2.5))"),
            ("fog", "(color: (0.85, 0.78, 0.66), start: 150.0, end: 900.0)"),
            ("volumetric_fog", CLEAR),
            ("weather", CLEAR),
            ("wind", "(strength: 0.8)"),
            ("post", "(heat_haze: (intensity: 0.7, mirage: 0.8), temperature: 10.0)"),
        ],
    },
    Mood {
        name: "sandstorm",
        about: "sand thick in the air on a gale, the sun a dim disc, the distance gone",
        fields: &[
            ("sun", "(hour: 14.0, intensity: 0.9, ground: (0.78, 0.6, 0.38))"),
            ("sky", "(mode: Physical, atmosphere: (mie: 6.0))"),
            ("fog", "(color: (0.8, 0.62, 0.4), start: 5.0, end: 80.0)"),
            ("volumetric_fog", CLEAR),
            ("weather", "(sandstorm: 1.0)"),
            ("wind", "(direction: (1.0, 0.0, 0.2), strength: 3.0)"),
            ("post", "(temperature: 25.0, saturation: 5.0)"),
        ],
    },
];

/// A mood by its name, however it is written.
pub fn mood(name: &str) -> Option<&'static Mood> {
    let wanted = name.trim().to_lowercase().replace(['_', '-'], " ");
    MOODS.iter().find(|m| m.name == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mood_is_fields_a_scene_reads() {
        for mood in MOODS {
            for (field, text) in mood.fields {
                assert!(LOOK_FIELDS.contains(field), "{}: {field}", mood.name);
                let mut scene = crate::scene::Scene::default();
                crate::scene::set_look_field(&mut scene, field, text)
                    .unwrap_or_else(|e| panic!("{}: {field}: {e}", mood.name));
            }
        }
        assert!(mood("Golden_Hour").is_some());
        assert!(mood("tuesday").is_none());
    }
}
