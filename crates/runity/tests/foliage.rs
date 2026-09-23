//! Foliage: a tall thing that sways leans its top downwind and keeps its
//! foot, a bender pushes it down, a rock does not move at all, and a leaf
//! with the sun behind it is lit through.

use runity::foliage::{Bender, Wind};
use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

/// The columns a white post covers on one row: where it stands, seen from
/// the side, with wind blowing along +x.
fn columns(pixels: &[u8], row: u32) -> Vec<u32> {
    (0..SIZE)
        .filter(|&x| OffscreenTarget::pixel(pixels, SIZE, x, row)[0] > 128)
        .collect()
}

fn centre(cols: &[u32]) -> f32 {
    cols.iter().sum::<u32>() as f32 / cols.len().max(1) as f32
}

#[test]
fn a_swaying_post_leans_downwind_at_the_top_and_keeps_its_foot() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // A post four metres tall; its middle is its origin, so its lower half
    // is below it and stays put, and its top is two metres above.
    let post = |wind: f32| Draw {
        mesh: cube,
        transform: Mat4::from_scale(Vec3::new(0.3, 4.0, 0.3))
            * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Unlit,
            wind,
            ..Material::new(1.0, 1.0, 1.0)
        },
        pose: None,
    };
    let shoot = |renderer: &mut Renderer, draw: Draw, wind: Wind, benders: Vec<Bender>| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 2.0, 8.0),
                target: Vec3::new(0.0, 2.0, 0.0),
                ..Camera::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: vec![draw],
            wind,
            benders,
            time: Some(1.0),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let gale = Wind {
        direction: Vec3::X,
        strength: 3.0,
    };
    let still = shoot(
        &mut renderer,
        post(2.0),
        Wind {
            strength: 0.0,
            ..gale
        },
        Vec::new(),
    );
    let blown = shoot(&mut renderer, post(2.0), gale, Vec::new());
    let rock = shoot(&mut renderer, post(0.0), gale, Vec::new());
    // Rows: near the top of the post, and near its foot.
    let (top, foot) = (SIZE / 4 + 4, SIZE * 3 / 4 - 2);
    let (still_top, blown_top) = (centre(&columns(&still, top)), centre(&columns(&blown, top)));
    assert!(
        blown_top > still_top + 2.0,
        "the top leans downwind (+x): {still_top} -> {blown_top}"
    );
    let (still_foot, blown_foot) = (
        centre(&columns(&still, foot)),
        centre(&columns(&blown, foot)),
    );
    assert!(
        (blown_foot - still_foot).abs() < 1.0,
        "the foot stays: {still_foot} -> {blown_foot}"
    );
    assert_eq!(
        rock, still,
        "what has no wind in its material does not move"
    );

    // A bender at the post's foot pushes it down: its top row goes dark.
    let bent = shoot(
        &mut renderer,
        post(2.0),
        Wind {
            strength: 0.0,
            ..gale
        },
        vec![Bender {
            position: Vec3::new(-0.2, 0.0, 0.0),
            radius: 1.5,
        }],
    );
    let highest = |p: &[u8]| {
        (0..SIZE)
            .find(|&y| !columns(p, y).is_empty())
            .unwrap_or(SIZE)
    };
    assert!(
        highest(&bent) > highest(&still) + 2,
        "pushed down: its top from row {} to {}",
        highest(&still),
        highest(&bent)
    );
}

#[test]
fn a_leaf_with_the_sun_behind_it_is_lit_through() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let leaf = |translucency: f32| Draw {
        mesh: plane,
        // Standing up, facing the camera (+z); the sun beyond it.
        transform: Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * Mat4::from_scale(Vec3::splat(4.0)),
        texture: TextureHandle::WHITE,
        material: Material {
            translucency,
            render_face: runity::material::RenderFace::Both,
            ..Material::new(0.3, 0.8, 0.2)
        },
        pose: None,
    };
    let shoot = |renderer: &mut Renderer, draw: Draw| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 0.0, 5.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            lighting: Lighting {
                // Travelling towards the camera: the sun is behind the leaf.
                sun_direction: Vec3::new(0.0, -0.2, 1.0).normalize(),
                sun_intensity: 1.0,
                sky_color: Vec3::ZERO,
                ground_color: Vec3::ZERO,
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: vec![draw],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let opaque = shoot(&mut renderer, leaf(0.0));
    let through = shoot(&mut renderer, leaf(1.0));
    assert!(
        opaque[1] < 20,
        "an opaque leaf, backlit, is dark: {opaque:?}"
    );
    assert!(
        through[1] > opaque[1] + 60 && through[1] > through[0],
        "lit through, green: {through:?}"
    );
}
