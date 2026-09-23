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
            // Counted in exact colours: no sky, no post-processing.
            sky: runity::render::Sky {
                mode: runity::render::SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            ray_tracing: Default::default(),
            camera: Camera::default(),
            lighting: Lighting::default(),
            fog: FogSettings::default(),
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::new(0.5, 0.0, 0.0),
            lights: Vec::new(),
            reflection_probes: Vec::new(),
            decals: Vec::new(),
            volumetric_fog: Default::default(),
            wind: Default::default(),
            benders: Vec::new(),
            time: None,
            weather: Default::default(),
            screen_space_reflections: Default::default(),
            draws: Vec::new(),
            overlay_draws: Vec::new(),
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

#[test]
fn an_overlay_draw_is_not_hidden_by_what_is_in_front_of_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    // A wall right in front of the camera, and a small thing behind it.
    // Drawn normally the small thing is invisible; as an overlay it is not.
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = runity::builtin::cube(1.0);
    let mesh = renderer.upload_mesh_owned(&gpu, &cube);

    let wall = runity::render::Draw {
        mesh,
        transform: runity::glam::Mat4::from_scale(Vec3::new(20.0, 20.0, 0.2)),
        texture: runity::TextureHandle::WHITE,
        material: runity::Material::new(0.05, 0.05, 0.05),
        pose: None,
    };
    let behind = runity::render::Draw {
        mesh,
        transform: runity::glam::Mat4::from_translation(Vec3::new(0.0, 0.0, -4.0)),
        texture: runity::TextureHandle::WHITE,
        material: runity::Material::new(1.0, 0.3, 0.0).unlit(),
        pose: None,
    };
    let base = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: runity::render::Sky {
            mode: runity::render::SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 6.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting::default(),
        fog: FogSettings {
            start: 1000.0,
            end: 2000.0,
            ..FogSettings::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        lights: Vec::new(),
        reflection_probes: Vec::new(),
        decals: Vec::new(),
        volumetric_fog: Default::default(),
        wind: Default::default(),
        benders: Vec::new(),
        time: None,
        weather: Default::default(),
        screen_space_reflections: Default::default(),
        draws: vec![wall],
        overlay_draws: Vec::new(),
        poses: Vec::new(),
    };

    let hidden = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: runity::render::Sky {
            mode: runity::render::SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        draws: vec![wall, behind],
        ..base.clone()
    };
    renderer.render(&gpu, &target, &hidden);
    let occluded = target.read_rgba(&gpu);

    let shown = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: runity::render::Sky {
            mode: runity::render::SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        overlay_draws: vec![behind],
        ..base
    };
    renderer.render(&gpu, &target, &shown);
    let overlaid = target.read_rgba(&gpu);

    // Looking for the orange rather than for darkness: the wall is grey, not
    // black, so "is it dark" would be testing the wall's material instead of
    // the depth behaviour.
    let orangeness = |pixels: &[u8]| {
        let p = OffscreenTarget::pixel(pixels, WIDTH, WIDTH / 2, HEIGHT / 2);
        p[0] as i32 - p[2] as i32
    };
    assert!(
        orangeness(&occluded) < 10,
        "drawn normally it is behind the wall, got {:?}",
        OffscreenTarget::pixel(&occluded, WIDTH, WIDTH / 2, HEIGHT / 2)
    );
    assert!(
        orangeness(&overlaid) > 150,
        "as an overlay it shows through, got {:?}",
        OffscreenTarget::pixel(&overlaid, WIDTH, WIDTH / 2, HEIGHT / 2)
    );
}

#[test]
fn a_menu_of_widgets_draws_where_it_says() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let (mut widgets, input, mut ui) = (runity::Widgets::new(), runity::Input::new(), Ui::new());
    let first = runity::Rect::new(20.0, 20.0, 160.0, 36.0);
    widgets.button(&mut ui, &input, first, "Play");
    let mut on = true;
    widgets.toggle(&mut ui, &input, first.below(8.0), "Shadows", &mut on);
    let mut volume = 0.7;
    widgets.slider(
        &mut ui,
        &input,
        first.below(8.0).below(8.0),
        "Volume",
        &mut volume,
        0.0..=1.0,
    );
    let pixels = shoot(&gpu, &ui);
    if let Ok(dir) = std::env::var("RUNITY_SHOT_DIR") {
        image::save_buffer(
            std::path::Path::new(&dir).join("widgets.png"),
            &pixels,
            WIDTH,
            HEIGHT,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    // The slider's filled part is the accent colour, 70% along.
    let filled = pixel(&pixels, 20 + 160 * 6 / 10, 20 + 2 * 44 + 18);
    let empty = pixel(&pixels, 20 + 160 * 9 / 10, 20 + 2 * 44 + 18);
    assert!(
        filled[0] > empty[0] + 60,
        "filled {filled:?} against empty {empty:?}"
    );
}
