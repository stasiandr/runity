//! Footprints: a walk across sand leaves prints pressed into it — darker
//! than the sand round them — and a puff of dust hangs in the air behind
//! the last steps.

use scrap::footprints::{Footprints, Trail};
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 128;

fn luminance(p: [u8; 4]) -> i32 {
    p[0] as i32 + p[1] as i32 + p[2] as i32
}

#[test]
fn a_walk_across_sand_leaves_prints_and_kicks_up_dust() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let sand = Material::new(0.85, 0.68, 0.45);
    let draws = vec![Draw {
        mesh: plane,
        transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
        texture: TextureHandle::WHITE,
        material: sand,
        pose: None,
    }];
    // Walked from z = 3 to z = -3 along x = 0, a sixtieth of a second a
    // frame at a walk.
    let mut trail = Trail::new(Footprints::default());
    for i in 0..=240 {
        trail.advance(Vec3::new(0.0, 0.0, 3.0 - 0.025 * i as f32), 1.0 / 60.0);
    }
    assert!(trail.prints() >= 7, "{}", trail.prints());
    let frame = |prints: bool, dust: bool| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        // Straight down on the trail from a man's height.
        camera: Camera {
            position: Vec3::new(0.0, 2.0, 0.3),
            target: Vec3::new(0.0, 0.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.4, -0.6, -0.3).normalize(),
            ..Lighting::default()
        },
        decals: if prints {
            trail.decals().collect()
        } else {
            Vec::new()
        },
        puffs: if dust {
            trail.dust([0.7, 0.45, 0.25]).collect()
        } else {
            Vec::new()
        },
        clear_color: Vec3::splat(0.5),
        draws: draws.clone(),
        ..Frame::default()
    };
    let shot = |renderer: &mut Renderer, prints, dust| {
        renderer.render(&gpu, &target, &frame(prints, dust));
        target.read_rgba(&gpu)
    };
    let bare = shot(&mut renderer, false, false);
    let walked = shot(&mut renderer, true, false);
    if let Ok(dir) = std::env::var("SCRAP_SHOT_DIR") {
        let _ = std::fs::write(format!("{dir}/footprints.rgba"), &walked);
    }
    // Somewhere on the trail is darker than the bare sand there: a print.
    let darkest = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            luminance(OffscreenTarget::pixel(&bare, SIZE, x, y))
                - luminance(OffscreenTarget::pixel(&walked, SIZE, x, y))
        })
        .max()
        .unwrap();
    assert!(darkest > 60, "a print is darker than the sand: {darkest}");
    // Away from the trail, the sand is as it was.
    let aside = (SIZE / 12, SIZE / 2);
    assert_eq!(
        OffscreenTarget::pixel(&bare, SIZE, aside.0, aside.1),
        OffscreenTarget::pixel(&walked, SIZE, aside.0, aside.1)
    );
    // The dust of the last steps is in the air: the picture changes, and
    // not only on the trail.
    let dusty = shot(&mut renderer, true, true);
    let changed = (0..SIZE * SIZE)
        .filter(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            (luminance(OffscreenTarget::pixel(&dusty, SIZE, x, y))
                - luminance(OffscreenTarget::pixel(&walked, SIZE, x, y)))
            .abs()
                > 6
        })
        .count();
    assert!(
        changed > 100,
        "dust hangs where the last steps were: {changed} pixels"
    );
}
