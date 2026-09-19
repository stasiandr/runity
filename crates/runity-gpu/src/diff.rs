//! Differential testing: one closure, two renderers, the same pixels.
//!
//! A golden image says a renderer has not changed. It cannot say two renderers
//! agree, because a reference committed from one of them is only ever evidence
//! about that one. So the GPU path is tested against the CPU path directly:
//! [`assert_agrees`] hands the same drawing closure to a [`Canvas`] backed by
//! the rasterizer and to a `Canvas` backed by an offscreen Metal target, and
//! compares the two frames with the same [`runity_render::golden::compare`]
//! the golden images use.
//!
//! ```no_run
//! use runity_gpu::diff;
//! use runity_render::{golden::Tolerance, Color, Mesh};
//! use runity_math::Mat4;
//!
//! diff::assert_agrees("cube", 96, 96, Tolerance::new(8, 0.02), |canvas| {
//!     let shader = runity_render::UnlitShader::new(Mat4::IDENTITY);
//!     canvas.draw(&Mesh::cube(1.0), &shader);
//! });
//! ```
//!
//! **Without a device.** Every one of these tests skips, loudly, on a machine
//! with no Metal — that is what makes `cargo test --workspace` green on a
//! build box. Set [`REQUIRE_ENV`] to turn a skip into a failure, which is what
//! a machine that *does* have a GPU should be doing.

use crate::shader::GpuShader;
use crate::{Gpu, GpuError, Target};
use runity_render::golden::{compare, diff_image, Tolerance};
use runity_render::{
    save_png, Blend, Color, CullMode, DrawStats, Framebuffer, Image, Mesh, Rasterizer,
};
use std::cell::RefCell;
use std::path::PathBuf;

/// Set this to make a missing GPU a test failure instead of a skip.
pub const REQUIRE_ENV: &str = "RUNITY_REQUIRE_GPU";
/// Where the cpu/gpu/diff triptych lands when a comparison fails.
pub const OUTPUT_ENV: &str = "RUNITY_DIFF_DIR";

/// The clear colour every comparison starts from, unless one is given.
///
/// Not black: a shader that writes nothing would then agree with a shader that
/// writes black, and a whole class of mistake would pass.
pub const DEFAULT_CLEAR: Color = Color::rgb(0.05, 0.06, 0.09);

thread_local! {
    /// One device per thread, opened on first use. `Gpu` holds Objective-C
    /// pointers and is not `Send`, and the test harness runs tests on several
    /// threads, so this is per-thread rather than a process-wide singleton.
    static DEVICE: RefCell<Option<Result<Gpu, GpuError>>> = const { RefCell::new(None) };
}

/// What a drawing closure is handed: the fixed-function state, and a `draw`
/// that reaches whichever renderer this pass is running.
pub struct Canvas<'a> {
    /// Cull mode, depth test, depth write and blend — the same struct the
    /// rasterizer carries, and the same fields the GPU pipeline keys on.
    pub state: Rasterizer,
    target: CanvasTarget<'a>,
}

enum CanvasTarget<'a> {
    Cpu(&'a mut Framebuffer),
    Gpu(&'a mut Gpu),
}

impl Canvas<'_> {
    /// Draw a mesh with a shader that exists on both sides.
    pub fn draw<S: GpuShader>(&mut self, mesh: &Mesh, shader: &S) -> DrawStats {
        match &mut self.target {
            CanvasTarget::Cpu(framebuffer) => self.state.draw_mesh(framebuffer, mesh, shader),
            CanvasTarget::Gpu(gpu) => gpu.draw(
                mesh,
                shader,
                self.state.cull,
                self.state.blend,
                self.state.depth_test,
                self.state.depth_write,
            ),
        }
    }

    /// Shorthand for the three fixed-function switches *Depth & blending*
    /// moves at runtime.
    pub fn set_blending(&mut self, blend: Blend, depth_test: bool, depth_write: bool) {
        self.state.blend = blend;
        self.state.depth_test = depth_test;
        self.state.depth_write = depth_write;
    }

    /// Shorthand for the cull mode a flat quad or a single triangle wants off.
    pub fn set_cull(&mut self, cull: CullMode) {
        self.state.cull = cull;
    }

    /// Which renderer this pass is.
    pub fn is_gpu(&self) -> bool {
        matches!(self.target, CanvasTarget::Gpu(_))
    }
}

/// What a comparison found.
#[derive(Debug)]
pub enum Agreement {
    /// The two renderers produced the same frame, within the tolerance.
    Agreed,
    /// There is no Metal device here, so nothing was compared.
    Skipped(GpuError),
    /// They disagreed. The three PNGs are written; the string explains.
    Disagreed(String),
}

/// Render `draw` with both renderers and compare the frames.
///
/// The GPU pass always goes through the offscreen target at scale 1, so the
/// comparison is between two images of exactly `width` by `height`.
pub fn compare_renderers<F>(
    width: usize,
    height: usize,
    clear: Color,
    tolerance: Tolerance,
    draw: F,
) -> Result<(Framebuffer, Framebuffer, Option<String>), GpuError>
where
    F: Fn(&mut Canvas<'_>),
{
    let mut cpu = Framebuffer::new(width, height);
    cpu.clear(clear);
    draw(&mut Canvas {
        state: Rasterizer::new(),
        target: CanvasTarget::Cpu(&mut cpu),
    });

    let mut gpu_frame = Framebuffer::new(width, height);
    DEVICE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let device = slot.get_or_insert_with(Gpu::new);
        let gpu = match device {
            Ok(gpu) => gpu,
            Err(error) => return Err(error.clone()),
        };
        if !gpu.begin_frame(Target::Offscreen { width, height }, clear) {
            return Err(GpuError::NoSurface);
        }
        draw(&mut Canvas {
            state: Rasterizer::new(),
            target: CanvasTarget::Gpu(gpu),
        });
        gpu.end_frame(Some(&mut gpu_frame));
        Ok(())
    })?;

    let expected = to_image(&cpu);
    let report = compare(&gpu_frame, &expected, tolerance)
        .expect("both frames are the size they were built at");
    let complaint = (!report.is_within(tolerance)).then(|| report.to_string());
    Ok((cpu, gpu_frame, complaint))
}

/// [`compare_renderers`], but as a test: skip without a device, panic on a
/// disagreement, and leave a cpu/gpu/diff triptych behind either way.
#[track_caller]
pub fn assert_agrees<F>(name: &str, width: usize, height: usize, tolerance: Tolerance, draw: F)
where
    F: Fn(&mut Canvas<'_>),
{
    assert_agrees_on(name, width, height, DEFAULT_CLEAR, tolerance, draw)
}

/// [`assert_agrees`] with a clear colour of your own.
#[track_caller]
pub fn assert_agrees_on<F>(
    name: &str,
    width: usize,
    height: usize,
    clear: Color,
    tolerance: Tolerance,
    draw: F,
) where
    F: Fn(&mut Canvas<'_>),
{
    match run(name, width, height, clear, tolerance, draw) {
        Agreement::Agreed => {}
        Agreement::Skipped(error) => {
            if require_gpu() {
                panic!("{REQUIRE_ENV} is set, but {name} could not run: {error}");
            }
            // Loud on purpose: a silent skip is a test that has stopped
            // testing and nobody noticed.
            eprintln!(
                "\n=== SKIPPED: {name} — no GPU to compare against ===\n    {error}\n    \
                 set {REQUIRE_ENV}=1 to make this a failure\n"
            );
        }
        Agreement::Disagreed(report) => panic!("{report}"),
    }
}

fn run<F>(
    name: &str,
    width: usize,
    height: usize,
    clear: Color,
    tolerance: Tolerance,
    draw: F,
) -> Agreement
where
    F: Fn(&mut Canvas<'_>),
{
    let (cpu, gpu, complaint) = match compare_renderers(width, height, clear, tolerance, draw) {
        Ok(result) => result,
        Err(error) => return Agreement::Skipped(error),
    };
    let Some(complaint) = complaint else {
        // Nothing stale left over from a previous failure.
        for suffix in ["cpu", "gpu", "diff"] {
            let _ = std::fs::remove_file(output_path(name, suffix));
        }
        return Agreement::Agreed;
    };

    let (cpu_path, gpu_path, diff_path) = (
        output_path(name, "cpu"),
        output_path(name, "gpu"),
        output_path(name, "diff"),
    );
    if let Some(parent) = cpu_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = save_png(&cpu_path, &cpu);
    let _ = save_png(&gpu_path, &gpu);
    let _ = save_png(&diff_path, &diff_image(&gpu, &to_image(&cpu)));

    Agreement::Disagreed(format!(
        "the GPU and the rasterizer disagree on `{name}`: {complaint}\n  \
         rasterizer: {cpu}\n  GPU:        {gpu}\n  difference: {diff}",
        cpu = cpu_path.display(),
        gpu = gpu_path.display(),
        diff = diff_path.display()
    ))
}

fn to_image(frame: &Framebuffer) -> Image {
    Image {
        width: frame.width(),
        height: frame.height(),
        pixels: frame.pixels().to_vec(),
    }
}

fn output_path(name: &str, suffix: &str) -> PathBuf {
    let directory = std::env::var_os(OUTPUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/diff"));
    directory.join(format!("{name}.{suffix}.png"))
}

fn require_gpu() -> bool {
    match std::env::var(REQUIRE_ENV) {
        Ok(value) => !matches!(value.as_str(), "" | "0" | "false"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_triptych_lands_where_the_environment_says() {
        let path = output_path("cube", "diff");
        assert!(path.ends_with("cube.diff.png"));
        assert_eq!(path.parent().unwrap(), PathBuf::from("tests/diff"));
    }

    #[test]
    fn the_default_clear_is_not_black() {
        // A shader that writes nothing must not be able to agree with one that
        // writes black just because the background is black too.
        assert_ne!(DEFAULT_CLEAR.to_argb8(), Color::BLACK.to_argb8());
    }

    #[test]
    fn a_framebuffer_becomes_an_image_of_the_same_size_and_pixels() {
        let mut frame = Framebuffer::new(3, 2);
        frame.clear(Color::RED);
        let image = to_image(&frame);
        assert_eq!((image.width, image.height), (3, 2));
        assert!(image.pixels.iter().all(|p| *p == Color::RED.to_argb8()));
    }
}
