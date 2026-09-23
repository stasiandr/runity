//! `showreel <out-dir> [--size WxH] [--fps N] [--only NAME]` — the engine's
//! look, as a video: each shot a scene of the valley example with a camera
//! moving through it, the clock running and the sun going round, written
//! frame by frame into ffmpeg as `<out-dir>/NN-name.mp4`, and a subtitle
//! file naming each shot. Needs `ffmpeg` on the path.
//!
//! Headless like `scene_shot`: the same loading, the same frame, only many
//! of them — routes, physics, footprints and particles run between frames
//! as a game's loop runs them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use runity::glam::Vec3;
use runity::render::Camera;
use runity::{builtin, Gpu, Library, MeshHandle, OffscreenTarget, Renderer, Scene};

struct Shot {
    name: &'static str,
    caption: &'static str,
    scene: &'static str,
    seconds: f32,
    /// Camera position and target at the start and the end.
    from: (Vec3, Vec3),
    to: (Vec3, Vec3),
    /// The hour at the start and the end; `None` keeps the scene's.
    hours: Option<(f32, f32)>,
    /// The clock at the start, and how fast it runs.
    clock: f32,
    speed: f32,
    /// Changes to the scene's look, as the `look` tool sets them.
    look: &'static [(&'static str, &'static str)],
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn shots() -> Vec<Shot> {
    vec![
        Shot {
            name: "desert-walk",
            caption: "Пустыня: марево и мираж, физическое небо, отражённый свет от песка. Ходок оставляет следы, из-под ног пыль",
            scene: "desert.ron",
            seconds: 9.0,
            from: (v(6.0, 1.9, 7.0), v(2.0, 0.6, -4.0)),
            to: (v(5.0, 1.8, -2.0), v(1.5, 0.4, -12.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "tumbleweed",
            caption: "Перекати-поле: ветер сцены тащит его по песку, в порывах — прыжки",
            scene: "desert.ron",
            seconds: 8.0,
            from: (v(-4.0, 0.7, 0.0), v(-11.0, 0.6, -8.0)),
            to: (v(-1.0, 0.8, 0.5), v(5.0, 0.5, -4.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "rays",
            caption: "Тени лучами (аппаратный рейтрейсинг): на закате полутень растёт от камня, тень перекати-поля — кружево веток",
            scene: "desert.ron",
            seconds: 8.0,
            from: (v(-12.5, 1.1, -5.0), v(-10.5, 0.1, -10.5)),
            to: (v(7.5, 1.1, 0.5), v(9.5, 0.1, -5.0)),
            hours: Some((17.7, 17.8)),
            clock: 0.0,
            speed: 1.0,
            look: &[(
                "ray_tracing",
                "(sun_shadows: true, ambient_occlusion: true, sun_size: 0.6, sun_rays: 8, occlusion_rays: 8, occlusion_radius: 1.5)",
            )],
        },
        Shot {
            name: "sand-close",
            caption: "Песок: рябь поперёк ветра, искры песчинок на низком солнце",
            scene: "desert.ron",
            seconds: 7.0,
            from: (v(-3.0, 0.9, 6.0), v(-1.0, 0.0, 2.0)),
            to: (v(1.0, 0.7, 5.0), v(2.5, 0.0, 1.0)),
            hours: Some((17.3, 17.6)),
            clock: 20.0,
            speed: 1.0,
            look: &[("post", "None")],
        },
        Shot {
            name: "sandstorm",
            caption: "Песчаная буря: песок в воздухе волнами, позёмка по поверхности, солнце тусклым диском",
            scene: "desert.ron",
            seconds: 7.0,
            from: (v(0.0, 1.7, 8.0), v(-10.0, 1.5, -30.0)),
            to: (v(0.0, 1.7, 3.0), v(10.0, 1.8, -30.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[
                ("sun", "(hour: 14.0, intensity: 0.9, ground: (0.78, 0.6, 0.38))"),
                ("sky", "(mode: Physical, atmosphere: (mie: 6.0))"),
                ("fog", "(color: (0.8, 0.62, 0.4), start: 5.0, end: 80.0)"),
                ("weather", "(sandstorm: 1.0)"),
                ("wind", "(direction: (1.0, 0.0, 0.2), strength: 3.0)"),
                ("post", "(temperature: 25.0, saturation: 5.0)"),
            ],
        },
        Shot {
            name: "haboob",
            caption: "Хабуб: ярусная стена пыли идёт на камеру и поглощает её (время ускорено)",
            scene: "haboob.ron",
            seconds: 13.0,
            from: (v(0.0, 1.8, 0.0), v(-60.0, 16.0, -8.0)),
            to: (v(3.0, 1.8, 1.0), v(-60.0, 6.0, -14.0)),
            hours: None,
            clock: 4.0,
            speed: 4.6,
            look: &[],
        },
        Shot {
            name: "bounce",
            caption: "Отражённый свет: пробы и SSGI, стены окрашивают пол, пол подсвечивает тень",
            scene: "bounce.ron",
            seconds: 7.0,
            from: (v(3.0, 2.2, 7.0), v(-0.5, 0.8, 0.0)),
            to: (v(-2.5, 2.0, 6.5), v(0.5, 0.8, 0.0)),
            hours: Some((8.2, 9.2)),
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "meadow-sunset",
            caption: "Луг: живая трава на ветру, закат на физическом небе, объёмная дымка",
            scene: "meadow.ron",
            seconds: 9.0,
            from: (v(6.5, 1.1, 1.5), v(-4.0, 1.4, -1.5)),
            to: (v(4.0, 1.4, 4.5), v(-4.0, 1.2, -3.0)),
            hours: Some((16.6, 18.6)),
            clock: 0.0,
            speed: 1.0,
            look: &[
                ("sky", "(mode: Physical, clouds: (coverage: 0.35))"),
                ("volumetric_fog", "(enabled: true, density: 0.015, anisotropy: 0.75, height_falloff: 0.2)"),
            ],
        },
        Shot {
            name: "clouds",
            caption: "Облака: объёмные, плывут по ветру, тени от них ползут по земле (время ускорено)",
            scene: "meadow.ron",
            seconds: 7.0,
            from: (v(8.0, 2.5, 6.0), v(-20.0, 12.0, -30.0)),
            to: (v(8.0, 2.5, 6.0), v(-30.0, 10.0, -10.0)),
            hours: Some((13.0, 14.0)),
            clock: 0.0,
            speed: 30.0,
            look: &[
                ("sky", "(mode: Physical, clouds: (coverage: 0.55, shadows: 0.8))"),
                ("wind", "(strength: 1.5)"),
            ],
        },
        Shot {
            name: "rain",
            caption: "Дождь: мокрые поверхности, лужи с кругами, отражения по экрану",
            scene: "rain.ron",
            seconds: 7.0,
            from: (v(0.0, 2.4, 7.5), v(0.0, 0.9, 0.0)),
            to: (v(3.5, 1.6, 5.5), v(-0.5, 0.5, 0.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "pond",
            caption: "Вода: волны, отражения, глубина и каустика",
            scene: "pond.ron",
            seconds: 7.0,
            from: (v(1.5, 3.6, 7.0), v(0.0, -0.8, -4.0)),
            to: (v(-3.0, 2.2, 5.0), v(1.0, -0.6, -4.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "reflections",
            caption: "Пробы отражений: хром, шероховатый металл, полированный пол",
            scene: "reflections.ron",
            seconds: 6.0,
            from: (v(-3.0, 2.4, 7.0), v(0.0, 0.9, 0.0)),
            to: (v(3.0, 1.8, 6.5), v(0.0, 0.8, 0.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[],
        },
        Shot {
            name: "night-camp",
            caption: "Ночь: лампы с тенями в дымке, Forward+",
            scene: "camp.ron",
            seconds: 7.0,
            from: (v(0.0, 2.4, 6.0), v(0.0, 0.3, -1.6)),
            to: (v(4.0, 1.6, 4.0), v(0.0, 0.4, -1.6)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            look: &[
                ("sun", "(hour: 21.0, intensity: 0.08)"),
                ("sky", "(mode: Procedural, zenith: (0.01, 0.015, 0.04), horizon: (0.03, 0.04, 0.07), ground: (0.01, 0.01, 0.015), sun_size: 0.0)"),
                ("fog", "(color: (0.03, 0.04, 0.07), start: 20.0, end: 150.0)"),
                ("volumetric_fog", "(enabled: true, density: 0.03, ambient: 0.3, lamps: 6.0)"),
                ("post", "(exposure: 0.8, bloom: (intensity: 0.6), temperature: -20.0)"),
            ],
        },
    ]
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let (mut width, mut height) = (1280u32, 720u32);
    let mut fps = 30u32;
    let mut only: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let size = args.next().ok_or("--size wants WIDTHxHEIGHT")?;
                let (w, h) = size.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                width = w.parse()?;
                height = h.parse()?;
            }
            "--fps" => fps = args.next().ok_or("--fps wants a number")?.parse()?,
            "--only" => only = args.next(),
            other => out = Some(PathBuf::from(other)),
        }
    }
    let out = out.ok_or("usage: showreel <out-dir>")?;
    std::fs::create_dir_all(&out)?;
    let scenes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/valley/scenes");
    let gpu = Gpu::headless_blocking(false)?;
    let target = OffscreenTarget::new(&gpu, width, height);

    let mut subtitles = String::new();
    let mut at = 0.0f32;
    let list = shots();
    let total: u32 = list
        .iter()
        .filter(|s| only.as_deref().is_none_or(|o| o == s.name))
        .map(|s| (s.seconds * fps as f32) as u32)
        .sum();
    let mut done = 0u32;
    for (index, shot) in list.iter().enumerate() {
        if only.as_deref().is_some_and(|o| o != shot.name) {
            continue;
        }
        let file = out.join(format!("{:02}-{}.mp4", index + 1, shot.name));
        render_shot(
            &gpu,
            &target,
            &scenes.join(shot.scene),
            shot,
            fps,
            &file,
            &mut done,
            total,
        )?;
        let end = at + shot.seconds;
        let stamp = |s: f32| {
            let ms = (s * 1000.0) as u32;
            format!(
                "{:02}:{:02}:{:02},{:03}",
                ms / 3_600_000,
                ms / 60_000 % 60,
                ms / 1000 % 60,
                ms % 1000
            )
        };
        subtitles.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            stamp(at),
            stamp(end),
            shot.caption
        ));
        at = end;
    }
    std::fs::write(out.join("captions.srt"), subtitles)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_shot(
    gpu: &Gpu,
    target: &OffscreenTarget,
    path: &Path,
    shot: &Shot,
    fps: u32,
    file: &Path,
    done: &mut u32,
    total: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut document = Scene::load(path)?;
    for (field, text) in shot.look {
        runity::scene::set_look_field(&mut document, field, text)?;
    }
    let project = runity::Project::find(path).ok();
    let (prefabs, _) = project
        .as_ref()
        .map(runity::Prefabs::of)
        .unwrap_or_default();
    let mut scene = runity::instantiate(&document, &prefabs).scene;

    let mut renderer = Renderer::new(gpu, target);
    if let Some(project) = &project {
        let mut shaders =
            runity::render::MaterialShaders::new(project.root().join(runity::project::SHADERS));
        for (name, result) in shaders.poll(&mut renderer, gpu) {
            if let Err(problem) = result {
                eprintln!("shader {name}: {problem}");
            }
        }
    }
    let library = project
        .as_ref()
        .map(|p| p.library())
        .filter(|dir| dir.is_dir())
        .and_then(|dir| Library::open(&dir).ok().map(|(l, _)| l));
    let mut world = hecs::World::new();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    runity::spawn_scene_with(
        &scene,
        &mut world,
        |name| {
            let name: &str = name;
            if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                return Some(found.1);
            }
            let handle = if let Some(mesh) = builtin::by_name(name) {
                renderer.upload_mesh_owned(gpu, &mesh)
            } else {
                let mesh = library.as_ref()?.mesh_by_name(name)?;
                renderer.upload_mesh(gpu, mesh)
            };
            uploaded.push((name.to_string(), handle));
            Some(handle)
        },
        |name| library.as_ref()?.material_by_name(name),
    );
    runity::terrain::upload_terrains(&mut world, gpu, &mut renderer);
    for problem in runity::world::upload_material_maps(&world, library.as_ref(), gpu, &mut renderer)
    {
        eprintln!("{problem}");
    }

    // Physics runs too: what the wind carries rolls across the shot.
    runity::physics::attach_scene_collision_meshes(&mut world, &scene, library.as_ref());
    let step = 1.0 / 60.0;
    let mut physics = runity::PhysicsWorld::new(step);
    physics.wind = scene.wind.unwrap_or_default();
    let mut owed = 0.0f32;

    let (w, h) = (target.width, target.height);
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ])
        .arg(format!("{w}x{h}"))
        .args([
            "-r",
            &fps.to_string(),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-crf", "18", "-preset", "medium"])
        .arg(file)
        .stdin(Stdio::piped())
        .spawn()?;
    let mut pipe = ffmpeg.stdin.take().ok_or("no pipe to ffmpeg")?;

    let frames = (shot.seconds * fps as f32) as u32;
    let dt = 1.0 / fps as f32;
    let base = runity::scene_camera(&scene.view);
    // A second of the scene first, unrecorded: the trail has steps, the
    // dust is up, and what reads the last frame has one.
    let warmup = fps;
    for i in 0..warmup + frames {
        let t = smooth(i.saturating_sub(warmup) as f32 / frames.max(1) as f32);
        let clock = shot.clock + (i as f32 - warmup as f32) * dt * shot.speed;
        runity::routes::run_routes(&mut world, dt * shot.speed);
        runity::world::apply_hierarchy(&mut world);
        owed += dt * shot.speed;
        while owed >= step {
            physics.run(&mut world);
            owed -= step;
        }
        runity::world::apply_hierarchy(&mut world);
        runity::footprints::run_footprints(&mut world, dt * shot.speed);
        runity::particles::run_particles(&mut world, dt * shot.speed);
        if let Some((a, b)) = shot.hours {
            scene.sun.hour = a + (b - a) * t;
        }
        let camera = Camera {
            position: shot.from.0.lerp(shot.to.0, t),
            target: shot.from.1.lerp(shot.to.1, t),
            ..base
        };
        let mut frame = runity::world::scene_frame(&world, camera, &scene);
        frame.time = Some(clock.max(0.0));
        renderer.render(gpu, target, &frame);
        if i >= warmup {
            pipe.write_all(&target.read_rgba(gpu))?;
            *done += 1;
            if *done % 30 == 0 {
                println!("{}/{} {}", done, total, shot.name);
            }
        }
    }
    drop(pipe);
    let status = ffmpeg.wait()?;
    if !status.success() {
        return Err(format!("ffmpeg failed on {}", shot.name).into());
    }
    Ok(())
}
