//! A screen written in HTML and CSS (`tests/screens/`), drawn on a GPU
//! off-screen and read back: what the model says is on the screen, in the
//! colours the stylesheet gives.
//!
//! Skipped, not failed, where there is no adapter at all. The picture is
//! written next to the test's temp files for a person or an agent to look
//! at: `scrap-ui-markup-hud.png`.

use scrap_gpu::gpu::{Gpu, OffscreenTarget};
use scrap_ui::markup::Screen;
use scrap_ui::render::UiRenderer;
use scrap_ui::{Color, Ui};
use serde::Serialize;

#[derive(Serialize)]
struct Order {
    dish: &'static str,
    left: u32,
}

fn screen() -> Screen {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/screens/hud.screen.html");
    let mut screen = Screen::load(path).unwrap_or_else(|p| panic!("{p}"));
    screen.set("round.clock", "1:30");
    screen.set("round.score", 240);
    screen.set("round.over", true);
    screen.set("player.hp", 90);
    screen.set("player.max", 120);
    screen.set(
        "orders",
        [
            Order {
                dish: "Soup",
                left: 42,
            },
            Order {
                dish: "Burger",
                left: 7,
            },
        ],
    );
    screen
}

#[test]
fn the_example_hud_reads_and_asks_for_nothing_the_model_lacks() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/screens/hud.screen.html");
    assert_eq!(Screen::check(path), []);
    let mut screen = screen();
    let mut ui = Ui::new();
    ui.set_viewport(1280.0, 720.0, 1.0);
    let root = ui.root();
    screen.mount(&mut ui, root);
    assert!(screen.missing().is_empty(), "{:?}", screen.missing());
    let dump = ui.dump();
    for words in [
        "\"1:30\"",
        "\"Score 240\"",
        "\"Burger\"",
        "\"7s\"",
        "\"Leave\"",
    ] {
        assert!(dump.contains(words), "{words} in\n{dump}");
    }
}

#[test]
fn an_html_screen_draws_as_its_css_says() {
    let gpu = match Gpu::headless_blocking(false) {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("no adapter ({e}); skipped");
            return;
        }
    };
    let (w, h) = (640u32, 360u32);
    let target = OffscreenTarget::new(&gpu, w, h);
    let mut ui = Ui::new();
    ui.set_viewport(w as f32, h as f32, 1.0);
    let mut screen = screen();
    let root = ui.root();
    screen.mount(&mut ui, root);
    let mut renderer = UiRenderer::new(&gpu, target.format().remove_srgb_suffix());
    renderer.draw(
        &gpu,
        &target.ui_view(),
        w,
        h,
        &mut ui,
        Some(Color::hex(0x000000)),
    );

    let pixels = target.read_rgba(&gpu);
    let out = std::env::temp_dir().join("scrap-ui-markup-hud.png");
    image::save_buffer(&out, &pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    eprintln!("wrote {} on {}", out.display(), gpu.describe());

    let at = |x: f32, y: f32| OffscreenTarget::pixel(&pixels, w, x as u32, y as u32);
    let close = |got: [u8; 4], want: u32, what: &str| {
        let want = Color::hex(want);
        let d = (got[0] as i32 - want.r as i32)
            .abs()
            .max((got[1] as i32 - want.g as i32).abs())
            .max((got[2] as i32 - want.b as i32).abs());
        assert!(d <= 2, "{what}: got {got:?}, want {want:?}");
    };
    close(at(4.0, 4.0), 0x161826, "the body's background");
    // 90 of 120: three quarters of 240 px green, the rest the card.
    let health = ui.rect(ui.children(ui.children(root)[0])[1]);
    close(
        at(health.x + 170.0, health.y + 8.0),
        0x4caf50,
        "health, filled",
    );
    close(
        at(health.x + 200.0, health.y + 8.0),
        0x232532,
        "health, empty",
    );
    // The late order is red, the other the card's colour.
    let orders = ui.children(screen.node("orders").unwrap());
    let soup = ui.rect(orders[0]);
    let burger = ui.rect(orders[1]);
    close(
        at(soup.x + soup.width - 6.0, soup.y + 4.0),
        0x232532,
        "an order on time",
    );
    close(
        at(burger.x + burger.width - 6.0, burger.y + 4.0),
        0xc0392b,
        "a late order",
    );
}
