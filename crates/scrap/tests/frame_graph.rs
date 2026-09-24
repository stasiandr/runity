//! The frame as a graph: the prepass runs when something reads its depth —
//! ambient occlusion, TAA — and not otherwise; and the graph the renderer
//! kept says so.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn the_prepass_runs_for_what_reads_its_depth_and_not_otherwise() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 64, 64);
    let graph = |ambient_occlusion: scrap::ssao::AmbientOcclusion, post: scrap::post::PostProcess| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = Frame {
            post,
            ambient_occlusion,
            shadows: scrap::render::ShadowSettings {
                contact: 0.0,
                ..Default::default()
            },
            camera: Camera {
                position: Vec3::new(0.0, 2.0, 4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            draws: vec![Draw {
                mesh: cube,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: Material::new(0.8, 0.8, 0.8),
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.frame_graph().clone()
    };
    let full = graph(Default::default(), scrap::post::PostProcess::default());
    assert!(full.runs("prepass") && full.runs("ssao") && full.runs("taa"), "{}", full.dot());
    assert!(full.problems(&["shadow map", "rays"]).is_empty());
    let bare = graph(scrap::ssao::AmbientOcclusion::OFF, scrap::post::PostProcess::OFF);
    assert!(!bare.runs("prepass"), "nothing reads the depth: {}", bare.dot());
    assert!(bare.runs("scene") && bare.runs("post"));
}
