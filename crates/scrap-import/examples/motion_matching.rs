//! Motion matching you can drive: a skeleton that walks and runs where
//! the keys point, every frame of it picked from mocap.
//!
//! ```text
//! cargo run --release -p scrap-import --example motion_matching
//! cargo run --release -p scrap-import --example motion_matching -- --video walk.mp4
//! ```
//!
//! WASD walks, shift runs, the left mouse button held turns the view,
//! Escape quits. `--video FILE` drives a set path headless and writes it
//! through ffmpeg instead. White balls are where the stick asks the
//! character to be a third, two thirds and one second on; a foot turns
//! orange while it is locked to the ground.
//!
//! The clips are LAFAN1's (Ubisoft La Forge, CC BY-NC-ND 4.0), read from
//! `$SCRAP_LAFAN1` or `~/.cache/scrap/lafan1` — kept out of the repository
//! and out of every build.

use std::path::PathBuf;

use scrap::glam::{Mat4, Quat, Vec3};
use scrap::matching::{Among, Ask, Database, Matcher, Setup};
use scrap::render::{Camera, Draw, Frame, MeshHandle, TextureHandle};
use scrap::shell::{run, Context, Game, StepContext, WindowConfig};
use scrap::{builtin, Gpu, Key, Material, Renderer};

const CLIPS: &[&str] = &[
    "walk1_subject1",
    "walk1_subject2",
    "walk1_subject5",
    "run1_subject2",
    "run1_subject5",
    "sprint1_subject2",
];

fn lafan1() -> PathBuf {
    std::env::var_os("SCRAP_LAFAN1")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/scrap/lafan1"))
}

fn database() -> anyhow::Result<Database> {
    let dir = lafan1();
    let started = std::time::Instant::now();
    let mut skeleton = None;
    let mut clips = Vec::new();
    for name in CLIPS {
        let path = dir.join(format!("{name}.bvh"));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!("{}: {e} — LAFAN1 goes in ~/.cache/scrap/lafan1 (or $SCRAP_LAFAN1)", path.display())
        })?;
        let (s, clip) = scrap_import::bvh::read(&text, name, 0.01)?;
        skeleton.get_or_insert(s);
        clips.push(clip);
    }
    let db = Database::build(&skeleton.unwrap(), &clips, Setup::default()).map_err(anyhow::Error::msg)?;
    eprintln!("{} frames of {} clips in {:.1?}", db.len(), clips.len(), started.elapsed());
    Ok(db)
}

/// The yard: a wall across the way, a flight of stairs up to a landing
/// and down again, and boxes to get over. `(centre, half size)`.
fn yard() -> Vec<(Vec3, Vec3)> {
    let mut boxes = vec![(Vec3::new(0.0, 1.0, 4.25), Vec3::new(2.0, 1.0, 0.25))];
    let rise = 0.17;
    for i in 0..6 {
        let top = rise * (i + 1) as f32;
        let up = 2.0 + 0.3 * i as f32 + 0.15;
        let down = 8.8 + 0.3 * (5 - i) as f32 + 0.15;
        boxes.push((Vec3::new(6.0, top / 2.0, up), Vec3::new(1.0, top / 2.0, 0.15)));
        boxes.push((Vec3::new(6.0, top / 2.0, down), Vec3::new(1.0, top / 2.0, 0.15)));
    }
    boxes.push((Vec3::new(6.0, 0.51, 6.3), Vec3::new(1.0, 0.51, 2.5)));
    boxes
}

/// The yard's boxes and a floor, as a physics world.
fn physics(boxes: &[(Vec3, Vec3)]) -> scrap::PhysicsWorld {
    let mut world = scrap::hecs::World::new();
    let floor = (Vec3::new(0.0, -0.5, 0.0), Vec3::new(200.0, 0.5, 200.0));
    for &(centre, half) in boxes.iter().chain([&floor]) {
        world.spawn((
            scrap::Transform { position: centre, ..Default::default() },
            scrap::world::WorldTransform(Mat4::from_translation(centre)),
            scrap::world::Physics(scrap::Body::Static),
            scrap::Shape(scrap::scene::Collider::Box { half, center: Vec3::ZERO }),
        ));
    }
    let mut physics = scrap::PhysicsWorld::new(1.0 / 60.0);
    physics.sync_from_world(&mut world);
    physics.refresh_queries();
    physics
}

/// Meshes the view draws with.
struct Meshes {
    bone: MeshHandle,
    ball: MeshHandle,
    tile: MeshHandle,
    cube: MeshHandle,
}

impl Meshes {
    fn upload(gpu: &Gpu, renderer: &mut Renderer) -> Self {
        Self {
            bone: renderer.upload_mesh_owned(gpu, &builtin::cylinder(0.5, 1.0, 12)),
            ball: renderer.upload_mesh_owned(gpu, &builtin::sphere(0.5, 12, 8)),
            tile: renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1)),
            cube: renderer.upload_mesh_owned(gpu, &builtin::cube(1.0)),
        }
    }
}

fn draw(mesh: MeshHandle, transform: Mat4, material: Material) -> Draw {
    Draw { mesh, transform, texture: TextureHandle::WHITE, pose: None, material }
}

/// The character: its matcher, what it was last asked, its pose.
struct Walker {
    db: Database,
    matcher: Matcher,
    ask: Ask,
    world: Vec<Mat4>,
    boxes: Vec<(Vec3, Vec3)>,
    physics: scrap::PhysicsWorld,
}

impl Walker {
    fn new(db: Database) -> Self {
        let matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
        let boxes = yard();
        let physics = physics(&boxes);
        Self { db, matcher, ask: Ask::default(), world: Vec::new(), boxes, physics }
    }

    fn step(&mut self, velocity: Vec3, dt: f32) {
        self.ask = Ask { velocity, facing: None };
        let pose = self.matcher.advance_in(&self.db, &self.ask, dt, &Among(&self.physics));
        self.world = self.matcher.world(&self.db, &pose);
    }

    /// The scene: a checkered floor round the character, its bones, where
    /// it is asked to go.
    fn frame(&self, meshes: &Meshes, camera: Camera) -> Frame {
        let mut draws = Vec::new();
        let at = self.matcher.root.0;
        let (cx, cz) = (at.x.round() as i32, at.z.round() as i32);
        for x in cx - 16..=cx + 16 {
            for z in cz - 16..=cz + 16 {
                let light = (x + z).rem_euclid(2) == 0;
                let shade = if light { 0.62 } else { 0.5 };
                draws.push(draw(
                    meshes.tile,
                    Mat4::from_translation(Vec3::new(x as f32, 0.0, z as f32)),
                    Material::new(shade, shade, shade * 0.97),
                ));
            }
        }
        for &(centre, half) in &self.boxes {
            draws.push(draw(
                meshes.cube,
                Mat4::from_scale_rotation_translation(half * 2.0, Quat::IDENTITY, centre),
                Material::new(0.72, 0.6, 0.45),
            ));
        }
        let skeleton = &self.db.skeleton;
        let down = self.db.contacts(self.matcher.frame);
        let feet = self.db.feet();
        for (j, joint) in skeleton.joints.iter().enumerate() {
            let here = self.world[j].w_axis.truncate();
            let locked = feet.iter().position(|&f| f == j).is_some_and(|side| down[side]);
            let colour = if locked { Material::new(1.0, 0.45, 0.1) } else { Material::new(0.2, 0.45, 0.85) };
            draws.push(draw(meshes.ball, Mat4::from_scale_rotation_translation(Vec3::splat(0.07), Quat::IDENTITY, here), colour));
            if let Some(parent) = joint.parent {
                let there = self.world[parent as usize].w_axis.truncate();
                let along = here - there;
                if along.length() > 1e-3 {
                    let turn = Quat::from_rotation_arc(Vec3::Y, along.normalize());
                    draws.push(draw(
                        meshes.bone,
                        Mat4::from_scale_rotation_translation(
                            Vec3::new(0.05, along.length(), 0.05),
                            turn,
                            (here + there) / 2.0,
                        ),
                        Material::new(0.85, 0.85, 0.8),
                    ));
                }
            }
        }
        for (i, (point, facing)) in self.matcher.wanted(&self.db, &self.ask, &Among(&self.physics)).into_iter().enumerate() {
            let size = if i == 0 { 0.1 } else { 0.07 };
            let lift = Vec3::Y * 0.03;
            draws.push(draw(meshes.ball, Mat4::from_scale_rotation_translation(Vec3::splat(size), Quat::IDENTITY, point + lift), Material::new(1.0, 1.0, 1.0)));
            draws.push(draw(
                meshes.bone,
                Mat4::from_scale_rotation_translation(Vec3::new(0.02, 0.25, 0.02), Quat::from_rotation_arc(Vec3::Y, facing), point + lift + facing * 0.125),
                Material::new(1.0, 1.0, 1.0),
            ));
        }
        Frame { camera, draws, ..Frame::default() }
    }

    fn camera(&self, yaw: f32) -> Camera {
        let at = self.matcher.root.0 + Vec3::Y * 0.9;
        let back = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        // Pulled in front of whatever is between it and the character.
        let away = -back * 4.5 + Vec3::Y * 1.6;
        let reach = match self.physics.cast_ray(at, away, away.length()) {
            Some(hit) => (hit.distance - 0.3).max(0.5),
            None => away.length(),
        };
        Camera { position: at + away.normalize() * reach, target: at, ..Camera::default() }
    }
}

struct Drive {
    walker: Walker,
    meshes: Option<Meshes>,
    yaw: f32,
}

impl Game for Drive {
    fn start(&mut self, ctx: &mut Context) {
        self.meshes = Some(Meshes::upload(ctx.gpu, ctx.renderer));
    }

    fn step(&mut self, ctx: &mut StepContext) {
        let dt = ctx.time.settings().fixed_delta;
        let stick = ctx.input.move_axis();
        let ahead = Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = Vec3::new(-ahead.z, 0.0, ahead.x);
        let speed = if ctx.input.held(Key::LeftShift) { 3.8 } else { 1.5 };
        let want = (ahead * stick.y + right * stick.x).clamp_length_max(1.0) * speed;
        self.walker.step(want, dt);
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if ctx.input.pressed(Key::Escape) {
            ctx.quit();
        }
        if ctx.input.mouse_held(scrap::MouseButton::Left) {
            self.yaw -= ctx.input.mouse_motion().x * 0.004;
        }
        match &self.meshes {
            Some(meshes) if !self.walker.world.is_empty() => self.walker.frame(meshes, self.walker.camera(self.yaw)),
            _ => Frame::default(),
        }
    }
}

/// The path `--video` drives, leg by leg: where to, how fast, and for no
/// longer than so many seconds. Into the wall (it stops), round it, up the
/// stairs, over the landing and down, then a run.
const LEGS: &[([f32; 2], f32, f32)] = &[
    ([0.0, 0.0], 0.0, 1.5),
    ([0.0, 6.0], 1.4, 4.5),
    ([3.5, 2.5], 1.4, 3.0),
    ([6.0, 1.0], 1.4, 3.0),
    ([6.0, 12.5], 1.2, 12.0),
    ([6.0, 14.0], 1.4, 2.0),
    ([14.0, 14.0], 3.8, 3.0),
    ([14.0, 14.0], 0.0, 2.0),
];

/// The velocity the legs ask for at a place and time; `None` past the end.
fn scripted(at: Vec3, t: f32) -> Option<Vec3> {
    let mut start = 0.0;
    for &([x, z], speed, seconds) in LEGS {
        if t < start + seconds {
            let to = Vec3::new(x, 0.0, z) - Vec3::new(at.x, 0.0, at.z);
            return Some(if to.length() < 0.3 { Vec3::ZERO } else { to.normalize() * speed });
        }
        start += seconds;
    }
    None
}

fn video(mut walker: Walker, out: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let (width, height, fps) = (1280u32, 720u32, 30u32);
    let gpu = Gpu::headless_blocking(false)?;
    let target = scrap::OffscreenTarget::new(&gpu, width, height);
    let mut renderer = Renderer::new(&gpu, &target);
    let meshes = Meshes::upload(&gpu, &mut renderer);
    let mut ffmpeg = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{width}x{height}"), "-r", &fps.to_string(), "-i", "-"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", out])
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    let stdin = ffmpeg.stdin.as_mut().unwrap();
    let dt = 1.0 / 60.0;
    let seconds: f32 = LEGS.iter().map(|l| l.2).sum();
    let mut yaw: f32 = 0.6;
    let frames = (seconds * fps as f32) as usize;
    let mut stepping = std::time::Duration::ZERO;
    for frame in 0..frames {
        for sub in 0..2 {
            let t = (frame * 2 + sub) as f32 * dt;
            let clock = std::time::Instant::now();
            walker.step(scripted(walker.matcher.root.0, t).unwrap_or_default(), dt);
            stepping += clock.elapsed();
        }
        // The camera swings slowly round behind the direction of travel.
        let v = walker.matcher.spring.1;
        if v.length() > 0.3 {
            let want = v.x.atan2(v.z) + 0.6;
            let gap = (want - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            yaw += gap * 0.02;
        }
        renderer.render(&gpu, &target, &walker.frame(&meshes, walker.camera(yaw)));
        stdin.write_all(&target.read_rgba(&gpu))?;
        if frame % 60 == 0 {
            eprintln!("{frame}/{frames}");
        }
    }
    drop(ffmpeg.stdin.take());
    ffmpeg.wait()?;
    eprintln!(
        "{} jumps, {:.0} µs a step; wrote {out}",
        walker.matcher.jumps,
        stepping.as_secs_f64() * 1e6 / (frames * 2) as f64
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let walker = Walker::new(database()?);
    if let Some(i) = args.iter().position(|a| a == "--video") {
        let out = args.get(i + 1).map(String::as_str).unwrap_or("motion_matching.mp4");
        return video(walker, out);
    }
    println!("WASD walks, shift runs, the left mouse button turns the view, Escape quits.");
    run(
        WindowConfig { title: "scrap — motion matching (LAFAN1)".into(), ..Default::default() },
        Drive { walker, meshes: None, yaw: 0.0 },
    )
}
