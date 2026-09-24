//! Hardware ray tracing, the experiment: a lamp behind a wall does not
//! light the floor on the far side when rays are asked for, and does
//! without them. Skipped on a device that does not trace.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{
    Camera, Draw, Frame, Lighting, PointLight, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 48;

/// Looking down on a floor with a lamp on one side of a wall: how bright
/// the floor is on the other side, where the wall stands between.
fn behind_the_wall(gpu: &Gpu, rays: scrap::ray::RayTracing) -> u32 {
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
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
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
    let through = behind_the_wall(&gpu, scrap::ray::RayTracing::default());
    let blocked = behind_the_wall(
        &gpu,
        scrap::ray::RayTracing {
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
    use scrap::terrain::{Dunes, Terrain, TerrainSurface};
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
        shading: scrap::material::Shading::Sand,
        ..Material::new(0.8, 0.62, 0.42)
    };
    let shot = |rays: scrap::ray::RayTracing| {
        let mut renderer = Renderer::new(&gpu, &target);
        let mesh = renderer.upload_mesh_owned(&gpu, &terrain.mesh());
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
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
    let cascades = shot(scrap::ray::RayTracing::default());
    let traced = shot(scrap::ray::RayTracing {
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

/// A frame for the mirror and the lens: a black sky, no sun, and a light
/// from all round so what the rays find shows its colour.
fn lit_by_all_round(camera: Camera, rays: scrap::ray::RayTracing, draws: Vec<Draw>) -> Frame {
    Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        ray_tracing: rays,
        camera,
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::splat(1.0),
            ground_color: Vec3::splat(1.0),
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws,
        ..Frame::default()
    }
}

/// A chrome ball in front of the camera, a red block behind it: by rays the
/// ball shows the block, which is not on the screen for anything else to
/// read.
#[test]
fn a_mirror_shows_what_is_behind_the_camera_only_with_rays() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |rays: scrap::ray::RayTracing| {
        let mut renderer = Renderer::new(&gpu, &target);
        let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 48, 24));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = lit_by_all_round(
            Camera {
                position: Vec3::new(0.0, 0.0, 3.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            rays,
            vec![
                Draw {
                    mesh: ball,
                    transform: Mat4::IDENTITY,
                    texture: TextureHandle::WHITE,
                    material: Material {
                        metallic: 1.0,
                        smoothness: 1.0,
                        ..Material::new(0.95, 0.95, 0.95)
                    },
                    pose: None,
                },
                // Behind the camera, where the ball's middle looks.
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(0.0, 0.0, 6.0))
                        * Mat4::from_scale(Vec3::splat(3.0)),
                    texture: TextureHandle::WHITE,
                    material: Material::new(0.9, 0.05, 0.05),
                    pose: None,
                },
            ],
        );
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let without = shot(scrap::ray::RayTracing::default());
    let with = shot(scrap::ray::RayTracing {
        reflections: true,
        ..Default::default()
    });
    let red = |p: [u8; 4]| p[0] as i32 - (p[1] as i32 + p[2] as i32) / 2;
    assert!(red(without) < 30, "without rays the ball does not see it: {without:?}");
    assert!(red(with) > 60, "by rays the ball shows the red block: {with:?}");
}

/// A glass ball before a wall red on the left and blue on the right: by
/// rays the ball is a lens and turns them round, so just left of its
/// middle it shows blue.
#[test]
fn a_glass_ball_is_a_lens_only_with_rays() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |rays: scrap::ray::RayTracing| {
        let mut renderer = Renderer::new(&gpu, &target);
        let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 48, 24));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let wall = |x: f32, color: Material| Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(x, 0.0, -4.0))
                * Mat4::from_scale(Vec3::new(8.0, 8.0, 0.2)),
            texture: TextureHandle::WHITE,
            material: color,
            pose: None,
        };
        let frame = lit_by_all_round(
            Camera {
                position: Vec3::new(0.0, 0.0, 3.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            rays,
            vec![
                wall(-4.0, Material::new(0.9, 0.05, 0.05)),
                wall(4.0, Material::new(0.05, 0.05, 0.9)),
                Draw {
                    mesh: ball,
                    transform: Mat4::IDENTITY,
                    texture: TextureHandle::WHITE,
                    material: Material {
                        alpha: 0.1,
                        surface: scrap::material::SurfaceType::Transparent,
                        smoothness: 1.0,
                        ..Material::new(1.0, 1.0, 1.0)
                    },
                    pose: None,
                },
            ],
        );
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2 - SIZE / 10, SIZE / 2)
    };
    let without = shot(scrap::ray::RayTracing::default());
    let with = shot(scrap::ray::RayTracing {
        refractions: true,
        ..Default::default()
    });
    assert!(without[0] > without[2], "unbent, left of middle is the red wall: {without:?}");
    assert!(with[2] > with[0], "through the lens, the blue wall: {with:?}");
}

/// Mist round a lamp with a wall beside it, no shadow map: by rays the
/// mist behind the wall is dark — the lamp's light stops at the wall in
/// the air as on the ground — and without them it glows through.
#[test]
fn a_wall_shadows_the_mist_behind_it_only_with_rays() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |rays: scrap::ray::RayTracing| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            ray_tracing: rays,
            // Looking along the wall, the lamp to the left of it.
            camera: Camera {
                position: Vec3::new(0.0, 1.0, 6.0),
                target: Vec3::new(0.0, 1.0, 0.0),
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
            volumetric_fog: scrap::volume::VolumetricFog {
                enabled: true,
                density: 0.3,
                ambient: 0.0,
                lamps: 30.0,
                height_falloff: 0.0,
                ..Default::default()
            },
            lights: vec![PointLight {
                position: Vec3::new(-1.0, 1.0, 0.0),
                color: Vec3::splat(4.0),
                range: 8.0,
                spot: None,
                shadows: false,
            }],
            draws: vec![Draw {
                mesh: cube,
                // A wall along the view, between the lamp and x > 0.
                transform: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::new(0.1, 4.0, 11.0)),
                texture: TextureHandle::WHITE,
                material: Material::new(0.0, 0.0, 0.0),
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        let pixels = target.read_rgba(&gpu);
        // The mist right of the wall, level with the lamp.
        let p = OffscreenTarget::pixel(&pixels, SIZE, SIZE * 3 / 4, SIZE / 2);
        p[0] as u32 + p[1] as u32 + p[2] as u32
    };
    let through = shot(scrap::ray::RayTracing::default());
    let blocked = shot(scrap::ray::RayTracing {
        light_shadows: true,
        ..Default::default()
    });
    assert!(through > 40, "without rays the mist behind the wall glows: {through}");
    assert!(
        blocked * 3 < through,
        "by rays the wall shadows the mist: {blocked} against {through}"
    );
}
