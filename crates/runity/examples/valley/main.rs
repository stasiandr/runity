//! Valley: a from-scratch scene, built one card at a time. For now this is
//! just a validator for the scene file format — see `scene.rs`. The actual
//! game assembly (rendering, movement, the day-night cycle) lands in later
//! cards on top of this.
//!
//! ```text
//! cargo run --example valley-scene -- assets/valley/scene/first-ten-minutes.txt
//! ```

mod scene;

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: valley <scene.txt>");
        return ExitCode::FAILURE;
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match scene::parse(&text) {
        Ok(parsed) => {
            println!(
                "{path}: {} model(s), {} box(es), {} place(s), {} scatter(s), {} settler(s)",
                parsed
                    .lines
                    .iter()
                    .filter(|l| matches!(l, scene::SceneLine::Model(_)))
                    .count(),
                parsed
                    .lines
                    .iter()
                    .filter(|l| matches!(l, scene::SceneLine::Box(_)))
                    .count(),
                parsed
                    .lines
                    .iter()
                    .filter(|l| matches!(l, scene::SceneLine::Place(_)))
                    .count(),
                parsed
                    .lines
                    .iter()
                    .filter(|l| matches!(l, scene::SceneLine::Scatter(_)))
                    .count(),
                parsed
                    .lines
                    .iter()
                    .filter(|l| matches!(l, scene::SceneLine::Settler(_)))
                    .count(),
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            ExitCode::FAILURE
        }
    }
}
