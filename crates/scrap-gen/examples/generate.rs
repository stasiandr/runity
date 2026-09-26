//! A draft model from words, into a project, and a picture of it.
//!
//! ```text
//! FAL_KEY=… cargo run -p scrap-gen --example generate -- \
//!     examples/valley/content/valley/maps/valley.scene.ron well "an old stone well with a wooden roof"
//! ```
//!
//! Puts a greybox cube in the open scene, generates the model into it, and
//! writes before and after to `target/gen_before.png` and
//! `target/gen_after.png`. The scene is not saved; the draft stays in the
//! project's `assets/drafts/`.

use std::time::Duration;

use scrap::glam::Vec3;
use scrap_editor::Session;
use scrap_gen::{Generator, Request};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(scene), Some(name), Some(prompt)) = (args.next(), args.next(), args.next()) else {
        anyhow::bail!("usage: generate <scene.ron> <name> <prompt>");
    };
    let mut generator = Generator::from_env()?;
    let mut session = Session::offscreen(960, 540).map_err(|e| anyhow::anyhow!("{e}"))?;
    session
        .open_scene(&scene)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let cube = session
        .add(None, "builtin:cube")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut place = session.transform(cube).expect("just added");
    place.position = Vec3::new(0.0, 1.0, 0.0);
    place.scale = Vec3::new(2.0, 2.0, 2.0);
    session
        .set_transform(cube, place)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    session.set_camera(Vec3::new(4.0, 3.5, 5.0), Vec3::new(0.0, 1.0, 0.0));
    shot(&mut session, "target/gen_before.png")?;

    let job = generator
        .start(&mut session, Request::text(&name, &prompt).fit(cube))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut last = String::new();
    let finished = loop {
        if let Some(f) = generator.wait(&mut session, job, Duration::from_secs(2)) {
            break f;
        }
        if let Some(stage) = generator.running().first().map(|j| j.stage.clone()) {
            if stage != last {
                println!("{stage}");
                last = stage;
            }
        }
    };
    let applied = finished.result.map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "{} in {:.0} s, {:.2} × {:.2} × {:.2} m",
        applied.file.display(),
        finished.seconds,
        applied.size.x,
        applied.size.y,
        applied.size.z
    );
    shot(&mut session, "target/gen_after.png")?;
    Ok(())
}

fn shot(session: &mut Session, path: &str) -> anyhow::Result<()> {
    session.render();
    let (w, h) = session.size();
    image_save(path, session.frame_pixels(), w, h)?;
    println!("wrote {path}");
    Ok(())
}

fn image_save(path: &str, rgba: &[u8], w: u32, h: u32) -> anyhow::Result<()> {
    image::save_buffer(path, rgba, w, h, image::ColorType::Rgba8)?;
    Ok(())
}
