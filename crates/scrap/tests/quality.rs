//! Quality presets: Low draws the materials scene well under Ultra's time
//! on the GPU, each preset runs no pass the one above does not, and a
//! device is given a start that suits it.

use scrap::prelude::*;
use scrap::quality::Quality;
use scrap::{Gpu, OffscreenTarget, Renderer};

#[test]
fn each_preset_runs_no_more_than_the_one_above_and_low_is_cheapest() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("skipping: no GPU timestamps");
        return;
    }
    let scene = scrap::scene::Scene::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/valley/scenes/materials.ron"
    )))
    .expect("the scene reads");
    let target = OffscreenTarget::new(&gpu, 1280, 720);
    let mut times: Vec<(f32, Vec<String>)> = Vec::new();
    // Twice round, each preset's least kept: the first measured would
    // otherwise pay for the GPU's clock coming up.
    for (round, quality) in Quality::ALL.iter().copied().cycle().take(Quality::ALL.len() * 2).enumerate() {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.set_quality(Some(quality));
        renderer.profile_gpu(true);
        let mut world = scrap::hecs::World::new();
        let mut meshes: Vec<(String, scrap::render::MeshHandle)> = Vec::new();
        scrap::spawn_scene(&scene, &mut world, |name| {
            let name: &str = name;
            if let Some((_, h)) = meshes.iter().find(|(n, _)| n == name) {
                return Some(*h);
            }
            let mesh = scrap::builtin::by_name(name).unwrap_or_else(|| scrap::builtin::cube(1.0));
            let handle = renderer.upload_mesh_owned(&gpu, &mesh);
            meshes.push((name.to_string(), handle));
            Some(handle)
        });
        let camera = scrap::scene_camera(&scene.view());
        let mut frame = scrap::build_frame(&world, camera, scrap::scene_lighting(&scene.sun()), Default::default());
        scrap::world::scene_look(&mut frame, &scene);
        // The least of a few settled readings: the GPU's clock and power
        // move a single one by more than the presets differ.
        let mut ms = f32::MAX;
        for _ in 0..3 {
            for _ in 0..20 {
                renderer.render(&gpu, &target, &frame);
                target.read_rgba(&gpu);
            }
            ms = ms.min(renderer.gpu_times().iter().map(|(_, t)| t).sum());
        }
        let lit: Vec<String> = renderer.gpu_times().into_iter().map(|(name, _)| name).collect();
        eprintln!("{quality:?}: {ms:.2} ms, {lit:?}");
        let at = round % Quality::ALL.len();
        match times.get_mut(at) {
            Some(kept) => kept.0 = kept.0.min(ms),
            None => times.push((ms, lit)),
        }
    }
    // What runs goes away going down (the upscale's own passes aside).
    let heavy = |passes: &Vec<String>| {
        passes
            .iter()
            .filter(|n| ["ssao", "taa", "lens", "ray", "fog"].iter().any(|h| n.contains(h)))
            .count()
    };
    for pair in times.windows(2) {
        assert!(heavy(&pair[0].1) <= heavy(&pair[1].1), "{times:?}");
    }
    // And Low is well under Ultra.
    assert!(times[0].0 < times[3].0 * 0.8, "{times:?}");
    let start = Quality::for_device(&gpu);
    eprintln!("this device starts on {start:?}");
    assert!(start >= Quality::Medium || !gpu.adapter.get_info().device_type.eq(&wgpu::DeviceType::IntegratedGpu));
}
