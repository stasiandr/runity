//! Hardware ray tracing, the experiment: a lamp behind a wall does not
//! light the floor on the far side when rays are asked for, and does
//! without them. Skipped on a device that does not trace.

use runity::glam::{Mat4, Vec3};
use runity::render::{
    Camera, Draw, Frame, Lighting, PointLight, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 48;

/// Looking down on a floor with a lamp on one side of a wall: how bright
/// the floor is on the other side, where the wall stands between.
fn behind_the_wall(gpu: &Gpu, rays: runity::ray::RayTracing) -> u32 {
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
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: rays,
        camera: Camera {
            position: Vec3::new(1.5, 6.0, 0.01),
            target: Vec3::new(1.5, 0.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        lights: vec![PointLight {
            position: Vec3::new(-1.0, 0.5, 0.0),
            color: Vec3::splat(3.0),
            range: 8.0,
            spot: None,
            shadows: false,
        }],
        draws: vec![
            draw(plane, Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0))),
            // A wall along z, between the lamp and x > 0.
            draw(
                cube,
                Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::new(0.2, 2.0, 10.0)),
            ),
        ],
        ..Frame::default()
    };
    renderer.render(gpu, &target, &frame);
    let p = OffscreenTarget::pixel(&target.read_rgba(gpu), SIZE, SIZE / 2, SIZE / 2);
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

#[test]
fn a_lamp_behind_a_wall_is_shadowed_by_it_only_with_rays() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let through = behind_the_wall(&gpu, runity::ray::RayTracing::default());
    let blocked = behind_the_wall(
        &gpu,
        runity::ray::RayTracing {
            light_shadows: true,
            ..Default::default()
        },
    );
    assert!(
        through > 60,
        "without rays the lamp lights through the wall: {through}"
    );
    assert!(blocked < 10, "with rays the wall shadows it: {blocked}");
}

/// Sand under a low sun, by cascades and by rays: how many pixels the rays
/// make much darker. The rays see the terrain as its coarse heightfield,
/// which the fine, rippled sand dips under; unless they start past it, every
/// ripple is cracked with shadow.
#[test]
fn rays_do_not_crack_the_sand_with_shadow() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    use runity::terrain::{Dunes, Terrain, TerrainSurface};
    const WIDE: u32 = 128;
    let target = OffscreenTarget::new(&gpu, WIDE, WIDE);
    let terrain = Terrain {
        size: 200.0,
        cells: 160,
        dunes: Dunes {
            height: 9.0,
            wavelength: 55.0,
            ..Dunes::default()
        },
    };
    let sand = Material {
        shading: runity::material::Shading::Sand,
        ..Material::new(0.8, 0.62, 0.42)
    };
    let shot = |rays: runity::ray::RayTracing| {
        let mut renderer = Renderer::new(&gpu, &target);
        let mesh = renderer.upload_mesh_owned(&gpu, &terrain.mesh());
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            ray_tracing: rays,
            camera: Camera {
                position: Vec3::new(0.0, 2.5, 0.0),
                target: Vec3::new(-3.0, 0.0, -5.0),
                ..Camera::default()
            },
            // Low in the west, behind the camera's left shoulder.
            lighting: Lighting {
                sun_direction: Vec3::new(-1.0, -0.2, -0.4).normalize(),
                ..Lighting::default()
            },
            draws: vec![Draw {
                mesh,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: sand,
                pose: None,
            }],
            terrain: Some(TerrainSurface {
                mesh,
                placed: Mat4::IDENTITY,
                terrain,
            }),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let cascades = shot(runity::ray::RayTracing::default());
    let traced = shot(runity::ray::RayTracing {
        sun_shadows: true,
        ambient_occlusion: true,
        ..Default::default()
    });
    let luma = |p: &[u8]| p[0] as f32 + p[1] as f32 + p[2] as f32;
    let cracked = cascades
        .chunks(4)
        .zip(traced.chunks(4))
        .filter(|(c, t)| luma(t) < luma(c) * 0.7)
        .count();
    // None here; the cracked sand of rays that do not skip the gap is some
    // forty.
    assert!(
        cracked < 12,
        "{cracked} of {} pixels much darker by rays",
        WIDE * WIDE
    );
}
