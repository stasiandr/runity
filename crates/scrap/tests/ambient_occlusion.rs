//! Ambient occlusion by both methods: a floor meeting a wall, lit only from
//! all round. Where they meet goes darker; the open floor stays as bright
//! as with no occlusion at all — a flat floor must not shadow itself.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::ssao::{AmbientOcclusion, Method};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn where_floor_meets_wall_is_darker_and_open_floor_is_not() {
    corner_and_open_floor(96);
}

/// On a screen this big the occlusion is found at half the size across
/// and blurred back up: the corner as dark.
#[test]
fn at_half_resolution_the_corner_is_still_darker() {
    corner_and_open_floor(768);
}

fn corner_and_open_floor(size: u32) {
    #[allow(non_snake_case)]
    let SIZE = size;
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |ao: AmbientOcclusion| {
        let mut renderer = Renderer::new(&gpu, &target);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let grey = Material::new(0.7, 0.7, 0.7);
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: ao,
            // Looking at the foot of a wall across the floor.
            camera: Camera {
                position: Vec3::new(0.0, 1.2, 2.2),
                target: Vec3::new(0.0, 0.2, -1.0),
                ..Camera::default()
            },
            lighting: Lighting {
                sun_intensity: 0.0,
                sky_color: Vec3::splat(1.0),
                ground_color: Vec3::splat(1.0),
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: vec![
                Draw {
                    mesh: plane,
                    transform: Mat4::from_scale(Vec3::splat(20.0)),
                    texture: TextureHandle::WHITE,
                    material: grey,
                    pose: None,
                },
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(0.0, 1.5, -1.6))
                        * Mat4::from_scale(Vec3::new(8.0, 3.0, 0.4)),
                    texture: TextureHandle::WHITE,
                    material: grey,
                    pose: None,
                },
            ],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let g = |pixels: &[u8], y: u32| {
        (SIZE / 3..SIZE * 2 / 3)
            .map(|x| OffscreenTarget::pixel(pixels, SIZE, x, y)[1] as f32)
            .sum::<f32>()
            / (SIZE / 3) as f32
    };
    let none = shot(AmbientOcclusion::OFF);
    // The row where the wall meets the floor: the darkest row of the
    // occluded picture's middle, in its lower half.
    let mut corners = Vec::new();
    for method in [Method::Gtao, Method::Ssao] {
        let with = shot(AmbientOcclusion {
            method,
            ..AmbientOcclusion::default()
        });
        let (corner, dark) = (SIZE / 3..SIZE)
            .map(|y| (y, g(&with, y)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let open_y = SIZE - 4;
        let (open, bare) = (g(&with, open_y), g(&none, open_y));
        let share = dark / g(&none, corner);
        eprintln!("{method:?}: corner row {corner} at {share:.3} of bare; open {open} (bare {bare})");
        assert!(share < 0.97, "{method:?}: the corner darker: {share}");
        assert!((open - bare).abs() < bare * 0.06, "{method:?}: open floor not shadowed: {open} against {bare}");
        corners.push(share);
    }
    // Both close the corner and leave the open floor; which is darker is
    // their own business (URP's SSAO at these numbers is the darker).
    assert_eq!(corners.len(), 2);
}
