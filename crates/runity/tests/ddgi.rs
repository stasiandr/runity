//! DDGI: an irradiance volume's probes, lit by rays, darken a closed room
//! the sky cannot reach and carry a sunlit red floor's colour onto a wall
//! in shade.

use runity::ddgi::{IrradianceVolume, PlacedVolume};
use runity::glam::{Mat4, Quat, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

fn slab(mesh: runity::render::MeshHandle, centre: Vec3, size: Vec3, colour: [f32; 3]) -> Draw {
    Draw {
        mesh,
        transform: Mat4::from_scale_rotation_translation(size, Quat::IDENTITY, centre),
        texture: TextureHandle::WHITE,
        material: Material::new(colour[0], colour[1], colour[2]),
        pose: None,
    }
}

/// Mean linear-ish colour of the middle of the picture.
fn middle(pixels: &[u8]) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut n = 0.0;
    for y in SIZE / 3..SIZE * 2 / 3 {
        for x in SIZE / 3..SIZE * 2 / 3 {
            let p = OffscreenTarget::pixel(pixels, SIZE, x, y);
            for c in 0..3 {
                sum[c] += p[c] as f32;
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn shot(gpu: &Gpu, target: &OffscreenTarget, draws: impl Fn(runity::render::MeshHandle) -> Vec<Draw>, camera: Camera, volume: Option<PlacedVolume>, lighting: Lighting) -> [f32; 3] {
    let mut renderer = Renderer::new(gpu, target);
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera,
        lighting,
        draws: draws(cube),
        irradiance_volumes: volume.into_iter().collect(),
        ..Frame::default()
    };
    for _ in 0..40 {
        renderer.render(gpu, target, &frame);
    }
    middle(&target.read_rgba(gpu))
}

#[test]
fn probes_darken_a_closed_room_and_carry_a_red_floors_light_onto_a_wall() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let white = [0.8, 0.8, 0.8];
    let volume = |size: Vec3, centre: Vec3| PlacedVolume {
        centre,
        volume: IrradianceVolume {
            size,
            spacing: 1.0,
            ..Default::default()
        },
    };

    // A closed room, 6 × 3 × 6: no sun gets in, and no sky.
    let room = |cube| {
        vec![
            slab(cube, Vec3::new(0.0, -0.1, 0.0), Vec3::new(6.4, 0.2, 6.4), white),
            slab(cube, Vec3::new(0.0, 3.1, 0.0), Vec3::new(6.4, 0.2, 6.4), white),
            slab(cube, Vec3::new(-3.1, 1.5, 0.0), Vec3::new(0.2, 3.0, 6.4), white),
            slab(cube, Vec3::new(3.1, 1.5, 0.0), Vec3::new(0.2, 3.0, 6.4), white),
            slab(cube, Vec3::new(0.0, 1.5, -3.1), Vec3::new(6.4, 3.0, 0.2), white),
            slab(cube, Vec3::new(0.0, 1.5, 3.1), Vec3::new(6.4, 3.0, 0.2), white),
        ]
    };
    let inside = Camera {
        position: Vec3::new(0.0, 1.5, 2.0),
        target: Vec3::new(0.0, 1.5, -3.0),
        ..Camera::default()
    };
    let bright = Lighting {
        sky_color: Vec3::splat(0.6),
        ..Lighting::default()
    };
    let sky_lit = shot(&gpu, &target, room, inside, None, bright);
    let probed = shot(&gpu, &target, room, inside, Some(volume(Vec3::new(5.0, 2.0, 5.0), Vec3::new(0.0, 1.5, 0.0))), bright);
    eprintln!("closed room: hemisphere {sky_lit:?}, probes {probed:?}");
    assert!(probed[1] < sky_lit[1] * 0.35, "the sky does not get in: {probed:?} vs {sky_lit:?}");

    // Open to the sky: a red floor in the sun, a white wall beside it in
    // its own shade (the sun behind it), looked at from the floor's side.
    let yard = |cube| {
        vec![
            slab(cube, Vec3::new(0.0, -0.1, 0.0), Vec3::new(8.0, 0.2, 8.0), [0.9, 0.05, 0.03]),
            slab(cube, Vec3::new(0.0, 1.5, -2.0), Vec3::new(6.0, 3.0, 0.2), white),
        ]
    };
    let facing = Camera {
        position: Vec3::new(0.0, 1.5, 3.0),
        target: Vec3::new(0.0, 1.5, -2.0),
        ..Camera::default()
    };
    let sun_behind = Lighting {
        sun_direction: Vec3::new(0.0, -0.9, 0.44).normalize(),
        sun_intensity: 3.0,
        ..Lighting::default()
    };
    let plain = shot(&gpu, &target, yard, facing, None, sun_behind);
    let bounced = shot(&gpu, &target, yard, facing, Some(volume(Vec3::new(8.0, 3.0, 8.0), Vec3::new(0.0, 1.5, 0.0))), sun_behind);
    eprintln!("wall in shade: hemisphere {plain:?}, probes {bounced:?}");
    let red = |c: [f32; 3]| c[0] / c[2].max(1.0);
    assert!(red(bounced) > red(plain) * 1.3, "the floor's red on the wall: {bounced:?} vs {plain:?}");
    assert!(bounced[0] > plain[0], "and more light: {bounced:?} vs {plain:?}");
}
