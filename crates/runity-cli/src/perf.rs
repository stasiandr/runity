//! `runity perf`: every scene of a project drawn offscreen and measured —
//! the GPU's time for a frame, the draws, the triangles — against the
//! project's budgets (`budgets.ron`), so what gets slower is caught on the
//! commit that made it so (DNA, postulate 6).
//!
//! ```ron
//! (
//!     size: (640, 360),
//!     scenes: {
//!         "bazaar": (gpu_ms: Some(9.0), draws: Some(160), triangles: Some(400000)),
//!     },
//! )
//! ```
//!
//! Beside the GPU, the CPU's part of a frame: a fixed step of the scene's
//! simulation (soft bodies, water, physics), the frame built from the
//! world, and the renderer's own work on it (preparing, recording,
//! submitting). What a render thread takes off the main thread is the
//! last of them; what it runs beside is the first. These are listed, not
//! held: a CPU's time depends on the machine more than a budget can say.
//!
//! A scene with no budget is measured and listed, not held to anything. A
//! GPU time is held only on a real GPU: a software renderer (a CI runner's)
//! draws the same frame at another speed altogether, but the same draws and
//! triangles — those are held everywhere. `--write` sets every scene's
//! budget from what it measures now, with room: half as much time again,
//! a fifth more draws and triangles.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use runity::Project;
use serde::{Deserialize, Serialize};

/// Where a project keeps its budgets.
pub const BUDGETS: &str = "budgets.ron";

/// What a scene may cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    /// Milliseconds a frame on the GPU, all its passes.
    pub gpu_ms: Option<f32>,
    /// Draws that reach the colour pass.
    pub draws: Option<u32>,
    /// Triangles the colour pass draws.
    pub triangles: Option<u64>,
}

/// A project's budgets: the size its scenes are measured at, and each
/// scene's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budgets {
    pub size: (u32, u32),
    pub scenes: BTreeMap<String, Budget>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            size: (640, 360),
            scenes: BTreeMap::new(),
        }
    }
}

/// What a scene cost.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Measured {
    /// `None` on a device without timestamps.
    pub gpu_ms: Option<f32>,
    pub draws: u32,
    pub triangles: u64,
    /// The CPU's milliseconds, medians over the frames: a fixed step, the
    /// frame built, the renderer's work.
    pub step_ms: f32,
    pub build_ms: f32,
    pub render_cpu_ms: f32,
    /// A whole frame of a game, step, build and draw, one after the other
    /// and with the drawing on a render thread beside the step.
    pub frame_ms: f32,
    pub pipelined_ms: f32,
}

/// A scene over one of its budgets, in words.
pub fn over(budget: &Budget, measured: &Measured, real_gpu: bool) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(most), Some(ms), true) = (budget.gpu_ms, measured.gpu_ms, real_gpu) {
        if ms > most {
            out.push(format!("{ms:.2} ms on the GPU, budget {most:.2}"));
        }
    }
    if let Some(most) = budget.draws {
        if measured.draws > most {
            out.push(format!("{} draws, budget {most}", measured.draws));
        }
    }
    if let Some(most) = budget.triangles {
        if measured.triangles > most {
            out.push(format!("{} triangles, budget {most}", measured.triangles));
        }
    }
    out
}

/// A budget from what was measured, with room.
pub fn with_room(measured: &Measured) -> Budget {
    Budget {
        gpu_ms: measured.gpu_ms.map(|ms| (ms * 1.5 * 10.0).ceil() / 10.0),
        draws: Some((measured.draws as f32 * 1.2).ceil() as u32),
        triangles: Some((measured.triangles as f64 * 1.2).ceil() as u64),
    }
}

/// Read a project's budgets; none written is none held.
pub fn read(project: &Project) -> Result<Budgets> {
    let path = project.root().join(BUDGETS);
    if !path.is_file() {
        return Ok(Budgets::default());
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    ron::from_str(&text).with_context(|| format!("{} does not read", path.display()))
}

/// Draw one scene and measure it: settled first, then `frames` frames
/// timed on the GPU.
pub fn measure(scene: &Path, size: (u32, u32), frames: u32) -> Result<(Measured, bool)> {
    let mut shot = runity::shot::Shot::open(scene, size.0, size.1, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    let real_gpu = !shot.software();
    let warm = shot.warm_frames();
    shot.draw(warm);
    shot.renderer.profile_gpu(true);
    for _ in 0..frames.max(1) {
        shot.draw(1);
        // Each waited for, so the timer's copies come back.
        shot.pixels();
    }
    let stats = shot.renderer.stats();
    let times = shot.renderer.gpu_times();
    // The CPU's part, apart: a pass is timed from the last one's end, so
    // the CPU's work between frames would land in the GPU's times.
    shot.renderer.profile_gpu(false);
    // The simulation's first step makes its bodies: not a step's cost.
    shot.step(1.0 / 60.0);
    let (mut step, mut build, mut render) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..frames.max(1) {
        let start = std::time::Instant::now();
        shot.step(1.0 / 60.0);
        let stepped = std::time::Instant::now();
        shot.build();
        let built = std::time::Instant::now();
        shot.draw(1);
        let drawn = std::time::Instant::now();
        step.push((stepped - start).as_secs_f32() * 1e3);
        build.push((built - stepped).as_secs_f32() * 1e3);
        render.push((drawn - built).as_secs_f32() * 1e3);
        shot.pixels();
    }
    // The same frame with a render thread, against the three in a row.
    let mut pipelined = Vec::new();
    for _ in 0..frames.max(1) {
        let start = std::time::Instant::now();
        shot.frame_pipelined(1.0 / 60.0);
        pipelined.push(start.elapsed().as_secs_f32() * 1e3);
        shot.pixels();
    }
    let sequential: Vec<f32> = step.iter().zip(&build).zip(&render).map(|((a, b), c)| a + b + c).collect();
    let median = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let gpu_ms = (!times.is_empty()).then(|| times.iter().map(|(_, t)| t).sum());
    Ok((
        Measured {
            gpu_ms,
            draws: stats.drawn,
            triangles: stats.triangles,
            step_ms: median(step),
            build_ms: median(build),
            render_cpu_ms: median(render),
            frame_ms: median(sequential),
            pipelined_ms: median(pipelined),
        },
        real_gpu,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_is_over_its_budget_by_what_it_says_and_time_only_on_a_real_gpu() {
        let budget = Budget {
            gpu_ms: Some(5.0),
            draws: Some(100),
            triangles: None,
        };
        let measured = Measured {
            gpu_ms: Some(7.0),
            draws: 120,
            triangles: 1_000_000,
            ..Default::default()
        };
        assert_eq!(over(&budget, &measured, true).len(), 2);
        assert_eq!(over(&budget, &measured, false).len(), 1, "time is not held on software");
        let room = with_room(&measured);
        assert!(over(&room, &measured, true).is_empty());
        assert_eq!(room.draws, Some(144));
        let text = ron::to_string(&Budgets::default()).unwrap();
        assert_eq!(ron::from_str::<Budgets>(&text).unwrap(), Budgets::default());
    }
}
