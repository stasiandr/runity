//! Magistral, a railway deckbuilder on scrap.
//!
//! The rules live in `rules/` (no engine there: the depth report plays
//! them by the thousand); this is the window around them. The whole game
//! is one HTML screen, `content/magistral/ui/screens/game.screen.html`,
//! filled from `front::Front::view` and read back as presses
//! (`table::Table`). Save the screen or its CSS while the game runs and it
//! is rebuilt in place; the numbers are `content/magistral/core/rules.ron`,
//! read when a run starts.

mod front;
mod table;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use magistral_rules::Config;
use scrap::glam::Vec3;
use scrap::render::Frame;
use scrap::shell::{self, Context, WindowConfig};

use front::Front;
use table::Table;

/// The numbers as saved, or as shipped when the file does not read.
pub fn config(root: &Path) -> Config {
    if cfg!(target_arch = "wasm32") {
        return Config::default();
    }
    let path = root.join("content/magistral/core/rules.ron");
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| Config::parse(&t).map_err(|e| e.to_string()))
    {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}: {e}; playing the shipped numbers", path.display());
            Config::default()
        }
    }
}

struct Game {
    front: Front,
    table: Table,
}

impl shell::Game for Game {
    fn frame(&mut self, ctx: &mut Context) -> Frame {
        self.table.frame(&mut self.front, ctx.time.delta());
        Frame {
            clear_color: Vec3::new(0.95, 0.92, 0.85),
            ..Default::default()
        }
    }

    fn ui(&mut self) -> Option<&mut scrap::scrap_ui::Ui> {
        Some(&mut self.table.ui)
    }
}

/// A seed nobody chose: the clock, or in the browser its randomness.
fn fresh_seed() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let random = || (js_sys::Math::random() * u32::MAX as f64) as u64;
        return random() << 32 | random();
    }
    #[allow(unreachable_code)]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

fn main() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let seed = std::env::var("MAGISTRAL_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(fresh_seed);
    let front = Front::new(Arc::new(config(root)), seed);
    let mut table = Table::load(root).map_err(anyhow::Error::msg)?;
    table.show(&front);
    let config = WindowConfig {
        title: "Магистраль".into(),
        width: 1280,
        height: 720,
        ..Default::default()
    };
    shell::run(config, Game { front, table })
}
