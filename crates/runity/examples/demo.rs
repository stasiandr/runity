//! Card #65 demo: a contact sheet of the prepared valley content.
//!
//! One headless render per committed model, each framed on its own bounding
//! box and textured with the one atlas, plus a zoom of the atlas itself. Run
//! it with `cargo run --release --example demo`; the pictures land in
//! `$STUDIO_ARTIFACTS` (or `target/demo` when that is not set).

use runity::prelude::*;
use std::path::PathBuf;

#[path = "valley/assets.rs"]
#[allow(dead_code)] // assets.rs is an example of its own; its `main` is unused here.
mod assets;

#[path = "valley/look.rs"]
#[allow(dead_code)] // same for look.rs: the sheet only borrows the morning light.
mod look;

const TILE_W: usize = 220;
const TILE_H: usize = 180;
const LABEL_H: usize = 18;
const COLUMNS: usize = 6;
const MORNING: f32 = 9.5;

/// The camera distance that fits a sphere of `radius` in the tile.
fn framing_distance(radius: f32, fov_y: f32, aspect: f32) -> f32 {
    let vertical = radius / (fov_y * 0.5).tan();
    let horizontal = radius / ((fov_y * 0.5).tan() * aspect);
    vertical.max(horizontal) * 1.25
}

/// One model, alone, on a patch of ground, under the morning sun.
fn contact_render(mesh: &Mesh, atlas: &Texture) -> Framebuffer {
    let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    for v in &mesh.vertices {
        min = Vec3::new(
            min.x.min(v.position.x),
            min.y.min(v.position.y),
            min.z.min(v.position.z),
        );
        max = Vec3::new(
            max.x.max(v.position.x),
            max.y.max(v.position.y),
            max.z.max(v.position.z),
        );
    }
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(0.1);
    let (light, sky) = look::sky_at(MORNING);

    headless::render(TILE_W, TILE_H, |engine| {
        engine.clear_color = sky;
        engine.framebuffer.clear(sky);
        engine.light = light;
        engine.ambient = sky.scale_rgb(0.55);

        let aspect = TILE_W as f32 / TILE_H as f32;
        let distance = framing_distance(radius, engine.camera.fov_y, aspect);
        let eye = Vec3::new(0.62, 0.42, 1.0).normalized() * distance;
        engine.camera.position = center + eye;
        engine.camera.target = center;
        engine.camera.far = distance * 4.0;

        // The ground is flat color, not the atlas: every cell of the atlas is
        // a surface of something, and bare earth is the next card's problem.
        let mut ground = engine.lit_shader(Mat4::from_translation(Vec3::new(0.0, -0.01, 0.0)));
        ground.base_color = Color::rgb(0.30, 0.33, 0.24);
        ground.specular_strength = 0.0;
        engine.draw(&Mesh::plane(radius * 8.0, 1), &ground);

        let mut shader = engine.lit_shader(Mat4::IDENTITY);
        shader.texture = Some(atlas);
        shader.specular_strength = 0.05;
        engine.draw(mesh, &shader);
    })
}

fn blit(sheet: &mut Framebuffer, tile: &Framebuffer, x0: usize, y0: usize) {
    for y in 0..tile.height() {
        for x in 0..tile.width() {
            sheet.set_pixel(x0 + x, y0 + y, tile.get_pixel(x, y));
        }
    }
}

/// The atlas at eight pixels to the texel, sampled the way the valley samples
/// it — nearest, so the cells stay cells.
fn atlas_zoom(atlas: &Texture, zoom: usize) -> Framebuffer {
    let (width, height) = (atlas.width() * zoom, atlas.height() * zoom);
    let mut frame = Framebuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            frame.set_pixel(x, y, atlas.sample(u, v));
        }
    }
    frame
}

fn artifacts_dir() -> PathBuf {
    std::env::var("STUDIO_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/demo"))
}

fn main() -> std::io::Result<()> {
    let out = artifacts_dir();
    std::fs::create_dir_all(&out)?;

    let atlas = assets::load_atlas();
    let font = Font::embedded();
    let label = TextStyle::new(&font)
        .size(11.0)
        .color(Color::rgb(0.9, 0.9, 0.86));

    let rows = assets::CONTENT.len().div_ceil(COLUMNS);
    let mut sheet = Framebuffer::new(COLUMNS * TILE_W, rows * (TILE_H + LABEL_H));
    sheet.clear(Color::rgb(0.07, 0.08, 0.09));

    for (i, item) in assets::CONTENT.iter().enumerate() {
        let mesh = assets::load_model(item.file).unwrap_or_else(|e| panic!("{e}"));
        let tile = contact_render(&mesh, &atlas);
        let (x0, y0) = ((i % COLUMNS) * TILE_W, (i / COLUMNS) * (TILE_H + LABEL_H));
        blit(&mut sheet, &tile, x0, y0);
        let caption = format!(
            "{}  {} tris",
            item.file.trim_end_matches(".obj"),
            mesh.triangle_count()
        );
        label.draw(
            &mut sheet,
            &caption,
            x0 as i32 + 6,
            (y0 + TILE_H) as i32 + 3,
        );
        println!(
            "{:<24} {:>5} tris  {}",
            item.file,
            mesh.triangle_count(),
            item.what
        );
    }

    save_png(out.join("valley-contact-sheet.png"), &sheet)?;
    save_png(out.join("valley-atlas.png"), &atlas_zoom(&atlas, 3))?;
    println!(
        "wrote valley-contact-sheet.png ({}x{}) and valley-atlas.png to {}",
        sheet.width(),
        sheet.height(),
        out.display()
    );
    println!(
        "the checklist, the atlas and CREDITS.txt are tested by \
         `cargo test -p runity --example valley-assets`"
    );
    Ok(())
}
