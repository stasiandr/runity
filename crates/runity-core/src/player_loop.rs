//! The game loop by phases, as Unity's PlayerLoop (DNA, "Фазы цикла — как
//! в Unity"): someone from Unity knows where a thing runs without asking.
//!
//! | Phase | Unity | runity |
//! |---|---|---|
//! | [`Phase::Initialization`] | Initialization | once, before the first step |
//! | [`Phase::EarlyUpdate`] | EarlyUpdate | a frame starts: input read, reloads |
//! | [`Phase::FixedUpdate`] | FixedUpdate | the fixed step — a game's `step` |
//! | [`Phase::Update`] | Update | once a frame, on the frame's time |
//! | [`Phase::LateUpdate`] | LateUpdate | after Update: cameras follow |
//! | [`Phase::PostLateUpdate`] | PostLateUpdate | the frame is built and drawn — a game's `frame` |
//!
//! A module puts its systems into phases (its `systems(&mut PlayerLoop)`),
//! the way a package adds systems to Unity's PlayerLoop; the engine gathers
//! the build's modules into one loop, and the game runs a phase where its
//! own systems want it — its own before or after, one line each, in `step`
//! and `frame`. Within a phase systems run in the order they were put in.

use hecs::World;

use crate::perf::Profiler;

/// Where in a frame a system runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    Initialization,
    EarlyUpdate,
    FixedUpdate,
    Update,
    LateUpdate,
    PostLateUpdate,
}

impl Phase {
    /// Every phase, in the order a frame runs them.
    pub const ALL: [Phase; 6] = [
        Phase::Initialization,
        Phase::EarlyUpdate,
        Phase::FixedUpdate,
        Phase::Update,
        Phase::LateUpdate,
        Phase::PostLateUpdate,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// A system: the world, and the seconds this run of its phase covers —
/// the fixed step in [`Phase::FixedUpdate`], the frame's time elsewhere.
pub type System = Box<dyn FnMut(&mut World, f32)>;

/// The loop: each phase's systems, by name, in order.
#[derive(Default)]
pub struct PlayerLoop {
    phases: [Vec<(String, System)>; 6],
}

impl std::fmt::Debug for PlayerLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe().join("\n"))
    }
}

impl PlayerLoop {
    pub fn new() -> Self {
        Self::default()
    }

    /// Put `system` last in `phase`, known by `name` — what the profiler
    /// and [`PlayerLoop::describe`] call it.
    pub fn add(
        &mut self,
        phase: Phase,
        name: impl Into<String>,
        system: impl FnMut(&mut World, f32) + 'static,
    ) -> &mut Self {
        self.phases[phase.index()].push((name.into(), Box::new(system)));
        self
    }

    /// The names of `phase`'s systems, in the order they run.
    pub fn names(&self, phase: Phase) -> Vec<&str> {
        self.phases[phase.index()].iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Run `phase`'s systems in order over `seconds`, each timed by
    /// `profile` under its name when there is one.
    pub fn run(&mut self, phase: Phase, world: &mut World, seconds: f32, mut profile: Option<&mut Profiler>) {
        for (name, system) in &mut self.phases[phase.index()] {
            match profile.as_deref_mut() {
                Some(profile) => profile.time(name, || system(world, seconds)),
                None => system(world, seconds),
            }
        }
    }

    /// The loop in words, a line a phase: what an agent reads to know
    /// where a thing runs.
    pub fn describe(&self) -> Vec<String> {
        Phase::ALL
            .iter()
            .map(|&phase| format!("{phase:?}: {}", self.names(phase).join(", ")))
            .collect()
    }
}

/// The core's own systems: the hierarchy placed at the end of the fixed
/// step, after whatever moved things in it.
pub fn systems(player_loop: &mut PlayerLoop) {
    player_loop.add(Phase::FixedUpdate, "hierarchy", |world, _| {
        crate::world::apply_hierarchy(world)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_phase_runs_its_systems_in_the_order_they_were_put_in() {
        let said = Arc::new(Mutex::new(Vec::new()));
        let mut player_loop = PlayerLoop::new();
        for (phase, name) in [
            (Phase::FixedUpdate, "routes"),
            (Phase::LateUpdate, "cameras"),
            (Phase::FixedUpdate, "motion"),
        ] {
            let said = said.clone();
            player_loop.add(phase, name, move |_, seconds| {
                said.lock().unwrap().push(format!("{name} {seconds}"))
            });
        }
        let mut world = World::new();
        let mut profile = Profiler::new(4);
        player_loop.run(Phase::FixedUpdate, &mut world, 0.5, Some(&mut profile));
        assert_eq!(*said.lock().unwrap(), ["routes 0.5", "motion 0.5"]);
        assert_eq!(player_loop.names(Phase::LateUpdate), ["cameras"]);
        assert!(profile.lines().iter().any(|l| l.contains("motion")));
        assert_eq!(player_loop.describe()[2], "FixedUpdate: routes, motion");
    }
}
