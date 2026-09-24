//! A lamp's own lens flare: its ghosts cross the picture while the lamp
//! is in sight, and are gone when a wall stands in front of it.

use scrap::glam::{Mat4, Vec3};
use scrap::material::Shading;
use scrap::render::{Camera, Draw, Flare, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

#[test]
fn a_lamps_flare_shows_in_sight_and_not_behind_a_wall() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let sphere = renderer.upload_mesh_owned(&gpu, &builtin::sphere(0.5, 16, 8));
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let lamp_at = Vec3::new(-1.2, 0.8, -4.0);
    let camera = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        ..Camera::default()
    };
    let bulb = Draw {
        mesh: sphere,
        transform: Mat4::from_translation(lamp_at) * Mat4::from_scale(Vec3::splat(0.3)),
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Unlit,
            emission: [30.0, 30.0, 30.0],
            ..Material::new(1.0, 1.0, 1.0)
        },
        pose: None,
    };
    let wall = Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(-0.6, 0.4, -2.0))
            * Mat4::from_scale(Vec3::new(1.0, 1.0, 0.1)),
        texture: TextureHandle::WHITE,
        material: Material::new(0.0, 0.0, 0.0),
        pose: None,
    };
    // Where the second ghost falls: past the middle from the lamp.
    let clip = camera.view_projection(1.0) * lamp_at.extend(1.0);
    let (u, v) = (clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    let ghost = (u + (0.5 - u) * 1.2, v + (0.5 - v) * 1.2);
    let brightness = |renderer: &mut Renderer, flare: bool, walled: bool| {
        let mut draws = vec![bulb];
        if walled {
            draws.push(wall);
        }
        let frame = Frame {
            camera,
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            clear_color: Vec3::ZERO,
            lighting: Lighting {
                sun_intensity: 0.0,
                sky_color: Vec3::ZERO,
                ground_color: Vec3::ZERO,
                ..Lighting::default()
            },
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            draws,
            flares: if flare {
                vec![Flare {
                    position: lamp_at,
                    color: Vec3::ONE,
                    intensity: 2.0,
                }]
            } else {
                Vec::new()
            },
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        let rgba = target.read_rgba(&gpu);
        let (x, y) = (
            (ghost.0 * SIZE as f32) as u32,
            (ghost.1 * SIZE as f32) as u32,
        );
        let p = OffscreenTarget::pixel(&rgba, SIZE, x, y);
        p[0] as u32 + p[1] as u32 + p[2] as u32
    };
    let plain = brightness(&mut renderer, false, false);
    let flared = brightness(&mut renderer, true, false);
    assert!(
        flared > plain + 20,
        "a ghost where it falls: {plain} → {flared}"
    );
    let hidden_plain = brightness(&mut renderer, false, true);
    let hidden = brightness(&mut renderer, true, true);
    assert!(
        hidden <= hidden_plain + 3,
        "behind the wall, no flare: {hidden_plain} → {hidden}"
    );
}
