//! The screen-space layer, over a real frame.
//!
//! What has to hold: the overlay draws on top of the scene rather than
//! replacing it, pixel coordinates mean what the mouse means, alpha blends,
//! and Cyrillic actually produces glyphs. The last one is the reason text
//! goes through shaping at all, and it is the one that fails silently — a
//! missing font renders nothing and looks like a layout bug.

use runity::glam::{Vec3, Vec4};
use runity::render::{Camera, FogSettings, Frame, Lighting, ShadowSettings};
use runity::{Gpu, OffscreenTarget, Quad, Renderer, TextRun, Ui, UiRenderer};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 128;

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    OffscreenTarget::pixel(pixels, WIDTH, x, y)
}

/// A frame with a known flat background, plus whatever the overlay adds.
fn shoot(gpu: &Gpu, ui: &Ui) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, &target);
    let mut overlay = UiRenderer::new(gpu, &target);

    renderer.render(
        gpu,
        &target,
        &Frame {
            camera: Camera::default(),
            lighting: Lighting::default(),
            fog: FogSettings::default(),
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::new(0.5, 0.0, 0.0),
            draws: Vec::new(),
            poses: Vec::new(),
        },
    );
    overlay.render(gpu, &target, ui);
    target.read_rgba(gpu)
}

#[test]
fn a_quad_lands_where_its_pixels_say_and_leaves_the_rest_alone() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let mut ui = Ui::new();
    // Opaque white, top-left quarter.
    ui.quad(Quad::new(
        0.0,
        0.0,
        (WIDTH / 2) as f32,
        (HEIGHT / 2) as f32,
        Vec4::ONE,
    ));
    let pixels = shoot(&gpu, &ui);

    let inside = pixel(&pixels, 10, 10);
    let outside = pixel(&pixels, WIDTH - 10, HEIGHT - 10);
    assert!(
        inside[0] > 200 && inside[1] > 200 && inside[2] > 200,
        "the quad should be white where it is, got {inside:?}"
    );
    assert!(
        outside[0] > 100 && outside[1] < 60 && outside[2] < 60,
        "and the scene should survive where it is not, got {outside:?}"
    );

    // Pixel coordinates run from the top left, the same origin a cursor
    // position uses. Getting this upside down is invisible in a symmetric
    // layout and wrong everywhere else.
    let below = pixel(&pixels, 10, HEIGHT - 10);
    assert_ne!(below, inside, "the quad covers the top, not the bottom");
}

#[test]
fn alpha_blends_over_the_scene_instead_of_replacing_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let mut ui = Ui::new();
    ui.quad(Quad::new(
        0.0,
        0.0,
        WIDTH as f32,
        HEIGHT as f32,
        Vec4::new(0.0, 0.0, 0.0, 0.5),
    ));
    let pixels = shoot(&gpu, &ui);
    let dimmed = pixel(&pixels, WIDTH / 2, HEIGHT / 2);

    assert!(
        dimmed[0] > 20 && dimmed[0] < 180,
        "half-transparent black should dim the red, not erase it, got {dimmed:?}"
    );
}

#[test]
fn cyrillic_text_produces_glyphs() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    // Shaping is the whole reason text is not a bitmap atlas. A font that
    // cannot draw these renders nothing at all, which looks like a layout
    // mistake and is not one.
    let mut with_text = Ui::new();
    with_text.text(TextRun::new(8.0, 30.0, 28.0, Vec4::ONE, "Долина ждёт"));
    let text = shoot(&gpu, &with_text);
    let blank = shoot(&gpu, &Ui::new());

    let changed = text
        .chunks_exact(4)
        .zip(blank.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 100,
        "eleven Cyrillic letters at 28px should mark more than {changed} pixels; \
         a font that cannot shape them draws nothing and says nothing"
    );
}
