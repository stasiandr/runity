//! The render path's passes switched off at run time: the game's graphics
//! settings (`scrap::passes`). The least is meshes with no light — what
//! the sun does no longer shows — and it is still a picture of the scene.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::path::Path;

use scrap::builtin;
use scrap::glam::Vec3;
use scrap::passes::Passes;
use scrap::render::{FogSettings, Lighting};
use scrap::{Gpu, MeshHandle, OffscreenTarget, Renderer, Scene};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 135;

/// The reference scene with the sun from `sun`, drawn by these passes.
fn shoot(gpu: &Gpu, scene: &Scene, sun: Vec3, passes: Passes) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, &target);
    renderer.set_passes(passes);
    let mut world = hecs::World::new();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    scrap::spawn_scene(scene, &mut world, |name| {
        let name: &str = name;
        if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
            return Some(found.1);
        }
        let handle = renderer.upload_mesh_owned(gpu, &builtin::by_name(name)?);
        uploaded.push((name.to_string(), handle));
        Some(handle)
    });
    let frame = scrap::build_frame(
        &world,
        scrap::scene_camera(&scene.view()),
        Lighting {
            sun_direction: sun,
            ..Lighting::default()
        },
        FogSettings::default(),
    );
    renderer.render(gpu, &target, &frame);
    target.read_rgba(gpu)
}

#[test]
fn with_the_least_passes_the_sun_does_not_show_and_the_scene_still_does() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/valley/content/valley/maps/first-light.scene.ron");
    let scene = Scene::load(path).expect("the reference scene");
    let morning = Vec3::new(-0.6, -0.7, -0.35).normalize();
    let evening = Vec3::new(0.85, -0.2, 0.3).normalize();

    let least = shoot(&gpu, &scene, morning, Passes::MIN);
    assert_eq!(least, shoot(&gpu, &scene, evening, Passes::MIN), "no light: the sun changes nothing");
    let first = &least[..4];
    assert!(least.chunks(4).any(|p| p != first), "still a picture, not one colour");

    assert_ne!(
        shoot(&gpu, &scene, morning, Passes::MAX),
        shoot(&gpu, &scene, evening, Passes::MAX),
        "with light the sun shows, or the check above proves nothing"
    );
}
