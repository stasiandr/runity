//! The far shadow cascades drawn every other frame, in turn: a still scene
//! looks the same as with every cascade drawn every frame, frame after
//! frame — the cascade not drawn is read by the view it was drawn from.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 160;

#[test]
fn a_still_scene_looks_the_same_with_its_far_cascades_drawn_in_turn() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shots = |stagger: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.set_shadow_stagger(stagger);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let draw = |mesh, transform| Draw { mesh, transform, texture: TextureHandle::WHITE, material: Material::new(0.7, 0.7, 0.7), pose: None };
        // Posts near and far, their shadows over every cascade.
        let mut draws = vec![draw(plane, Mat4::from_scale(Vec3::splat(200.0)))];
        for i in 0..12 {
            let z = -(i as f32).powf(1.6) * 2.0;
            draws.push(draw(cube, Mat4::from_translation(Vec3::new(1.5, 1.0, z)) * Mat4::from_scale(Vec3::new(0.3, 2.0, 0.3))));
        }
        let frame = Frame {
            sky: Sky { mode: SkyMode::Color, ..Default::default() },
            post: scrap::post::PostProcess::OFF,
            camera: Camera { position: Vec3::new(0.0, 2.0, 4.0), target: Vec3::new(0.0, 0.0, -20.0), ..Camera::default() },
            lighting: Lighting { sun_direction: Vec3::new(-0.6, -0.5, -0.3).normalize(), ..Lighting::default() },
            shadows: ShadowSettings { max_distance: 60.0, cascades: 4, ..Default::default() },
            draws,
            ..Frame::default()
        };
        let pictures = (0..5)
            .map(|_| {
                renderer.render(&gpu, &target, &frame);
                target.read_rgba(&gpu)
            })
            .collect::<Vec<_>>();
        // How many cascades the next frame draws.
        renderer.render(&gpu, &target, &frame);
        let drawn = renderer.stats().cascades_drawn;
        (pictures, drawn)
    };
    let (every, all_drawn) = shots(false);
    let (turns, some_drawn) = shots(true);
    assert_eq!(all_drawn, 4);
    assert_eq!(some_drawn, 3, "the far cascades are drawn one a frame");
    for (i, (a, b)) in every.iter().zip(&turns).enumerate() {
        let worst = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        assert!(worst <= 1, "frame {i}: the cascades drawn in turn differ by {worst}");
    }
}
