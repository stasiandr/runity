//! A screen on a thing in the world: the crosshair presses its button,
//! and the thing shows what is drawn on it.

use runity::glam::{Mat4, Vec2, Vec3, Vec4};
use runity::render::{Camera, FogSettings, Lighting, SkyMode};
use runity::ui::Quad;
use runity::widgets::{Rect, Widgets};
use runity::world::{build_frame, Model, Surface, WorldTransform, WorldUi};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn the_crosshair_presses_a_button_on_a_screen_in_the_world() {
    // A metre-square screen standing up, facing +z, two metres ahead.
    let placed = Mat4::from_translation(Vec3::new(0.0, 1.5, -2.0))
        * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mut screen = WorldUi::new("terminal", (200, 200));
    let eye = Vec3::new(0.0, 1.5, 0.0);
    let ahead = Vec3::new(0.0, 0.0, -1.0);
    let at = screen.hit(placed, eye, ahead).expect("aimed at its middle");
    assert!((at - Vec2::new(100.0, 100.0)).length() < 1.0, "{at}");
    assert!(screen.hit(placed, eye, -ahead).is_none(), "the other way");
    let mut widgets = Widgets::new();
    let buy = Rect::new(50.0, 50.0, 100.0, 100.0);
    let mut press = |screen: &mut WorldUi, down: bool, up: bool| {
        screen.aim(Some(at), down, up);
        screen.ui.clear();
        widgets.button(&mut screen.ui, &screen.pointer, buy, "Buy")
    };
    assert!(!press(&mut screen, true, false));
    assert!(press(&mut screen, false, true), "pressed and let go on it");
}

#[test]
fn a_screen_in_the_world_shows_what_is_drawn_on_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    const SIZE: u32 = 32;
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut overlay = runity::ui_render::UiRenderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let mut world = runity::hecs::World::new();
    let mut screen = WorldUi::new("terminal", (64, 64));
    screen.ui.quad(Quad::new(
        0.0,
        0.0,
        64.0,
        64.0,
        Vec4::new(1.0, 0.0, 0.0, 1.0),
    ));
    world.spawn((
        WorldTransform(Mat4::from_scale(Vec3::splat(4.0))),
        Model(plane),
        Surface(Material {
            shading: runity::material::Shading::Unlit,
            ..Material::new(1.0, 1.0, 1.0)
        }),
        screen,
    ));
    let camera = Camera {
        position: Vec3::new(0.0, 3.0, 0.001),
        target: Vec3::ZERO,
        ..Camera::default()
    };
    let mut frame = build_frame(&world, camera, Lighting::default(), FogSettings::default());
    frame.sky.mode = SkyMode::Color;
    frame.clear_color = Vec3::ZERO;
    frame.post = runity::post::PostProcess::OFF;
    frame.ambient_occlusion = runity::ssao::AmbientOcclusion::OFF;
    renderer.draw_ui_pictures(&gpu, &mut overlay, &frame);
    renderer.render(&gpu, &target, &frame);
    let p = OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2);
    assert!(p[0] > 150 && p[1] < 80, "the red it shows: {p:?}");
}
