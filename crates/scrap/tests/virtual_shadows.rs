//! Virtual shadow maps: the sun's shadow from pages falls where the
//! cascades' does, and a still scene draws no pages once they are drawn —
//! a moved caster draws again only the few it was and is over.

use scrap::glam::{Mat4, Quat, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 128;

fn scene(cube: scrap::render::MeshHandle, box_at: Vec3, virtual_maps: bool) -> Frame {
    let solid = |transform| Draw {
        mesh: cube,
        transform,
        texture: TextureHandle::WHITE,
        material: Material::new(0.8, 0.8, 0.8),
        pose: None,
    };
    Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        shadows: ShadowSettings {
            virtual_maps,
            contact: 0.0,
            ..ShadowSettings::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(-0.4, -0.8, -0.3).normalize(),
            ..Lighting::default()
        },
        camera: Camera {
            position: Vec3::new(0.0, 7.0, 9.0),
            target: Vec3::new(0.0, 0.0, 0.0),
            ..Camera::default()
        },
        draws: vec![
            solid(Mat4::from_scale_rotation_translation(Vec3::new(30.0, 0.2, 30.0), Quat::IDENTITY, Vec3::new(0.0, -0.1, 0.0))),
            solid(Mat4::from_translation(box_at + Vec3::Y * 0.5)),
            solid(Mat4::from_scale_rotation_translation(Vec3::new(0.3, 3.0, 0.3), Quat::IDENTITY, Vec3::new(2.0, 1.5, -1.0))),
        ],
        ..Frame::default()
    }
}

/// Which pixels are in shadow: darker than the lit ground by a margin.
fn shadowed(pixels: &[u8]) -> Vec<bool> {
    let lit = (0..SIZE * SIZE)
        .map(|i| pixels[i as usize * 4] as u32)
        .max()
        .unwrap_or(0);
    (0..SIZE * SIZE)
        .map(|i| (pixels[i as usize * 4] as u32) < lit * 6 / 10)
        .collect()
}

#[test]
fn pages_shadow_where_the_cascades_do_and_a_still_scene_draws_none() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |virtual_maps: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = scene(cube, Vec3::ZERO, virtual_maps);
        for _ in 0..4 {
            renderer.render(&gpu, &target, &frame);
        }
        (target.read_rgba(&gpu), renderer, cube)
    };
    let (cascaded, _, _) = shot(false);
    let (paged, mut renderer, cube) = shot(true);
    if let Ok(dir) = std::env::var("VSM_DUMP") {
        std::fs::write(format!("{dir}/cascaded.rgba"), &cascaded).unwrap();
        std::fs::write(format!("{dir}/paged.rgba"), &paged).unwrap();
    }
    let (a, b) = (shadowed(&cascaded), shadowed(&paged));
    let both = a.iter().zip(&b).filter(|(x, y)| **x && **y).count();
    let either = a.iter().zip(&b).filter(|(x, y)| **x || **y).count();
    let dark = b.iter().filter(|x| **x).count();
    eprintln!("shadowed: cascades {}, pages {dark}, both {both} of {either}", a.iter().filter(|x| **x).count());
    let cascade_dark = a.iter().filter(|x| **x).count();
    assert!(cascade_dark > 40 && dark * 10 > cascade_dark * 7, "the pages shadow as much: {dark} of {cascade_dark}");
    assert!(both * 10 > either * 7, "in the same place: {both} of {either}");

    // Still: nothing more to draw.
    let (kept, _) = renderer.virtual_shadow_pages();
    assert!(kept > 10, "pages kept: {kept}");
    let still = scene(cube, Vec3::ZERO, true);
    renderer.render(&gpu, &target, &still);
    renderer.render(&gpu, &target, &still);
    assert_eq!(renderer.virtual_shadow_pages().1, 0, "a still scene draws no pages");
    // The box moves a little: only the pages it was and is over.
    renderer.render(&gpu, &target, &scene(cube, Vec3::new(0.3, 0.0, 0.0), true));
    let (kept, drawn) = renderer.virtual_shadow_pages();
    eprintln!("moved: {drawn} of {kept} pages drawn again");
    // A page or two a level where the box is, not the whole view.
    assert!(drawn > 0 && drawn <= 3 * scrap::vsm::LEVELS as usize && drawn * 2 < kept, "{drawn} of {kept}");
}
