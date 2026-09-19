//! Card #18 in three pictures: the Metal renderer standing next to the
//! rasterizer, and the seventh showcase scene drawn by both.
//!
//! ```text
//! cargo run --release --example demo
//! ```
//!
//! Headless on purpose. The window half of this card is a window — run
//! `cargo run --release --example smallworld`, walk the seven pages with `Tab`
//! and press `Space` on the last one. What a window cannot do is leave evidence
//! behind, so this writes files into `$STUDIO_ARTIFACTS` (or the working
//! directory when that is unset):
//!
//! * `1-teapot-parity.png` and `2-blend-and-texture-parity.png` — rasterizer |
//!   Metal | difference, the same triptych a failing differential test leaves
//!   behind, measured at the tolerance those tests use.
//! * `3-teapot-grid.png` — the seventh scene's grid after twenty fixed-step
//!   frames: rasterizer on the left, Metal on the right.
//! * `report.txt` — the numbers behind those pictures, the wall clock each
//!   renderer took, and what each debug view does under the GPU.
//!
//! With no Metal device the parity pictures come out as the rasterizer's half
//! alone and the report says so, which is the same way the differential tests
//! behave on a build machine.

use runity::gpu::diff::{self, Canvas};
use runity::gpu::Gpu;
use runity::prelude::*;
use std::f32::consts::TAU;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Small on purpose: the rasterizer is the slow half, and the point is the
/// difference between two pictures rather than either picture's size.
const PARITY_WIDTH: usize = 200;
const PARITY_HEIGHT: usize = 150;

const GRID_WIDTH: u32 = 320;
const GRID_HEIGHT: u32 = 240;
const GRID_FRAMES: u64 = 20;
/// Nine teapots — 56,880 triangles a frame, uploaded to the GPU every frame
/// with no resource cache behind it, exactly as the showcase does it.
const GRID_ROWS: usize = 3;

/// The budget the geometric differential tests use: an edge may land a pixel
/// either side of itself, a surface may not have the wrong colour on it.
fn geometry_tolerance() -> golden::Tolerance {
    golden::Tolerance::new(8, 0.02)
}

fn main() -> io::Result<()> {
    let out = artifacts_dir();
    std::fs::create_dir_all(&out)?;
    let mut report = String::new();

    let device = match Gpu::new() {
        Ok(gpu) => {
            let name = gpu.device_name();
            say(&mut report, format!("Metal device: {name}"));
            Some(name)
        }
        Err(error) => {
            say(&mut report, format!("no GPU here: {error}"));
            None
        }
    };
    say(&mut report, String::new());

    // --- the two parity triptychs -------------------------------------------
    say(
        &mut report,
        "parity — one closure, two renderers, compared pixel by pixel:".to_string(),
    );
    let teapot = Mesh::from_obj(include_str!("assets/teapot.obj"))
        .expect("assets/teapot.obj ships with this example and parses");
    parity(
        &out,
        &mut report,
        "1-teapot-parity.png",
        "teapot",
        |canvas| {
            let eye = Vec3::new(0.0, 2.2, 5.0);
            let vp = perspective(eye, Vec3::new(0.0, 1.0, 0.0));
            canvas.draw(
                &teapot,
                &lit(
                    Mat4::from_rotation_y(0.9),
                    vp,
                    eye,
                    Color::rgb(0.8, 0.78, 0.74),
                ),
            );
        },
    )?;

    let mut floor = Texture::checker(
        64,
        8,
        Color::rgb(0.20, 0.22, 0.26),
        Color::rgb(0.32, 0.34, 0.40),
    );
    floor.wrap = Wrap::Repeat;
    parity(
        &out,
        &mut report,
        "2-blend-and-texture-parity.png",
        "blend-and-texture",
        |canvas| {
            let eye = Vec3::new(2.6, 2.0, 3.6);
            let vp = perspective(eye, Vec3::ZERO);

            let mut ground = lit(
                Mat4::from_translation(Vec3::new(0.0, -1.4, 0.0)),
                vp,
                eye,
                Color::WHITE,
            );
            ground.texture = Some(&floor);
            canvas.draw(&Mesh::plane(8.0, 1), &ground);

            // Three translucent planes through one another: the blend state,
            // the depth test and the depth write all have to be right at once.
            canvas.set_cull(CullMode::None);
            canvas.set_blending(Blend::Alpha, true, false);
            let quad = Mesh::plane(2.6, 1);
            for (turn, tint) in [
                (0.0_f32, Color::rgba(0.95, 0.35, 0.35, 0.55)),
                (TAU / 3.0, Color::rgba(0.35, 0.90, 0.45, 0.55)),
                (2.0 * TAU / 3.0, Color::rgba(0.40, 0.55, 0.98, 0.55)),
            ] {
                let model = Mat4::from_rotation_y(turn) * Mat4::from_rotation_x(0.35);
                let mut shader = UnlitShader::new(vp * model);
                shader.tint = tint;
                canvas.draw(&quad, &shader);
            }
        },
    )?;
    say(&mut report, String::new());

    // --- the seventh scene, run by the main loop on each renderer ------------
    say(
        &mut report,
        format!(
            "the seventh scene's grid — {} teapots, {} triangles a frame, \
             {GRID_FRAMES} headless frames at a fixed 1/60 s:",
            GRID_ROWS * GRID_ROWS,
            GRID_ROWS * GRID_ROWS * teapot.triangle_count()
        ),
    );
    let (cpu_frame, cpu_ms, cpu_name) = run_grid(Renderer::Cpu)?;
    say(
        &mut report,
        format!("  {cpu_name:<3} {cpu_ms:7.2} ms a frame (wall clock)"),
    );
    let gpu_frame = match device {
        Some(_) => {
            let (frame, ms, name) = run_grid(Renderer::Gpu)?;
            say(
                &mut report,
                format!(
                    "  {name:<3} {ms:7.2} ms a frame (wall clock, including the \
                     read-back the window path does not do)"
                ),
            );
            Some(frame)
        }
        None => None,
    };

    let panels: Vec<&Framebuffer> = match &gpu_frame {
        Some(gpu) => vec![&cpu_frame, gpu],
        None => vec![&cpu_frame],
    };
    let path = out.join("3-teapot-grid.png");
    save_png(&path, &strip(&panels))?;
    say(&mut report, format!("  wrote {}", path.display()));
    say(&mut report, String::new());

    // --- what the debug keys do under the GPU -------------------------------
    say(
        &mut report,
        "debug views under the GPU renderer (keys 1-4 in smallworld):".to_string(),
    );
    for (key, view) in [
        ('1', DebugView::Shaded),
        ('2', DebugView::Wireframe),
        ('3', DebugView::Depth),
        ('4', DebugView::Overdraw),
    ] {
        let verdict = match view.is_supported_by(Renderer::Gpu) {
            true => "shown".to_string(),
            false => UnsupportedView {
                view,
                renderer: Renderer::Gpu,
            }
            .to_string(),
        };
        say(&mut report, format!("  {key}  {view:?}: {verdict}"));
    }

    let report_path = out.join("report.txt");
    std::fs::write(&report_path, report)?;
    println!("\nwrote {}", report_path.display());
    Ok(())
}

/// Render `draw` with both renderers, save `cpu | gpu | difference` as one
/// strip, and put the comparison's numbers in the report.
fn parity(
    out: &Path,
    report: &mut String,
    file: &str,
    name: &str,
    draw: impl Fn(&mut Canvas<'_>),
) -> io::Result<()> {
    let tolerance = geometry_tolerance();
    let (cpu, gpu, _) = match diff::compare_renderers(
        PARITY_WIDTH,
        PARITY_HEIGHT,
        diff::DEFAULT_CLEAR,
        tolerance,
        draw,
    ) {
        Ok(result) => result,
        Err(error) => {
            // The rasterizer's half on its own, so the picture is still worth
            // opening, and a line saying why the other two panels are missing.
            let mut alone = Framebuffer::new(PARITY_WIDTH, PARITY_HEIGHT);
            alone.clear(diff::DEFAULT_CLEAR);
            save_png(out.join(file), &alone)?;
            say(report, format!("  {name}: not compared — {error}"));
            return Ok(());
        }
    };

    let expected = Image {
        width: cpu.width(),
        height: cpu.height(),
        pixels: cpu.pixels().to_vec(),
    };
    let measured = golden::compare(&gpu, &expected, tolerance)
        .expect("both frames were built at the same size");
    let strip = strip(&[&cpu, &gpu, &golden::diff_image(&gpu, &expected)]);
    let path = out.join(file);
    save_png(&path, &strip)?;

    say(
        report,
        format!(
            "  {name}: worst channel off by {delta}, {differing} of {total} pixels differ \
             ({percent:.3}%) — {verdict} at {allowed}/255 and {fraction:.0}%; {file}",
            delta = measured.max_channel_delta,
            differing = measured.differing_pixels,
            total = measured.total_pixels(),
            percent = measured.differing_fraction() * 100.0,
            verdict = match measured.is_within(tolerance) {
                true => "agree",
                false => "DISAGREE",
            },
            allowed = tolerance.channel_delta,
            fraction = tolerance.differing_fraction * 100.0,
        ),
    );
    Ok(())
}

/// Run the grid through the real main loop on one renderer, and time it.
fn run_grid(renderer: Renderer) -> io::Result<(Framebuffer, f32, &'static str)> {
    let config = WindowConfig::new("runity — demo (headless)", GRID_WIDTH, GRID_HEIGHT);
    let started = Instant::now();
    let engine = App::new(config.clone())
        .with_renderer(renderer)
        .with_max_frames(GRID_FRAMES)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(Box::new(HeadlessWindow::new(&config)), TeapotGrid::new())?;
    let ms = started.elapsed().as_secs_f32() * 1000.0 / GRID_FRAMES as f32;
    // Which renderer actually ran, rather than which one was asked for:
    // RUNITY_RENDERER outranks the program's own choice.
    Ok((engine.framebuffer.clone(), ms, engine.renderer().name()))
}

/// The seventh showcase scene, cut down to what fits in a demo: a square grid
/// of teapots turning about a common centre.
struct TeapotGrid {
    teapot: Mesh,
    angle: f32,
}

/// The showcase's own spacing and scale, so the picture is recognisably the
/// same scene.
const SPACING: f32 = 2.4;
const SCALE: f32 = 0.34;

impl TeapotGrid {
    fn new() -> Self {
        Self {
            teapot: Mesh::from_obj(include_str!("assets/teapot.obj"))
                .expect("assets/teapot.obj ships with this example and parses"),
            angle: 0.0,
        }
    }

    fn instances(&self) -> Vec<Mat4> {
        let half = (GRID_ROWS - 1) as f32 * 0.5;
        let ring = Mat4::from_rotation_y(self.angle);
        let mut out = Vec::with_capacity(GRID_ROWS * GRID_ROWS);
        for row in 0..GRID_ROWS {
            for column in 0..GRID_ROWS {
                let offset = Vec3::new(
                    (column as f32 - half) * SPACING,
                    -0.9,
                    (row as f32 - half) * SPACING,
                );
                let spin = self.angle * (1.0 + ((row * GRID_ROWS + column) % 3) as f32 * 0.5);
                out.push(
                    ring * Mat4::from_translation(offset)
                        * Mat4::from_rotation_y(spin)
                        * Mat4::from_scale(Vec3::splat(SCALE)),
                );
            }
        }
        out
    }
}

impl Game for TeapotGrid {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.clear_color = Color::rgb(0.05, 0.05, 0.08);
        engine.camera = Camera::look_at(Vec3::new(5.5, 4.5, 8.5), Vec3::new(0.0, -0.4, 0.0));
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.4, -0.85, -0.35).normalized(),
            color: Color::rgb(1.0, 0.97, 0.92),
            intensity: 1.1,
        };
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        self.angle += engine.time.delta() * 0.6;
    }

    fn render(&mut self, engine: &mut Engine) {
        const PALETTE: [Color; 3] = [
            Color::rgb(0.86, 0.80, 0.72),
            Color::rgb(0.80, 0.55, 0.40),
            Color::rgb(0.55, 0.72, 0.68),
        ];
        for (index, model) in self.instances().into_iter().enumerate() {
            let mut shader = engine.lit_shader(model);
            shader.base_color = PALETTE[index % PALETTE.len()];
            shader.specular_strength = 0.3;
            engine.draw(&self.teapot, &shader);
        }
    }
}

/// A light with no highlight. `pow` is the one operation two implementations of
/// the same formula are entitled to round apart, and the differential tests
/// measure the highlight on its own, with room to breathe.
fn lit(model: Mat4, view_projection: Mat4, eye: Vec3, color: Color) -> BasicShader<'static> {
    let mut shader = BasicShader::new(model, view_projection)
        .with_base_color(color)
        .with_camera_position(eye)
        .with_light(DirectionalLight {
            direction: Vec3::new(-0.35, -0.8, -0.5).normalized(),
            color: Color::WHITE,
            intensity: 1.05,
        });
    shader.specular_strength = 0.0;
    shader
}

fn perspective(eye: Vec3, target: Vec3) -> Mat4 {
    Mat4::perspective(0.9, PARITY_WIDTH as f32 / PARITY_HEIGHT as f32, 0.1, 100.0)
        * Mat4::look_at(eye, target, Vec3::Y)
}

/// Lay frames out left to right in one image, so a person opens one file.
fn strip(panels: &[&Framebuffer]) -> Framebuffer {
    const GAP: usize = 6;
    let width: usize =
        panels.iter().map(|p| p.width()).sum::<usize>() + GAP * panels.len().saturating_sub(1);
    let height = panels.iter().map(|p| p.height()).max().unwrap_or(0);
    let mut out = Framebuffer::new(width, height);
    out.clear(Color::rgb(0.10, 0.10, 0.12));

    let mut left = 0;
    for panel in panels {
        for y in 0..panel.height() {
            for x in 0..panel.width() {
                out.set_pixel(left + x, y, panel.get_pixel(x, y));
            }
        }
        left += panel.width() + GAP;
    }
    out
}

/// Where the studio looks for what this run produced.
fn artifacts_dir() -> PathBuf {
    std::env::var_os("STUDIO_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// One line, onto the terminal and into the report at the same time.
fn say(report: &mut String, line: String) {
    println!("{line}");
    report.push_str(&line);
    report.push('\n');
}
