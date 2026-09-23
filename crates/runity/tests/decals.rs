//! Decals: a picture pressed onto the floor in its box — lit as the floor
//! is, cut where its alpha says, left off what does not face the way it is
//! pressed.

use runity::asset::{AssetId, AssetKind, TextureAsset};
use runity::decals::Decal;
use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

/// Left half clear, right half opaque white — as an imported texture is.
fn half_texture() -> Vec<u8> {
    let mut pixels = Vec::new();
    for _y in 0..8 {
        for x in 0..8 {
            pixels.extend_from_slice(if x < 4 {
                &[255, 255, 255, 0]
            } else {
                &[255, 255, 255, 255]
            });
        }
    }
    let asset = TextureAsset {
        id: AssetId(7),
        name: "half".into(),
        width: 8,
        height: 8,
        pixels,
        mips: Vec::new(),
        srgb: true,
    };
    runity::asset::to_bytes(&asset, AssetKind::Texture).unwrap()
}

#[test]
fn a_decal_paints_the_floor_in_its_box_and_nothing_else() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let bytes = half_texture();
    renderer.upload_texture(&gpu, runity::asset::view::<TextureAsset>(&bytes).unwrap());
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let grey = Material::new(0.8, 0.8, 0.8);
    let draws = vec![
        Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: grey,
            pose: None,
        },
        // A thin wall standing up at z = -1.6, out of the square but inside a
        // taller box.
        Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(0.0, 0.5, -1.6))
                * Mat4::from_scale(Vec3::new(1.0, 1.0, 0.05)),
            texture: TextureHandle::WHITE,
            material: grey,
            pose: None,
        },
    ];
    let red = Material::new(1.0, 0.0, 0.0);
    let frame = |decals: Vec<Decal>| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 6.0, 0.01),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.0, -1.0, 0.1),
            sun_intensity: 1.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws: draws.clone(),
        decals,
        ..Frame::default()
    };
    let box_of = |size: Vec3| Mat4::from_scale(size);
    let pixel = |renderer: &mut Renderer, decals: Vec<Decal>, x: u32, y: u32| {
        renderer.render(&gpu, &target, &frame(decals));
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, x, y)
    };
    let middle = (SIZE / 2, SIZE / 2);
    let plain = pixel(&mut renderer, Vec::new(), middle.0, middle.1);
    assert!(
        plain[0] > 100 && (plain[0] as i32 - plain[1] as i32).abs() < 10,
        "{plain:?}"
    );

    // A red square, 2 m, on the floor.
    let square = Decal {
        transform: box_of(Vec3::new(2.0, 1.0, 2.0)),
        material: red,
    };
    let inside = pixel(&mut renderer, vec![square], middle.0, middle.1);
    assert!(
        inside[0] > 100 && inside[1] < 30,
        "red where it lands: {inside:?}"
    );
    // Two metres from the middle is well outside a two-metre square: the
    // view is about six across.
    let outside = pixel(&mut renderer, vec![square], SIZE / 2 + SIZE / 3, middle.1);
    assert!(
        (outside[0] as i32 - outside[1] as i32).abs() < 10,
        "grey past it: {outside:?}"
    );

    // Its picture's clear half leaves the floor as it was.
    let halved = Decal {
        material: Material {
            base_map: Some(AssetId(7)),
            ..red
        },
        ..square
    };
    let left = pixel(&mut renderer, vec![halved], SIZE / 2 - SIZE / 12, middle.1);
    let right = pixel(&mut renderer, vec![halved], SIZE / 2 + SIZE / 12, middle.1);
    assert!(
        (left[0] as i32 - left[1] as i32).abs() < 10,
        "clear: {left:?}"
    );
    assert!(right[0] > 100 && right[1] < 30, "opaque: {right:?}");

    // A tall box that also holds the wall's face: the wall stands across
    // the way the decal is pressed, and keeps its grey. Seen from above,
    // the wall's top is what shows — and that faces up, so it is painted;
    // it is the wall's side that must not be. Looked at from the side
    // instead:
    let tall = Decal {
        transform: box_of(Vec3::new(4.0, 3.0, 4.0)),
        material: red,
    };
    renderer.render(
        &gpu,
        &target,
        &Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.5, 3.0),
                target: Vec3::new(0.0, 0.5, -1.6),
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(0.0, -0.5, -1.0).normalize(),
                sun_intensity: 1.0,
                sky_color: Vec3::ZERO,
                ground_color: Vec3::ZERO,
                ..Lighting::default()
            },
            ..frame(vec![tall])
        },
    );
    let wall = OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, middle.0, middle.1);
    assert!(
        wall[0] > 40 && (wall[0] as i32 - wall[1] as i32).abs() < 10,
        "the wall's lit side is not painted: {wall:?}"
    );
}
