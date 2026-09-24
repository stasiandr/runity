//! The prefab scene, expanded and rendered.
//!
//! `examples/valley/scenes/camp.ron` places the same campfire three times and never spells
//! one out. What has to hold is the whole reason prefabs exist: three lines
//! in a scene produce three identical arrangements, standing in different
//! places, and editing the one file moves all of them.
//!
//! Rendered rather than only expanded, because the failure that matters is
//! not "the tree is the wrong shape" — a unit test catches that — but "a
//! tool forgot to expand", which produces a scene that opens fine and shows
//! nothing.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::path::{Path, PathBuf};

use scrap::{Gpu, MeshHandle, OffscreenTarget, Renderer, Scene};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;

fn scene_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/scrap is two levels down")
        .join("examples/valley/scenes/camp.ron")
}

/// The scene as a tool sees it: loaded, then expanded with its project's
/// prefabs.
fn expanded() -> (Scene, scrap::Instanced) {
    let document = Scene::load(scene_path()).expect("the camp scene");
    let project = scrap::Project::find(scene_path()).expect("the example is a project");
    let (prefabs, problems) = scrap::Prefabs::of(&project);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(
        prefabs.get("campfire").is_some(),
        "the project's prefabs/ has it: {:?}",
        prefabs.names()
    );
    let instanced = scrap::instantiate(&document, &prefabs);
    assert!(instanced.problems.is_empty(), "{:?}", instanced.problems);
    (document, instanced)
}

#[test]
fn three_lines_in_a_scene_become_three_of_the_same_thing() {
    let (document, instanced) = expanded();

    // The document stays small: that is the saving. Nine entries — the
    // ground, three fires, the flame and sparks of the two that burn, and
    // the kettle standing beside the cold one.
    assert_eq!(document.flatten().len(), 9);
    // And the scene that gets drawn is the whole thing: each fire brings a
    // root and five parts.
    assert_eq!(instanced.scene.flatten().len(), 9 + 3 * 5);

    // The three arrangements are the same arrangement. Compared by the names
    // and materials under each root, which is what "the same prefab" means;
    // their transforms differ, and that is the point of placing them.
    let shape = |name: &str| -> Vec<(String, [f32; 3])> {
        instanced
            .scene
            .find(name)
            .unwrap_or_else(|| panic!("{name} is in the scene"))
            .children
            .iter()
            .flat_map(|child| child.flatten())
            .map(|(e, _)| (e.name.clone(), e.material().base_color))
            .collect()
    };
    assert_eq!(shape("west fire"), shape("east fire"));
    assert_eq!(
        shape("west fire").len(),
        7,
        "an ember and four stones, and the flame and sparks the scene put in it"
    );
    assert_eq!(
        shape("cold fire")[..5],
        shape("west fire")[..5],
        "an override on the root does not reach into the prefab — which is \
         stated out loud in prefab.rs, and is the thing to revisit first"
    );
    assert_eq!(
        shape("cold fire").len(),
        6,
        "and the kettle put beside it is not part of the prefab"
    );

    // Placed by the instance: two fires, two places.
    let position = |name: &str| {
        instanced
            .scene
            .flatten()
            .into_iter()
            .find(|(e, _)| e.name == name)
            .map(|(_, world)| world.w_axis.truncate())
            .unwrap()
    };
    assert!(
        (position("west fire").x - position("east fire").x).abs() > 6.0,
        "the two fires stand apart"
    );

    // Everything a prefab brought belongs to the instance that brought it,
    // and the kettle — which has its own line in the scene — does not.
    let owner_name = |name: &str| {
        let expanded = instanced.scene.find(name).unwrap().id;
        let owner = instanced.owner_of(expanded).expect("an owner");
        document.get(owner).unwrap().name.clone()
    };
    assert_eq!(
        owner_name("kettle"),
        "kettle",
        "the kettle is its own entry in the document"
    );
    assert_eq!(
        owner_name("stone n"),
        "west fire",
        "a stone points back at the instance that brought it"
    );
}

#[test]
fn every_instance_reaches_the_frame() {
    // The failure this is really about: a tool that forgets to expand draws
    // a scene that looks fine and is missing everything the prefabs brought.
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter — install mesa-vulkan-drivers to render here");
        return;
    };
    let (_, instanced) = expanded();
    let scene = instanced.scene;

    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut world = hecs::World::new();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    let missing = scrap::spawn_scene(&scene, &mut world, |name| {
        let name: &str = name;
        if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
            return Some(found.1);
        }
        let mesh = scrap::builtin::by_name(name)?;
        let handle = renderer.upload_mesh_owned(&gpu, &mesh);
        uploaded.push((name.to_string(), handle));
        Some(handle)
    });
    assert!(missing.is_empty(), "{missing:?}");

    let frame = scrap::build_frame(
        &world,
        scrap::scene_camera(&scene.view()),
        scrap::render::Lighting {
            sun_direction: scrap::glam::Vec3::new(-0.4, -0.75, -0.5).normalize(),
            sun_intensity: scene.sun().intensity,
            ..Default::default()
        },
        scrap::render::FogSettings {
            color: scrap::glam::Vec3::from_array(scene.fog().color),
            start: scene.fog().start,
            end: scene.fog().end,
            ..Default::default()
        },
    );
    assert_eq!(
        frame.draws.len(),
        20,
        "every part of every instance is a draw"
    );
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);

    let out = std::env::temp_dir().join("scrap-camp.png");
    image::save_buffer(&out, &pixels, WIDTH, HEIGHT, image::ColorType::Rgba8).unwrap();
    eprintln!("wrote {}", out.display());

    // The ember is the one unlit thing in the prefab, so it is the one
    // colour nothing else in the frame can produce: orange, at full
    // brightness, whatever the sun is doing. Counting which half of the
    // frame it lands in counts instances.
    let is_ember = |p: [u8; 4]| p[0] > 200 && (110..210).contains(&p[1]) && p[2] < 140;
    let (mut left, mut right) = (0u32, 0u32);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if is_ember(OffscreenTarget::pixel(&pixels, WIDTH, x, y)) {
                if x < WIDTH / 2 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
    }
    assert!(
        left > 20 && right > 20,
        "an ember should burn on both sides of the frame: left {left}, right {right}"
    );
}
