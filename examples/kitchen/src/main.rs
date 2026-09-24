//! Kitchen Rush: a little Overcooked, played together over Steam. Up to
//! four cooks, one kitchen, orders for soup coming in faster than is
//! comfortable. Chop tomatoes or onions on a
//! board, three of one kind into a pot, the soup onto a plate, the plate
//! out of the window before the order walks out. The kitchen is the scene,
//! greyboxed from the engine's shapes; the rules are `src/systems/`.
//!
//! `cargo run` opens a window on `scenes/main.ron`. Save the scene, a
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
//! The game is played together: `party` is who else is in it. The host
//! makes a Steam lobby (`lobby.rs`), friends join it from an invite or the
//! friends list, and the host opens the doors. Alone it is a party of one.
//! Started with several players — from the editor, or
//! `scrap run --players 4` — each window is one of them over UDP, and
//! what each owns moves in the others' windows too.

#[allow(unused_imports)]
use scrap::prelude::*;
use scrap::hecs::World;
use scrap::party::{Event, Party};
use scrap::physics::PhysicsWorld;
use scrap::render::Frame;
use scrap::shell::{self, run, Context, StepContext, WindowConfig};
use scrap::ui::{TextRun, Ui};
use scrap::widgets::Widgets;
use scrap::{Actions, Components, LiveScene, Tuned};
use serde::Deserialize;

/// Every file in src/components/, registered by its file name.
mod components {
    include!(concat!(env!("OUT_DIR"), "/components.rs"));
}

/// Every file in src/systems/.
mod systems {
    include!(concat!(env!("OUT_DIR"), "/systems.rs"));
}

/// What the game keeps while it runs: hands, pots, orders.
mod state;

/// The menu, the HUD and the results; the keys into the cooks.
mod front;

/// A frame's work around the party: acts, seats, spawns.
mod session;

/// Steam: the lobby, invites, and the session over Steam's networking.
mod lobby;


/// The kitchen's sounds, from what changes in it.
mod noise;

/// The kitchen played without a window, alone and together.
#[cfg(test)]
mod play_tests;

/// Numbers from `tuning/world.ron`, reloaded while the game runs.
#[derive(Deserialize)]
struct WorldNumbers {
    gravity: f32,
}

struct Game {
    live: LiveScene,
    actions: Actions,
    tuning: Tuned<WorldNumbers>,
    /// The camera's tour while players gather, from `tuning/flyby.ron`.
    tour: Tuned<scrap::tour::Tour>,
    flyby: scrap::tour::Flyby,
    layers: Tuned<scrap::layers::Layers>,
    /// The menu, the HUD, the results.
    front: front::Front,
    /// The scene the kitchen is, by name: what a session plays.
    scene: String,
    strings: scrap::strings::Strings,
    profile: scrap::perf::Profiler,
    show_profile: bool,
    widgets: Widgets,
    /// Materials' own shaders, put in and reloaded as they are saved.
    shaders: scrap::render::MaterialShaders,
    ui: Ui,
    world: World,
    physics: PhysicsWorld,
    party: Party,
    /// Steam, when it runs: lobbies and invites.
    lobby: lobby::Lobby,
    components: Components,
    /// The mixer; `None` on a machine with no sound device.
    audio: Option<scrap::audio::Audio>,
    /// The scene's `sound`s, played.
    sounds: scrap::audio::Sources,
    /// The graphs and clips that lines' `animator`s play.
    motions: scrap::motion::Motions,
    /// What the kitchen sounded like last frame.
    noise: noise::Noise,
    /// The player's choices kept between runs: volumes, the best score.
    prefs: scrap::player_prefs::PlayerPrefs,
}

impl Game {
    /// What the steps asked to be made: made from its prefab, put where
    /// it goes.
    /// Out of any session, and the kitchen as the scene has it.
    fn leave(&mut self, ctx: &mut Context) {
        self.lobby.leave();
        self.front.level = self.scene.clone();
        self.front.host_lost = None;
        self.party = Party::alone(&self.scene, &self.components);
        match self.live.switch(&self.scene, &mut self.world, ctx.gpu, ctx.renderer) {
            Ok((_, problems)) => problems.iter().for_each(|p| eprintln!("{p}")),
            Err(e) => eprintln!("{e}"),
        }
        self.start_physics(ctx);
        self.front.phase = front::Phase::Menu;
    }

    /// A one-shot sound by name, in the effects group.
    fn play(&mut self, name: &str) {
        if let (Some(audio), Some(library)) = (self.audio.as_mut(), self.live.library()) {
            if let Some(sound) = library.sound_by_name(name) {
                let _ = audio.play_in("sfx", sound, 0.8);
            }
        }
    }

    /// A round just over that beat the best on this machine: kept.
    fn best_score(&mut self) {
        let over = front::round_of(&self.world).filter(|r| r.over);
        match over {
            Some(round) if !self.front.new_best && i64::from(round.score) > self.front.best => {
                self.front.best = i64::from(round.score);
                self.front.new_best = true;
                self.prefs.set("best", scrap::player_prefs::Pref::Int(self.front.best));
                let _ = self.prefs.save();
            }
            None => self.front.new_best = false,
            _ => {}
        }
    }

    /// What a player pressed on a screen.
    fn wish(&mut self, wish: front::Wish, ctx: &mut Context) {
        use front::{Phase, Wish};
        match wish {
            // A Steam lobby friends can join; without Steam, the kitchen
            // alone. Either way it opens shut, and the host opens the doors.
            Wish::Host => {
                let name = self.lobby.name().unwrap_or_else(|| "Cook".into());
                if let Some(party) = self.lobby.host(&self.scene, &name, &self.components) {
                    self.party = party;
                }
                self.front.phase = Phase::Kitchen;
            }
            Wish::Friends => self.lobby.friends(),
            Wish::Invite => self.lobby.invite(),
            Wish::Ready(ready) => front::set_ready(&mut self.world, &self.party, ready),
            // Everyone to the other kitchen: each peer switches as the
            // session says (Event::SceneRequired), this one too.
            Wish::Level(level) => self.party.set_scene(level),
            Wish::Start => self.party.publish(state::ACT, &state::Act::Restart),
            Wish::Again => self.party.publish(state::ACT, &state::Act::Restart),
            Wish::Leave => self.leave(ctx),
            Wish::Quit => ctx.quit(),
            Wish::Volume(group, volume) => {
                if let Some(audio) = self.audio.as_mut() {
                    let _ = audio.set_group_volume(group, volume);
                }
                self.prefs.set(group, scrap::player_prefs::Pref::Float(volume as f64));
                let _ = self.prefs.save();
            }
            Wish::Language(language) => {
                let dir = scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "strings");
                match scrap::strings::Strings::load(dir, language) {
                    Ok(strings) => self.strings = strings,
                    Err(e) => eprintln!("{e}"),
                }
                self.prefs.set("language", scrap::player_prefs::Pref::Text(language.into()));
                let _ = self.prefs.save();
            }
        }
    }

    /// Bodies from the world as it is: at the start, and after a patch.
    fn start_physics(&mut self, ctx: &mut Context) {
        self.physics = PhysicsWorld::new(ctx.time.settings().fixed_delta);
        self.physics.set_layers((*self.layers).clone(), &self.world);
    }
}

/// One fixed step of the game: the systems in order, then physics. The
/// window runs it, and so does the play-mode test below — the same step
/// with nothing drawn.
fn tick(world: &mut World, physics: &mut PhysicsWorld, profile: &mut scrap::perf::Profiler, seconds: f32) {
    // systems, in order
    profile.time("orders", || systems::orders::run(world, seconds));
    profile.time("walk", || systems::walk::run(world, seconds));
    profile.time("hands", || systems::hands::run(world, seconds));
    profile.time("cook", || systems::cook::run(world, seconds));
    profile.time("fly", || systems::fly::run(world, seconds));
    profile.time("show", || systems::show::run(world, seconds));
    profile.time("pose", || systems::pose::run(world, seconds));
    profile.time("board", || systems::board::run(world, seconds));
    // Platforms and lifts on their routes, and lines with an `animator`
    // moving what is under them; then everything placed.
    profile.time("routes", || scrap::routes::run_routes(world, seconds));
    profile.time("motion", || scrap::motion::run(world, seconds));
    // Characters: their graphs pick the clip, the skeleton takes the pose.
    profile.time("animation", || {
        scrap::animgraph::run_controllers(world);
        scrap::advance_animations(world, seconds);
    });
    scrap::world::apply_hierarchy(world);
    // Food just thrown: a body now, and its first speed.
    launch(world, physics);
    // Physics is a system too: bodies from the scene, a fixed step, and
    // where the dynamic ones went written back.
    profile.time("physics", || physics.run(world));
}

/// What was thrown this step becomes a body, and is given its speed.
fn launch(world: &mut World, physics: &mut PhysicsWorld) {
    let thrown: Vec<(scrap::hecs::Entity, scrap::glam::Vec3)> = world
        .query::<(scrap::hecs::Entity, &state::Flying)>()
        .iter()
        .filter_map(|(e, f)| Some((e, f.launch?)))
        .collect();
    if thrown.is_empty() {
        return;
    }
    physics.sync_from_world(world);
    for (item, velocity) in thrown {
        physics.set_velocity(world, item, velocity);
        if let Ok(mut f) = world.get::<&mut state::Flying>(item) {
            f.launch = None;
        }
    }
}

/// In the project, write what the components look like, for the editor's
/// Inspector and `scrap check` (library/components.ron). Nothing in a build.
fn write_shapes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if root.join("scrap.ron").is_file() {
        if let Err(e) = game_components().write_shapes(root.join(scrap::project::SHAPES)) {
            eprintln!("{e}");
        }
    }
}

/// Every component the game has, by name.
fn game_components() -> Components {
    let mut components = Components::new();
    components::register(&mut components);
    components
}

impl shell::Game for Game {
    fn start(&mut self, ctx: &mut Context) {
        if let Err(problem) = ctx.overlay.use_font(front::FONT.to_vec()) {
            eprintln!("the kitchen's font: {problem}");
        }
        for line in self.live.spawn(&mut self.world, ctx.gpu, ctx.renderer).lines() {
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
        // The menu is a picture of the kitchen: nothing runs behind it.
        // Paused alone, the kitchen waits; together, it cannot.
        if self.front.phase != front::Phase::Kitchen || (self.front.paused && self.party.is_alone()) {
            return;
        }
        let seconds = ctx.time.settings().fixed_delta;
        self.physics.gravity.y = self.tuning.gravity;
        tick(&mut self.world, &mut self.physics, &mut self.profile, seconds);
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if let Some(Err(problem)) = self.actions.reload_if_changed() {
            eprintln!("{problem}");
        }
        if let Some(Err(problem)) = self.tuning.poll(ctx.time.delta()) {
            eprintln!("{problem}");
        }
        match self.layers.poll(ctx.time.delta()) {
            Some(Ok(())) => self.physics.set_layers((*self.layers).clone(), &self.world),
            Some(Err(problem)) => eprintln!("{problem}"),
            None => {}
        }
        for problem in self.front.poll(ctx.time.delta()) {
            eprintln!("{problem}");
        }
        self.ui.clear();
        let size = scrap::glam::Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
        if let Some(Err(problem)) = self.strings.poll(ctx.time.delta()) {
            eprintln!("{problem}");
        }
        // The pad's moves between the screen's widgets, before they draw.
        // In a round the pad cooks: South grabs, it does not press the
        // HUD's Leave. On the menus, the lobby, the pause and the results
        // it works the screen.
        let cooking = self.front.phase == front::Phase::Kitchen
            && !self.front.paused
            && self.front.host_lost.is_none()
            && front::round_of(&self.world).is_some_and(|r| r.open && !r.over);
        if cooking {
            self.widgets.begin_frame_without_pad();
        } else {
            self.widgets.begin_frame(ctx.input);
        }
        self.best_score();
        self.front.listen(ctx.time.delta());
        let wish = self.front.draw(
            &self.world,
            &self.party,
            &mut self.widgets,
            &mut self.ui,
            ctx.input,
            size,
            &self.strings,
        );
        if let Some(wish) = wish {
            self.wish(wish, ctx);
        }
        // Escape: the pause card in the kitchen (again to carry on), out of
        // the menu to the desk.
        if self.actions.pressed(ctx.input, "quit") {
            match self.front.phase {
                front::Phase::Menu => ctx.quit(),
                front::Phase::Joining => self.leave(ctx),
                front::Phase::Kitchen => self.front.paused = !self.front.paused,
            }
        }
        // Into a friend's lobby — an invite accepted, a game joined from
        // the friends list: out of whatever this was, into their session.
        let name = self.lobby.name().unwrap_or_else(|| "Cook".into());
        if let Some(party) = self.lobby.poll(&self.scene, &name, &self.components) {
            self.party = party;
            self.front.phase = front::Phase::Joining;
        }
        let reload = self.live.poll(ctx.time.delta(), &mut self.world, ctx.gpu, ctx.renderer);
        for line in reload.lines() {
            eprintln!("{line}");
        }
        if self.front.phase == front::Phase::Kitchen {
            if self.front.paused {
                front::halt(&mut self.world, &self.party);
            } else {
                self.front.drive(&mut self.world, &mut self.party, &self.actions, ctx.input);
            }
            let (live, gpu, renderer) = (&mut self.live, ctx.gpu, &mut *ctx.renderer);
            session::frame(&mut self.world, &mut self.party, |world, prefab| {
                let made = live.spawn_prefab(prefab, scrap::Transform::default(), None, world, gpu, renderer);
                made.map_err(|e| eprintln!("{e}")).ok().map(|i| i.root)
            });
        }
        // Lines that came in with an `animator` start their graphs.
        let library = self.live.library();
        let skins = |model: &scrap::AssetLink| library?.mesh_by_name(model)?.skin_owned();
        for problem in scrap::motion::attach(&mut self.world, &self.motions, skins) {
            eprintln!("{problem}");
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
        let events = self.party.update(&mut self.world, &self.components, ctx.time.delta(), |world, prefab, at| {
            live.spawn_prefab(prefab, at, None, world, gpu, renderer).ok().map(|i| i.root)
        });
        for event in events {
            match event {
                // The session plays another scene: go there, then say so.
                Event::SceneRequired { scene } => {
                    self.front.level = scene.clone();
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
                Event::Welcomed => {
                    eprintln!("in the game as {}", self.party.name());
                    if self.front.phase == front::Phase::Joining {
                        self.front.phase = front::Phase::Kitchen;
                    }
                }
                Event::Joined { name, .. } => eprintln!("{} joined", name),
                Event::Left { name, clean, .. } => {
                    eprintln!("{} {}", name, if clean { "left" } else { "went quiet, and is gone" })
                }
                Event::HostLost { quit } => {
                    eprintln!("the host {}", if quit { "left" } else { "is not answering" });
                    self.front.host_lost = Some(quit);
                }
                Event::HostBack => {
                    eprintln!("the host is back");
                    self.front.host_lost = None;
                }
                Event::Rejected(why) => {
                    self.front.status = format!("Could not join: {why}");
                    self.front.phase = front::Phase::Menu;
                    self.lobby.leave();
                    self.party = Party::alone(&self.scene, &self.components);
                }
                // What someone's hands did: the host acts on it.
                Event::Message { .. } => {
                    if let Some(ping) = event.decode::<state::Ping>(state::PING) {
                        self.front.ping(ping);
                        self.play("pop");
                    }
                    session::act(&mut self.world, &self.party, std::slice::from_ref(&event));
                }
                Event::ClaimLost { .. } => {}
                Event::Silent { id, owner } => eprintln!("{id}: player {} stopped saying where it is", owner.0 + 1),
                Event::Problem(why) => eprintln!("{why}"),
            }
        }
        // Started from the editor: tell it where things are, who this is
        // and what the systems cost.
        self.live.note(self.party.me().0, &self.profile);
        if let Err(problem) = self.live.report(&self.world, ctx.time.delta()) {
            eprintln!("{problem}");
        }
        // F3: what each part costs, over the game.
        if self.actions.pressed(ctx.input, "profile") {
            self.show_profile = !self.show_profile;
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
        // Sparks and dust move on the frame's time: they are for the eye.
        scrap::particles::run_particles(&mut self.world, ctx.time.delta());
        let scene = self.live.scene();
        // Cameras that follow keep after their targets, then the one that
        // looks is found, blended and shaken on real time (docs/feel.md);
        // what the systems asked of the clock goes to it.
        ctx.ask_time(scrap::time::sync(&mut self.world, ctx.time));
        scrap::world::run_cameras(&mut self.world, ctx.time.delta());
        // A camera on an entity — a child of the player follows the player —
        // or the scene's view when there is none.
        // What the fixed steps move is drawn between the last two of
        // them, by how far this frame is into the next.
        scrap::world::interpolate(&mut self.world, ctx.time.interpolation());
        let camera = scrap::world::camera_of(&self.world)
            .unwrap_or_else(|| scrap::scene_camera(&scene.view()));
        // While players gather — joining, or in the kitchen before the doors
        // open — the camera tours it.
        if let Some(Err(problem)) = self.tour.poll(ctx.time.delta()) {
            eprintln!("{problem}");
        }
        let touring = match self.front.phase {
            front::Phase::Menu => false,
            front::Phase::Joining => true,
            front::Phase::Kitchen => !front::round_of(&self.world).is_some_and(|r| r.open),
        };
        let camera = self.flyby.camera(&self.tour, touring, ctx.time.delta(), camera);
        let size = scrap::glam::Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
        self.front.floaters(&camera, size, &mut self.ui, ctx.time.delta());
        let started = std::time::Instant::now();
        // Everything the scene says about how it looks: sun, fog, sky and
        // post-processing.
        let mut frame = scrap::world::scene_frame(&self.world, camera, scene);
        // Close up, the tour's lens: in focus where it looks, the rest soft.
        self.flyby.lens(&self.tour, &camera, &mut frame.post.depth_of_field);
        self.profile.record("frame", started.elapsed());
        // The scene's sounds, heard from where the camera is; the
        // kitchen's, from what changed in it.
        let heard = self.noise.listen(&self.world);
        if heard.contains(&"ding") {
            front::ring(&mut self.world);
        }
        if let (Some(audio), Some(library)) = (self.audio.as_mut(), self.live.library()) {
            for name in heard {
                if let Some(sound) = library.sound_by_name(name) {
                    if let Err(e) = audio.play_in("sfx", sound, 0.8) {
                        eprintln!("{e}");
                    }
                }
            }
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
    // Volumes and the best score, kept in the player's folder.
    let prefs = scrap::player_prefs::PlayerPrefs::of_game(&project_name).unwrap_or_default();
    // `scrap run --scene cave` plays scenes/cave.ron.
    let playing = std::env::var("SCRAP_SCENE").unwrap_or_else(|_| settings.start_scene.clone());
    // Started from the editor, the game watches the editor's document as it
    // stands (SCRAP_SCENE_FILE), so an edit shows here without a save.
    let scene = match std::env::var_os("SCRAP_SCENE_FILE") {
        Some(file) => std::path::PathBuf::from(file),
        None => scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), &format!("scenes/{}.ron", playing)),
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
    let (motions, problems) = scrap::motion::Motions::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), ""));
    for problem in &problems {
        eprintln!("{problem}");
    }
    let actions = Actions::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "input.ron"))?;
    for problem in actions.missing(&["quit"]) {
        eprintln!("{problem}");
    }
    let tuning = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "tuning/world.ron"))
        .map_err(anyhow::Error::msg)?;
    let tour = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "tuning/flyby.ron"))
        .map_err(anyhow::Error::msg)?;
    let layers = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "layers.ron"))
        .map_err(anyhow::Error::msg)?;
    let mut front = front::Front::load(&scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "ui"))
        .map_err(anyhow::Error::msg)?;
    // Steam as Spacewar; a session from the command line is UDP, and needs
    // no Steam.
    let lobby = if party.is_alone() { lobby::Lobby::start() } else { lobby::Lobby::off("a session by address") };
    front.steam = lobby.on();
    match (lobby.name(), &lobby.why) {
        (Some(name), _) => front.menu.set_text("who", format!("Steam: {name}")),
        (None, Some(why)) => {
            eprintln!("no Steam ({why}): the kitchen alone");
            front.status = "No Steam: start Steam to cook with friends".into();
        }
        _ => {}
    }
    // Started into a session (`scrap run --players 2`): straight in.
    if !party.is_alone() {
        front.phase = if party.is_host() { front::Phase::Kitchen } else { front::Phase::Joining };
    }
    let strings = scrap::strings::Strings::load(
        scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "strings"),
        &settings.language,
    )
    .map_err(anyhow::Error::msg)?;
    // What the player chose last time.
    front.best = prefs.int("best", 0);
    let mut audio = scrap::audio::Audio::new().map_err(|e| eprintln!("no sound: {e}")).ok();
    for group in ["music", "sfx"] {
        let volume = prefs.float(group, 0.8) as f32;
        front.menu.set_value(group, volume);
        if let Some(audio) = audio.as_mut() {
            let _ = audio.set_group_volume(group, volume);
        }
    }
    let language = prefs.text("language", &settings.language).to_string();
    if let Some(i) = front::LANGUAGES.iter().position(|l| *l == language) {
        front.menu.set_chosen("language", i);
    }
    let strings = scrap::strings::Strings::load(
        scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "strings"),
        &language,
    )
    .unwrap_or(strings);
    let game = Game {
        live,
        actions,
        tuning,
        tour,
        flyby: scrap::tour::Flyby::default(),
        layers,
        front,
        scene: playing.clone(),
        strings,
        profile: scrap::perf::Profiler::new(600),
        show_profile: false,
        widgets: Widgets::with_style(front::style()),
        shaders: scrap::render::MaterialShaders::new(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "shaders")),
        ui: Ui::new(),
        world: World::new(),
        physics: PhysicsWorld::default(),
        party,
        lobby,
        components: game_components(),
        audio,
        sounds: scrap::audio::Sources::new(),
        motions,
        noise: noise::Noise::default(),
        prefs,
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
        let scene = scrap::project::data_file(
            env!("CARGO_MANIFEST_DIR"),
            &format!("scenes/{}.ron", settings.start_scene),
        );
        let (live, problems) = LiveScene::open(&scene).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let mut live = live.with_components(game_components());
        let mut world = World::new();
        let spawned = live.spawn_headless(&mut world);
        assert!(spawned.lines().is_empty(), "{:?}", spawned.lines());

        let seconds = settings.fixed_delta();
        let mut physics = PhysicsWorld::new(seconds);
        let mut profile = scrap::perf::Profiler::new(8);
        for _ in 0..(2.0 / seconds) as usize {
            tick(&mut world, &mut physics, &mut profile, seconds);
        }
        for (_, transform) in world.query::<(scrap::hecs::Entity, &scrap::Transform)>().iter() {
            assert!(transform.position.is_finite(), "something flew off: {transform:?}");
        }
    }
}
