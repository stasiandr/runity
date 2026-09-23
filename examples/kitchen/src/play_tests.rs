//! The kitchen played without a window: the rules alone, one player at two
//! cooks, and a host and a guest over a loopback — each a world from the
//! scene, a party, and the same frame the window runs (`session::frame`),
//! with prefabs spawned headless.

use runity::glam::Vec3;
use runity::hecs::{Entity, World};
use runity::net::{Loopback, Transport};
use runity::party::{Event, Party};
use runity::physics::PhysicsWorld;
use runity::{LiveScene, Transform};

use crate::components::item::{Food, Thing};
use crate::components::Item;
use crate::state::*;
use crate::{game_components, session, tick};

/// Tiles of the kitchen by what is there (see scenes/main.ron).
const TOMATOES: (f32, f32) = (-3.0, -3.0);
const ONIONS: (f32, f32) = (-5.0, 2.0);
const BOARD: (f32, f32) = (-5.0, -2.0);
const STOVE: (f32, f32) = (1.0, -3.0);
const PLATES: (f32, f32) = (5.0, -1.0);
const WINDOW: (f32, f32) = (-2.0, 3.0);
const BIN: (f32, f32) = (5.0, 2.0);

/// One peer: its world from the scene, its party, and what runs a frame.
struct Peer {
    live: LiveScene,
    world: World,
    party: Party,
    physics: PhysicsWorld,
    profile: runity::perf::Profiler,
    events: Vec<Event>,
    motions: runity::motion::Motions,
    noise: crate::noise::Noise,
    /// A GPU to draw with, for the screenshots; `None` plays headless.
    render: Option<Render>,
}

/// What draws a peer's frames without a window.
struct Render {
    gpu: runity::Gpu,
    target: runity::OffscreenTarget,
    renderer: runity::Renderer,
    overlay: runity::ui_render::UiRenderer,
}

impl Peer {
    fn new(party: Party) -> Self {
        Self::with(party, None)
    }

    fn with(party: Party, mut render: Option<Render>) -> Self {
        let scene = runity::project::data_file(env!("CARGO_MANIFEST_DIR"), "scenes/main.ron");
        let (live, problems) = LiveScene::open(&scene).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let mut live = live.with_components(game_components());
        let mut world = World::new();
        match render.as_mut() {
            Some(r) => {
                live.spawn(&mut world, &r.gpu, &mut r.renderer);
            }
            None => {
                live.spawn_headless(&mut world);
            }
        }
        Self {
            live,
            world,
            party,
            physics: PhysicsWorld::new(1.0 / 60.0),
            profile: runity::perf::Profiler::new(8),
            events: Vec::new(),
            motions: runity::motion::Motions::load(env!("CARGO_MANIFEST_DIR")).0,
            noise: crate::noise::Noise::default(),
            render,
        }
    }

    fn alone() -> Self {
        Self::new(Party::alone("main", &game_components()))
    }

    /// A frame of a thirtieth of a second: the party, the kitchen's work
    /// around it, and two steps of the rules.
    fn frame(&mut self) {
        let (live, render) = (&mut self.live, &mut self.render);
        let mut make = |w: &mut World, prefab: &str, t: Transform| match render.as_mut() {
            Some(r) => live.spawn_prefab(prefab, t, None, w, &r.gpu, &mut r.renderer).ok().map(|i| i.root),
            None => live.spawn_prefab_headless(prefab, t, None, w).ok().map(|i| i.root),
        };
        let events = self.party.update(&mut self.world, &game_components(), 1.0 / 30.0, &mut make);
        session::act(&mut self.world, &self.party, &events);
        self.events.extend(events);
        session::frame(&mut self.world, &mut self.party, |w, prefab| make(w, prefab, Transform::default()));
        // What the window does after the party: the graphs, the sounds.
        let problems = runity::motion::attach(&mut self.world, &self.motions, |_| None);
        assert!(problems.is_empty(), "{problems:?}");
        if self.noise.listen(&self.world).contains(&"ding") {
            crate::front::ring(&mut self.world);
        }
        for _ in 0..2 {
            tick(&mut self.world, &mut self.physics, &mut self.profile, 1.0 / 60.0);
        }
    }

    fn seconds(&mut self, seconds: f32) {
        for _ in 0..(seconds * 30.0).round().max(1.0) as usize {
            self.frame();
        }
    }

    fn cook(&self, index: u32) -> Entity {
        cook(&self.world, index).unwrap()
    }

    /// Stand cook `index` beside a tile, on the side `from`, facing it.
    fn stand(&mut self, index: u32, tile: (f32, f32), from: Vec3) {
        let c = self.cook(index);
        let mut t = self.world.get::<&mut Transform>(c).unwrap();
        t.position = Vec3::new(tile.0, 0.45, tile.1) + from;
        t.rotation_deg.y = (-from.x).atan2(-from.z).to_degrees();
    }

    fn act(&mut self, act: Act) {
        self.party.publish(ACT, &act);
        self.seconds(0.1);
    }

    fn grab(&mut self, index: u32) {
        self.act(Act::Grab { cook: index });
    }

    fn work(&mut self, index: u32, seconds: f32) {
        self.act(Act::Work { cook: index, on: true });
        self.seconds(seconds);
        self.act(Act::Work { cook: index, on: false });
    }

    fn round(&self) -> Round {
        self.world.query::<&Round>().iter().next().cloned().unwrap()
    }

    fn held(&self, index: u32) -> Option<Thing> {
        let item = self.world.get::<&Hands>(self.cook(index)).ok()?.0?;
        self.world.get::<&Item>(item).ok().map(|i| i.thing)
    }

    fn pot(&self) -> Pot {
        let stove = station_at(&self.world, STOVE);
        self.world.get::<&Pot>(stove).map(|p| (*p).clone()).unwrap_or_default()
    }

    fn want(&mut self, food: Food) {
        let kitchen = kitchen(&self.world).unwrap().0;
        self.world.get::<&mut Round>(kitchen).unwrap().orders.push(Order {
            food,
            left: 60.0,
            total: 60.0,
        });
    }

    /// A chopped food of `food` into the pot, by cook `index`.
    fn chop_into_pot(&mut self, index: u32, food: Food) {
        let from = if food == Food::Tomato { (TOMATOES, Vec3::Z) } else { (ONIONS, Vec3::X) };
        self.stand(index, from.0, from.1);
        self.grab(index);
        assert_eq!(self.held(index), Some(Thing::Food(food)), "{:?}", self.round().note);
        self.stand(index, BOARD, Vec3::X);
        self.grab(index);
        self.work(index, CHOP_SECONDS + 0.2);
        self.grab(index);
        self.stand(index, STOVE, Vec3::Z);
        self.grab(index);
    }

    /// Whether the mark `name` on the station at `tile` shows.
    fn mark_on(&self, tile: (f32, f32), name: &str) -> bool {
        let station = station_at(&self.world, tile);
        marked(&self.world, station)
            .into_iter()
            .find(|(_, n)| n == name)
            .is_some_and(|(e, _)| self.world.get::<&runity::world::Inactive>(e).is_err())
    }

    /// How loud the stove's pot boils.
    fn boiling(&self) -> f32 {
        let stove = station_at(&self.world, STOVE);
        self.world.get::<&runity::world::Sounding>(stove).map_or(0.0, |s| s.0.volume)
    }

    /// A plate from the stack, filled at the stove.
    fn plate_up(&mut self, index: u32) {
        self.stand(index, PLATES, -Vec3::X);
        self.grab(index);
        assert_eq!(self.held(index), Some(Thing::Plate));
        self.stand(index, STOVE, Vec3::Z);
        self.grab(index);
    }
}

fn station_at(world: &World, tile: (f32, f32)) -> Entity {
    world
        .query::<(Entity, &Transform, &crate::components::Station)>()
        .iter()
        .find(|(_, t, _)| (t.position.x - tile.0).abs() < 0.1 && (t.position.z - tile.1).abs() < 0.1)
        .map(|(e, ..)| e)
        .unwrap_or_else(|| panic!("no station at {tile:?}"))
}

#[test]
fn alone_one_keyboard_plays_the_first_two_cooks() {
    let mut k = Peer::alone();
    k.seconds(0.5);
    let me = k.party.me().0;
    let seats: Vec<Option<u32>> = (0..4)
        .map(|i| k.world.get::<&Seat>(k.cook(i)).ok().map(|s| s.0))
        .collect();
    assert_eq!(seats, [Some(me), Some(me), None, None]);
    let off = |k: &Peer, i| k.world.get::<&runity::world::Inactive>(k.cook(i)).is_ok();
    assert!(!off(&k, 0) && !off(&k, 1) && off(&k, 2) && off(&k, 3), "the two nobody plays are out");
    assert_eq!(crate::front::local_cooks(&k.world, &k.party).len(), 2);
}

#[test]
fn a_tomato_soup_goes_from_the_crate_to_the_window() {
    let mut k = Peer::alone();
    k.seconds(0.2);
    k.want(Food::Tomato);
    for _ in 0..3 {
        k.chop_into_pot(0, Food::Tomato);
    }
    assert_eq!(k.pot().foods, [Food::Tomato; 3], "{:?}", k.round().note);
    k.seconds(COOK_SECONDS + 0.5);
    assert!(k.pot().done());
    assert!(k.mark_on(STOVE, "steam") && !k.mark_on(STOVE, "smoke"), "a done soup steams");
    assert!(k.boiling() > 0.5, "and boils loudly");
    let mut noise = crate::noise::Noise::default();
    noise.listen(&k.world);
    k.plate_up(0);
    assert!(k.pot().foods.is_empty(), "the soup is on the plate");
    k.stand(0, WINDOW, -Vec3::Z);
    k.grab(0);
    assert!(noise.listen(&k.world).contains(&"ding"), "the bell for a soup served");
    // And the bell over the window hops.
    let heights: Vec<f32> = k
        .world
        .query::<(&Transform, &runity::world::Parent)>()
        .iter()
        .filter(|(t, _)| t.position.y > 1.3 && t.position.y < 1.5 && t.position.x == 0.5)
        .map(|(t, _)| t.position.y)
        .collect();
    assert!(!heights.is_empty(), "the bell is up, ringing");
    let round = k.round();
    assert_eq!(round.served, 1, "{:?}", round.note);
    assert!(round.score >= 20, "{}", round.score);
    assert_eq!(k.held(0), None);
}

#[test]
fn raw_food_does_not_go_in_the_pot_and_mixed_soup_is_nobodys() {
    let mut k = Peer::alone();
    k.seconds(0.2);
    k.stand(0, TOMATOES, Vec3::Z);
    k.grab(0);
    k.stand(0, STOVE, Vec3::Z);
    k.grab(0);
    assert!(k.pot().foods.is_empty());
    assert_eq!(k.round().note.map(|n| n.0).as_deref(), Some("Chop it first"));
    // Into the bin with it.
    k.stand(0, BIN, -Vec3::X);
    k.grab(0);
    assert_eq!(k.held(0), None);

    k.chop_into_pot(0, Food::Tomato);
    k.chop_into_pot(1, Food::Onion);
    k.chop_into_pot(0, Food::Tomato);
    assert_eq!(k.pot().soup(), Some(Soup::Mixed));
    k.seconds(COOK_SECONDS + 0.5);
    k.plate_up(1);
    let before = k.round().score;
    k.stand(1, WINDOW, -Vec3::Z);
    k.grab(1);
    assert_eq!(k.round().score, before - 5, "nobody ordered that");
}

#[test]
fn a_pot_left_too_long_burns_and_is_scraped() {
    let mut k = Peer::alone();
    k.seconds(0.2);
    for _ in 0..3 {
        k.chop_into_pot(0, Food::Onion);
    }
    k.seconds(BURN_SECONDS + 0.5);
    assert!(k.pot().burnt());
    assert!(k.mark_on(STOVE, "smoke") && k.mark_on(STOVE, "soup burnt"), "a burnt pot smokes");
    k.plate_up(1);
    assert!(k.pot().burnt(), "a burnt pot fills no plate");
    k.stand(0, STOVE, Vec3::Z);
    k.work(0, 0.2);
    assert!(k.pot().foods.is_empty(), "scraped clean");
}

#[test]
fn orders_come_in_walk_out_and_the_round_ends_and_starts_again() {
    let mut k = Peer::alone();
    let kitchen = kitchen(&k.world).map(|(e, _)| e).unwrap();
    k.seconds(2.0);
    assert!(!k.round().orders.is_empty(), "the first order is in");
    // Make the first order impatient.
    k.world.get::<&mut Round>(kitchen).unwrap().orders[0].left = 0.1;
    let before = k.round().score;
    k.seconds(0.3);
    assert_eq!(k.round().score, before - 10, "it walked out");
    // Food about, then the clock runs out.
    k.stand(0, TOMATOES, Vec3::Z);
    k.grab(0);
    k.world.get::<&mut Round>(kitchen).unwrap().time_left = 0.05;
    k.seconds(0.2);
    assert!(k.round().over);
    k.grab(1);
    assert_eq!(k.held(1), None, "nothing is done once time is up");
    k.act(Act::Restart);
    let round = k.round();
    assert!(!round.over && round.score == 0 && round.time_left > 100.0);
    let items = k.world.query::<&Item>().iter().count();
    assert_eq!(items, 0, "the kitchen is cleared");
}

/// A host and a guest over a loopback.
fn together() -> (Peer, Peer) {
    let components = game_components();
    let mut ends = Loopback::network(2).into_iter();
    let listener: Box<dyn Transport + Send> = Box::new(ends.next().unwrap());
    let host = Peer::new(Party::host("main", "host", &components, vec![listener], false));
    let guest = Peer::new(Party::join(ends.next().unwrap(), "main", "guest", &components));
    let (mut host, mut guest) = (host, guest);
    for _ in 0..60 {
        host.frame();
        guest.frame();
        std::thread::sleep(std::time::Duration::from_millis(1));
        if guest.party.welcomed() && owns(&guest, 1) {
            break;
        }
    }
    (host, guest)
}

fn owns(peer: &Peer, index: u32) -> bool {
    peer.world.get::<&runity::net::Owned>(peer.cook(index)).is_ok()
}

/// Frames on both until `done`.
fn until(host: &mut Peer, guest: &mut Peer, done: impl Fn(&Peer, &Peer) -> bool) {
    for _ in 0..200 {
        if done(host, guest) {
            return;
        }
        host.frame();
        guest.frame();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(done(host, guest), "not after 200 frames");
}

#[test]
fn a_guest_is_seated_at_the_second_cook_and_walks_it() {
    let (mut host, mut guest) = together();
    assert!(guest.party.welcomed());
    let me = guest.party.me().0;
    assert_eq!(host.world.get::<&Seat>(host.cook(1)).unwrap().0, me);
    assert!(owns(&guest, 1), "the guest drives its own cook");
    assert!(!owns(&guest, 0) && owns(&host, 0));
    assert_eq!(crate::front::local_cooks(&guest.world, &guest.party).len(), 1);
    // The guest walks; the host sees it arrive.
    let c = guest.cook(1);
    guest.world.insert_one(c, Controls { x: 1.0, ..Default::default() }).unwrap();
    guest.seconds(0.5);
    let there = guest.world.get::<&Transform>(c).unwrap().position;
    let hc = host.cook(1);
    until(&mut host, &mut guest, |h, _| {
        (h.world.get::<&Transform>(hc).unwrap().position - there).length() < 0.2
    });
    // Nobody runs the rules but the host.
    assert!(kitchen(&host.world).unwrap().1 && !kitchen(&guest.world).unwrap().1);
}

#[test]
fn what_a_guests_hands_do_the_host_does_and_everyone_sees() {
    let (mut host, mut guest) = together();
    // The round reaches the guest.
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some());
    // The guest at the tomatoes, grabbing: the host spawns one into its
    // hands, and the guest's world has it too.
    guest.stand(1, TOMATOES, Vec3::Z);
    until(&mut host, &mut guest, |h, g| {
        let hc = h.cook(1);
        let gc = g.cook(1);
        let a = h.world.get::<&Transform>(hc).unwrap().position;
        let b = g.world.get::<&Transform>(gc).unwrap().position;
        (a - b).length() < 0.05
    });
    guest.grab(1);
    until(&mut host, &mut guest, |h, g| {
        h.held(1) == Some(Thing::Food(Food::Tomato)) && g.world.query::<&Item>().iter().count() == 1
    });
    // Chopped on the host, the guest sees the chopping.
    guest.stand(1, BOARD, Vec3::X);
    until(&mut host, &mut guest, |h, _| {
        let at = h.world.get::<&Transform>(h.cook(1)).unwrap().position;
        (at - Vec3::new(-4.0, 0.45, -2.0)).length() < 0.05
    });
    guest.grab(1);
    guest.act(Act::Work { cook: 1, on: true });
    until(&mut host, &mut guest, |_, g| {
        g.world.query::<&Chop>().iter().any(|c| c.0 >= 1.0)
    });
    guest.act(Act::Work { cook: 1, on: false });
    let score = host.round().score;
    until(&mut host, &mut guest, |_, g| g.round().score == score);
}

#[test]
fn the_screens_load_and_every_word_is_in_both_languages() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let front = crate::front::Front::load(&root.join("ui")).unwrap();
    // The ids the code asks for are in the files.
    let ids = |screen: &runity::screen::Screen| -> Vec<String> {
        screen.layout().elements.iter().map(|e| e.id.clone()).collect()
    };
    for id in ["local", "host", "join", "address", "music", "sfx", "language", "quit", "status", "best"] {
        assert!(ids(&front.menu).contains(&id.to_string()), "menu has no `{id}`");
    }
    for id in ["clock", "score", "players", "note", "keys", "leave"] {
        assert!(ids(&front.hud).contains(&id.to_string()), "hud has no `{id}`");
    }
    for id in ["score", "served", "best", "again", "menu", "wait"] {
        assert!(ids(&front.results).contains(&id.to_string()), "results have no `{id}`");
    }
    // Every @key the screens, the code and the chef use, in English and
    // Russian.
    let mut keys: Vec<String> = [&front.menu, &front.hud, &front.results]
        .iter()
        .flat_map(|s| runity::screen::Screen::keys(s.layout()))
        .collect();
    let code = std::fs::read_to_string(root.join("src/front.rs")).unwrap();
    for piece in code.split("\"@").skip(1) {
        keys.push(piece.split('"').next().unwrap().to_string());
    }
    let chef = runity::dialogue::Dialogue::load(root.join("dialogues/chef.ron")).unwrap();
    assert!(chef.problems().is_empty(), "{:?}", chef.problems());
    keys.extend(chef.keys());
    for language in crate::front::LANGUAGES {
        let strings = runity::strings::Strings::load(root.join("strings"), language).unwrap();
        for key in &keys {
            assert_ne!(strings.get(key), key.as_str(), "{language} lacks `{key}`");
        }
    }
}

#[test]
fn the_head_chef_says_every_line_and_stops() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut front = crate::front::Front::load(&root.join("ui")).unwrap();
    assert!(front.line().is_none());
    front.brief();
    let mut said = Vec::new();
    for _ in 0..200 {
        if let Some((line, speaker)) = front.line() {
            assert_eq!(speaker, "@chef.name");
            if said.last() != Some(&line) {
                said.push(line);
            }
        }
        front.listen(0.1);
    }
    assert_eq!(said, ["@chef.hello", "@chef.soup", "@chef.pots", "@chef.go"]);
    assert!(front.line().is_none(), "and then the kitchen is theirs");
}

#[test]
fn the_board_on_the_wall_shows_the_orders_on_every_peer() {
    let (mut host, mut guest) = together();
    let kitchen_e = kitchen(&host.world).unwrap().0;
    host.world.get::<&mut Round>(kitchen_e).unwrap().orders = vec![
        Order { food: Food::Tomato, left: 30.0, total: 60.0 },
        Order { food: Food::Onion, left: 60.0, total: 60.0 },
    ];
    let words = |peer: &Peer| -> Vec<String> {
        peer.world
            .query::<&runity::world::WorldUi>()
            .iter()
            .flat_map(|w| w.ui.texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>())
            .collect()
    };
    until(&mut host, &mut guest, |h, g| {
        [h, g].iter().all(|p| {
            let w = words(p);
            w.contains(&"TOMATO".to_string()) && w.contains(&"ONION".to_string())
        })
    });
}

impl Render {
    /// A GPU without a window, if the machine has one.
    fn new(width: u32, height: u32) -> Option<Self> {
        let gpu = runity::Gpu::headless_blocking(false).ok()?;
        let target = runity::OffscreenTarget::new(&gpu, width, height);
        let renderer = runity::Renderer::new(&gpu, &target);
        let overlay = runity::ui_render::UiRenderer::new(&gpu, &target);
        Some(Self {
            gpu,
            target,
            renderer,
            overlay,
        })
    }
}

impl Peer {
    /// What the window would show now — the kitchen from the scene's view,
    /// the board on the wall, and the screens over it — as RGBA, and as a
    /// PNG in target/shots/ to look at.
    fn shot(&mut self, front: &mut crate::front::Front, name: &str) -> Vec<u8> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let r = self.render.as_mut().expect("a peer that draws");
        let scene = self.live.scene().clone();
        let camera = runity::scene_camera(&scene.view);
        let frame = runity::world::scene_frame(&self.world, camera, &scene);
        r.overlay.draw_pictures(&r.gpu, &mut r.renderer, &frame);
        r.renderer.render(&r.gpu, &r.target, &frame);
        let mut ui = runity::ui::Ui::new();
        let mut widgets = runity::widgets::Widgets::new();
        let strings = runity::strings::Strings::load(root.join("strings"), "en").unwrap();
        let size = runity::glam::Vec2::new(r.target.width as f32, r.target.height as f32);
        front.draw(
            &self.world,
            &self.party,
            &mut widgets,
            &mut ui,
            &runity::input::Input::default(),
            size,
            &strings,
        );
        r.overlay.render(&r.gpu, &r.target, &ui);
        let pixels = r.target.read_rgba(&r.gpu);
        let dir = root.join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        image::save_buffer(
            dir.join(format!("{name}.png")),
            &pixels,
            r.target.width,
            r.target.height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        pixels
    }
}

/// How different a picture's pixels are: none for a blank one.
fn spread(pixels: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for px in pixels.chunks(4).step_by(97) {
        seen.insert([px[0] / 16, px[1] / 16, px[2] / 16]);
    }
    seen.len()
}

#[test]
fn a_screenshot_of_the_menu_and_of_a_round_in_full_swing() {
    let Some(render) = Render::new(1280, 720) else {
        eprintln!("skipping: no GPU");
        return;
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut front = crate::front::Front::load(&root.join("ui")).unwrap();
    let mut k = Peer::with(Party::alone("main", &game_components()), Some(render));
    k.seconds(0.2);
    let menu = k.shot(&mut front, "menu");
    assert!(spread(&menu) > 20, "the menu over the kitchen");

    // A round going: soup done and steaming, a plate of it in hand, an
    // onion half chopped, a tomato carried, the chef talking.
    front.phase = crate::front::Phase::Kitchen;
    front.brief();
    k.want(Food::Tomato);
    k.want(Food::Onion);
    for _ in 0..3 {
        k.chop_into_pot(0, Food::Tomato);
    }
    k.seconds(COOK_SECONDS + 0.5);
    k.plate_up(1);
    k.stand(1, (3.0, 0.0), Vec3::ZERO);
    k.chop_into_pot(0, Food::Tomato);
    k.stand(0, ONIONS, Vec3::X);
    k.grab(0);
    k.stand(0, BOARD, Vec3::X);
    k.grab(0);
    k.work(0, 0.5);
    k.stand(0, TOMATOES, Vec3::Z);
    k.grab(0);
    let round = k.shot(&mut front, "round");
    assert!(spread(&round) > 20);
    // The board on the wall has the orders on it.
    let words: Vec<String> = k
        .world
        .query::<&runity::world::WorldUi>()
        .iter()
        .flat_map(|w| w.ui.texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>())
        .collect();
    assert!(words.contains(&"ONION".to_string()), "{words:?}");

    // Time up: the results over the kitchen.
    let kitchen_e = kitchen(&k.world).unwrap().0;
    k.world.get::<&mut Round>(kitchen_e).unwrap().time_left = 0.05;
    k.seconds(0.2);
    front.new_best = true;
    let results = k.shot(&mut front, "results");
    assert!(spread(&results) > 20);
}

#[test]
fn a_guest_who_leaves_takes_their_cook_out_and_the_host_keeps_it() {
    let (mut host, mut guest) = together();
    let hc = host.cook(1);
    assert!(host.world.get::<&Seat>(hc).is_ok());
    // With a tomato in hand.
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some());
    guest.stand(1, TOMATOES, Vec3::Z);
    until(&mut host, &mut guest, |h, g| {
        let a = h.world.get::<&Transform>(h.cook(1)).unwrap().position;
        let b = g.world.get::<&Transform>(g.cook(1)).unwrap().position;
        (a - b).length() < 0.05
    });
    guest.grab(1);
    until(&mut host, &mut guest, |h, _| h.held(1).is_some());
    // Gone, saying so.
    drop(guest);
    for _ in 0..120 {
        host.frame();
        std::thread::sleep(std::time::Duration::from_millis(1));
        if host.world.get::<&Seat>(hc).is_err() && owns(&host, 1) {
            break;
        }
    }
    assert!(host.world.get::<&Seat>(hc).is_err(), "nobody plays the second cook");
    assert!(owns(&host, 1), "the host has it back");
    assert!(
        host.world.get::<&runity::world::Inactive>(hc).is_ok(),
        "and it is out of the kitchen"
    );
    assert!(host.events.iter().any(|e| matches!(e, Event::Left { clean: true, .. })));
    assert_eq!(host.held(1), None, "what they held is down");
    let item = host.world.query::<(Entity, &Item)>().iter().next().map(|(e, _)| e).unwrap();
    assert!(host.world.get::<&At>(item).is_err(), "loose on the floor, for anyone");
}

#[test]
fn only_the_host_starts_the_next_round() {
    let (mut host, mut guest) = together();
    let kitchen_e = kitchen(&host.world).unwrap().0;
    host.world.get::<&mut Round>(kitchen_e).unwrap().time_left = 0.05;
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some_and(|r| r.over));
    // A guest asking changes nothing: acts are the host's to carry out, and
    // the results screen gives only the host the button.
    guest.act(Act::Restart);
    host.seconds(0.2);
    assert!(host.round().over);
    host.act(Act::Restart);
    until(&mut host, &mut guest, |h, g| !h.round().over && !g.round().over);
}

#[test]
fn a_guest_cannot_work_someone_elses_cook() {
    let (mut host, mut guest) = together();
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some());
    host.stand(0, TOMATOES, Vec3::Z);
    guest.grab(0);
    host.seconds(0.3);
    assert_eq!(host.held(0), None, "the host's cook is the host's");
}

#[test]
fn thrown_food_flies_and_lands_on_a_counter_in_a_pot_or_on_the_floor() {
    let mut k = Peer::alone();
    k.seconds(0.2);
    // From the tomatoes towards the island: it comes down on the floor or a
    // counter, and lies still.
    k.stand(0, TOMATOES, Vec3::Z);
    k.grab(0);
    // Facing the room.
    k.world.get::<&mut Transform>(k.cook(0)).unwrap().rotation_deg.y = 0.0;
    k.act(Act::Throw { cook: 0 });
    assert_eq!(k.held(0), None, "out of the hands");
    let flying = |k: &Peer| k.world.query::<&Flying>().iter().count();
    assert_eq!(flying(&k), 1, "in the air");
    let start = k.world.query::<(&Transform, &Item)>().iter().next().unwrap().0.position;
    k.seconds(3.0);
    assert_eq!(flying(&k), 0, "landed");
    let (t, _) = k.world.query::<(&Transform, &Item)>().iter().next().map(|(t, i)| (*t, *i)).unwrap();
    assert!((t.position - start).length() > 1.0, "it went somewhere: {start} → {}", t.position);
    assert!(
        t.position.y > 0.15 && t.position.y < 1.3,
        "on the floor or a counter, not in the air or under it: {}",
        t.position
    );

    // Chopped food thrown at a stove from beside it goes in the pot.
    k.stand(0, ONIONS, Vec3::X);
    k.grab(0);
    k.stand(0, BOARD, Vec3::X);
    k.grab(0);
    k.work(0, CHOP_SECONDS + 0.2);
    k.grab(0);
    // A lob from right before the stove, straight up and in.
    k.stand(0, STOVE, Vec3::Z * 0.9);
    let c = k.cook(0);
    k.world.get::<&mut Transform>(c).unwrap().rotation_deg.y = 180.0;
    k.act(Act::Throw { cook: 0 });
    k.seconds(3.0);
    assert_eq!(k.pot().foods, [Food::Onion], "in the pot");
    // What lies on the floor is picked up again.
    let (t, _) = k.world.query::<(&Transform, &Item)>().iter().next().map(|(t, i)| (*t, *i)).unwrap();
    let c = k.cook(0);
    {
        let mut ct = k.world.get::<&mut Transform>(c).unwrap();
        ct.position = Vec3::new(t.position.x, 0.45, t.position.z + 0.7);
        ct.rotation_deg.y = 180.0;
    }
    k.grab(0);
    assert_eq!(k.held(0), Some(Thing::Food(Food::Tomato)), "back in hand from where it lay");
}

#[test]
fn what_a_guest_throws_flies_on_the_hosts_physics_and_everyone_sees_it() {
    let (mut host, mut guest) = together();
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some());
    guest.stand(1, TOMATOES, Vec3::Z);
    until(&mut host, &mut guest, |h, g| {
        let a = h.world.get::<&Transform>(h.cook(1)).unwrap().position;
        let b = g.world.get::<&Transform>(g.cook(1)).unwrap().position;
        (a - b).length() < 0.05
    });
    guest.grab(1);
    until(&mut host, &mut guest, |_, g| g.world.query::<&Item>().iter().count() == 1);
    // Turn round to the room, and throw.
    guest.world.get::<&mut Transform>(guest.cook(1)).unwrap().rotation_deg.y = 0.0;
    guest.seconds(0.2);
    host.seconds(0.2);
    guest.act(Act::Throw { cook: 1 });
    until(&mut host, &mut guest, |h, _| h.world.query::<&Flying>().iter().count() == 1);
    let spot = |p: &Peer| p.world.query::<(&Transform, &Item)>().iter().next().map(|(t, _)| t.position).unwrap();
    let before = spot(&guest);
    until(&mut host, &mut guest, |_, g| (spot(g) - before).length() > 1.0);
}

/// Everything in its one place: every item held by at most one cook or on
/// at most one station — and they agree — no pot past three, no plate
/// soup and pot soup from nowhere.
fn consistent(k: &Peer) -> Result<(), String> {
    let mut places: std::collections::HashMap<Entity, Vec<String>> = Default::default();
    for (cook, hands) in k.world.query::<(Entity, &Hands)>().iter() {
        if let Some(item) = hands.0 {
            places.entry(item).or_default().push(format!("hand {cook:?}"));
            match k.world.get::<&At>(item).map(|a| a.0) {
                Ok(Place::Hand(c)) if c == cook => {}
                other => return Err(format!("{cook:?} holds {item:?}, which is at {other:?}")),
            }
        }
    }
    for (station, top) in k.world.query::<(Entity, &Top)>().iter() {
        if let Some(item) = top.0 {
            places.entry(item).or_default().push(format!("on {station:?}"));
            match k.world.get::<&At>(item).map(|a| a.0) {
                Ok(Place::On(s)) if s == station => {}
                other => return Err(format!("{station:?} has {item:?}, which is at {other:?}")),
            }
            if k.world.get::<&Item>(item).is_err() {
                return Err(format!("{station:?} has {item:?}, which is gone"));
            }
        }
    }
    if let Some((item, at)) = places.iter().find(|(_, at)| at.len() > 1) {
        return Err(format!("{item:?} is in two places: {at:?}"));
    }
    for pot in k.world.query::<&Pot>().iter() {
        if pot.foods.len() > POT_HOLDS {
            return Err(format!("a pot of {}", pot.foods.len()));
        }
    }
    Ok(())
}

#[test]
fn two_cooks_doing_anything_for_half_a_minute_keep_the_kitchen_consistent() {
    let mut k = Peer::alone();
    k.seconds(0.2);
    let mut seed = 12_345u32;
    let mut next = |n: u32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 16) % n
    };
    let tiles = [TOMATOES, ONIONS, BOARD, STOVE, PLATES, WINDOW, BIN, (-5.0, 0.0), (0.0, 0.0)];
    for step in 0..300 {
        let index = next(2);
        match next(6) {
            0 | 1 => {
                // Walk somewhere for a moment.
                let c = k.cook(index);
                let (x, z) = (next(3) as f32 - 1.0, next(3) as f32 - 1.0);
                k.world.insert_one(c, Controls { x, z, ..Default::default() }).unwrap();
                k.seconds(0.2);
                k.world.insert_one(c, Controls::default()).unwrap();
            }
            2 => {
                let tile = tiles[next(tiles.len() as u32) as usize];
                let from = [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z][next(4) as usize];
                k.stand(index, tile, from);
                k.grab(index);
            }
            3 => k.work(index, 0.3 + next(10) as f32 * 0.1),
            4 => k.act(Act::Throw { cook: index }),
            _ => k.grab(index),
        }
        if step % 5 == 0 {
            if let Err(e) = consistent(&k) {
                panic!("after step {step}: {e}");
            }
        }
    }
    k.seconds(3.0);
    consistent(&k).unwrap();
}

#[test]
fn a_guest_sees_what_its_cook_holds_in_its_hands_at_once() {
    let (mut host, mut guest) = together();
    until(&mut host, &mut guest, |_, g| g.world.query::<&Round>().iter().next().is_some());
    guest.stand(1, TOMATOES, Vec3::Z);
    until(&mut host, &mut guest, |h, g| {
        let a = h.world.get::<&Transform>(h.cook(1)).unwrap().position;
        let b = g.world.get::<&Transform>(g.cook(1)).unwrap().position;
        (a - b).length() < 0.05
    });
    guest.grab(1);
    until(&mut host, &mut guest, |_, g| g.world.query::<&crate::components::HeldBy>().iter().count() == 1);
    // The guest walks off; the tomato goes with its cook on the guest's
    // screen at once, before the host has heard.
    let c = guest.cook(1);
    guest.world.get::<&mut Transform>(c).unwrap().position += Vec3::new(1.5, 0.0, 1.5);
    guest.frame();
    let cook_at = guest.world.get::<&Transform>(c).unwrap().position;
    let item_at = guest.world.query::<(&Transform, &Item)>().iter().next().unwrap().0.position;
    let flat = Vec3::new(item_at.x - cook_at.x, 0.0, item_at.z - cook_at.z).length();
    assert!(flat < 0.7, "in hand: {cook_at} / {item_at}");
}
