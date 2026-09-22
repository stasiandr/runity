//! A window you can walk around in.
//!
//! ```text
//! cargo run --release --features desktop-shell --example walk -- scenes/first-light.ron
//! ```
//!
//! WASD moves, the mouse looks, shift runs, space rises, control sinks, and
//! Escape quits. It exists to prove the loop: the world advances in fixed
//! steps while the camera turns on the frame, so looking around stays smooth
//! at any frame rate and movement does not change speed with it.

use runity::builtin;
use runity::glam::Vec3;
use runity::render::{Camera, FogSettings, Frame, Lighting};
use runity::shell::{run, Context, Game, WindowConfig};
use runity::{Key, MeshHandle, Scene};

struct Walk {
    scene: Scene,
    world: hecs::World,
    /// Where the eye is. Moved on the fixed step, so two machines walking the
    /// same input end up in the same place.
    eye: Vec3,
    /// Where it looks. Turned on the frame, because a head that waits for a
    /// tick feels like a head in treacle.
    yaw: f32,
    pitch: f32,
    uploaded: Vec<(String, MeshHandle)>,
    started: bool,
}

impl Walk {
    fn new(scene: Scene) -> Self {
        Self {
            scene,
            world: hecs::World::new(),
            eye: Vec3::new(0.0, 1.7, 12.0),
            yaw: 0.0,
            pitch: -0.1,
            uploaded: Vec::new(),
            started: false,
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
        let mut uploaded = std::mem::take(&mut self.uploaded);
        let renderer = &mut *ctx.renderer;
        let gpu = ctx.gpu;
        let missing = runity::spawn_scene(&self.scene, &mut self.world, |name| {
            if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                return Some(found.1);
            }
            let mesh = builtin::by_name(name)?;
            let handle = renderer.upload_mesh_owned(gpu, &mesh);
            uploaded.push((name.to_string(), handle));
            Some(handle)
        });
        for m in &missing {
            eprintln!("{}: no model named {}", m.entity_name, m.model);
        }
        self.uploaded = uploaded;
        self.started = true;
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
        // On the frame, not the step: the head turns as fast as the screen
        // refreshes, and waiting for a tick is what makes 15 Hz feel sticky
        // rather than merely slow.
        let motion = ctx.input.mouse_motion();
        if ctx.input.mouse_held(runity::MouseButton::Left) {
            self.yaw += motion.x * 0.003;
            self.pitch = (self.pitch - motion.y * 0.003).clamp(-1.4, 1.4);
        }

        let fog = FogSettings {
            color: Vec3::from_array(self.scene.fog.color),
            start: self.scene.fog.start,
            end: self.scene.fog.end,
        };
        let camera = Camera {
            position: self.eye,
            target: self.eye + self.forward(),
            ..Camera::default()
        };
        runity::build_frame(&self.world, camera, Lighting::default(), fog)
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scenes/first-light.ron".into());
    let scene = Scene::load(&path)?;
    println!("{path}: WASD to walk, hold the left mouse button to look, Escape quits");
    run(
        WindowConfig {
            title: format!("runity — {path}"),
            ..Default::default()
        },
        Walk::new(scene),
    )
}
