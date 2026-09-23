//! Lamps as URP's Forward+ has them: a point and a spot shadowed by a wall
//! between them and the floor, by their own shadow maps — no rays — and a
//! floor lit by many more lamps than a fixed handful.

use runity::glam::{Mat4, Vec3};
use runity::render::{
    Camera, Draw, Frame, Lighting, PointLight, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

fn dark_scene(camera: Camera, lights: Vec<PointLight>, draws: Vec<Draw>) -> Frame {
    Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera,
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::default(),
        clear_color: Vec3::ZERO,
        lights,
        draws,
        ..Frame::default()
    }
}

fn brightness(pixels: &[u8], x: u32, y: u32) -> u32 {
    let p = OffscreenTarget::pixel(pixels, SIZE, x, y);
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

/// Looking down on a floor with a lamp on one side of a wall: how bright
/// the floor is beyond it, where the wall stands between, and on the
/// lamp's own side.
fn across_the_wall(gpu: &Gpu, lamp: PointLight) -> (u32, u32) {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(gpu, &target);
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let draw = |mesh, transform| Draw {
        mesh,
        transform,
        texture: TextureHandle::WHITE,
        material: Material::new(0.8, 0.8, 0.8),
        pose: None,
    };
    let frame = dark_scene(
        Camera {
            position: Vec3::new(0.0, 7.0, 0.01),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        vec![lamp],
        vec![
            draw(plane, Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0))),
            // A wall along z at x = 0.
            draw(
                cube,
                Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::new(0.2, 2.0, 10.0)),
            ),
        ],
    );
    renderer.render(gpu, &target, &frame);
    let pixels = target.read_rgba(gpu);
    // The view is about 7 m across: a sixth of it is a metre and a bit.
    let beyond = brightness(&pixels, SIZE / 2 + SIZE / 5, SIZE / 2);
    let near_side = brightness(&pixels, SIZE / 2 - SIZE / 5, SIZE / 2);
    (beyond, near_side)
}

#[test]
fn a_wall_shadows_a_point_lamp_and_a_spot_behind_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let point = PointLight {
        position: Vec3::new(-1.0, 0.6, 0.0),
        color: Vec3::splat(3.0),
        range: 8.0,
        spot: None,
        shadows: true,
    };
    let (beyond, near_side) = across_the_wall(&gpu, point);
    assert!(near_side > 60, "the lamp's own side is lit: {near_side}");
    assert!(beyond < 10, "the wall shadows the far side: {beyond}");
    let (through, _) = across_the_wall(
        &gpu,
        PointLight {
            shadows: false,
            ..point
        },
    );
    assert!(
        through > 60,
        "with shadows off it lights through: {through}"
    );

    // A spot aimed down past the wall, at the far side.
    let spot = PointLight {
        spot: Some((Vec3::new(1.0, -0.6, 0.0).normalize(), 90.0)),
        ..point
    };
    let (beyond, _) = across_the_wall(&gpu, spot);
    let (through, _) = across_the_wall(
        &gpu,
        PointLight {
            shadows: false,
            ..spot
        },
    );
    assert!(
        through > 60,
        "unshadowed, the spot reaches past the wall: {through}"
    );
    assert!(beyond < 10, "shadowed, the wall stops it: {beyond}");
}

#[test]
fn many_lamps_all_light_the_floor() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    // Eight by eight lamps, a metre and a half apart, each lighting a small
    // pool: sixty-four, far past the eight the renderer once had.
    let lights: Vec<PointLight> = (0..64)
        .map(|i| PointLight {
            position: Vec3::new(
                (i % 8) as f32 * 1.5 - 5.25,
                0.4,
                (i / 8) as f32 * 1.5 - 5.25,
            ),
            color: Vec3::splat(2.0),
            range: 1.0,
            spot: None,
            shadows: false,
        })
        .collect();
    let frame = dark_scene(
        Camera {
            position: Vec3::new(0.0, 14.0, 0.01),
            target: Vec3::ZERO,
            fov_y_degrees: 50.0,
            ..Camera::default()
        },
        lights.clone(),
        vec![Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.8, 0.8, 0.8),
            pose: None,
        }],
    );
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);
    let size = glam_size();
    let mut lit = 0;
    for light in &lights {
        let Some(at) = frame.camera.screen_point(light.position.with_y(0.0), size) else {
            continue;
        };
        if brightness(&pixels, at.x as u32, at.y as u32) > 60 {
            lit += 1;
        }
    }
    assert_eq!(lit, lights.len(), "every lamp lights its pool");
}

fn glam_size() -> runity::glam::Vec2 {
    runity::glam::Vec2::splat(SIZE as f32)
}
