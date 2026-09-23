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

#[test]
fn a_dust_wall_stands_upwind_behind_what_is_in_front_of_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // A blue post twenty metres upwind, off to the side.
    let post = Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(-20.0, 5.0, -4.0))
            * Mat4::from_scale(Vec3::new(1.0, 10.0, 1.0)),
        texture: TextureHandle::WHITE,
        material: runity::Material {
            shading: runity::material::Shading::Unlit,
            ..Material::new(0.1, 0.2, 0.9)
        },
        pose: None,
    };
    let shoot = |renderer: &mut Renderer, wall: f32| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Physical,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            // Looking upwind (the wind blows along +x), a little up.
            camera: Camera {
                position: Vec3::new(0.0, 1.7, 0.0),
                target: Vec3::new(-10.0, 3.0, 0.0),
                ..Camera::default()
            },
            wind: runity::foliage::Wind {
                direction: Vec3::X,
                strength: 1.0,
            },
            shadows: ShadowSettings::OFF,
            draws: vec![post],
            weather: Weather {
                dust_wall: wall,
                dust_wall_distance: 400.0,
                ..Default::default()
            },
            time: Some(0.0),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let clear = shoot(&mut renderer, 0.0);
    let dusty = shoot(&mut renderer, 1.0);
    let sky = |p: &[u8]| OffscreenTarget::pixel(p, SIZE, SIZE / 4, SIZE * 2 / 5);
    assert!(
        sky(&clear)[2] > sky(&clear)[0],
        "a blue sky upwind: {:?}",
        sky(&clear)
    );
    assert!(
        sky(&dusty)[0] > sky(&dusty)[2],
        "a wall of dust there: {:?}",
        sky(&dusty)
    );
    // The post, twenty metres off, stands in front of the wall.
    let post_at = |p: &[u8]| {
        (0..SIZE)
            .map(|x| OffscreenTarget::pixel(p, SIZE, x, SIZE / 2))
            .find(|c| c[2] as i32 > c[0] as i32 + 100)
    };
    assert!(post_at(&clear).is_some(), "the post is there");
    assert!(
        post_at(&dusty).is_some(),
        "and still blue in front of the wall"
    );
}

/// Clay looked down on: while it is wet an even dark mud, and once the
/// ground has dried cracked into plates — many pixels far darker than the
/// plates round them. Wet sand dries patchily: half way, some of it dry
/// and some still dark.
#[test]
fn clay_cracks_as_it_dries_and_the_ground_dries_in_patches() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    const WIDE: u32 = 96;
    let target = OffscreenTarget::new(&gpu, WIDE, WIDE);
    let shot = |material: Material, weather: Weather| {
        let mut renderer = Renderer::new(&gpu, &target);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let frame = Frame {
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
            lighting: Lighting::default(),
            shadows: ShadowSettings::OFF,
            weather,
            draws: vec![Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(40.0, 1.0, 40.0)),
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        let pixels = target.read_rgba(&gpu);
        let mut luma: Vec<u32> = pixels.chunks(4).map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32).collect();
        luma.sort();
        luma
    };
    let clay = Material {
        clay: true,
        ..Material::new(0.7, 0.5, 0.35)
    };
    let wet = Weather {
        wetness: 1.0,
        ..Weather::default()
    };
    let dry = Weather {
        drying: 1.0,
        ..wet
    };
    // How much darker the darkest tenth is than the middle.
    let spread = |l: &[u32]| l[l.len() / 2] as f32 / l[l.len() / 10].max(1) as f32;
    let mud = shot(clay, wet);
    let cracked = shot(clay, dry);
    assert!(spread(&mud) < 1.2, "wet mud is even: {}", spread(&mud));
    assert!(spread(&cracked) > 1.8, "dry clay is cracked: {}", spread(&cracked));
    assert!(mud[mud.len() / 2] < cracked[cracked.len() / 2], "mud is darker than dry clay");

    // A plain ground half dried: patches of both.
    let ground = Material::new(0.7, 0.55, 0.4);
    let half = shot(ground, Weather { drying: 0.5, ..wet });
    let (still_wet, gone_dry) = (shot(ground, wet), shot(ground, dry));
    let middle = |l: &[u32]| l[l.len() / 2];
    let darkest = half[half.len() / 20];
    let brightest = half[half.len() * 19 / 20];
    assert!(
        (darkest as f32) < middle(&still_wet) as f32 * 1.15 && brightest as f32 > middle(&gone_dry) as f32 * 0.85,
        "half dried: {darkest}..{brightest}, wet {} dry {}",
        middle(&still_wet),
        middle(&gone_dry)
    );
}
