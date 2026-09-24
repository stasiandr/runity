//! `showreel <out-dir> [--size WxH] [--fps N] [--only NAME] [--reel main|rays|sim]` — the engine's
//! look, as a video: each shot a scene of the valley example with a camera
//! moving through it, the clock running and the sun going round, written
//! frame by frame into ffmpeg as `<out-dir>/NN-name.mp4`, and a subtitle
//! file naming each shot. Needs `ffmpeg` on the path.
//!
//! Headless like `scene_shot`: the same loading, the same frame, only many
//! of them — routes, physics, footprints and particles run between frames
//! as a game's loop runs them.

#[allow(unused_imports)]
use runity::prelude::*;
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
    /// Split down the middle: on the left drawn without rays, on the right
    /// with the scene's `ray_tracing` — the same world, the same moment.
    compare: bool,
    /// Seconds of the scene run before the recording starts; a second when
    /// not said.
    warmup: Option<f32>,
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn shots() -> Vec<Shot> {
    vec![
        Shot {
            name: "desert-walk",
            caption: "Пустыня: марево и мираж, физическое небо, отражённый свет от песка. Ходок оставляет следы, из-под ног пыль, ветер гонит перекати-поле",
            scene: "desert.ron",
            seconds: 9.0,
            from: (v(6.0, 1.9, 7.0), v(2.0, 0.6, -4.0)),
            to: (v(5.0, 1.8, -2.0), v(1.5, 0.4, -12.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[("weather", "(drifted: 1.0)")],
        },
        Shot {
            name: "passage",
            caption: "Автоэкспозиция: из тёмного проезда на полуденное солнце — глаз привыкает за пару секунд",
            scene: "desert.ron",
            seconds: 8.0,
            from: (v(-40.0, 1.6, 25.0), v(-40.0, 1.4, 5.0)),
            to: (v(-40.0, 1.6, 5.0), v(-38.0, 1.3, -15.0)),
            hours: None,
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "dunes",
            caption: "Дюны: рельеф с тесселяцией у камеры, с гребней ветер срывает песок, по полю бродят пылевые вихри",
            scene: "desert.ron",
            seconds: 9.0,
            from: (v(-6.0, 3.0, -60.0), v(-20.0, 4.0, -160.0)),
            to: (v(6.0, 3.5, -64.0), v(25.0, 4.0, -160.0)),
            hours: Some((15.5, 15.8)),
            clock: 30.0,
            speed: 1.0,
            compare: false,
            look: &[
                ("weather", "(dust_devils: 1.0)"),
                ("wind", "(direction: (1.0, 0.0, 0.3), strength: 1.8)"),
            ],
            warmup: None,
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
            compare: false,
            look: &[
                ("sun", "(hour: 14.0, intensity: 0.9, ground: (0.78, 0.6, 0.38))"),
                ("sky", "(mode: Physical, atmosphere: (mie: 6.0))"),
                ("fog", "(color: (0.8, 0.62, 0.4), start: 5.0, end: 80.0)"),
                ("weather", "(sandstorm: 1.0)"),
                ("wind", "(direction: (1.0, 0.0, 0.2), strength: 3.0)"),
                ("post", "(temperature: 25.0, saturation: 5.0)"),
            ],
            warmup: None,
        },
        Shot {
            name: "haboob",
            caption: "Хабуб: ярусная стена пыли идёт на камеру и поглощает её (время ускорено)",
            scene: "haboob.ron",
            seconds: 19.0,
            from: (v(0.0, 1.8, 0.0), v(-60.0, 16.0, -8.0)),
            to: (v(3.0, 1.8, 1.0), v(-60.0, 6.0, -14.0)),
            hours: None,
            clock: 4.0,
            speed: 4.6,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "desert-night",
            caption: "Ночь в пустыне: солнце садится, светит луна, выходят звёзды и Млечный Путь (время ускорено)",
            scene: "desert.ron",
            seconds: 10.0,
            from: (v(0.0, 1.7, 8.0), v(-40.0, 2.0, -40.0)),
            to: (v(0.0, 1.7, 8.0), v(-60.0, 14.0, -20.0)),
            hours: Some((18.0, 22.5)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
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
            compare: false,
            warmup: None,
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
            compare: false,
            look: &[
                ("sky", "(mode: Physical, clouds: (coverage: 0.35))"),
                ("volumetric_fog", "(enabled: true, density: 0.015, anisotropy: 0.75, height_falloff: 0.2)"),
            ],
            warmup: None,
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
            compare: false,
            look: &[
                ("sky", "(mode: Physical, clouds: (coverage: 0.55, shadows: 0.8))"),
                ("wind", "(strength: 1.5)"),
            ],
            warmup: None,
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
            compare: false,
            warmup: None,
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
            compare: false,
            warmup: None,
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
            compare: false,
            look: &[
                ("sun", "(hour: 21.0, intensity: 0.08)"),
                ("sky", "(mode: Procedural, zenith: (0.01, 0.015, 0.04), horizon: (0.03, 0.04, 0.07), ground: (0.01, 0.01, 0.015), sun_size: 0.0)"),
                ("fog", "(color: (0.03, 0.04, 0.07), start: 20.0, end: 150.0)"),
                ("volumetric_fog", "(enabled: true, density: 0.03, ambient: 0.3, lamps: 6.0)"),
                ("post", "(exposure: 0.8, bloom: (intensity: 0.6), temperature: -20.0)"),
            ],
            warmup: None,
        },
    ]
}

/// The ray-tracing reel (`--reel rays`): a courtyard made for the rays
/// (lanterns.ron) — lattice shadows, mirrors that see behind the camera,
/// glass, and a score of lamps each shadowing every bar.
fn ray_shots() -> Vec<Shot> {
    vec![
        Shot {
            name: "rt-pergola",
            caption: "Лучи: солнце сквозь перголу. Узор резкий у балок и мягкий вдали — полутень растёт с расстоянием, как у настоящего солнца",
            scene: "lanterns.ron",
            seconds: 8.0,
            from: (v(-6.5, 1.5, 6.0), v(3.0, 0.5, -2.0)),
            to: (v(-5.5, 2.0, 2.5), v(4.0, 1.0, -4.0)),
            hours: Some((15.4, 16.4)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "rt-mirrors",
            caption: "Лучи: отражения. Вода с волнами, хром и бронза отражают весь двор — и то, что за камерой, чего на экране нет",
            scene: "lanterns.ron",
            seconds: 9.0,
            from: (v(-4.5, 1.3, 7.5), v(-1.5, 0.4, 2.5)),
            to: (v(4.5, 1.1, 7.0), v(1.5, 0.4, 2.8)),
            hours: Some((16.3, 16.5)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "rt-glass",
            caption: "Лучи: преломление. Стеклянный шар — линза, двор в нём перевёрнут; дно бассейна изгибается под волнами",
            scene: "lanterns.ron",
            seconds: 8.0,
            from: (v(-1.2, 0.9, 7.4), v(-2.9, 0.45, 4.9)),
            to: (v(-4.6, 0.8, 6.8), v(-2.8, 0.45, 4.9)),
            hours: Some((16.5, 16.6)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "rt-dusk",
            caption: "Лучи: садится солнце, зажигаются фонари. Каждый из четырнадцати даёт свою мягкую тень каждой планки (время ускорено)",
            scene: "lanterns.ron",
            seconds: 10.0,
            from: (v(0.0, 1.8, 8.0), v(0.0, 1.0, -4.0)),
            to: (v(-1.5, 1.6, 6.0), v(1.0, 1.0, -4.0)),
            hours: Some((17.0, 19.6)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "rt-lanterns",
            caption: "Лучи: ночь. Фонари качаются — тени решёток ходят по стенам и полу, огни отражаются в воде и в шарах",
            scene: "lanterns.ron",
            seconds: 10.0,
            from: (v(5.5, 1.5, 6.0), v(0.0, 1.0, -1.0)),
            to: (v(-5.0, 1.7, 5.0), v(0.0, 1.0, -2.0)),
            hours: Some((21.5, 21.5)),
            clock: 0.0,
            speed: 1.0,
            compare: false,
            warmup: None,
            look: &[],
        },
        Shot {
            name: "rt-without",
            caption: "Для сравнения: слева тот же двор без лучей, справа с ними",
            scene: "lanterns.ron",
            seconds: 7.0,
            from: (v(0.0, 1.5, 8.0), v(0.0, 0.6, 1.0)),
            to: (v(1.5, 1.6, 7.5), v(0.0, 0.6, 1.0)),
            hours: Some((21.5, 21.5)),
            clock: 0.0,
            speed: 1.0,
            compare: true,
            warmup: None,
            look: &[("screen_space_reflections", "(enabled: true)")],
        },
    ]
}

/// The simulation reel (`--reel sim`): one shot for each thing the soft
/// and fluid modules simulate (docs/simulation.md), in the order of the
/// list.
fn sim_shots() -> Vec<Shot> {
    let shot = |name, caption, scene, seconds, from: (Vec3, Vec3), to: (Vec3, Vec3)| Shot {
        name,
        caption,
        scene,
        seconds,
        from,
        to,
        hours: None,
        clock: 0.0,
        speed: 1.0,
        look: &[],
        compare: false,
        warmup: Some(0.3),
    };
    vec![
        shot(
            "ropes",
            "Верёвки, кабели, цепи: стержни Коссера на XPBD, трубка по сплайну и звенья инстансами; ветер качает, ящик держит",
            "ropes.ron",
            8.0,
            (v(-3.0, 2.2, 5.5), v(-1.5, 1.4, -3.0)),
            (v(3.0, 2.4, 5.0), v(2.5, 1.6, -4.0)),
        ),
        shot(
            "cloth",
            "Ткань: знамя, флаг, простыня на верёвке, навес и скатерть, упавшая на стол и не проходящая сквозь себя",
            "cloth.ron",
            8.0,
            (v(-3.0, 3.8, 7.0), v(-3.0, 2.0, -3.0)),
            (v(3.5, 4.0, 6.5), v(3.0, 1.6, -4.0)),
        ),
        shot(
            "hair",
            "Волосы и мех: направляющие пряди — стержни Коссера, остальные интерполируются; ветер треплет",
            "hair.ron",
            7.0,
            (v(-1.5, 2.0, 3.0), v(0.0, 1.5, 0.0)),
            (v(1.5, 2.1, 3.0), v(0.0, 1.5, 0.0)),
        ),
        shot(
            "softbody",
            "Мягкие тела: желе на тетраэдрах, мяч, мармелад на сопоставлении формы, слизь; плоть на кости и jiggle-кости с карточками",
            "softbody.ron",
            9.0,
            (v(-2.5, 2.4, 5.5), v(-1.0, 0.8, -0.5)),
            (v(2.5, 2.6, 5.5), v(1.0, 0.8, -0.5)),
        ),
        shot(
            "destruction",
            "Разрушение: Вороной заранее и в момент удара, куски бьются снова, крошки на GPU, вмятины на машине",
            "destruction.ron",
            9.0,
            (v(-6.0, 3.5, 9.0), v(-3.0, 1.5, -1.0)),
            (v(6.0, 4.0, 10.0), v(4.0, 1.2, 0.0)),
        ),
        shot(
            "fluids",
            "Жидкости на частицах: прорыв плотины — PBF (поверхность Surface Nets) и SPH (капли), вода MPM с FLIP-переносом",
            "fluids.ron",
            6.0,
            (v(-6.5, 1.6, 2.6), v(-4.5, 0.3, -1.0)),
            (v(-2.5, 1.8, 2.6), v(-2.5, 0.3, -1.2)),
        ),
        shot(
            "flood",
            "Мелкая вода на трубах: паводок за плотиной обтекает камни, ящики плывут, едущий блок гонит волну",
            "fluids.ron",
            7.0,
            (v(3.5, 3.2, 4.5), v(3.5, 0.0, -1.0)),
            (v(5.5, 2.6, 3.5), v(3.8, 0.0, -1.0)),
        ),
        shot(
            "ocean",
            "Открытое море: FFT-океан под ветер сцены, буи, ящики и плот качаются на волнах",
            "ocean.ron",
            8.0,
            (v(-10.0, 5.0, 18.0), v(0.0, 0.0, 0.0)),
            (v(10.0, 3.0, 16.0), v(0.0, 0.5, 0.0)),
        ),
        shot(
            "smoke",
            "Дым, огонь и пар: Stable Fluids на разреженной сетке блоками 8³, в тумане лучом; костёр светится, пар обтекает навес",
            "smoke.ron",
            8.0,
            (v(-4.5, 1.5, 4.5), v(-3.0, 1.4, 0.0)),
            (v(3.5, 2.0, 5.5), v(3.5, 1.8, 0.0)),
        ),
        shot(
            "granular",
            "Сыпучее: песок MPM ложится кучей, снежный ком разбивается о ящик, гравий DEM сыплется с пандуса, санки и шар оставляют колею в снегу",
            "granular.ron",
            9.0,
            (v(-3.5, 2.2, 3.5), v(-2.0, 0.4, -1.0)),
            (v(4.0, 2.5, 5.0), v(3.0, 0.2, 1.2)),
        ),
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
    let mut reel: Option<String> = None;
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
            "--reel" => reel = args.next(),
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
    let list = match reel.as_deref() {
        None | Some("main") => shots(),
        Some("rays") => ray_shots(),
        Some("sim") => sim_shots(),
        Some(other) => return Err(format!("no reel `{other}`: main, rays or sim").into()),
    };
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

    // For a split shot, a second renderer of the same world: its own TAA
    // and exposure history, so neither half bleeds into the other. The
    // world's meshes go in in the same order, so the handles are the same.
    let mut twin = if shot.compare {
        let mut twin = Renderer::new(gpu, target);
        if let Some(project) = &project {
            let mut shaders =
                runity::render::MaterialShaders::new(project.root().join(runity::project::SHADERS));
            let _ = shaders.poll(&mut twin, gpu);
        }
        for (name, handle) in &uploaded {
            let again = if let Some(mesh) = builtin::by_name(name) {
                twin.upload_mesh_owned(gpu, &mesh)
            } else {
                let mesh = library
                    .as_ref()
                    .and_then(|l| l.mesh_by_name(name))
                    .ok_or("a mesh gone from the library")?;
                twin.upload_mesh(gpu, mesh)
            };
            assert_eq!(again, *handle, "the twin's meshes out of step at {name}");
        }
        for (relief, model) in
            world.query_mut::<(&runity::terrain::Relief, &runity::world::Model)>()
        {
            let again = twin.upload_mesh_owned(gpu, &relief.terrain.mesh());
            assert_eq!(again, model.0, "the twin's terrain out of step");
        }
        for problem in runity::world::upload_material_maps(&world, library.as_ref(), gpu, &mut twin)
        {
            eprintln!("{problem}");
        }
        Some(twin)
    } else {
        None
    };

    // Physics runs too: what the wind carries rolls across the shot.
    runity::physics::attach_scene_collision_meshes(&mut world, &scene, library.as_ref());
    let step = 1.0 / 60.0;
    let mut physics = runity::PhysicsWorld::new(step);
    physics.wind = scene.wind().unwrap_or_default();
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
    let base = runity::scene_camera(&scene.view());
    // A second of the scene first, unrecorded: the trail has steps, the
    // dust is up, and what reads the last frame has one.
    let warmup = (shot.warmup.unwrap_or(1.0) * fps as f32) as u32;
    for i in 0..warmup + frames {
        let t = smooth(i.saturating_sub(warmup) as f32 / frames.max(1) as f32);
        let clock = shot.clock + (i as f32 - warmup as f32) * dt * shot.speed;
        runity::routes::run_routes(&mut world, dt * shot.speed);
        runity::world::apply_hierarchy(&mut world);
        owed += dt * shot.speed;
        while owed >= step {
            runity::fluid::float(&world, &mut physics);
            physics.run(&mut world);
            runity::fluid::step(&mut world, step);
            runity::destruction::step(&mut world, &mut physics, step);
            owed -= step;
        }
        runity::world::apply_hierarchy(&mut world);
        // What bends, hangs and flows, on its own fixed steps.
        runity::soft::step(&mut world, dt * shot.speed);
        runity::soft::show(&mut world, 0.0);
        runity::destruction::show(&mut world, 0.0);
        runity::fluid::show(&mut world, 0.0);
        runity::footprints::run_footprints(&mut world, dt * shot.speed);
        runity::particles::run_particles(&mut world, dt * shot.speed);
        if let Some((a, b)) = shot.hours {
            let mut sun = scene.sun();
            sun.hour = a + (b - a) * t;
            scene.set_part(&sun);
        }
        let camera = Camera {
            position: shot.from.0.lerp(shot.to.0, t),
            target: shot.from.1.lerp(shot.to.1, t),
            ..base
        };
        let mut frame = runity::world::scene_frame(&world, camera, &scene);
        frame.time = Some(clock.max(0.0));
        let pixels = if let Some(twin) = twin.as_mut() {
            // Left without rays, right with them, and a thin line between.
            let traced = frame.ray_tracing;
            frame.ray_tracing = runity::ray::RayTracing::default();
            renderer.render(gpu, target, &frame);
            let mut left = target.read_rgba(gpu);
            frame.ray_tracing = traced;
            twin.render(gpu, target, &frame);
            let right = target.read_rgba(gpu);
            let (w, h) = (target.width as usize, target.height as usize);
            for y in 0..h {
                let row = y * w * 4;
                left[row + w * 2..row + w * 4].copy_from_slice(&right[row + w * 2..row + w * 4]);
                for x in w / 2 - 1..w / 2 + 1 {
                    left[row + x * 4..row + x * 4 + 3].copy_from_slice(&[235, 235, 235]);
                }
            }
            left
        } else {
            renderer.render(gpu, target, &frame);
            target.read_rgba(gpu)
        };
        if i >= warmup {
            pipe.write_all(&pixels)?;
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
