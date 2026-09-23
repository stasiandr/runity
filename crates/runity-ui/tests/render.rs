//! The UI drawn on a GPU, off-screen, and read back.
//!
//! Skipped, not failed, where there is no adapter at all. The picture is
//! written next to the test's temp files so a person or an agent can look at
//! it: `runity-ui-panel.png`.

use runity::gpu::{Gpu, OffscreenTarget};
use runity_ui::render::UiRenderer;
use runity_ui::{Color, ImageId, Style, Ui};

const BG: Color = Color::hex(0x161826);
const SURFACE: Color = Color::hex(0x232532);
const TEXT: Color = Color::hex(0xe9e9ed);
const ACCENT: Color = Color::hex(0x9184d9);

fn gpu() -> Option<Gpu> {
    match Gpu::headless_blocking(false) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("no adapter ({e}); skipped");
            None
        }
    }
}

/// A panel as Nocturne draws one: a card on the ground with a small-caps
/// title, a selected line tinted from the accent ramp with an accent edge,
/// an outlined primary button, and a picture from another texture.
fn panel(ui: &mut Ui) {
    let root = ui.root();
    ui.set_style(
        root,
        Style::row().full().background(BG).padding(8.0).gap(8.0),
    );
    let card = ui.add(
        root,
        Style::column()
            .width(220.0)
            .full_height()
            .background(SURFACE)
            .radius(8.0)
            .padding(8.0)
            .gap(2.0),
    );
    ui.set_name(card, "card");
    ui.add_text(
        card,
        Style::default().text_size(10.5).text_color(TEXT.alpha(55)),
        "HIERARCHY",
    );
    for (i, name) in ["ground", "tree near", "камень", "crate"]
        .iter()
        .enumerate()
    {
        let selected = i == 3;
        let mut style = Style::row()
            .height(24.0)
            .fixed()
            .padding_x(8.0)
            .center_items()
            .radius(4.0);
        if selected {
            style = style
                .background(Color::hex(0x2b2741))
                .border(1.0, ACCENT.alpha(40));
        }
        let line = ui.add(card, style);
        ui.add_text(
            line,
            Style::default()
                .text_size(12.5)
                .nowrap()
                .text_color(if selected { Color::hex(0xe7e5fe) } else { TEXT }),
            name,
        );
    }
    let divider = ui.add(
        card,
        Style::row()
            .height(1.0)
            .fixed()
            .full_width()
            .background(TEXT.alpha(16)),
    );
    ui.set_name(divider, "divider");
    let button = ui.add(
        card,
        Style::row()
            .height(26.0)
            .fixed()
            .padding_x(10.0)
            .center()
            .radius(8.0)
            .border(1.0, ACCENT),
    );
    ui.set_name(button, "save");
    ui.add_text(
        button,
        Style::default().text_size(12.5).text_color(ACCENT),
        "Save",
    );

    let view = ui.add_image(root, Style::default().fill().radius(8.0), ImageId(1));
    ui.set_name(view, "view");
}

#[test]
fn a_nocturne_panel_draws_as_the_design_file_says() {
    let Some(gpu) = gpu() else { return };
    let (w, h) = (480u32, 240u32);
    let scale = 1.0;
    let target = OffscreenTarget::new(&gpu, w, h);
    // What the Scene view would be: another texture on the same device,
    // shown with no copy.
    let scene = OffscreenTarget::new(&gpu, 64, 64);
    let mut scene_ui = Ui::new();
    scene_ui.set_viewport(64.0, 64.0, 1.0);
    let mut renderer = UiRenderer::new(&gpu, target.format().remove_srgb_suffix());
    renderer.draw(
        &gpu,
        &scene.ui_view(),
        64,
        64,
        &mut scene_ui,
        Some(Color::hex(0x6a7a4a)),
    );

    let mut ui = Ui::new();
    ui.set_viewport(w as f32 / scale, h as f32 / scale, scale);
    panel(&mut ui);
    let mut renderer = UiRenderer::new(&gpu, target.format().remove_srgb_suffix());
    renderer.set_image(&gpu, ImageId(1), scene.view());
    renderer.draw(&gpu, &target.ui_view(), w, h, &mut ui, Some(BG));

    let pixels = target.read_rgba(&gpu);
    let out = std::env::temp_dir().join("runity-ui-panel.png");
    image::save_buffer(&out, &pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    eprintln!("wrote {} on {}", out.display(), gpu.describe());

    let at = |x: f32, y: f32| OffscreenTarget::pixel(&pixels, w, x as u32, y as u32);
    let close = |got: [u8; 4], want: Color, what: &str| {
        let d = (got[0] as i32 - want.r as i32)
            .abs()
            .max((got[1] as i32 - want.g as i32).abs())
            .max((got[2] as i32 - want.b as i32).abs());
        assert!(d <= 2, "{what}: got {got:?}, want {want:?}");
    };
    close(at(2.0, 2.0), BG, "the ground");
    let card = ui.rect(ui.find("card").unwrap());
    close(
        at(card.x + 4.0, card.y + card.height - 4.0),
        SURFACE,
        "the card",
    );
    // A corner is round: the very corner pixel is the ground, not the card.
    close(at(card.x, card.y), BG, "the card's corner");

    // 16% of #e9e9ed over #232532, blended as CSS blends it — in sRGB:
    // 0x23 + (0xe9 - 0x23) * 0.16 ≈ 0x43.
    let d = ui.rect(ui.find("divider").unwrap());
    // `alpha(16)` is 40/255.
    let mix =
        |over: u8, under: u8| (under as f32 + (over as f32 - under as f32) * 40.0 / 255.0).round();
    let want = Color::rgba(
        mix(0xe9, 0x23) as u8,
        mix(0xe9, 0x25) as u8,
        mix(0xed, 0x32) as u8,
        255,
    );
    close(
        at(d.x + d.width / 2.0, d.y),
        want,
        "the divider, blended as in a browser",
    );

    // The button's outline is the accent; its inside is the card.
    let b = ui.rect(ui.find("save").unwrap());
    close(at(b.x + b.width / 2.0, b.y), ACCENT, "the button's outline");
    // Its label is in there: some pixel in the middle is not the card.
    let lit = (0..b.width as u32)
        .map(|dx| at(b.x + dx as f32, b.y + b.height / 2.0))
        .any(|p| p[0] > 0x50);
    assert!(lit, "the label drew");

    // The picture is the other texture, as it is.
    let v = ui.rect(ui.find("view").unwrap());
    let (cx, cy) = v.center();
    close(
        at(cx, cy),
        Color::hex(0x6a7a4a),
        "the picture from another texture",
    );
}
