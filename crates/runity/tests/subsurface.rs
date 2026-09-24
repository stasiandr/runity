//! Light under the surface: a thin slab between the camera and the sun,
//! and a thick block beside it, both of a waxy material. Seen from the
//! shaded side, the thin one glows with its colour — light through it —
//! and the thick one does not; with no light under the surface, neither
//! does.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

#[test]
fn a_thin_slab_glows_through_toward_the_sun_and_a_thick_block_does_not() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |under: [f32; 3]| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let wax = Material {
            subsurface: under,
            subsurface_radius: 0.03,
            ..Material::new(0.8, 0.7, 0.6)
        };
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            // Looking along +z at the shaded faces; the sun behind them,
            // shining toward the camera.
            camera: Camera {
                position: Vec3::new(0.0, 0.0, -4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(0.0, -0.2, -1.0).normalize(),
                sky_color: Vec3::splat(0.02),
                ground_color: Vec3::splat(0.02),
                ..Lighting::default()
            },
            shadows: ShadowSettings {
                contact: 0.0,
                ..ShadowSettings::default()
            },
            clear_color: Vec3::ZERO,
            draws: vec![
                // Thin, on the left of the view.
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(0.8, 0.0, 0.0))
                        * Mat4::from_scale(Vec3::new(1.0, 1.0, 0.02)),
                    texture: TextureHandle::WHITE,
                    material: wax,
                    pose: None,
                },
                // Thick, on the right.
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(-0.8, 0.0, 0.0))
                        * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0)),
                    texture: TextureHandle::WHITE,
                    material: wax,
                    pose: None,
                },
            ],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        let pixels = target.read_rgba(&gpu);
        let at = |x: u32| {
            let p = OffscreenTarget::pixel(&pixels, SIZE, x, SIZE / 2);
            p[0] as u32 + p[1] as u32 + p[2] as u32
        };
        (at(SIZE / 4), at(SIZE * 3 / 4))
    };
    let (thin_none, thick_none) = shot([0.0; 3]);
    let (thin, thick) = shot([0.9, 0.35, 0.2]);
    eprintln!("without: thin {thin_none} thick {thick_none}; with: thin {thin} thick {thick}");
    assert!(thin > thin_none + 60, "the thin slab glows through: {thin} against {thin_none}");
    assert!(thick < thick_none + 25, "the thick block does not: {thick} against {thick_none}");
}
