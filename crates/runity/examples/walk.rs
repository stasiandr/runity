//! A window you can walk around in.
//!
//! ```text
//! cargo run --release --features desktop-shell --example walk -- examples/valley/scenes/first-light.ron
//! ```
//!
//! WASD moves, the mouse looks, shift runs, space rises, control sinks, and
//! Escape quits. It exists to prove the loop: the world advances in fixed
//! steps while the camera turns on the frame, so looking around stays smooth
//! at any frame rate and movement does not change speed with it.
//!
//! And the other loop: save the scene, a prefab, or re-import an asset while
//! it runs, and the change is in the next frames — patched into the world,
//! not reloaded over it. Save the engine's `render.wgsl` and the next frame
//! draws with it; a shader that does not compile is reported and the old
//! one keeps drawing.

#[allow(unused_imports)]
use runity::prelude::*;
use runity::glam::Vec3;
use runity::render::{Camera, Frame};
use runity::shell::{run, Context, Game, WindowConfig};
use runity::{Key, LiveScene, TextRun, Ui};

struct Walk {
    live: LiveScene,
    /// The engine's shader source, reloaded when it is saved.
    shader: runity::render::ShaderFile,
    /// The last few seconds of frames: the median and the stutters.
    times: runity::FrameTimes,
    world: hecs::World,
    /// Where the eye is. Moved on the fixed step, so two machines walking the
    /// same input end up in the same place.
    eye: Vec3,
    /// Where it looks. Turned on the frame, because a head that waits for a
    /// tick feels like a head in treacle.
    yaw: f32,
    pitch: f32,
    ui: Ui,
}

impl Walk {
    fn new(live: LiveScene) -> Self {
        // Start where the scene says it is looked at from, so walking in and
        // rendering headlessly begin from the same place. A hardcoded
        // viewpoint here meant the two disagreed, and the one you were
        // looking at was whichever tool you happened to run.
        let view = live.scene().view();
        let eye = view.position;
        let look = (view.target - eye).normalize_or_zero();
        Self {
            live,
            shader: runity::render::ShaderFile::new(runity::render::SHADER_PATH),
            times: runity::FrameTimes::new(300),
            world: hecs::World::new(),
            eye,
            yaw: look.x.atan2(-look.z),
            pitch: look.y.clamp(-1.0, 1.0).asin(),
            ui: Ui::new(),
        }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }
}

impl Game for Walk {
    fn start(&mut self, ctx: &mut Context) {
        for line in self
            .live
            .spawn(&mut self.world, ctx.gpu, ctx.renderer)
            .lines()
        {
            eprintln!("{line}");
        }
    }

    fn step(&mut self, ctx: &mut Context) {
        let dt = ctx.time.settings().fixed_delta;
        let speed = if ctx.input.held(Key::LeftShift) {
            8.0
        } else {
            3.4
        };
        let plane = ctx.input.move_axis();
        let forward = self.forward();
        // Flattened, so looking up does not lift you off the ground.
        let ahead = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        let right = ahead.cross(Vec3::Y).normalize_or_zero();
        self.eye += (ahead * plane.y + right * plane.x) * speed * dt;
        self.eye.y += (f32::from(ctx.input.held(Key::Space))
            - f32::from(ctx.input.held(Key::LeftControl)))
            * speed
            * dt;
        self.eye.y = self.eye.y.max(0.4);
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if ctx.input.pressed(Key::Escape) {
            ctx.quit();
        }
        let done = self
            .live
            .poll(ctx.time.delta(), &mut self.world, ctx.gpu, ctx.renderer);
        for line in done.lines() {
            eprintln!("{line}");
        }
        match self.shader.poll(ctx.renderer, ctx.gpu) {
            Some(Ok(())) => eprintln!("shader reloaded"),
            Some(Err(problem)) => eprintln!("{problem}"),
            None => {}
        }
        // On the frame, not the step: the head turns as fast as the screen
        // refreshes, and waiting for a tick is what makes 15 Hz feel sticky
        // rather than merely slow.
        let motion = ctx.input.mouse_motion();
        if ctx.input.mouse_held(runity::MouseButton::Left) {
            self.yaw += motion.x * 0.003;
            self.pitch = (self.pitch - motion.y * 0.003).clamp(-1.4, 1.4);
        }

        let fog = runity::scene_fog(&self.live.scene().fog());
        let camera = Camera {
            position: self.eye,
            target: self.eye + self.forward(),
            ..Camera::default()
        };
        // The overlay is rebuilt every frame from scratch: there is no
        // retained widget tree to keep in step with anything.
        self.times
            .record(std::time::Duration::from_secs_f32(ctx.time.delta()));
        let times = self
            .times
            .summary()
            .map(|s| {
                format!(
                    "медиана {:.1} мс, рывков {}",
                    s.median.as_secs_f64() * 1e3,
                    s.hitches
                )
            })
            .unwrap_or_default();
        self.ui.clear();
        self.ui.text(TextRun::new(
            12.0,
            12.0,
            18.0,
            runity::glam::Vec4::new(0.9, 0.9, 0.88, 0.85),
            format!(
                "{:.0} кадр/с   {times}   такт {:.0} Гц   {:.1}, {:.1}, {:.1}",
                1.0 / ctx.time.delta().max(1e-4),
                1.0 / ctx.time.settings().fixed_delta,
                self.eye.x,
                self.eye.y,
                self.eye.z
            ),
        ));

        runity::build_frame(
            &self.world,
            camera,
            runity::scene_lighting(&self.live.scene().sun()),
            fog,
        )
    }

    fn overlay(&mut self) -> &Ui {
        &self.ui
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/valley/scenes/first-light.ron".into());
    // Prefabs and the library come from the project the scene is in, the
    // same way the headless render finds them — walking into a scene and
    // rendering it have to show the same thing.
    let (live, problems) = LiveScene::open(&path)?;
    for problem in &problems {
        eprintln!("{problem}");
    }
    println!(
        "{path}: WASD to walk, hold the left mouse button to look, Escape quits. \
         Save the scene and watch it change."
    );
    run(
        WindowConfig {
            title: format!("runity — {path}"),
            ..Default::default()
        },
        Walk::new(live),
    )
}
