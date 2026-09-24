//! What casts a shadow and what the shadow looks like: a card cut out by
//! its alpha throws a shadow with the same holes, glass throws none, and
//! the shadow of something far down the view is still there — the far
//! cascades' job.

use scrap::glam::{Mat4, Vec3};
use scrap::material::SurfaceType;
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

/// A white floor under a sun straight down, with `card` a metre above it,
/// seen from half a metre up looking straight down — under the card, so the
/// card itself is behind the camera and only its shadow is in the picture.
fn shadow_under(
    card: Material,
    texture: impl FnOnce(&mut Renderer, &Gpu) -> TextureHandle,
) -> Option<Vec<u8>> {
    let gpu = Gpu::headless_blocking(false).ok()?;
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let texture = texture(&mut renderer, &gpu);
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 0.5, 0.01),
            target: Vec3::ZERO,
            fov_y_degrees: 120.0,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.0, -1.0, 0.001).normalize(),
            sun_intensity: 1.0,
            sky_color: Vec3::splat(0.1),
            ground_color: Vec3::splat(0.1),
            ..Lighting::default()
        },
        shadows: ShadowSettings::default(),
        clear_color: Vec3::ZERO,
        draws: vec![
            Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
                texture: TextureHandle::WHITE,
                material: Material::new(1.0, 1.0, 1.0),
                pose: None,
            },
            Draw {
                mesh: plane,
                transform: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::new(2.0, 1.0, 2.0)),
                texture,
                material: card,
                pose: None,
            },
        ],
        ..Frame::default()
    };
    renderer.render(&gpu, &target, &frame);
    Some(target.read_rgba(&gpu))
}

/// How many of the middle pixels are dark (shadowed) and how many lit.
fn census(pixels: &[u8]) -> (usize, usize) {
    let (mut dark, mut lit) = (0, 0);
    for y in SIZE / 4..SIZE * 3 / 4 {
        for x in SIZE / 4..SIZE * 3 / 4 {
            let p = OffscreenTarget::pixel(pixels, SIZE, x, y);
            if p[0] < 110 {
                dark += 1;
            } else if p[0] > 180 {
                lit += 1;
            }
        }
    }
    (dark, lit)
}

/// Opaque squares and holes, eight texels each.
fn checker(renderer: &mut Renderer, gpu: &Gpu) -> TextureHandle {
    let n = 64;
    let mut pixels = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let solid = ((x / 8) + (y / 8)) % 2 == 0;
            pixels.extend_from_slice(&[255, 255, 255, if solid { 255 } else { 0 }]);
        }
    }
    renderer.upload_texture_rgba(gpu, n as u32, n as u32, &pixels, true)
}

#[test]
fn a_card_cut_out_by_its_alpha_throws_a_shadow_with_the_same_holes() {
    let solid = Material::new(0.5, 0.5, 0.5);
    let cut = Material {
        alpha_clip: 0.5,
        render_face: scrap::material::RenderFace::Both,
        ..solid
    };
    let Some(whole) = shadow_under(solid, |_, _| TextureHandle::WHITE) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let holed = shadow_under(cut, checker).unwrap();
    let (whole_dark, whole_lit) = census(&whole);
    let (holed_dark, holed_lit) = census(&holed);
    assert!(
        whole_dark > whole_lit * 4,
        "a solid card shadows the floor: {whole_dark} dark, {whole_lit} lit"
    );
    assert!(
        holed_lit > holed_dark / 3 && holed_dark > holed_lit / 3,
        "a cut-out card lets light through its holes: {holed_dark} dark, {holed_lit} lit"
    );
}

#[test]
fn glass_throws_no_shadow() {
    let glass = Material {
        alpha: 0.3,
        surface: SurfaceType::Transparent,
        ..Material::new(0.5, 0.8, 1.0)
    };
    let Some(pixels) = shadow_under(glass, |_, _| TextureHandle::WHITE) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let (dark, lit) = census(&pixels);
    assert!(
        lit > dark * 10,
        "the floor under glass is lit: {dark} dark, {lit} lit"
    );
}

#[test]
fn the_shadow_distance_is_split_into_cascades_finest_first() {
    let settings = ShadowSettings::default();
    let ends = settings.cascade_ends();
    assert_eq!(ends.len(), 4);
    assert!(ends.windows(2).all(|w| w[0] < w[1]), "{ends:?}");
    assert_eq!(*ends.last().unwrap(), settings.max_distance);
    let two = ShadowSettings {
        cascades: 2,
        ..settings
    }
    .cascade_ends();
    assert_eq!(
        two,
        vec![settings.max_distance * 0.25, settings.max_distance]
    );
}

/// A pebble on the floor, too small for a coarse shadow map to hold, under
/// a low sun: with contact shadows the floor just behind it, away from the
/// sun, is dark; without them only its own shaded sides are.
#[test]
fn contact_shadows_hold_the_dark_behind_a_pebble_the_map_misses() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |contact: f32| {
        let mut renderer = Renderer::new(&gpu, &target);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            // Looking down at the pebble and the floor past it, from the side.
            camera: Camera {
                position: Vec3::new(0.05, 0.3, 0.35),
                target: Vec3::new(0.08, 0.0, 0.0),
                ..Camera::default()
            },
            lighting: Lighting {
                // Low from −x: its shadow falls toward +x.
                sun_direction: Vec3::new(1.0, -0.35, 0.0).normalize(),
                sky_color: Vec3::splat(0.05),
                ground_color: Vec3::splat(0.05),
                ..Lighting::default()
            },
            // A map far too coarse for a pebble.
            shadows: ShadowSettings {
                resolution: 64,
                contact,
                ..ShadowSettings::default()
            },
            clear_color: Vec3::ZERO,
            draws: vec![
                Draw {
                    mesh: plane,
                    transform: Mat4::from_scale(Vec3::splat(20.0)),
                    texture: TextureHandle::WHITE,
                    material: Material::new(0.8, 0.8, 0.8),
                    pose: None,
                },
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(0.0, 0.03, 0.0))
                        * Mat4::from_scale(Vec3::splat(0.06)),
                    texture: TextureHandle::WHITE,
                    material: Material::new(0.8, 0.8, 0.8),
                    pose: None,
                },
            ],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        let pixels = target.read_rgba(&gpu);
        // How many pixels are dark: the pebble's own shaded sides in any
        // case, and its shadow on the floor where it is held.
        let open = OffscreenTarget::pixel(&pixels, SIZE, SIZE / 10, SIZE / 10)[1] as i32;
        let dark = pixels.chunks(4).filter(|p| (p[1] as i32) < open / 2).count();
        if let Ok(dir) = std::env::var("DUMP") {
            image::save_buffer(format!("{dir}/contact_{contact}.png"), &pixels, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        }
        (dark, open)
    };
    let (without, _) = shot(0.0);
    let (with, _) = shot(0.35);
    eprintln!("dark pixels: without {without}, with {with}");
    assert!(
        with > without + 20,
        "contact shadows lay a shadow the map misses: {with} dark pixels against {without}"
    );
}
