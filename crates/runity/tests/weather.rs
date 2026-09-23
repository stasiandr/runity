//! Weather: a wet floor is darker, snow whitens what faces up and not what
//! stands up, and rain draws streaks over the view.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::weather::Weather;
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

#[test]
fn wet_darkens_snow_whitens_what_faces_up_and_rain_streaks_the_view() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let stone = Material::new(0.4, 0.35, 0.3);
    let draws = vec![
        Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: stone,
            pose: None,
        },
        // A wall facing the camera, on the right half of the view.
        Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(2.0, 1.0, -1.0))
                * Mat4::from_scale(Vec3::new(3.0, 2.0, 0.2)),
            texture: TextureHandle::WHITE,
            material: stone,
            pose: None,
        },
    ];
    let shoot = |renderer: &mut Renderer, weather: Weather| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 1.5, 4.0),
                target: Vec3::new(0.0, 0.5, 0.0),
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(0.2, -1.0, -0.3).normalize(),
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: draws.clone(),
            weather,
            time: Some(0.5),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    let floor = |p: &[u8]| sum(OffscreenTarget::pixel(p, SIZE, SIZE / 4, SIZE * 7 / 8));
    let wall = |p: &[u8]| sum(OffscreenTarget::pixel(p, SIZE, SIZE * 3 / 4, SIZE / 2));

    let dry = shoot(&mut renderer, Weather::default());
    let wet = shoot(
        &mut renderer,
        Weather {
            wetness: 1.0,
            ..Default::default()
        },
    );
    assert!(
        floor(&wet) < floor(&dry) * 9 / 10,
        "wet is darker: {} -> {}",
        floor(&dry),
        floor(&wet)
    );

    let snowy = shoot(
        &mut renderer,
        Weather {
            snow: 1.0,
            ..Default::default()
        },
    );
    assert!(
        floor(&snowy) > floor(&dry) + 150,
        "snow on the floor: {} -> {}",
        floor(&dry),
        floor(&snowy)
    );
    assert!(
        wall(&snowy) < wall(&dry) + 20,
        "none on the wall's face: {} -> {}",
        wall(&dry),
        wall(&snowy)
    );

    // Rain over the black sky above the wall: some streaks, not a sheet.
    let rainy = shoot(
        &mut renderer,
        Weather {
            rain: 1.0,
            ..Default::default()
        },
    );
    let lit = (0..SIZE)
        .flat_map(|x| (0..SIZE / 5).map(move |y| (x, y)))
        .filter(|&(x, y)| sum(OffscreenTarget::pixel(&rainy, SIZE, x, y)) > 15)
        .count();
    let total = (SIZE * SIZE / 5) as usize;
    assert!(lit > 5 && lit < total / 2, "streaks: {lit} of {total}");
}

#[test]
fn a_sandstorm_hides_the_distance_in_sand_coloured_air() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // A dark blue wall a hundred metres off, filling the view.
    let wall = Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0))
            * Mat4::from_scale(Vec3::new(400.0, 400.0, 1.0)),
        texture: TextureHandle::WHITE,
        material: Material::new(0.05, 0.1, 0.4),
        pose: None,
    };
    let shoot = |renderer: &mut Renderer, sandstorm: f32| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 1.7, 0.0),
                target: Vec3::new(0.0, 1.7, -1.0),
                ..Camera::default()
            },
            fog: runity::render::FogSettings {
                start: 1000.0,
                end: 2000.0,
                ..Default::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: vec![wall],
            weather: Weather {
                sandstorm,
                ..Default::default()
            },
            time: Some(1.0),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let clear = shoot(&mut renderer, 0.0);
    let storm = shoot(&mut renderer, 1.0);
    assert!(clear[2] > clear[0], "a blue wall on a clear day: {clear:?}");
    assert!(
        storm[0] > storm[2],
        "gone into sand-coloured air: {storm:?}"
    );
}
