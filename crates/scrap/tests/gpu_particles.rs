//! Particles on the GPU: an emitter giving off thousands a second, upward,
//! on a black field. A few frames in, the air above it is full of them and
//! the air below it is empty; the pool holds as many as live at once.

use scrap::glam::{Mat4, Vec3};
use scrap::particles_gpu::GpuEmitter;
use scrap::render::{Camera, Frame, Sky, SkyMode};
use scrap::scene::Emitter;
use scrap::{Gpu, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

#[test]
fn thousands_rise_from_the_emitter_and_none_fall_below_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let emitter = Emitter {
        rate: 5000.0,
        life: 1.0,
        speed: 2.0,
        spread_deg: 12.0,
        size: 0.05,
        gravity: 0.0,
        color: (1.0, 1.0, 1.0),
        gpu: true,
        ..Emitter::default()
    };
    let mut frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        clear_color: Vec3::ZERO,
        camera: Camera {
            position: Vec3::new(0.0, 1.0, 4.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            ..Camera::default()
        },
        ..Frame::default()
    };
    let dt = 1.0 / 30.0;
    for i in 1..=20u32 {
        let lived = i as f32 * dt;
        frame.gpu_particles = vec![GpuEmitter {
            key: 7,
            placed: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            emitter: emitter.clone(),
            born: (lived * emitter.rate) as u64,
            lived,
            picture: None,
        }];
        renderer.render(&gpu, &target, &frame);
    }
    let pixels = target.read_rgba(&gpu);
    if let Ok(path) = std::env::var("DUMP") {
        image::save_buffer(path, &pixels, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
    }
    let lit = |rows: std::ops::Range<u32>| {
        rows.flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| OffscreenTarget::pixel(&pixels, SIZE, x, y)[1] > 40)
            .count()
    };
    // The emitter is at the height of the view's middle; they rise from it.
    let above = lit(0..SIZE / 2 - 2);
    let below = lit(SIZE / 2 + 6..SIZE);
    assert!(above > 150, "the air above full of them: {above} pixels");
    assert_eq!(below, 0, "none below the emitter: {below}");
    assert!(renderer.gpu_particle_slots() >= 5000, "slots for a second of them: {}", renderer.gpu_particle_slots());
}
