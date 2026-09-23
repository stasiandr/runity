//! Temporal antialiasing: a line thinner than a pixel, which multisampling
//! alone breaks into dashes, is whole after a few frames of it.

use runity::glam::{Mat4, Quat, Vec3};
use runity::material::Shading;
use runity::render::{Camera, Draw, Frame, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

#[test]
fn a_hairline_breaks_into_dashes_without_it_and_is_whole_with_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let rows_seen = |taa: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess {
                taa,
                ..runity::post::PostProcess::OFF
            },
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 0.0, 5.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            clear_color: Vec3::ZERO,
            // A white hair, a fifth of a pixel wide, leaning a little.
            draws: vec![Draw {
                mesh: cube,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::new(0.012, 4.0, 0.012),
                    Quat::from_rotation_z(0.08),
                    Vec3::ZERO,
                ),
                texture: TextureHandle::WHITE,
                material: Material {
                    shading: Shading::Unlit,
                    ..Material::new(1.0, 1.0, 1.0)
                },
                pose: None,
            }],
            ..Frame::default()
        };
        for _ in 0..24 {
            renderer.render(&gpu, &target, &frame);
        }
        let pixels = target.read_rgba(&gpu);
        (SIZE / 8..SIZE * 7 / 8)
            .filter(|&y| (0..SIZE).any(|x| OffscreenTarget::pixel(&pixels, SIZE, x, y)[0] > 3))
            .count() as u32
    };
    let rows = SIZE * 3 / 4;
    let without = rows_seen(false);
    let with = rows_seen(true);
    assert!(
        without < rows * 17 / 20,
        "multisampling alone leaves gaps: {without} of {rows}"
    );
    assert!(with > rows * 19 / 20, "whole with it: {with} of {rows}");
}
