//! Running the engine with no window at all.
//!
//! Headless is the primary way this engine is developed: a frame rendered into
//! memory can be diffed against a reference, saved as a PNG, or asserted on in
//! a unit test, none of which needs a display, a GPU or a compositor.

use crate::app::{App, Engine, Game};
use runity_platform::{HeadlessWindow, WindowConfig};
use runity_render::{save_png, Framebuffer};
use std::cell::RefCell;
use std::io;
use std::path::Path;
use std::rc::Rc;

/// Render one frame with no game loop: clear, run `draw`, hand back the frame.
///
/// ```
/// use runity_core::headless;
/// use runity_math::Mat4;
/// use runity_render::{Color, Mesh};
///
/// let frame = headless::render(64, 48, |engine| {
///     let shader = engine.lit_shader(Mat4::IDENTITY);
///     engine.draw(&Mesh::cube(1.0), &shader);
/// });
/// assert_eq!(frame.width(), 64);
/// ```
pub fn render(width: usize, height: usize, draw: impl FnOnce(&mut Engine)) -> Framebuffer {
    let mut engine = Engine::new(width, height);
    let clear = engine.clear_color;
    engine.framebuffer.clear(clear);
    draw(&mut engine);
    engine.framebuffer.clone()
}

/// Run a game for a fixed number of frames at a fixed delta, with no window.
///
/// The timestep is fixed on purpose: two runs of the same build produce the
/// same pixels, which is what makes golden images meaningful.
pub fn run(
    game: impl Game,
    width: u32,
    height: u32,
    frames: u64,
    delta: f32,
) -> io::Result<Engine> {
    let config = WindowConfig::new("runity (headless)", width, height);
    App::new(config.clone())
        .with_max_frames(frames)
        .with_frame_delta(delta)
        .with_target_fps(None)
        .run_with_window(Box::new(HeadlessWindow::new(&config)), game)
}

/// As [`run`], but keeps every frame — for debugging animation and motion.
pub fn record(
    game: impl Game,
    width: u32,
    height: u32,
    frames: u64,
    delta: f32,
) -> io::Result<Vec<Framebuffer>> {
    let recorded = Rc::new(RefCell::new(Vec::with_capacity(frames as usize)));
    let recorder = Recorder {
        inner: game,
        frames: Rc::clone(&recorded),
    };
    run(recorder, width, height, frames, delta)?;
    Ok(Rc::try_unwrap(recorded)
        .expect("the recorder is dropped by now")
        .into_inner())
}

/// Write frames as `<prefix>0000.png`, `<prefix>0001.png`, ... into `directory`.
pub fn save_frames(
    directory: impl AsRef<Path>,
    prefix: &str,
    frames: &[Framebuffer],
) -> io::Result<Vec<std::path::PathBuf>> {
    let directory = directory.as_ref();
    std::fs::create_dir_all(directory)?;
    let mut written = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let path = directory.join(format!("{prefix}{index:04}.png"));
        save_png(&path, frame)?;
        written.push(path);
    }
    Ok(written)
}

/// Wraps a game and keeps a copy of every rendered frame.
struct Recorder<G> {
    inner: G,
    frames: Rc<RefCell<Vec<Framebuffer>>>,
}

impl<G: Game> Game for Recorder<G> {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        self.inner.start(engine)
    }
    fn on_event(&mut self, engine: &mut Engine, event: &runity_platform::Event) {
        self.inner.on_event(engine, event);
    }
    fn update(&mut self, engine: &mut Engine) {
        self.inner.update(engine);
    }
    fn fixed_update(&mut self, engine: &mut Engine) {
        self.inner.fixed_update(engine);
    }
    fn render(&mut self, engine: &mut Engine) {
        self.inner.render(engine);
        // After the game has drawn, before the loop presents.
        self.frames.borrow_mut().push(engine.framebuffer.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::{Mat4, Vec3};
    use runity_render::{Color, Mesh};

    struct Spinner {
        angle: f32,
    }

    impl Game for Spinner {
        fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
            engine.clear_color = Color::BLACK;
            engine.camera.position = Vec3::new(0.0, 0.0, 4.0);
            Ok(())
        }
        fn update(&mut self, engine: &mut Engine) {
            self.angle += engine.time.delta();
        }
        fn render(&mut self, engine: &mut Engine) {
            let shader = engine.lit_shader(Mat4::from_rotation_y(self.angle));
            engine.draw(&Mesh::cube(1.5), &shader);
        }
    }

    #[test]
    fn a_single_frame_needs_no_loop_and_no_window() {
        let frame = render(40, 30, |engine| {
            engine.camera.position = Vec3::new(0.0, 0.0, 3.0);
            let shader = engine.lit_shader(Mat4::IDENTITY);
            engine.draw(&Mesh::cube(1.0), &shader);
        });
        assert_eq!((frame.width(), frame.height()), (40, 30));
        let background = Color::rgb(0.05, 0.06, 0.09).to_argb8();
        assert!(
            frame.pixels().iter().any(|p| *p != background),
            "something was drawn"
        );
    }

    #[test]
    fn a_headless_run_is_deterministic() {
        let first = run(Spinner { angle: 0.0 }, 32, 24, 10, 1.0 / 60.0).unwrap();
        let second = run(Spinner { angle: 0.0 }, 32, 24, 10, 1.0 / 60.0).unwrap();
        assert_eq!(first.framebuffer.pixels(), second.framebuffer.pixels());
        assert_eq!(first.time.frame(), 10);
        // Ten frames at 1/60 s each.
        assert!((first.time.elapsed() - 10.0 / 60.0).abs() < 1e-5);
    }

    #[test]
    fn recording_keeps_one_frame_per_tick_and_they_differ() {
        let frames = record(Spinner { angle: 0.0 }, 32, 24, 4, 0.1).unwrap();
        assert_eq!(frames.len(), 4);
        assert_ne!(
            frames[0].pixels(),
            frames[3].pixels(),
            "the cube rotates, so the frames must not be identical"
        );
    }

    #[test]
    fn frames_can_be_written_to_disk() {
        let frames = record(Spinner { angle: 0.0 }, 16, 16, 2, 0.1).unwrap();
        let dir = std::env::temp_dir().join(format!("runity-frames-{}", std::process::id()));
        let paths = save_frames(&dir, "frame", &frames).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[1].ends_with("frame0001.png"));
        assert!(paths.iter().all(|p| p.exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
