//! The reference scene, rendered and checked.
//!
//! `examples/valley/scenes/first-light.ron` is what the engine is aimed at: it opens with no
//! library and no import step, and every object in it is checking something.
//! This test renders it and asserts the properties that object was put there
//! for. It is not a golden image — a software adapter does not produce a
//! card's bytes — so what it compares are relationships inside one frame,
//! which hold on any correct renderer.

#[allow(unused_imports)]
use runity::prelude::*;
use std::path::{Path, PathBuf};

use runity::builtin;
use runity::glam::Vec3;
use runity::material::Shading;
use runity::render::{Camera, FogSettings, Lighting, ShadowSettings};
use runity::{Gpu, MeshHandle, OffscreenTarget, Renderer, Scene};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;

fn scene_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/runity is two levels down")
        .join("examples/valley/scenes/first-light.ron")
}

struct Shot {
    pixels: Vec<u8>,
}

impl Shot {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        OffscreenTarget::pixel(&self.pixels, WIDTH, x, y)
    }
    fn brightness(&self, x: u32, y: u32) -> u32 {
        let p = self.at(x, y);
        p[0] as u32 + p[1] as u32 + p[2] as u32
    }
}

/// Render the reference scene with the sun at a given hour.
fn shoot(gpu: &Gpu, scene: &Scene, sun_direction: Vec3) -> Shot {
    shoot_with(gpu, scene, sun_direction, ShadowSettings::default())
}

fn shoot_with(gpu: &Gpu, scene: &Scene, sun_direction: Vec3, shadows: ShadowSettings) -> Shot {
    let target = OffscreenTarget::new(gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, &target);

    let mut world = hecs::World::new();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    let missing = runity::spawn_scene(scene, &mut world, |name| {
        let name: &str = name;
        if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
            return Some(found.1);
        }
        let mesh = builtin::by_name(name)?;
        let handle = renderer.upload_mesh_owned(gpu, &mesh);
        uploaded.push((name.to_string(), handle));
        Some(handle)
    });
    assert!(
        missing.is_empty(),
        "the reference scene must open with builtins alone, missing: {missing:?}"
    );

    let mut frame = runity::build_frame(
        &world,
        // From the scene, not from here: the assertions below are about a
        // framing, and a framing that lives in the test is one the file
        // cannot be changed to match.
        runity::scene_camera(&scene.view()),
        Lighting {
            sun_direction,
            sun_intensity: scene.sun().intensity,
            ..Lighting::default()
        },
        FogSettings {
            color: Vec3::from_array(scene.fog().color),
            start: scene.fog().start,
            end: scene.fog().end,
            ..Default::default()
        },
    );
    frame.shadows = shadows;
    // What each object is there to check is its shading: bloom from a lit
    // neighbour glowing into an unlit one is post-processing doing its job,
    // and not what these count.
    frame.post = runity::post::PostProcess::OFF;
    renderer.render(gpu, &target, &frame);
    Shot {
        pixels: target.read_rgba(gpu),
    }
}

#[test]
fn the_reference_scene_renders_what_each_object_is_there_to_check() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter — install mesa-vulkan-drivers to render here");
        return;
    };
    let scene = Scene::load(scene_path()).expect("the reference scene");
    let morning = Vec3::new(-0.6, -0.7, -0.35).normalize();
    let shot = shoot(&gpu, &scene, morning);

    // Sky above, ground below. Without the ground everything floats, which
    // was most of what used to look wrong.
    let sky = shot.at(8, 8);
    let ground = shot.at(8, HEIGHT - 8);
    assert_ne!(sky, ground, "the horizon should divide the frame");
    assert!(
        ground[1] > ground[2],
        "the ground is grass, so green should lead blue: {ground:?}"
    );

    // Fog: the same cone at four distances has to fade toward the sky.
    // Sampling the top row of each tree's silhouette, left to right.
    let near_tree = shot.brightness(148, 300 * HEIGHT / 540);
    let far_tree = shot.brightness(420, 200 * HEIGHT / 540);
    let sky_brightness = sky[0] as u32 + sky[1] as u32 + sky[2] as u32;
    assert!(
        far_tree.abs_diff(sky_brightness) < near_tree.abs_diff(sky_brightness),
        "the far tree should sit closer to the sky than the near one \
         (far {far_tree}, near {near_tree}, sky {sky_brightness})"
    );
}

#[test]
fn an_unlit_material_ignores_the_sun_while_everything_else_follows_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let scene = Scene::load(scene_path()).expect("the reference scene");

    // The ember is the one thing in the scene that emits rather than
    // reflects, and this is the property that makes that true rather than
    // merely intended.
    assert_eq!(
        scene
            .find("ember")
            .expect("the scene has an ember")
            .material()
            .shading,
        Shading::Unlit
    );

    let morning = shoot(&gpu, &scene, Vec3::new(-0.6, -0.7, -0.35).normalize());
    let evening = shoot(&gpu, &scene, Vec3::new(0.85, -0.2, 0.3).normalize());

    // Find the ember: the only strongly orange pixels in a scene of greens
    // and greys — and take one from the middle of them, since an edge pixel
    // is antialiased, part ember and part the lit ground around it.
    let orange: Vec<(u32, u32)> = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            // Red well over green well over blue: the ember glows, so its
            // green is lifted too, but it stays the only thing this orange.
            let p = morning.at(x, y).map(|c| c as i32);
            p[0] > 200 && p[0] > p[1] + 40 && p[1] > p[2] + 40
        })
        .collect();
    assert!(!orange.is_empty(), "the ember should be visible");
    let (sx, sy) = orange.iter().fold((0, 0), |(a, b), (x, y)| (a + x, b + y));
    let centre = (sx / orange.len() as u32, sy / orange.len() as u32);
    let ember = *orange
        .iter()
        .min_by_key(|(x, y)| x.abs_diff(centre.0).pow(2) + y.abs_diff(centre.1).pow(2))
        .expect("not empty");

    assert_eq!(
        morning.at(ember.0, ember.1),
        evening.at(ember.0, ember.1),
        "an unlit surface must not change when the sun moves"
    );

    // And the check that gives that meaning: a lit surface does change.
    let lit_spot = (WIDTH / 2, HEIGHT - 20);
    assert_ne!(
        morning.at(lit_spot.0, lit_spot.1),
        evening.at(lit_spot.0, lit_spot.1),
        "the lit ground should follow the sun, or the comparison above proves nothing"
    );
}

#[test]
fn a_shadow_only_ever_takes_light_away() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let scene = Scene::load(scene_path()).expect("the reference scene");
    let sun = Vec3::new(-0.6, -0.7, -0.35).normalize();

    let lit = shoot_with(&gpu, &scene, sun, ShadowSettings::OFF);
    let shadowed = shoot_with(&gpu, &scene, sun, ShadowSettings::default());

    // The invariant worth having: a shadow pass removes light from some
    // pixels and adds it to none. It catches the two ways this goes wrong —
    // a sign flipped somewhere, which brightens what should darken, and a
    // comparison inverted, which shadows everything except what is occluded.
    let (mut darker, mut brighter, mut deepest) = (0u32, 0u32, 0i64);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let before = lit.brightness(x, y) as i64;
            let after = shadowed.brightness(x, y) as i64;
            let delta = before - after;
            if delta > 6 {
                darker += 1;
                deepest = deepest.max(delta);
            } else if delta < -6 {
                brighter += 1;
            }
        }
    }

    assert_eq!(
        brighter, 0,
        "turning shadows on made {brighter} pixels brighter; a shadow cannot add light"
    );
    // Six objects over open ground cast thin shapes, not a blanket: the
    // measured area at this size is a few hundred pixels, so the bar is set
    // where "some shadow" and "no shadow" are far apart rather than where a
    // round fraction would put it.
    assert!(
        darker > 250,
        "only {darker} pixels fell into shadow, which is too few to be the \
         ground under six objects"
    );
    assert!(
        deepest > 40,
        "the deepest shadow only took {deepest} off; that is a tint, not a shadow"
    );
}

#[test]
fn nothing_shadows_itself_into_stripes() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let scene = Scene::load(scene_path()).expect("the reference scene");
    // A low sun is where shadow acne shows: the depth error at a grazing
    // angle grows with the slope, and a constant bias cannot cover it. If
    // the normal offset is wrong, open ground breaks into stripes.
    let low_sun = Vec3::new(-0.95, -0.18, -0.25).normalize();
    let shot = shoot_with(&gpu, &scene, low_sun, ShadowSettings::default());

    // Walk a row of open ground in front of the camera and count how often
    // brightness reverses direction. Smooth ground changes slowly; acne
    // oscillates every few pixels.
    let row = HEIGHT - 24;
    let mut reversals = 0;
    let mut previous_delta = 0i64;
    for x in 1..WIDTH / 3 {
        let delta = shot.brightness(x, row) as i64 - shot.brightness(x - 1, row) as i64;
        if delta.abs() > 8 && delta.signum() != previous_delta.signum() && previous_delta != 0 {
            reversals += 1;
        }
        if delta.abs() > 8 {
            previous_delta = delta;
        }
    }
    assert!(
        reversals < 6,
        "open ground reversed brightness {reversals} times across one row, \
         which is what shadow acne looks like"
    );
}

#[test]
fn what_is_behind_the_camera_is_not_drawn_but_still_casts() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let scene = Scene::load(scene_path()).expect("the reference scene");
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);

    let mut world = hecs::World::new();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    runity::spawn_scene(&scene, &mut world, |name| {
        if let Some(found) = uploaded.iter().find(|(n, _)| **n == **name) {
            return Some(found.1);
        }
        let mesh = builtin::by_name(name)?;
        let handle = renderer.upload_mesh_owned(&gpu, &mesh);
        uploaded.push((name.to_string(), handle));
        Some(handle)
    });

    let looking_at_it = Camera {
        position: Vec3::new(0.0, 3.4, 12.0),
        target: Vec3::new(0.0, 1.4, -4.0),
        ..Camera::default()
    };
    // Same spot, turned around. The ground is enormous and stays visible
    // either way; what changes is everything standing on it.
    let looking_away = Camera {
        target: Vec3::new(0.0, 3.4, 40.0),
        ..looking_at_it
    };

    let frame = |camera| {
        runity::build_frame(
            &world,
            camera,
            Lighting::default(),
            FogSettings {
                color: Vec3::from_array(scene.fog().color),
                start: scene.fog().start,
                end: scene.fog().end,
                ..Default::default()
            },
        )
    };

    renderer.render(&gpu, &target, &frame(looking_at_it));
    let facing = renderer.stats();
    renderer.render(&gpu, &target, &frame(looking_away));
    let away = renderer.stats();

    assert_eq!(facing.submitted, away.submitted, "the scene did not change");
    assert!(
        away.culled > facing.culled,
        "turning around should cull more, not the same ({} against {})",
        away.culled,
        facing.culled
    );
    assert!(
        facing.drawn > 4,
        "facing the scene should draw most of it, drew {}",
        facing.drawn
    );

    // The shadow pass is deliberately not culled by the camera. Something
    // behind you casts into what is in front of you, and culling it leaves a
    // hole in the ground where its shadow was.
    assert_eq!(
        away.shadow_casters, away.submitted,
        "every object casts, however the camera is pointed"
    );
}

#[test]
fn the_shadow_map_is_spent_on_what_the_camera_can_see() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let renderer = Renderer::new(&gpu, &target);

    let mut frame = runity::build_frame(
        &hecs::World::new(),
        Camera {
            position: Vec3::new(0.0, 3.4, 12.0),
            target: Vec3::new(0.0, 1.4, -4.0),
            ..Camera::default()
        },
        Lighting::default(),
        FogSettings::default(),
    );
    let aspect = WIDTH as f32 / HEIGHT as f32;

    // The ground in the reference scene is 120 metres across. Spreading the
    // map over all of it is what made every shadow a staircase; spending it
    // on the near view is what makes the same map sharp.
    frame.shadows.max_distance = 40.0;
    let near = renderer.shadow_texel_size(&frame, aspect);
    frame.shadows.max_distance = 400.0;
    let far = renderer.shadow_texel_size(&frame, aspect);

    assert!(near > 0.0 && far > 0.0);
    assert!(
        far > near * 5.0,
        "a longer shadow distance must spend the same texels over more \
         ground: {near:.4} m per texel against {far:.4}"
    );
    assert!(
        near < 0.05,
        "at forty metres a 2048 map should give centimetres per texel, got \
         {near:.4} m"
    );
}
