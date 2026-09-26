//! graphs.
//!
//! `cargo run` opens a window on the `main` scene. Save the scene, a
//! prefab, or re-import an asset while it runs, and the change is in the
//! next frames without the game losing its state. Run under
//! `dx serve --hotpatch` and a rebuilt system, `step` or `frame` takes
//! effect without closing the window; a component given a new field is
//! carried across too (`patched` below), keeping what a save keeps.
//!
//! Components are files in `src/components/`, systems files in
//! `src/systems/` (`scrap add component NAME`, `scrap add system NAME`);
//! `build.rs` finds them, and `step` below runs the systems in order.
//!
//! The game is played together by default: `party` is who else is in it.
//! Alone it is a party of one. Started with several players — from the
//! editor, or `scrap run --players 4` — each window is one of them, and
//! what each owns moves in the others' windows too.

use scrap::hecs::World;
use scrap::party::{Event, Party};
use scrap::player_loop::{Phase, PlayerLoop};
use scrap::prelude::*;
use scrap::physics::PhysicsWorld;
use scrap::render::Frame;
use scrap::shell::{self, run, Context, StepContext, WindowConfig};
use scrap::screen::Screen;
use scrap::ui::{TextRun, Ui};
use scrap::widgets::Widgets;
use scrap::{Actions, Components, LiveScene, Tables, Tuned};
use serde::Deserialize;

/// Every file in src/components/, registered by its file name.
mod components {
    include!(concat!(env!("OUT_DIR"), "/components.rs"));
}

/// Every file in src/systems/.
mod systems {
    include!(concat!(env!("OUT_DIR"), "/systems.rs"));
}

/// Numbers from `core/world.ron`, reloaded while the game runs.
#[derive(Deserialize)]
struct WorldNumbers {
    gravity: f32,
}

struct Game {
    live: LiveScene,
    actions: Actions,
    tuning: Tuned<WorldNumbers>,
    layers: Tuned<scrap::layers::Layers>,
    hud: Screen,
    strings: scrap::strings::Strings,
    profile: scrap::perf::Profiler,
    show_profile: bool,
    /// ` : the game's console — `help`, `set world.gravity -3`, the cheats
    /// in [`cheats`]; the editor's Console types into it too.
    console: scrap::console::Console,
    /// ' : what the thing looked at is doing, written over it.
    debug: scrap::debug_overlay::DebugOverlay,
    widgets: Widgets,
    /// Materials' own shaders, put in and reloaded as they are saved.
    shaders: scrap::render::MaterialShaders,
    ui: Ui,
    world: World,
    physics: PhysicsWorld,
    /// The modules' systems, by phase (Unity's PlayerLoop).
    modules: PlayerLoop,
    party: Party,
    components: Components,
    /// The mixer; `None` on a machine with no sound device.
    audio: Option<scrap::audio::Audio>,
    /// The scene's `sound`s, played.
    sounds: scrap::audio::Sources,
}

impl Game {
    /// Bodies from the world as it is: at the start, and after a patch.
    fn start_physics(&mut self, ctx: &mut Context) {
        self.physics = PhysicsWorld::new(ctx.time.settings().fixed_delta);
        self.physics.set_layers((*self.layers).clone(), &self.world);
        // The scene's wind carries what it says is `blown`.
        self.physics.wind = self.live.scene().wind().unwrap_or_default();
    }
}

/// One fixed step of the game (Unity's FixedUpdate): the game's systems in
/// order, then the modules' — platforms on their routes, clips and
/// characters' animation, everything placed — then physics. The window
/// runs it, and so does the play-mode test below — the same step with
/// nothing drawn.
fn tick(world: &mut World, physics: &mut PhysicsWorld, modules: &mut PlayerLoop, profile: &mut scrap::perf::Profiler, seconds: f32) {
    // systems, in order
    profile.time("turn", || systems::turn::run(world, seconds));
    // The modules' systems of the fixed step (`scrap::player_loop`).
    modules.run(Phase::FixedUpdate, world, seconds, Some(profile));
    // Physics is a system too: bodies from the scene, a fixed step, and
    // where the dynamic ones went written back.
    profile.time("physics", || physics.run(world));
}

/// The game's console commands besides the engine's `help`, `get` and
/// `set` — its cheats. Each is a function of the world and the words typed
/// after its name (Unreal's CheatManager); add yours here. The console is
/// on in a debug build, and in a release one run with SCRAP_CONSOLE=1.
fn cheats() -> scrap::console::Commands {
    let tuning = scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "tuning");
    let mut commands = scrap::console::Commands::new().with_tuning(tuning);
    commands.add("spin", "spin DEGREES: everything that spins turns this fast", |world, words| {
        let speed: f32 = words.first().and_then(|w| w.parse().ok()).ok_or("spin DEGREES, as: spin 90")?;
        let mut turned = 0;
        for spin in world.query_mut::<&mut components::Spin>() {
            spin.degrees_per_second = speed;
            turned += 1;
        }
        Ok(format!("{turned} spinning at {speed} degrees a second"))
    });
    commands
}

/// In the project, write what the components and the tables look like,
/// for the editor's Inspector and Configs window and `scrap check`
/// (library/components.ron, library/tables.ron). Nothing in a build.
fn write_shapes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if root.join("scrap.ron").is_file() {
        if let Err(e) = game_components().write_shapes(root.join(scrap::project::SHAPES)) {
            eprintln!("{e}");
        }
        if let Err(e) = game_tables().write_shapes(root.join(scrap::project::TABLE_SHAPES)) {
            eprintln!("{e}");
        }
    }
}

/// Every table the game reads, and what its records are: a
/// `Wolf` that is a `scrap::Record`, read with
/// `scrap::Table::<Wolf>::load(...)`, is registered here as
/// `tables.register::<Wolf>("content/graphs/wolves/wolves.ron")` — and then the editor
/// knows its fields and `scrap check` its links.
fn game_tables() -> Tables {
    Tables::new()
}

/// Every component the game has, by name.
fn game_components() -> Components {
    let mut components = Components::new();
    components::register(&mut components);
    // What core/world.ron is read as: the editor's Table shows its columns and
    // `scrap check` its misspelt fields.
    components.register_tuning::<WorldNumbers>("world");
    components
}

impl shell::Game for Game {
    fn start(&mut self, ctx: &mut Context) {
        for line in self.live.spawn(&mut self.world, ctx.gpu, ctx.renderer).lines() {
            eprintln!("{line}");
        }
        // Play from Here in the editor: what the scene marks
        // `player_start: true` stands where the editor was looking.
        for line in self.live.start_here(&mut self.world) {
            eprintln!("{line}");
        }
        self.start_physics(ctx);
    }

    /// A hot patch landed: the component types may have new fields, so the
    /// world starts again from the scene under the new code, keeping every
    /// transform and every component registered as saved.
    fn patched(&mut self, ctx: &mut Context) {
        let restored = self.live.reinstance(&mut self.world, game_components(), ctx.gpu, ctx.renderer);
        for problem in &restored.problems {
            eprintln!("{problem}");
        }
        self.start_physics(ctx);
    }

    /// Fixed-step game logic: [`tick`].
    fn step(&mut self, ctx: &mut StepContext) {
        let seconds = ctx.time.settings().fixed_delta;
        self.physics.gravity.y = self.tuning.gravity;
        tick(&mut self.world, &mut self.physics, &mut self.modules, &mut self.profile, seconds);
        // Slow motion or a hit-stop the step's systems asked for
        // (`scrap::time::hit_stop`), to the clock.
        ctx.ask_time(scrap::time::sync(&mut self.world, ctx.time));
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if let Some(Err(problem)) = self.actions.reload_if_changed() {
            eprintln!("{problem}");
        }
        if let Some(Err(problem)) = self.tuning.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        match self.layers.poll(ctx.time.unscaled_delta()) {
            Some(Ok(())) => self.physics.set_layers((*self.layers).clone(), &self.world),
            Some(Err(problem)) => eprintln!("{problem}"),
            None => {}
        }
        if let Some(Err(problem)) = self.hud.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        self.ui.clear();
        let size = scrap::glam::Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
        if let Some(Err(problem)) = self.strings.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        // The pad's moves between the screen's widgets, before they draw.
        self.widgets.begin_frame(ctx.input);
        let done = self.hud.draw_localized(&mut self.widgets, &mut self.ui, ctx.input, size, &self.strings);
        // The console: typed into here, or sent from the editor's Console.
        // While it has the keyboard, the game's keys are not the game's.
        let toggled = self.actions.pressed(ctx.input, "console");
        self.console.frame(&mut self.world, ctx.input, toggled, ctx.commands, &mut self.ui, size);
        let keys = !self.console.has_keyboard();
        if done.clicked("quit") || (keys && self.actions.pressed(ctx.input, "quit")) {
            ctx.quit();
        }
        let reload = self.live.poll(ctx.time.unscaled_delta(), &mut self.world, ctx.gpu, ctx.renderer);
        for line in reload.lines() {
            eprintln!("{line}");
        }
        for (shader, result) in self.shaders.poll(ctx.renderer, ctx.gpu) {
            match result {
                Ok(()) => eprintln!("shader {shader}: in"),
                Err(problem) => eprintln!("{problem}"),
            }
        }
        // Played together: what the others own comes in, what this player
        // owns goes out, and whatever they spawn is spawned here too.
        let (live, gpu, renderer) = (&mut self.live, ctx.gpu, &mut *ctx.renderer);
        let events = self.party.update(&mut self.world, &self.components, ctx.time.unscaled_delta(), |world, prefab, at| {
            live.spawn_prefab(prefab, at, None, world, gpu, renderer).ok().map(|i| i.root)
        });
        for event in events {
            match event {
                // The session plays another scene: go there, then say so.
                Event::SceneRequired { scene } => {
                    match self.live.switch(&scene, &mut self.world, ctx.gpu, ctx.renderer) {
                        Ok((spawned, problems)) => {
                            for line in spawned.lines().into_iter().chain(problems) {
                                eprintln!("{line}");
                            }
                            self.start_physics(ctx);
                        }
                        Err(e) => eprintln!("{e}"),
                    }
                    self.party.ready_in(&scene);
                }
                Event::Welcomed => eprintln!("in the game as {}", self.party.name()),
                Event::Joined { name, .. } => eprintln!("{} joined", name),
                Event::Left { name, clean, .. } => {
                    eprintln!("{} {}", name, if clean { "left" } else { "went quiet, and is gone" })
                }
                Event::HostLost { quit } => eprintln!(
                    "the host {}; waiting for them to come back",
                    if quit { "left" } else { "is not answering" }
                ),
                Event::HostBack => eprintln!("the host is back"),
                Event::Rejected(why) => eprintln!("could not join: {why}"),
                Event::ClaimLost { .. } | Event::Message { .. } => {}
                Event::Silent { id, owner } => eprintln!("{id}: player {} stopped saying where it is", owner.0 + 1),
                Event::Problem(why) => eprintln!("{why}"),
            }
        }
        // Started from the editor: tell it where things are, who this is
        // and what the systems cost.
        self.live.note(self.party.me().0, &self.profile);
        if let Err(problem) = self.live.report(&self.world, ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        // F3: what each part costs, over the game.
        if keys && self.actions.pressed(ctx.input, "profile") {
            self.show_profile = !self.show_profile;
        }
        if keys && self.actions.pressed(ctx.input, "debug") {
            self.debug.toggle();
        }
        if self.show_profile {
            // The game's parts, then the loop's own: steps, frame, drawing and
            // the wait for the screen.
            let lines = self.profile.lines().into_iter().chain(ctx.loop_times.lines().into_iter().map(|l| format!("loop {l}")));
            for (i, line) in lines.enumerate() {
                let at = 60.0 + 20.0 * i as f32;
                self.ui.text(TextRun::new(20.0, at, 16.0, scrap::glam::Vec4::ONE, line));
            }
        }
        // The clock into the world — the real delta a camera's blend and
        // shake run on — and what the systems asked of it, to the clock.
        ctx.ask_time(scrap::time::sync(&mut self.world, ctx.time));
        // The modules' late systems: cameras that follow keep after their
        // targets and blend and shake; sparks and dust move on the frame's
        // time.
        let delta = ctx.time.delta();
        for phase in [Phase::Update, Phase::LateUpdate, Phase::PostLateUpdate] {
            self.modules.run(phase, &mut self.world, delta, Some(&mut self.profile));
        }
        // A camera on an entity — a child of the player follows the player —
        // or the scene's view when there is none.
        // What the fixed steps move is drawn between the last two of
        // them, by how far this frame is into the next.
        scrap::world::interpolate(&mut self.world, ctx.time.interpolation());
        let camera = scrap::world::camera_of(&self.world)
            .unwrap_or_else(|| scrap::scene_camera(&self.live.scene().view()));
        // The world's streamed regions, in and out by where it looks from.
        for event in self.live.stream(&mut self.world, camera.position, ctx.gpu, ctx.renderer) {
            if let scrap::streaming::StreamEvent::Failed(scene, why) = event {
                eprintln!("stream {scene}: {why}");
            }
        }
        let scene = self.live.scene();
        // Everything the scene says about how it looks: sun, fog, sky and
        // post-processing.
        let mut frame = self.profile.time("frame", || scrap::world::scene_frame(&self.world, camera, scene));
        // ' : the state of the thing in the middle of the view, over it.
        self.debug.draw(&self.world, &self.components, &camera, size, &mut self.ui, &mut frame, ctx.gpu, ctx.renderer);
        // The scene's sounds, heard from where the camera is.
        if let (Some(audio), Some(library)) = (self.audio.as_mut(), self.live.library()) {
            audio.set_listener(camera.position);
            for problem in self.sounds.update(audio, &self.world, |link| library.sound_of(link)) {
                eprintln!("{problem}");
            }
        }
        frame
    }

    fn overlay(&mut self) -> &Ui {
        &self.ui
    }
}

fn main() -> anyhow::Result<()> {
    // `data/` beside the executable in a build, the project in development.
    // scrap.ron's `game`: window, clock, first scene, language.
    let (project_name, settings) =
        scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).map_err(anyhow::Error::msg)?;
    // A panic is written down in the player's folder: scrap::crash::pending
    // finds it on the next start.
    scrap::crash::install(&project_name, env!("CARGO_PKG_VERSION"));
    // `scrap run --scene cave` plays the scene called `cave`, wherever it lies.
    let playing = std::env::var("SCRAP_SCENE").unwrap_or_else(|_| settings.start_scene.clone());
    // Started from the editor, the game watches the editor's document as it
    // stands (SCRAP_SCENE_FILE), so an edit shows here without a save.
    let scene = match std::env::var_os("SCRAP_SCENE_FILE") {
        Some(file) => std::path::PathBuf::from(file),
        None => scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &playing),
    };
    let (live, problems) = LiveScene::open(&scene)?;
    let live = live.with_components(game_components());
    write_shapes();
    for problem in &problems {
        eprintln!("{problem}");
    }
    // SCRAP_NET: host or join a game — alone without it, which is the
    // same thing with nobody else in it.
    let party = Party::from_env(&playing, &game_components()).map_err(anyhow::Error::msg)?;
    let mut title = if settings.title.is_empty() { project_name } else { settings.title.clone() };
    if !party.is_alone() {
        title = format!("{title} — {}", party.name());
    }
    let config = WindowConfig {
        title,
        width: settings.width,
        height: settings.height,
        time: scrap::TimeSettings {
            fixed_delta: settings.fixed_delta(),
            ..Default::default()
        },
        ..Default::default()
    };
    let actions = Actions::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "config/input.ron"))?;
    for problem in actions.missing(&["quit"]) {
        eprintln!("{problem}");
    }
    let tuning = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "content/graphs/core/world.ron"))
        .map_err(anyhow::Error::msg)?;
    let layers = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "config/layers.ron"))
        .map_err(anyhow::Error::msg)?;
    let hud = Screen::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "content/graphs/ui/screens/hud.screen.ron"))
        .map_err(anyhow::Error::msg)?;
    let strings = scrap::strings::Strings::load(
        scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), scrap::strings::DIR),
        &settings.language,
    )
    .map_err(anyhow::Error::msg)?;
    let game = Game {
        live,
        actions,
        tuning,
        layers,
        hud,
        strings,
        profile: scrap::perf::Profiler::new(600),
        show_profile: false,
        console: scrap::console::Console::for_build(cheats(), cfg!(debug_assertions)),
        debug: scrap::debug_overlay::DebugOverlay::new(),
        widgets: Widgets::new(),
        shaders: scrap::render::MaterialShaders::new(scrap::project::data_root(env!("CARGO_MANIFEST_DIR"))),
        ui: Ui::new(),
        world: World::new(),
        physics: PhysicsWorld::default(),
        modules: scrap::player_loop::modules(),
        party,
        components: game_components(),
        audio: scrap::audio::Audio::new().map_err(|e| eprintln!("no sound: {e}")).ok(),
        sounds: scrap::audio::Sources::new(),
    };

    run(config, game)
}

/// Play mode without a window: `scrap test` (or `cargo test`).
#[cfg(test)]
mod tests {
    use super::*;

    /// The start scene opens with nothing unresolved and plays two seconds
    /// of the game's own steps — systems and physics — with everything
    /// still somewhere real at the end.
    #[test]
    fn the_start_scene_plays() {
        write_shapes();
        let (_, settings) = scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).unwrap();
        let scene = scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &settings.start_scene);
        let (live, problems) = LiveScene::open(&scene).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let mut live = live.with_components(game_components());
        let mut world = World::new();
        let spawned = live.spawn_headless(&mut world);
        assert!(spawned.lines().is_empty(), "{:?}", spawned.lines());

        let seconds = settings.fixed_delta();
        let mut physics = PhysicsWorld::new(seconds);
        let mut modules = scrap::player_loop::modules();
        let mut profile = scrap::perf::Profiler::new(8);
        for _ in 0..(2.0 / seconds) as usize {
            tick(&mut world, &mut physics, &mut modules, &mut profile, seconds);
        }
        for (_, transform) in world.query::<(scrap::hecs::Entity, &scrap::Transform)>().iter() {
            assert!(transform.position.is_finite(), "something flew off: {transform:?}");
        }
    }

    /// Every table loads into its type, keeps its records' rules, and links
    /// only to records that are there.
    #[test]
    fn the_tables_hold() {
        let problems = game_tables().problems(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
