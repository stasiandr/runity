//! Water: over shallows the floor shows through, over the deep the water's
//! own colour takes over; and without water both floors look the same.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

#[test]
fn the_floor_shows_through_shallow_water_and_not_deep() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let floor = |x: f32, depth: f32| Draw {
        mesh: plane,
        transform: Mat4::from_translation(Vec3::new(x, -depth, 0.0))
            * Mat4::from_scale(Vec3::new(4.0, 1.0, 8.0)),
        texture: TextureHandle::WHITE,
        material: Material::new(0.9, 0.9, 0.9),
        pose: None,
    };
    let water = Draw {
        mesh: plane,
        transform: Mat4::from_scale(Vec3::new(8.0, 1.0, 8.0)),
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Water,
            clarity: 1.0,
            foam: 0.0,
            ..Material::new(0.02, 0.1, 0.15)
        },
        pose: None,
    };
    let shoot = |renderer: &mut Renderer, with_water: bool| {
        // Shallow on the left, deep on the right.
        let mut draws = vec![floor(-2.0, 0.15), floor(2.0, 4.0)];
        if with_water {
            draws.push(water);
        }
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 6.0, 0.01),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(0.1, -1.0, 0.1).normalize(),
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws,
            time: Some(0.0),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    let (left, right) = (SIZE / 4, SIZE * 3 / 4);
    let dry = shoot(&mut renderer, false);
    let wet = shoot(&mut renderer, true);
    let at = |p: &[u8], x: u32| sum(OffscreenTarget::pixel(p, SIZE, x, SIZE / 2));
    assert!(
        (at(&dry, left) as i32 - at(&dry, right) as i32).abs() < 60,
        "dry, the two floors are alike: {} {}",
        at(&dry, left),
        at(&dry, right)
    );
    assert!(
        at(&wet, left) > at(&wet, right) + 150,
        "under water, the shallow floor shows and the deep does not: {} {}",
        at(&wet, left),
        at(&wet, right)
    );
    let deep = OffscreenTarget::pixel(&wet, SIZE, right, SIZE / 2);
    assert!(deep[2] > deep[0], "the deep is the water's blue: {deep:?}");
}
