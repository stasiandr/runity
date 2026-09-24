//! `netreel <out-dir> [--size WxH] [--only NAME]` — simulations over the
//! network on film (docs/netsim.md): each shot one of the bench's scenes
//! played by a host and a guest in one process over a poor link (80 ms
//! there and back, 3% lost), the host's picture on the left and the
//! guest's on the right, written as `<out-dir>/NN-name.mp4` with a
//! subtitle file. Needs `ffmpeg` on the path.
//!
//! The session runs at the pace of real time — its link's delays are the
//! clock's — and only gathers what each peer would draw; the drawing comes
//! after, as slowly as it likes, so filming does not make the link better
//! than it is.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use runity::glam::Vec3;
use runity::net::wire::Conditions;
use runity::netsim::bench::{ticks, Peer, Session, HZ};
use runity::netsim::scenes::*;
use runity::render::{Camera, Frame};
use runity::scene::{Scene, Transform};
#[allow(unused_imports)]
use runity::prelude::*;
use runity::{builtin, Gpu, MeshHandle, OffscreenTarget, Renderer};

/// What a shot does on each tick, by peer, before the physics.
type Game = Box<dyn FnMut(usize, usize, &mut Peer)>;

struct Shot {
    name: &'static str,
    caption: &'static str,
    scene: Scene,
    seconds: f32,
    camera: (Vec3, Vec3),
    /// Before the recording: who claims what (peer, id), and the ticks
    /// run first.
    claims: Vec<(usize, u64)>,
    warmup: f32,
    pawns: Vec<u64>,
    /// The camera goes along with the middle of these entities (as the
    /// host shows them) in x.
    follow: Vec<u64>,
    game: Game,
}

fn shots() -> Vec<Shot> {
    let mut out = Vec::new();
    out.push(Shot {
        name: "chain",
        caption: "Full: цепь с грузом 10 кг передают на лету — гость, хост, гость; слева картинка хоста, справа гостя; связь 80 мс, 3% потерь",
        scene: pendulum(runity::netsim::NetMode::Full),
        seconds: 5.0,
        camera: (Vec3::new(0.3, 2.3, 5.0), Vec3::new(0.3, 2.0, 0.0)),
        claims: vec![],
        warmup: 0.2,
        pawns: vec![],
        follow: vec![],
        game: Box::new(|tick, peer, p| {
            let claim = |p: &mut Peer| {
                if let Some(e) = p.entity(POST) {
                    p.party.claim(&mut p.world, e);
                }
            };
            if (tick == ticks(0.8) || tick == ticks(2.8)) && peer == 1 {
                claim(p);
            }
            if tick == ticks(1.8) && peer == 0 {
                claim(p);
            }
        }),
    });
    out.push(Shot {
        name: "tug",
        caption: "Верёвку тянут двое: синий (хост) тянет влево 200 Н, оранжевый (гость) вправо 700 Н — у обоих перетягивает оранжевый, верёвка не длиннее себя",
        scene: tug(runity::netsim::NetMode::Full),
        seconds: 4.0,
        camera: (Vec3::new(0.0, 2.2, 6.0), Vec3::new(0.0, 0.4, 0.0)),
        claims: vec![(1, PAWN_B)],
        warmup: 0.6,
        pawns: vec![PAWN_A, PAWN_B],
        follow: vec![PAWN_A, PAWN_B],
        game: Box::new(|_, _, p| {
            for (id, force) in [(PAWN_A, Vec3::new(-200.0, 0.0, 0.0)), (PAWN_B, Vec3::new(700.0, 0.0, 0.0))] {
                let Some(e) = p.entity(id) else { continue };
                if p.world.get::<&runity::net::Owned>(e).is_ok() {
                    p.physics.add_force(&p.world, e, force);
                }
            }
        }),
    });
    out.push(Shot {
        name: "cape",
        caption: "Rough и Local: плащ и хвост у бегуна гостя — каждый считает сам, от гладко показанного тела и общих часов ветра",
        scene: runner(runity::netsim::NetMode::Rough),
        seconds: 5.0,
        camera: (Vec3::new(0.0, 5.0, 8.5), Vec3::new(0.0, 0.6, 0.0)),
        claims: vec![(1, RUNNER)],
        warmup: 1.0,
        pawns: vec![RUNNER],
        follow: vec![],
        game: Box::new(|tick, _, p| {
            let Some(e) = p.entity(RUNNER) else { return };
            if p.world.get::<&runity::net::Owned>(e).is_err() {
                return;
            }
            let angle = tick as f32 / HZ;
            if let Ok(mut t) = p.world.get::<&mut Transform>(e) {
                t.position = Vec3::new(angle.cos() * 3.0, 0.8, angle.sin() * 3.0);
                t.set_rotation(runity::glam::Quat::from_rotation_y(-angle));
            }
        }),
    });
    out.push(Shot {
        name: "wall",
        caption: "Event: стену хоста ломает хост; гость получает удар и ломает её так же — те же куски под теми же именами, дальше они тела хоста",
        scene: wall(),
        seconds: 4.0,
        camera: (Vec3::new(4.5, 2.2, 3.5), Vec3::new(0.0, 0.8, 0.0)),
        claims: vec![],
        warmup: 0.2,
        pawns: vec![],
        follow: vec![],
        game: Box::new(|_, _, _| {}),
    });
    out.push(Shot {
        name: "ragdoll",
        caption: "Full ragdoll гостя: его толкают у гостя, он обмякает, оседает и встаёт — у хоста так же",
        scene: person(),
        seconds: 6.0,
        camera: (Vec3::new(2.8, 1.6, 3.2), Vec3::new(0.0, 0.8, 0.0)),
        claims: vec![(1, PERSON)],
        warmup: 1.5,
        pawns: vec![PERSON],
        follow: vec![],
        game: Box::new(|tick, peer, p| {
            if peer == 1 && (ticks(0.5)..ticks(0.63)).contains(&tick) {
                let Some(e) = p.entity(PERSON) else { return };
                let parts = p.world.get::<&runity::character::RagdollState>(e).map(|s| s.parts.clone()).unwrap_or_default();
                if parts.len() > 2 {
                    p.physics.set_velocity(&p.world, parts[1], Vec3::new(6.0, 0.0, 0.0));
                    p.physics.set_velocity(&p.world, parts[2], Vec3::new(6.0, 0.0, 0.0));
                }
            }
        }),
    });
    out.push(Shot {
        name: "crates",
        caption: "Заявка до касания: ящики хоста и гостя толкают друг в друга — хост берёт оба до удара, удар решает одна машина",
        scene: crates(),
        seconds: 3.0,
        camera: (Vec3::new(0.0, 2.5, 6.5), Vec3::new(0.0, 0.3, 0.0)),
        claims: vec![(1, CRATE_B)],
        warmup: 0.7,
        pawns: vec![],
        follow: vec![],
        game: Box::new(|tick, _, p| {
            if tick != ticks(0.3) {
                return;
            }
            for (id, v) in [(CRATE_A, 9.0), (CRATE_B, -9.0)] {
                let Some(e) = p.entity(id) else { continue };
                if p.world.get::<&runity::net::Owned>(e).is_ok() {
                    p.physics.set_velocity(&p.world, e, Vec3::new(v, 0.0, 0.0));
                }
            }
        }),
    });
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let (mut width, mut height) = (1280u32, 720u32);
    let mut only: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let size = args.next().ok_or("--size wants WIDTHxHEIGHT")?;
                let (w, h) = size.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                width = w.parse()?;
                height = h.parse()?;
            }
            "--only" => only = args.next(),
            other => out = Some(PathBuf::from(other)),
        }
    }
    let out = out.ok_or("usage: netreel <out-dir>")?;
    std::fs::create_dir_all(&out)?;
    let gpu = Gpu::headless_blocking(false)?;
    // Each half of the picture is a peer's.
    let half = OffscreenTarget::new(&gpu, width / 2, height);
    let mut subtitles = String::new();
    let mut at = 0.0f32;
    for (index, shot) in shots().into_iter().enumerate() {
        if only.as_deref().is_some_and(|o| o != shot.name) {
            continue;
        }
        let (name, caption, seconds) = (shot.name, shot.caption, shot.seconds);
        film(&gpu, &half, shot, &out.join(format!("{:02}-{name}.mp4", index + 1)))?;
        let stamp = |s: f32| {
            let ms = (s * 1000.0) as u32;
            format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
        };
        subtitles.push_str(&format!("{}\n{} --> {}\n{}\n\n", index + 1, stamp(at), stamp(at + seconds), caption));
        at += seconds;
        println!("filmed {name}");
    }
    std::fs::write(out.join("captions.srt"), subtitles)?;
    Ok(())
}

/// Play a shot in real time, gathering both peers' frames; then draw them
/// side by side into ffmpeg.
fn film(gpu: &Gpu, half: &OffscreenTarget, mut shot: Shot, file: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut renderers = [Renderer::new(gpu, half), Renderer::new(gpu, half)];
    let mut uploaded: [Vec<(String, MeshHandle)>; 2] = [Vec::new(), Vec::new()];
    let mut meshes = |peer: usize, name: &runity::AssetLink| {
        let name: &str = name;
        if let Some(found) = uploaded[peer].iter().find(|(n, _)| n == name) {
            return Some(found.1);
        }
        let mesh = builtin::by_name(name)?;
        let handle = renderers[peer].upload_mesh_owned(gpu, &mesh);
        uploaded[peer].push((name.to_string(), handle));
        Some(handle)
    };
    let mut session = Session::filmed(&shot.scene, 2, Conditions::POOR, 7, &mut meshes);
    for p in &mut session.peers {
        for id in &shot.pawns {
            if let Some(e) = p.entity(*id) {
                let _ = p.world.insert_one(e, runity::netsim::Pawn);
            }
        }
    }
    if !session.join() {
        return Err(format!("{}: nobody got in", shot.name).into());
    }
    for (peer, id) in &shot.claims {
        session.claim(*peer, *id);
    }
    for _ in 0..ticks(shot.warmup) {
        session.step();
    }
    let lighting = runity::scene_lighting(&shot.scene.sun());
    let camera = Camera { position: shot.camera.0, target: shot.camera.1, ..runity::scene_camera(&shot.scene.view()) };
    let fog = runity::render::FogSettings::default();
    // Thirty frames a second: every other step of sixty.
    let mut frames: Vec<[Frame; 2]> = Vec::new();
    for tick in 0..ticks(shot.seconds) {
        let game = &mut shot.game;
        session.step_with(|peer, p| game(tick, peer, p));
        if tick % 2 == 0 {
            let host = &session.peers[0];
            let xs: Vec<f32> = shot
                .follow
                .iter()
                .filter_map(|id| host.entity(*id).and_then(|e| host.world.get::<&Transform>(e).ok().map(|t| t.position.x)))
                .collect();
            let along = if xs.is_empty() { 0.0 } else { xs.iter().sum::<f32>() / xs.len() as f32 };
            let camera = Camera { position: camera.position + Vec3::X * along, target: camera.target + Vec3::X * along, ..camera };
            let pair = [0, 1].map(|i| {
                let p = &mut session.peers[i];
                runity::soft::show(&mut p.world, 0.0);
                runity::destruction::show(&mut p.world, 0.0);
                runity::character::show(&mut p.world, 0.0);
                runity::build_frame(&p.world, camera, lighting.clone(), fog.clone())
            });
            frames.push(pair);
        }
    }
    let problems = session.problems();
    if !problems.is_empty() {
        eprintln!("{}: {problems:?}", shot.name);
    }
    let (w, h) = (half.width * 2, half.height);
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
        .arg(format!("{w}x{h}"))
        .args(["-r", "30", "-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium"])
        .arg(file)
        .stdin(Stdio::piped())
        .spawn()?;
    let mut pipe = ffmpeg.stdin.take().ok_or("no pipe to ffmpeg")?;
    let row = half.width as usize * 4;
    for pair in &frames {
        let halves: Vec<Vec<u8>> = pair
            .iter()
            .enumerate()
            .map(|(i, frame)| {
                renderers[i].render(gpu, half, frame);
                half.read_rgba(gpu)
            })
            .collect();
        let mut picture = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h as usize {
            for (i, image) in halves.iter().enumerate() {
                picture[y * row * 2 + i * row..y * row * 2 + (i + 1) * row].copy_from_slice(&image[y * row..(y + 1) * row]);
            }
            // A thin line between the two.
            for x in [row - 4, row] {
                picture[y * row * 2 + x..y * row * 2 + x + 3].copy_from_slice(&[235, 235, 235]);
            }
        }
        pipe.write_all(&picture)?;
    }
    drop(pipe);
    if !ffmpeg.wait()?.success() {
        return Err(format!("ffmpeg failed on {}", shot.name).into());
    }
    Ok(())
}
